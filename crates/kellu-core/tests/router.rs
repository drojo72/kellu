//! Router behavior with scripted fake providers.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use kellu_common::{KelluError, ProviderError};
use kellu_core::config::{Config, load_defaults, parse_config};
use kellu_core::router::{Clock, Router, TaskKind};
use kellu_providers::{
    ChatRequest, ChatResponse, EventStream, Message, Provider, StreamEvent, ToolSpec,
};
use secrecy::SecretString;
use serde_json::json;

const USER: &str = r#"
[providers.a]
kind = "openai_compat"
base_url = "https://a.example.com/v1"
model = "m"
timeout_s = 5
[providers.b]
kind = "openai_compat"
base_url = "https://b.example.com/v1"
model = "m"
timeout_s = 5
[providers.c]
kind = "openai_compat"
base_url = "https://c.example.com/v1"
model = "m"
timeout_s = 5
[routing.general]
primary = "a"
backup = ["b", "c"]
[routing.code]
primary = "c"
backup = ["a"]
[router]
total_timeout_s = 100
"#;

fn config() -> Config {
    let user: toml::Table = USER.parse().unwrap();
    parse_config(&load_defaults().unwrap(), Some(&user), None).unwrap()
}

type ChatOutcome = Result<ChatResponse, ProviderError>;
type StreamScript = Vec<Result<StreamEvent, ProviderError>>;

struct Fake {
    name: String,
    tools: bool,
    chats: Mutex<VecDeque<ChatOutcome>>,
    streams: Mutex<VecDeque<StreamScript>>,
    calls: Arc<AtomicUsize>,
    clock: Option<(Arc<Mutex<f64>>, f64)>,
}

impl Provider for Fake {
    fn name(&self) -> &str {
        &self.name
    }

    fn supports_tools(&self) -> bool {
        self.tools
    }

    fn chat(&self, _request: &ChatRequest) -> ChatOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some((time, step)) = &self.clock {
            *time.lock().unwrap() += step;
        }
        self.chats
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(ProviderError::new(&self.name, "unscripted")))
    }

    fn stream(&self, _request: &ChatRequest) -> EventStream {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some((time, step)) = &self.clock {
            *time.lock().unwrap() += step;
        }
        let script = self.streams.lock().unwrap().pop_front().unwrap_or_default();
        Box::new(script.into_iter())
    }
}

struct Harness {
    router: Router,
    calls: BTreeMap<String, Arc<AtomicUsize>>,
    time: Arc<Mutex<f64>>,
}

impl Harness {
    fn calls(&self, name: &str) -> usize {
        self.calls[name].load(Ordering::SeqCst)
    }
}

struct Spec {
    name: &'static str,
    tools: bool,
    chats: Vec<ChatOutcome>,
    streams: Vec<StreamScript>,
    advance: Option<f64>,
}

fn spec(name: &'static str) -> Spec {
    Spec {
        name,
        tools: true,
        chats: Vec::new(),
        streams: Vec::new(),
        advance: None,
    }
}

impl Spec {
    fn chat(mut self, outcome: ChatOutcome) -> Self {
        self.chats.push(outcome);
        self
    }
    fn stream(mut self, script: StreamScript) -> Self {
        self.streams.push(script);
        self
    }
    fn no_tools(mut self) -> Self {
        self.tools = false;
        self
    }
    fn advancing(mut self, seconds: f64) -> Self {
        self.advance = Some(seconds);
        self
    }
}

fn harness(specs: Vec<Spec>) -> Harness {
    let time = Arc::new(Mutex::new(0.0));
    let clock: Clock = {
        let time = Arc::clone(&time);
        Arc::new(move || *time.lock().unwrap())
    };
    let mut providers: BTreeMap<String, Box<dyn Provider>> = BTreeMap::new();
    let mut calls = BTreeMap::new();
    for spec in specs {
        let counter = Arc::new(AtomicUsize::new(0));
        calls.insert(spec.name.to_owned(), Arc::clone(&counter));
        providers.insert(
            spec.name.to_owned(),
            Box::new(Fake {
                name: spec.name.to_owned(),
                tools: spec.tools,
                chats: Mutex::new(spec.chats.into()),
                streams: Mutex::new(spec.streams.into()),
                calls: counter,
                clock: spec.advance.map(|step| (Arc::clone(&time), step)),
            }),
        );
    }
    Harness {
        router: Router::new(config(), providers, clock),
        calls,
        time,
    }
}

