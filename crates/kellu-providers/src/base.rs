//! The provider interface.

use std::sync::Arc;

use kellu_common::ProviderError;
use secrecy::SecretString;

use crate::types::{ChatRequest, ChatResponse, StreamEvent};

/// Maps a keyring name (for example `openrouter`) to its secret, or `None` when it is missing.
///
/// The real implementation is the keyring vault (milestone M3); tests inject a fake. Keys are
/// looked up per request and never stored by the providers beyond the request.
pub type KeyLookup = Arc<dyn Fn(&str) -> Option<SecretString> + Send + Sync>;

/// A streamed answer. Nothing is sent until the first item is requested, and every failure,
/// including an HTTP error status, arrives as an `Err` item. A successful stream ends with
/// exactly one [`StreamEvent::Done`].
pub type EventStream = Box<dyn Iterator<Item = Result<StreamEvent, ProviderError>> + Send>;

/// A model provider. Implementations never log or return messages containing secrets.
pub trait Provider: Send + Sync {
    /// Name from the configuration.
    fn name(&self) -> &str;

    /// Whether the provider can do tool calling.
    fn supports_tools(&self) -> bool;

    /// Send `request` and return the complete answer.
    ///
    /// # Errors
    /// A [`ProviderError`] whose `retryable` flag tells the router whether another attempt or
    /// provider may succeed.
    fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError>;

    /// Send `request` and return text deltas followed by one final `Done` event.
    fn stream(&self, request: &ChatRequest) -> EventStream;
}

/// A stream that fails immediately with `error` (for example, a missing key).
pub(crate) fn failed_stream(error: ProviderError) -> EventStream {
    Box::new(std::iter::once(Err(error)))
}
