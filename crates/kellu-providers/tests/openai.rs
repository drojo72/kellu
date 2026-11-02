//! OpenAI-compatible client against a scripted local server.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::time::Duration;

use kellu_common::ProviderError;
use kellu_providers::http::Limits;
use kellu_providers::openai_compat::OpenAiCompatProvider;
use kellu_providers::{
    ChatRequest, Message, Provider, ProviderKind, StreamEvent, ToolCall, ToolSpec,
};
use serde_json::{Value, json};
use support::{KEY, Reply, Server, ask, config, lookup, sse};

fn provider_for(server: &Server) -> OpenAiCompatProvider {
    let config = config(
        ProviderKind::OpenAiCompat,
        &server.url("/v1"),
        "m1",
        Some("test"),
    );
    OpenAiCompatProvider::new(config, lookup(Some(KEY)))
}

fn completion(content: Value) -> String {
    json!({
        "model": "m1-real",
        "choices": [{"message": {"role": "assistant", "content": content}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 3, "completion_tokens": 5},
    })
    .to_string()
}

fn chat_error(server: &Server) -> ProviderError {
    provider_for(server).chat(&ask("hi")).unwrap_err()
}

fn stream_items(
    provider: &OpenAiCompatProvider,
    request: &ChatRequest,
) -> Vec<Result<StreamEvent, ProviderError>> {
    provider.stream(request).collect()
}

#[test]
fn request_goes_to_chat_completions_with_header_auth_only() {
    let server = Server::start(vec![Reply::Json(200, completion(json!("ok")))]);
    let provider = provider_for(&server);
    let request = ask("hi").with_temperature(0.25).with_max_tokens(64);
    provider.chat(&request).unwrap();
    let seen = &server.requests()[0];
    assert_eq!(seen.path, "/v1/chat/completions");
    assert_eq!(seen.headers["authorization"], format!("Bearer {KEY}"));
    assert!(!seen.path.contains(KEY));
    let body = seen.json();
    assert_eq!(body["model"], "m1");
    assert_eq!(body["stream"], false);
    assert_eq!(body["messages"], json!([{"role": "user", "content": "hi"}]));
    assert_eq!(body["temperature"], 0.25);
    assert_eq!(body["max_tokens"], 64);
    assert!(body.get("tools").is_none());
}

#[test]
fn chat_returns_text_and_usage() {
    let server = Server::start(vec![Reply::Json(200, completion(json!("hello")))]);
    let response = provider_for(&server).chat(&ask("hi")).unwrap();
    assert_eq!(response.text, "hello");
    assert_eq!(response.provider, "test");
    assert_eq!(response.model.as_deref(), Some("m1-real"));
    assert_eq!(response.finish_reason.as_deref(), Some("stop"));
    let usage = response.usage.unwrap();
    assert_eq!(
        (usage.prompt_tokens, usage.completion_tokens),
        (Some(3), Some(5))
    );
}

#[test]
fn content_given_as_parts_is_joined() {
    let parts = json!([{"type": "text", "text": "he"}, {"type": "image"}, {"text": "llo"}]);
    let server = Server::start(vec![Reply::Json(200, completion(parts))]);
    assert_eq!(
        provider_for(&server).chat(&ask("hi")).unwrap().text,
        "hello"
    );
}

#[test]
fn provider_without_keyring_name_sends_no_authorization() {
    let server = Server::start(vec![Reply::Json(200, completion(json!("ok")))]);
    let config = config(ProviderKind::OpenAiCompat, &server.url("/v1"), "m1", None);
    let provider = OpenAiCompatProvider::new(config, lookup(None));
    provider.chat(&ask("hi")).unwrap();
    assert!(!server.requests()[0].headers.contains_key("authorization"));
}

#[test]
fn missing_key_fails_before_any_network_call() {
    let server = Server::start(vec![Reply::Json(200, completion(json!("ok")))]);
    let config = config(
        ProviderKind::OpenAiCompat,
        &server.url("/v1"),
        "m1",
        Some("test"),
    );
    let provider = OpenAiCompatProvider::new(config, lookup(None));
    let error = provider.chat(&ask("hi")).unwrap_err();
    assert!(!error.retryable);
    assert!(
        error.reason.contains("kellu keys set test"),
        "{}",
        error.reason
    );
    let items = stream_items(&provider, &ask("hi"));
    assert_eq!(items.len(), 1);
    assert!(items[0].is_err());
    assert!(server.requests().is_empty());
}

#[test]
fn key_with_trailing_newline_is_accepted() {
    let server = Server::start(vec![Reply::Json(200, completion(json!("ok")))]);
    let config = config(
        ProviderKind::OpenAiCompat,
        &server.url("/v1"),
        "m1",
        Some("test"),
    );
    let provider = OpenAiCompatProvider::new(config, lookup(Some("sk-abc123\n")));
    provider.chat(&ask("hi")).unwrap();
    assert_eq!(
        server.requests()[0].headers["authorization"],
        "Bearer sk-abc123"
    );
}

#[test]
fn malformed_key_is_rejected_to_prevent_header_injection() {
    let server = Server::start(vec![Reply::Json(200, completion(json!("ok")))]);
    for bad in ["sk-abc\r\nX-Evil: 1", "sk abc", "sk-ab\tc", "sk-\u{7f}abc"] {
        let config = config(
            ProviderKind::OpenAiCompat,
            &server.url("/v1"),
            "m1",
            Some("test"),
        );
        let provider = OpenAiCompatProvider::new(config, lookup(Some(bad)));
        let error = provider.chat(&ask("hi")).unwrap_err();
        assert_eq!(error.reason, "the stored API key is malformed", "{bad:?}");
        assert!(!error.retryable);
    }
    assert!(server.requests().is_empty());
}

#[test]
fn http_status_maps_to_retryable_flag() {
    let cases = [
        (400, false),
        (401, false),
        (402, false),
        (404, false),
        (408, true),
        (409, true),
        (425, true),
        (429, true),
        (500, true),
        (503, true),
    ];
    for (status, retryable) in cases {
        let body = json!({"error": {"message": "nope"}}).to_string();
        let server = Server::start(vec![Reply::Json(status, body)]);
        let error = chat_error(&server);
        assert_eq!(error.status, status, "{status}");
        assert_eq!(error.retryable, retryable, "{status}");
        assert_eq!(error.reason, "nope");
    }
}

#[test]
fn error_text_never_contains_the_key_or_control_characters() {
    let body =
        json!({"error": {"message": format!("bad key {KEY}\u{1b}[31m\nsecond line\u{202e}")}})
            .to_string();
    let server = Server::start(vec![Reply::Json(401, body)]);
    let error = chat_error(&server);
    assert!(!error.reason.contains(KEY), "{}", error.reason);
    assert!(error.reason.contains("***"));
    assert!(
        !error
            .reason
            .chars()
            .any(|c| c.is_control() || c == '\u{202e}')
    );
    assert!(!error.reason.contains('\n'));
    assert!(error.reason.chars().count() < 340);

    let huge = "x".repeat(10_000);
    let server = Server::start(vec![Reply::Raw(500, huge.into_bytes())]);
    assert!(chat_error(&server).reason.chars().count() < 340);
}

#[test]
fn error_object_inside_http_200_is_an_error() {
    let body = json!({"error": {"message": "upstream exploded", "code": 502}}).to_string();
    let server = Server::start(vec![Reply::Json(200, body)]);
    let error = chat_error(&server);
    assert_eq!((error.status, error.retryable), (502, true));
    let body = json!({"error": {"message": "no credits", "code": 402}}).to_string();
    let server = Server::start(vec![Reply::Json(200, body)]);
    let error = chat_error(&server);
    assert_eq!((error.status, error.retryable), (402, false));
}

#[test]
fn malformed_responses_are_not_retryable() {
    let bodies = [
        "not json".to_owned(),
        "[1, 2]".to_owned(),
        json!({}).to_string(),
        json!({"choices": []}).to_string(),
        json!({"choices": ["x"]}).to_string(),
        json!({"choices": [{"finish_reason": "stop"}]}).to_string(),
        json!({"choices": [{"message": {"tool_calls": "x"}}]}).to_string(),
    ];
    for body in bodies {
        let server = Server::start(vec![Reply::Json(200, body.clone())]);
        let error = chat_error(&server);
        assert!(!error.retryable, "{body}: {error}");
        assert!(
            error.reason.starts_with("malformed response"),
            "{body}: {}",
            error.reason
        );
    }
}

#[test]
fn oversized_response_is_rejected() {
    let server = Server::start(vec![Reply::Big(200_000)]);
    let config = config(
        ProviderKind::OpenAiCompat,
        &server.url("/v1"),
        "m1",
        Some("test"),
    );
    let limits = Limits {
        max_response_bytes: 100_000,
        ..Limits::default()
    };
    let provider = OpenAiCompatProvider::with_limits(config, lookup(Some(KEY)), limits);
    let error = provider.chat(&ask("hi")).unwrap_err();
    assert_eq!(error.reason, "response too large");
    assert!(!error.retryable);
}

#[test]
fn redirects_are_refused_and_not_followed() {
    let target = Server::start(vec![Reply::Json(200, completion(json!("stolen")))]);
    let server = Server::start(vec![Reply::Redirect(target.url("/steal"))]);
    let error = chat_error(&server);
    assert_eq!(error.status, 302);
    assert!(!error.retryable);
    assert!(error.reason.contains("redirect refused"));
    assert!(
        target.requests().is_empty(),
        "the redirect target must never be contacted"
    );
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn timeout_is_retryable() {
    let server = Server::start(vec![Reply::Stall(Duration::from_secs(3))]);
    let mut config = config(
        ProviderKind::OpenAiCompat,
        &server.url("/v1"),
        "m1",
        Some("test"),
    );
    config.timeout_s = 0.4;
    let provider = OpenAiCompatProvider::new(config, lookup(Some(KEY)));
    let error = provider.chat(&ask("hi")).unwrap_err();
    assert_eq!(error.reason, "request timed out");
    assert!(error.retryable);
}

#[test]
fn unreachable_server_is_retryable() {
    let config = config(
        ProviderKind::OpenAiCompat,
        "http://127.0.0.1:1/v1",
        "m1",
        Some("test"),
    );
    let provider = OpenAiCompatProvider::new(config, lookup(Some(KEY)));
    let error = provider.chat(&ask("hi")).unwrap_err();
    assert!(
        error.reason.starts_with("network error"),
        "{}",
        error.reason
    );
    assert!(error.retryable);
    let dropped = Server::start(vec![Reply::Drop]);
    assert!(chat_error(&dropped).retryable);
}

#[test]
fn transport_refuses_unsafe_endpoints_even_if_configured() {
    for base in [
        "http://example.com/v1",
        "ftp://example.com/v1",
        "https://user:pw@example.com/v1",
        "not a url",
    ] {
        let config = config(ProviderKind::OpenAiCompat, base, "m1", Some("test"));
        let provider = OpenAiCompatProvider::new(config, lookup(Some(KEY)));
        let error = provider.chat(&ask("hi")).unwrap_err();
        assert!(
            error.reason.starts_with("unsafe endpoint"),
            "{base}: {}",
            error.reason
        );
        assert!(!error.retryable, "{base}");
        let items = stream_items(&provider, &ask("hi"));
        assert!(
            items[0]
                .as_ref()
                .unwrap_err()
                .reason
                .starts_with("unsafe endpoint")
        );
    }
}

#[test]
fn a_peer_that_does_not_speak_tls_is_a_retryable_network_error_without_the_key() {
    // An https URL pointing at a plain-text server: the handshake ends in EOF, like a dropped
    // connection. (Certificate failures are classified as non-retryable TLS errors; that mapping
    // is unit-tested in `http.rs` and was verified by hand against a self-signed server.)
    let server = Server::start(vec![Reply::Raw(200, b"plain".to_vec())]);
    let base = server.url("/v1").replacen("http://", "https://", 1);
    let config = config(ProviderKind::OpenAiCompat, &base, "m1", Some("test"));
    let provider = OpenAiCompatProvider::new(config, lookup(Some(KEY)));
    let error = provider.chat(&ask("hi")).unwrap_err();
    assert!(
        error.reason.starts_with("network error"),
        "{}",
        error.reason
    );
    assert!(error.retryable);
    assert!(!error.reason.contains(KEY));
}

fn tool_spec() -> ToolSpec {
    ToolSpec {
        name: "read_file".to_owned(),
        description: "Read a file".to_owned(),
        parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
    }
}

#[test]
fn tools_are_sent_and_tool_calls_are_parsed() {
    let reply = json!({
        "choices": [{
            "message": {"content": null, "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\": \"a.txt\"}"}},
                {"function": {"name": "list.dir", "arguments": ""}},
            ]},
            "finish_reason": "tool_calls",
        }],
    })
    .to_string();
    let server = Server::start(vec![Reply::Json(200, reply)]);
    let request = ask("go").with_tools(vec![tool_spec()]);
    let response = provider_for(&server).chat(&request).unwrap();
    assert_eq!(response.text, "");
    assert_eq!(response.tool_calls.len(), 2);
    assert_eq!(response.tool_calls[0].id, "c1");
    assert_eq!(response.tool_calls[0].arguments["path"], "a.txt");
    assert_eq!(response.tool_calls[1].id, "call_1_list.dir");
    assert!(response.tool_calls[1].arguments.is_empty());
    let sent = server.requests()[0].json();
    assert_eq!(sent["tools"][0]["type"], "function");
    assert_eq!(sent["tools"][0]["function"]["name"], "read_file");
    assert_eq!(sent["tools"][0]["function"]["parameters"]["type"], "object");
}