fn answer(provider: &str, text: &str) -> ChatOutcome {
    Ok(ChatResponse {
        text: text.to_owned(),
        provider: provider.to_owned(),
        model: None,
        tool_calls: Vec::new(),
        finish_reason: None,
        usage: None,
    })
}

fn fail(provider: &str, reason: &str, status: u16, retryable: bool) -> ProviderError {
    ProviderError::new(provider, reason)
        .with_status(status)
        .retryable(retryable)
}

fn request() -> ChatRequest {
    ChatRequest::new(vec![Message::user("hi")]).unwrap()
}

fn with_tools() -> ChatRequest {
    request().with_tools(vec![ToolSpec {
        name: "t".to_owned(),
        description: "d".to_owned(),
        parameters: json!({"type": "object"}),
    }])
}

fn failures(error: KelluError) -> Vec<(String, u16, String, bool)> {
    let KelluError::AllProvidersFailed { failures, .. } = error else {
        panic!("not an all-providers-failed error: {error}");
    };
    failures
        .into_iter()
        .map(|f| (f.provider, f.status, f.message, f.retryable))
        .collect()
}

fn done(provider: &str, text: &str) -> Result<StreamEvent, ProviderError> {
    Ok(StreamEvent::Done(Box::new(answer(provider, text).unwrap())))
}

fn text(piece: &str) -> Result<StreamEvent, ProviderError> {
    Ok(StreamEvent::Text(piece.to_owned()))
}

#[test]
fn first_provider_answers_when_it_works() {
    let h = harness(vec![
        spec("a").chat(answer("a", "from a")),
        spec("b"),
        spec("c"),
    ]);
    let response = h.router.chat(TaskKind::General, &request()).unwrap();
    assert_eq!(
        (response.text.as_str(), h.calls("a"), h.calls("b")),
        ("from a", 1, 0)
    );
}

#[test]
fn retryable_error_falls_back_to_the_next_provider() {
    let h = harness(vec![
        spec("a").chat(Err(fail("a", "slow", 503, true))),
        spec("b").chat(answer("b", "from b")),
        spec("c"),
    ]);
    assert_eq!(
        h.router.chat(TaskKind::General, &request()).unwrap().text,
        "from b"
    );
    assert_eq!((h.calls("a"), h.calls("b"), h.calls("c")), (1, 1, 0));
}

#[test]
fn non_retryable_error_also_falls_back() {
    // HTTP 402 (no balance) is not retryable on the same provider, yet another one may answer.
    let h = harness(vec![
        spec("a").chat(Err(fail("a", "insufficient balance", 402, false))),
        spec("b").chat(answer("b", "from b")),
        spec("c"),
    ]);
    assert_eq!(
        h.router
            .chat(TaskKind::General, &request())
            .unwrap()
            .provider,
        "b"
    );
}

#[test]
fn all_failures_are_reported_in_order() {
    let h = harness(vec![
        spec("a").chat(Err(fail("a", "one", 500, true))),
        spec("b").chat(Err(fail("b", "two", 402, false))),
        spec("c").chat(Err(fail("c", "three", 0, true))),
    ]);
    let error = h.router.chat(TaskKind::General, &request()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "all providers failed for task 'general': a: one; b: two; c: three"
    );
    assert_eq!(
        failures(error),
        vec![
            ("a".to_owned(), 500, "one".to_owned(), true),
            ("b".to_owned(), 402, "two".to_owned(), false),
            ("c".to_owned(), 0, "three".to_owned(), true),
        ]
    );
}

