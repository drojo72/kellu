//! The router end to end: real provider clients talking to scripted local servers.
//!
//! This file installs a global logger (the `log` facade has exactly one per process), so it
//! lives in its own test binary.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex, Once};

use kellu_common::KelluError;
use kellu_core::config::{load_defaults, parse_config};
use kellu_core::router::{Router, TaskKind, monotonic_clock};
use kellu_providers::{ChatRequest, KeyLookup, Message, StreamEvent};
use log::{Level, LevelFilter, Log, Metadata, Record};
use secrecy::SecretString;
use serde_json::json;

const KEY: &str = "sk-router-test-key-9876";

/// Answers each connection with the next `(status, body)`, then closes it.
fn server(replies: Vec<(u16, String)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut replies = replies.into_iter();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
            let (status, body) = replies.next().unwrap_or((500, "{}".to_owned()));
            let head = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}/v1")
}

fn build_router(first: &str, second: &str) -> Router {
    let user = format!(
        r#"
[providers.first]
kind = "openai_compat"
base_url = "{first}"
model = "m"
timeout_s = 5
keyring_name = "first"
[providers.second]
kind = "openai_compat"
base_url = "{second}"
model = "m"
timeout_s = 5
keyring_name = "second"
[routing.general]
primary = "first"
backup = ["second"]
"#
    );
    let user: toml::Table = user.parse().unwrap();
    let config = parse_config(&load_defaults().unwrap(), Some(&user), None).unwrap();
    let lookup: KeyLookup = Arc::new(|_| Some(SecretString::from(KEY.to_owned())));
    Router::from_config(config, &lookup, monotonic_clock())
}

fn request() -> ChatRequest {
    ChatRequest::new(vec![Message::user("hi")]).unwrap()
}

fn completion(text: &str) -> (u16, String) {
    let body = json!({"choices": [{"message": {"content": text}, "finish_reason": "stop"}]});
    (200, body.to_string())
}

#[test]
fn http_402_falls_back_to_the_next_provider() {
    // The prototype's real DeepSeek run returned 402 (no balance): the next provider must answer.
    let no_balance = (
        402,
        json!({"error": {"message": "Insufficient Balance"}}).to_string(),
    );
    let router = build_router(
        &server(vec![no_balance.clone()]),
        &server(vec![completion("from second")]),
    );
    let response = router.chat(TaskKind::General, &request()).unwrap();
    assert_eq!(
        (response.provider.as_str(), response.text.as_str()),
        ("second", "from second")
    );

    // Streaming: the failing status arrives before any text, so it fails over too.
    let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
    let second = {
        let body = sse.to_owned();
        // `server` speaks JSON framing; an SSE body with a Content-Length is still valid HTTP.
        server(vec![(200, body)])
    };
    let router = build_router(&server(vec![no_balance]), &second);
    let events: Vec<_> = router
        .stream(TaskKind::General, &request())
        .map(Result::unwrap)
        .collect();
    assert_eq!(events[0], StreamEvent::Text("hi".to_owned()));
    assert!(matches!(events.last(), Some(StreamEvent::Done(r)) if r.provider == "second"));
}

#[test]
fn every_failure_is_reported_when_both_providers_fail() {
    let router = build_router(
        &server(vec![(
            401,
            json!({"error": {"message": "bad key"}}).to_string(),
        )]),
        &server(vec![(503, "{}".to_owned())]),
    );
    let error = router.chat(TaskKind::General, &request()).unwrap_err();
    let KelluError::AllProvidersFailed { failures, .. } = error else {
        panic!("{error}")
    };
    let summary: Vec<_> = failures
        .iter()
        .map(|f| (f.provider.as_str(), f.status, f.retryable))
        .collect();
    assert_eq!(summary, [("first", 401, false), ("second", 503, true)]);
}

struct Capture(Mutex<Vec<String>>);

impl Log for Capture {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= Level::Warn
    }
    fn log(&self, record: &Record<'_>) {
        self.0.lock().unwrap().push(format!("{}", record.args()));
    }
    fn flush(&self) {}
}

static CAPTURE: Capture = Capture(Mutex::new(Vec::new()));
static INSTALL: Once = Once::new();

#[test]
fn failures_are_logged_without_secrets() {
    INSTALL.call_once(|| {
        log::set_logger(&CAPTURE).unwrap();
        log::set_max_level(LevelFilter::Warn);
    });
    // The server echoes the key in its error text, as a careless gateway might.
    let echo = (
        401,
        json!({"error": {"message": format!("invalid key {KEY}\u{1b}[31m")}}).to_string(),
    );
    let router = build_router(&server(vec![echo]), &server(vec![completion("ok")]));
    router.chat(TaskKind::General, &request()).unwrap();
    let logged = CAPTURE.0.lock().unwrap().join("\n");
    assert!(
        logged.contains("provider first failed (status 401, retryable=false)"),
        "{logged}"
    );
    assert!(
        !logged.contains(KEY),
        "the key must never reach the log: {logged}"
    );
    assert!(!logged.contains('\u{1b}'));
}
