//! A model provider that records what it is sent, and an `Assistant` built over it.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use kellu_common::ProviderError;
use kellu_core::assist::Assistant;
use kellu_core::config::{load_defaults, parse_config};
use kellu_core::router::{Router, monotonic_clock};
use kellu_providers::{ChatRequest, ChatResponse, EventStream, Message, Provider, StreamEvent};
use kellu_security::PatternRedactor;

const USER: &str = r#"
[providers.a]
kind = "openai_compat"
base_url = "https://a.example.com/v1"
model = "m"
timeout_s = 5
[routing.general]
primary = "a"
[routing.code]
primary = "a"
[routing.explain]
primary = "a"
"#;

#[derive(Clone)]
pub struct Probe {
    pub seen: Arc<Mutex<Vec<ChatRequest>>>,
    pub answer: Arc<Mutex<String>>,
    pub fail: Arc<AtomicBool>,
}

impl Probe {
    pub fn calls(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    /// Every text the provider was ever sent.
    pub fn everything_sent(&self) -> String {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .flat_map(|r| r.messages.iter())
            .map(|m| match m {
                Message::System(t) | Message::User(t) => t.clone(),
                Message::Assistant { content, .. } | Message::Tool { content, .. } => {
                    content.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("\n---\n")
    }

    pub fn say(&self, text: &str) {
        *self.answer.lock().unwrap() = text.to_owned();
    }
}

struct Fake(Probe);

fn reply(text: &str) -> ChatResponse {
    ChatResponse {
        text: text.to_owned(),
        provider: "a".to_owned(),
        model: None,
        tool_calls: Vec::new(),
        finish_reason: None,
        usage: None,
    }
}

impl Provider for Fake {
    fn name(&self) -> &str {
        "a"
    }
    fn supports_tools(&self) -> bool {
        false
    }
    fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.0.seen.lock().unwrap().push(request.clone());
        if self.0.fail.load(Ordering::SeqCst) {
            return Err(ProviderError::new("a", "boom"));
        }
        Ok(reply(&self.0.answer.lock().unwrap()))
    }
    fn stream(&self, request: &ChatRequest) -> EventStream {
        self.0.seen.lock().unwrap().push(request.clone());
        if self.0.fail.load(Ordering::SeqCst) {
            return Box::new(std::iter::once(Err(ProviderError::new("a", "boom"))));
        }
        let text = self.0.answer.lock().unwrap().clone();
        Box::new(
            vec![
                Ok(StreamEvent::Text(text.clone())),
                Ok(StreamEvent::Done(Box::new(reply(&text)))),
            ]
            .into_iter(),
        )
    }
}

pub fn assistant(answer: &str) -> (Assistant, Probe) {
    let user: toml::Table = USER.parse().unwrap();
    let config = parse_config(&load_defaults().unwrap(), Some(&user), None).unwrap();
    let probe = Probe {
        seen: Arc::new(Mutex::new(Vec::new())),
        answer: Arc::new(Mutex::new(answer.to_owned())),
        fail: Arc::new(AtomicBool::new(false)),
    };
    let provider: Box<dyn Provider> = Box::new(Fake(probe.clone()));
    let router = Router::new(
        config,
        BTreeMap::from([("a".to_owned(), provider)]),
        monotonic_clock(),
    );
    (
        Assistant::new(router, Arc::new(PatternRedactor::new())),
        probe,
    )
}