#[test]
fn missing_provider_is_recorded_and_skipped() {
    let h = harness(vec![
        spec("a").chat(Err(fail("a", "down", 500, true))),
        spec("c").chat(answer("c", "from c")),
    ]);
    // "b" is in the configured chain but was never built.
    assert_eq!(
        h.router.chat(TaskKind::General, &request()).unwrap().text,
        "from c"
    );
    let h = harness(vec![spec("a").chat(Err(fail("a", "down", 500, true)))]);
    let list = failures(h.router.chat(TaskKind::General, &request()).unwrap_err());
    assert_eq!(
        list[1],
        (
            "b".to_owned(),
            0,
            "provider is not available".to_owned(),
            false
        )
    );
    assert_eq!(list.len(), 3);
}

#[test]
fn providers_without_tool_support_are_skipped_when_tools_are_requested() {
    let h = harness(vec![
        spec("a").no_tools().chat(answer("a", "no tools here")),
        spec("b").chat(answer("b", "from b")),
        spec("c"),
    ]);
    assert_eq!(
        h.router
            .chat(TaskKind::General, &with_tools())
            .unwrap()
            .provider,
        "b"
    );
    assert_eq!(h.calls("a"), 0);
    // Without tools, the first provider is used again.
    assert_eq!(
        h.router
            .chat(TaskKind::General, &request())
            .unwrap()
            .provider,
        "a"
    );
}

#[test]
fn nothing_is_tried_when_no_provider_supports_the_request() {
    let h = harness(vec![
        spec("a").no_tools(),
        spec("b").no_tools(),
        spec("c").no_tools(),
    ]);
    let list = failures(h.router.chat(TaskKind::General, &with_tools()).unwrap_err());
    assert_eq!(list.len(), 3);
    assert!(
        list.iter()
            .all(|(_, _, message, _)| message == "does not support tool calling")
    );
    assert_eq!(h.calls("a") + h.calls("b") + h.calls("c"), 0);
}

#[test]
fn task_selects_its_own_chain() {
    let h = harness(vec![
        spec("a").chat(answer("a", "A")),
        spec("b"),
        spec("c").chat(answer("c", "C")),
    ]);
    assert_eq!(h.router.chat(TaskKind::Code, &request()).unwrap().text, "C");
    assert_eq!(
        h.router.chat(TaskKind::General, &request()).unwrap().text,
        "A"
    );
    // `explain` keeps its packaged chain (gemma_direct, openrouter), which this harness did not build.
    let h = harness(vec![spec("a"), spec("b"), spec("c")]);
    let list = failures(h.router.chat(TaskKind::Explain, &request()).unwrap_err());
    let names: Vec<_> = list.iter().map(|f| f.0.as_str()).collect();
    assert_eq!(names, ["gemma_direct", "openrouter"]);
}

#[test]
fn total_time_budget_stops_new_attempts() {
    // The first provider burns the whole 100 s budget before failing.
    let h = harness(vec![
        spec("a")
            .advancing(150.0)
            .chat(Err(fail("a", "slow", 504, true))),
        spec("b").chat(answer("b", "never asked")),
        spec("c"),
    ]);
    let list = failures(h.router.chat(TaskKind::General, &request()).unwrap_err());
    assert_eq!(h.calls("b"), 0);
    assert_eq!(list.len(), 2);
    assert_eq!(
        list[1],
        (
            "b".to_owned(),
            0,
            "total time budget exhausted".to_owned(),
            false
        )
    );
    assert!(*h.time.lock().unwrap() >= 100.0);
}

#[test]
fn from_config_builds_one_provider_per_configured_entry() {
    let lookup: kellu_providers::KeyLookup = Arc::new(|_| None::<SecretString>);
    let config = parse_config(&load_defaults().unwrap(), None, None).unwrap();
    let router = Router::from_config(config, &lookup, kellu_core::router::monotonic_clock());
    let names: Vec<_> = router.provider_names().collect();
    assert_eq!(
        names,
        [
            "deepseek",
            "gemma_direct",
            "google",
            "grok",
            "groq",
            "openrouter"
        ]
    );
}

