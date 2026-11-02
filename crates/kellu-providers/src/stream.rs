//! Lazy server-sent-event streams shared by the provider clients.

use kellu_common::ProviderError;
use serde_json::Value;

use crate::http::{Credential, SseEvents, Transport};
use crate::types::{ChatResponse, StreamEvent};

/// What one SSE event contributed to the answer.
pub(crate) enum Feed {
    /// Nothing visible (metadata, empty delta).
    Skip,
    /// A piece of answer text.
    Text(String),
    /// The provider said the stream is over (`[DONE]`).
    Done,
}

/// Provider-specific interpretation of the events of one stream.
pub(crate) trait StreamParser: Send {
    /// Consume the `data` of one event.
    fn feed(&mut self, data: &str) -> Result<Feed, ProviderError>;
    /// Build the final response once the stream is over.
    fn finish(self) -> Result<ChatResponse, ProviderError>;
}

/// Everything needed to send the request when the stream is first polled.
pub(crate) struct Opening {
    pub(crate) transport: Transport,
    pub(crate) provider: String,
    pub(crate) url: String,
    pub(crate) headers: Vec<(&'static str, String)>,
    pub(crate) credential: Option<Credential>,
    pub(crate) payload: Value,
}

enum State<P> {
    Opening(Box<Opening>, P),
    Reading(Box<SseEvents>, P),
    Finished,
}

/// A stream that sends its request on the first `next()` and ends after the first error.
pub(crate) struct ProviderStream<P> {
    state: State<P>,
}

impl<P: StreamParser> ProviderStream<P> {
    pub(crate) fn new(opening: Opening, parser: P) -> Self {
        Self {
            state: State::Opening(Box::new(opening), parser),
        }
    }
}

fn done(parser: impl StreamParser) -> Option<Result<StreamEvent, ProviderError>> {
    Some(
        parser
            .finish()
            .map(|response| StreamEvent::Done(Box::new(response))),
    )
}

impl<P: StreamParser> Iterator for ProviderStream<P> {
    type Item = Result<StreamEvent, ProviderError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match std::mem::replace(&mut self.state, State::Finished) {
                State::Finished => return None,
                State::Opening(opening, parser) => {
                    let headers: Vec<(&str, &str)> = opening
                        .headers
                        .iter()
                        .map(|(name, value)| (*name, value.as_str()))
                        .collect();
                    match opening.transport.post_sse(
                        &opening.provider,
                        &opening.url,
                        &headers,
                        opening.credential.as_ref(),
                        &opening.payload,
                    ) {
                        Ok(events) => self.state = State::Reading(Box::new(events), parser),
                        Err(error) => return Some(Err(error)),
                    }
                }
                State::Reading(mut events, mut parser) => match events.next() {
                    None => return done(parser),
                    Some(Err(error)) => return Some(Err(error)),
                    Some(Ok(data)) => match parser.feed(&data) {
                        Err(error) => return Some(Err(error)),
                        Ok(Feed::Done) => return done(parser),
                        Ok(Feed::Skip) => self.state = State::Reading(events, parser),
                        Ok(Feed::Text(text)) => {
                            self.state = State::Reading(events, parser);
                            return Some(Ok(StreamEvent::Text(text)));
                        }
                    },
                },
            }
        }
    }
}