#[test]
fn server_supplied_ids_and_labels_are_made_safe_and_ids_are_unique() {
    let reply = json!({
        "model": "m\u{1b}[31m-real\n",
        "choices": [{
            "message": {"content": "", "tool_calls": [
                {"id": "bad\u{1b}[0m id", "function": {"name": "t", "arguments": "{}"}},
                {"function": {"name": "t", "arguments": "{}"}},
                {"id": "call_ok-1", "function": {"name": "t", "arguments": "{}"}},
            ]},
            "finish_reason": "tool\u{202e}_calls",
        }],
    })
    .to_string();
    let server = Server::start(vec![Reply::Json(200, reply)]);
    let response = provider_for(&server).chat(&ask("go")).unwrap();
    let ids: Vec<_> = response.tool_calls.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["call_0_t", "call_1_t", "call_ok-1"]);
    assert_eq!(response.model.as_deref(), Some("m[31m-real"));
    assert_eq!(response.finish_reason.as_deref(), Some("tool_calls"));
}

#[test]
fn assistant_tool_calls_and_tool_results_are_serialized() {
    let server = Server::start(vec![Reply::Json(200, completion(json!("done")))]);
    let call = ToolCall {
        id: "c1".to_owned(),
        name: "read_file".to_owned(),
        arguments: json!({"path": "a.txt"}).as_object().unwrap().clone(),
    };
    let request = ChatRequest::new(vec![
        Message::system("be brief"),
        Message::user("read it"),
        Message::assistant_with_calls("", vec![call]),
        Message::tool("c1", "file contents"),
    ])
    .unwrap();
    provider_for(&server).chat(&request).unwrap();
    let messages = server.requests()[0].json()["messages"].clone();
    assert_eq!(
        messages[0],
        json!({"role": "system", "content": "be brief"})
    );
    assert_eq!(messages[2]["role"], "assistant");
    assert_eq!(messages[2]["tool_calls"][0]["id"], "c1");
    assert_eq!(messages[2]["tool_calls"][0]["type"], "function");
    let arguments = messages[2]["tool_calls"][0]["function"]["arguments"]
        .as_str()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(arguments).unwrap(),
        json!({"path": "a.txt"})
    );
    assert_eq!(
        messages[3],
        json!({"role": "tool", "content": "file contents", "tool_call_id": "c1"})
    );
    // A tool result without an id is refused before anything is sent.
    let bad = ChatRequest::new(vec![Message::tool("", "x")]).unwrap();
    assert!(provider_for(&server).chat(&bad).is_err());
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn invalid_tool_arguments_are_rejected() {
    for arguments in ["not json", "[1]", "7"] {
        let reply = json!({"choices": [{"message": {"content": "", "tool_calls": [
            {"id": "c", "function": {"name": "t", "arguments": arguments}}]}}]})
        .to_string();
        let server = Server::start(vec![Reply::Json(200, reply)]);
        let error = chat_error(&server);
        assert!(!error.retryable, "{arguments}");
        assert!(
            error.reason.contains("tool call arguments"),
            "{}",
            error.reason
        );
    }
    let many: Vec<Value> = (0..33)
        .map(|i| json!({"id": format!("c{i}"), "function": {"name": "t", "arguments": "{}"}}))
        .collect();
    let reply = json!({"choices": [{"message": {"content": "", "tool_calls": many}}]}).to_string();
    let server = Server::start(vec![Reply::Json(200, reply)]);
    assert!(chat_error(&server).reason.contains("too many tool calls"));
}