#[test]
fn stream_returns_events_from_the_first_provider() {
    let h = harness(vec![
        spec("a").stream(vec![text("Hel"), text("lo"), done("a", "Hello")]),
        spec("b"),
        spec("c"),
    ]);
    let events: Vec<_> = h
        .router
        .stream(TaskKind::General, &request())
        .map(Result::unwrap)
        .collect();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0], StreamEvent::Text("Hel".to_owned()));
    assert!(matches!(&events[2], StreamEvent::Done(r) if r.text == "Hello"));
    assert_eq!((h.calls("a"), h.calls("b")), (1, 0));
}

#[test]
fn stream_fails_over_before_any_text_was_delivered() {
    let h = harness(vec![
        spec("a").stream(vec![Err(fail("a", "rate limited", 429, true))]),
        spec("b").stream(vec![text("ok"), done("b", "ok")]),
        spec("c"),
    ]);
    let events: Vec<_> = h
        .router
        .stream(TaskKind::General, &request())
        .map(Result::unwrap)
        .collect();
    assert_eq!(events[0], StreamEvent::Text("ok".to_owned()));
    assert_eq!((h.calls("a"), h.calls("b")), (1, 1));
}

#[test]
fn stream_does_not_fail_over_after_text_was_delivered() {
    let h = harness(vec![
        spec("a").stream(vec![
            text("partial"),
            Err(fail("a", "connection lost", 0, true)),
        ]),
        spec("b").stream(vec![text("from b"), done("b", "from b")]),
        spec("c"),
    ]);
    let items: Vec<_> = h.router.stream(TaskKind::General, &request()).collect();
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0].as_ref().unwrap(),
        &StreamEvent::Text("partial".to_owned())
    );
    assert!(matches!(&items[1], Err(KelluError::Provider(e)) if e.reason == "connection lost"));
    assert_eq!(
        h.calls("b"),
        0,
        "switching providers would contradict what the user saw"
    );
}

#[test]
fn stream_reports_all_failures() {
    let h = harness(vec![
        spec("a").stream(vec![Err(fail("a", "one", 500, true))]),
        spec("b").stream(vec![Err(fail("b", "two", 402, false))]),
        spec("c").stream(vec![Err(fail("c", "three", 0, true))]),
    ]);
    let mut stream = h.router.stream(TaskKind::General, &request());
    let error = stream.next().unwrap().unwrap_err();
    assert_eq!(
        error.to_string(),
        "all providers failed for task 'general': a: one; b: two; c: three"
    );
    assert!(
        stream.next().is_none(),
        "the stream ends after the final error"
    );
}

#[test]
fn stream_that_never_finishes_counts_as_a_failure() {
    // No events at all: counts as a failure and the next provider is tried.
    let h = harness(vec![
        spec("a").stream(vec![]),
        spec("b").stream(vec![text("ok"), done("b", "ok")]),
        spec("c"),
    ]);
    let events: Vec<_> = h
        .router
        .stream(TaskKind::General, &request())
        .map(Result::unwrap)
        .collect();
    assert_eq!(events.len(), 2);
    // Text without a final event: the user already saw text, so the error is raised.
    let h = harness(vec![
        spec("a").stream(vec![text("cut off")]),
        spec("b"),
        spec("c"),
    ]);
    let items: Vec<_> = h.router.stream(TaskKind::General, &request()).collect();
    assert!(
        matches!(&items[1], Err(KelluError::Provider(e)) if e.reason == "stream ended without a final event")
    );
    assert_eq!(h.calls("b"), 0);
}

#[test]
fn stream_stops_starting_providers_when_the_time_budget_is_spent() {
    let h = harness(vec![
        spec("a")
            .advancing(150.0)
            .stream(vec![Err(fail("a", "slow", 504, true))]),
        spec("b").stream(vec![text("never"), done("b", "never")]),
        spec("c"),
    ]);
    let items: Vec<_> = h.router.stream(TaskKind::General, &request()).collect();
    assert_eq!(items.len(), 1);
    let error = items.into_iter().next().unwrap().unwrap_err();
    let list = failures(error);
    assert_eq!(list[1].2, "total time budget exhausted");
    assert_eq!(h.calls("b"), 0);
}