#[test]
fn tool_name_with_unsafe_characters_is_rejected() {
    for name in ["rm -rf /", "a;b", "../x", "", "x".repeat(65).as_str()] {
        let reply = json!({"choices": [{"message": {"content": "", "tool_calls": [
            {"id": "c", "function": {"name": name, "arguments": "{}"}}]}}]})
        .to_string();
        let server = Server::start(vec![Reply::Json(200, reply)]);
        let error = chat_error(&server);
        assert!(
            error.reason.contains("invalid name"),
            "{name:?}: {}",
            error.reason
        );
        assert!(!error.retryable);
    }
}

fn delta(content: &str) -> Value {
    json!({"model": "m1-real", "choices": [{"delta": {"content": content}}]})
}

#[test]
fn stream_yields_text_then_a_final_response() {
    let finish = json!({"choices": [{"delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 2, "completion_tokens": 4}});
    let body = format!(
        "{}data: [DONE]\n\n",
        sse(&[delta("Hel"), delta("lo"), finish])
    );
    let server = Server::start(vec![Reply::Sse(body)]);
    let provider = provider_for(&server);
    let mut stream = provider.stream(&ask("hi"));
    assert!(
        server.requests().is_empty(),
        "nothing is sent until the stream is polled"
    );
    let items: Vec<_> = stream.by_ref().collect();
    let events: Vec<_> = items.into_iter().map(Result::unwrap).collect();
    assert_eq!(events[0], StreamEvent::Text("Hel".to_owned()));
    assert_eq!(events[1], StreamEvent::Text("lo".to_owned()));
    let StreamEvent::Done(response) = &events[2] else {
        panic!("no final event: {events:?}")
    };
    assert_eq!(events.len(), 3);
    assert_eq!(response.text, "Hello");
    assert_eq!(response.model.as_deref(), Some("m1-real"));
    assert_eq!(response.finish_reason.as_deref(), Some("stop"));
    assert_eq!(response.usage.unwrap().completion_tokens, Some(4));
    assert!(stream.next().is_none());
    let seen = &server.requests()[0];
    assert_eq!(seen.json()["stream"], true);
    assert_eq!(seen.headers["accept"], "text/event-stream");
}

#[test]
fn stream_accumulates_tool_call_fragments() {
    let first = json!({"choices": [{"delta": {"tool_calls": [
        {"index": 0, "id": "c1", "function": {"name": "read_file", "arguments": "{\"pa"}},
        {"index": 1, "id": "c2", "function": {"name": "other", "arguments": "{}"}}]}}]});
    let second = json!({"choices": [{"delta": {"tool_calls": [
        {"index": 0, "function": {"arguments": "th\": \"a.txt\"}"}}]}, "finish_reason": "tool_calls"}]});
    let server = Server::start(vec![Reply::Sse(sse(&[first, second]))]);
    let request = ask("go").with_tools(vec![tool_spec()]);
    let items = stream_items(&provider_for(&server), &request);
    let Ok(StreamEvent::Done(response)) = items.last().unwrap() else {
        panic!("{items:?}")
    };
    assert_eq!(response.tool_calls.len(), 2);
    assert_eq!(response.tool_calls[0].id, "c1");
    assert_eq!(response.tool_calls[0].arguments["path"], "a.txt");
    assert_eq!(response.tool_calls[1].name, "other");
    assert_eq!(response.finish_reason.as_deref(), Some("tool_calls"));
}

#[test]
fn stream_with_invalid_tool_index_is_rejected() {
    let bad =
        json!({"choices": [{"delta": {"tool_calls": [{"index": 99, "function": {"name": "t"}}]}}]});
    let server = Server::start(vec![Reply::Sse(sse(&[bad]))]);
    let items = stream_items(&provider_for(&server), &ask("go"));
    assert!(
        items[0]
            .as_ref()
            .unwrap_err()
            .reason
            .contains("invalid index")
    );
}

#[test]
fn stream_that_ends_without_done_or_finish_reason_fails() {
    let server = Server::start(vec![Reply::Sse(sse(&[delta("partial")]))]);
    let items = stream_items(&provider_for(&server), &ask("hi"));
    assert_eq!(items[0], Ok(StreamEvent::Text("partial".to_owned())));
    assert_eq!(
        items[1].as_ref().unwrap_err().reason,
        "stream ended unexpectedly"
    );
    assert_eq!(items.len(), 2);
}

#[test]
fn stream_with_invalid_json_event_fails() {
    for body in ["data: {nope\n\n", "data: [1]\n\n"] {
        let server = Server::start(vec![Reply::Sse(body.to_owned())]);
        let items = stream_items(&provider_for(&server), &ask("hi"));
        let error = items[0].as_ref().unwrap_err();
        assert!(
            error.reason.starts_with("malformed response"),
            "{}",
            error.reason
        );
        assert!(!error.retryable);
        assert_eq!(items.len(), 1);
    }
}

#[test]
fn stream_error_event_raises() {
    let error_event = json!({"error": {"message": format!("boom {KEY}"), "code": 503}});
    let server = Server::start(vec![Reply::Sse(sse(&[delta("a"), error_event]))]);
    let items = stream_items(&provider_for(&server), &ask("hi"));
    assert_eq!(items[0], Ok(StreamEvent::Text("a".to_owned())));
    let error = items[1].as_ref().unwrap_err();
    assert_eq!((error.status, error.retryable), (503, true));
    assert!(!error.reason.contains(KEY));
}

#[test]
fn stream_http_error_is_raised_when_iterating() {
    let body = json!({"error": {"message": "rate limited"}}).to_string();
    let server = Server::start(vec![Reply::Json(429, body)]);
    let provider = provider_for(&server);
    let mut stream = provider.stream(&ask("hi"));
    let error = stream.next().unwrap().unwrap_err();
    assert_eq!((error.status, error.retryable), (429, true));
    assert!(stream.next().is_none());
}

#[test]
fn stream_line_and_duration_limits_apply() {
    let long = format!("data: {}\n\n", "x".repeat(5_000));
    let server = Server::start(vec![Reply::Sse(long)]);
    let cfg = config(
        ProviderKind::OpenAiCompat,
        &server.url("/v1"),
        "m1",
        Some("test"),
    );
    let limits = Limits {
        max_line_bytes: 1_000,
        ..Limits::default()
    };
    let provider = OpenAiCompatProvider::with_limits(cfg, lookup(Some(KEY)), limits);
    let items = stream_items(&provider, &ask("hi"));
    assert_eq!(
        items[0].as_ref().unwrap_err().reason,
        "stream line too long"
    );

    let server = Server::start(vec![Reply::SseStall(
        sse(&[delta("a")]),
        Duration::from_secs(5),
    )]);
    let cfg = config(
        ProviderKind::OpenAiCompat,
        &server.url("/v1"),
        "m1",
        Some("test"),
    );
    let limits = Limits {
        max_duration: Duration::from_millis(500),
        ..Limits::default()
    };
    let provider = OpenAiCompatProvider::with_limits(cfg, lookup(Some(KEY)), limits);
    let items = stream_items(&provider, &ask("hi"));
    assert_eq!(items[0], Ok(StreamEvent::Text("a".to_owned())));
    assert!(items[1].as_ref().unwrap_err().retryable, "{items:?}");
    assert_eq!(items.len(), 2);
}
