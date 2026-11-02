//! Google (Gemini and Gemma) client against a scripted local server.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use kellu_common::ProviderError;
use kellu_providers::google::GoogleProvider;
use kellu_providers::{ChatRequest, Message, Provider, ProviderKind, StreamEvent, ToolSpec};
use serde_json::{Value, json};
use support::{KEY, Reply, Server, ask, config, lookup, sse};

fn provider(server: &Server, model: &str) -> GoogleProvider {
    let config = config(
        ProviderKind::Google,
        &server.url("/v1beta"),
        model,
        Some("test"),
    );
    GoogleProvider::new(config, lookup(Some(KEY)))
}

fn answer(text: &str) -> String {
    json!({
        "modelVersion": "gemini-real",
        "candidates": [{"content": {"parts": [{"text": text}]}, "finishReason": "STOP"}],
        "usageMetadata": {"promptTokenCount": 7, "candidatesTokenCount": 9},
    })
    .to_string()
}

fn chunk(text: &str) -> Value {
    json!({"modelVersion": "gemini-real", "candidates": [{"content": {"parts": [{"text": text}]}}]})
}

fn items(
    provider: &GoogleProvider,
    request: &ChatRequest,
) -> Vec<Result<StreamEvent, ProviderError>> {
    provider.stream(request).collect()
}

#[test]
fn chat_uses_header_auth_and_never_puts_the_key_in_the_url() {
    let server = Server::start(vec![Reply::Json(200, answer("hello"))]);
    let response = provider(&server, "gemini-flash-latest")
        .chat(&ask("hi"))
        .unwrap();
    assert_eq!(response.text, "hello");
    assert_eq!(response.model.as_deref(), Some("gemini-real"));
    assert_eq!(response.finish_reason.as_deref(), Some("STOP"));
    assert_eq!(response.usage.unwrap().prompt_tokens, Some(7));
    let seen = &server.requests()[0];
    assert_eq!(
        seen.path,
        "/v1beta/models/gemini-flash-latest:generateContent"
    );
    assert_eq!(seen.headers["x-goog-api-key"], KEY);
    assert!(!seen.headers.contains_key("authorization"));
    assert!(!seen.path.contains(KEY) && !seen.path.contains("key="));
}

#[test]
fn model_names_are_encoded_into_the_path_and_models_prefix_is_dropped() {
    let server = Server::start(vec![Reply::Json(200, answer("ok"))]);
    provider(&server, "models/gem ini/x?y")
        .chat(&ask("hi"))
        .unwrap();
    assert_eq!(
        server.requests()[0].path,
        "/v1beta/models/gem%20ini%2Fx%3Fy:generateContent"
    );
}

#[test]
fn missing_key_fails_before_the_network() {
    let server = Server::start(vec![Reply::Json(200, answer("ok"))]);
    let config = config(
        ProviderKind::Google,
        &server.url("/v1beta"),
        "g",
        Some("test"),
    );
    let provider = GoogleProvider::new(config, lookup(None));
    let error = provider.chat(&ask("hi")).unwrap_err();
    assert!(!error.retryable && error.reason.contains("kellu keys set test"));
    assert!(items(&provider, &ask("hi"))[0].is_err());
    assert!(server.requests().is_empty());
}

#[test]
fn gemini_uses_a_system_instruction() {
    let server = Server::start(vec![Reply::Json(200, answer("ok"))]);
    let request = ChatRequest::new(vec![
        Message::system("be brief"),
        Message::system("be kind"),
        Message::user("hi"),
    ])
    .unwrap();
    provider(&server, "gemini-flash-latest")
        .chat(&request)
        .unwrap();
    let body = server.requests()[0].json();
    assert_eq!(
        body["systemInstruction"],
        json!({"parts": [{"text": "be brief\n\nbe kind"}]})
    );
    assert_eq!(
        body["contents"],
        json!([{"role": "user", "parts": [{"text": "hi"}]}])
    );
}

#[test]
fn gemma_folds_the_system_message_into_the_first_user_turn() {
    for model in ["gemma-4-26b-a4b-it", "models/Gemma-3"] {
        let server = Server::start(vec![Reply::Json(200, answer("ok"))]);
        let request =
            ChatRequest::new(vec![Message::system("be brief"), Message::user("hi")]).unwrap();
        provider(&server, model).chat(&request).unwrap();
        let body = server.requests()[0].json();
        assert!(body.get("systemInstruction").is_none(), "{model}");
        assert_eq!(
            body["contents"][0]["parts"][0]["text"], "be brief\n\nhi",
            "{model}"
        );
    }
    // Without a leading user turn, the folded text becomes its own user turn.
    let server = Server::start(vec![Reply::Json(200, answer("ok"))]);
    let request =
        ChatRequest::new(vec![Message::system("sys"), Message::assistant("earlier")]).unwrap();
    provider(&server, "gemma-x").chat(&request).unwrap();
    let contents = server.requests()[0].json()["contents"].clone();
    assert_eq!(
        contents[0],
        json!({"role": "user", "parts": [{"text": "sys"}]})
    );
    assert_eq!(contents[1]["role"], "model");
}

#[test]
fn roles_are_mapped_and_generation_config_is_sent() {
    let server = Server::start(vec![Reply::Json(200, answer("ok"))]);
    let request = ChatRequest::new(vec![
        Message::user("q1"),
        Message::assistant("a1"),
        Message::user("q2"),
    ])
    .unwrap()
    .with_temperature(0.5)
    .with_max_tokens(128);
    provider(&server, "g").chat(&request).unwrap();
    let body = server.requests()[0].json();
    let roles: Vec<_> = body["contents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["role"].clone())
        .collect();
    assert_eq!(roles, vec![json!("user"), json!("model"), json!("user")]);
    assert_eq!(
        body["generationConfig"],
        json!({"temperature": 0.5, "maxOutputTokens": 128})
    );
    let server = Server::start(vec![Reply::Json(200, answer("ok"))]);
    provider(&server, "g").chat(&ask("hi")).unwrap();
    assert!(
        server.requests()[0]
            .json()
            .get("generationConfig")
            .is_none()
    );
}

#[test]
fn tools_are_not_supported_yet() {
    let server = Server::start(vec![Reply::Json(200, answer("ok"))]);
    let tool = ToolSpec {
        name: "t".to_owned(),
        description: "d".to_owned(),
        parameters: json!({"type": "object"}),
    };
    let provider = provider(&server, "g");
    assert!(!provider.supports_tools());
    let error = provider
        .chat(&ask("hi").with_tools(vec![tool]))
        .unwrap_err();
    assert!(error.reason.contains("tool calling is not supported") && !error.retryable);
    for message in [
        Message::tool("c1", "x"),
        Message::assistant_with_calls(
            "",
            vec![kellu_providers::ToolCall {
                id: "c".into(),
                name: "t".into(),
                arguments: serde_json::Map::new(),
            }],
        ),
    ] {
        let request = ChatRequest::new(vec![Message::user("hi"), message]).unwrap();
        assert!(
            provider
                .chat(&request)
                .unwrap_err()
                .reason
                .contains("tool messages")
        );
    }
    let only_system = ChatRequest::new(vec![Message::system("s")]).unwrap();
    assert!(
        provider
            .chat(&only_system)
            .unwrap_err()
            .reason
            .contains("no user or assistant turns")
    );
    assert!(server.requests().is_empty());
}

#[test]
fn thinking_parts_are_not_part_of_the_answer() {
    let body = json!({"candidates": [{"content": {"parts": [
        {"text": "secret reasoning", "thought": true},
        {"text": "visible "},
        {"text": "answer", "thought": false},
    ]}}]})
    .to_string();
    let server = Server::start(vec![Reply::Json(200, body)]);
    assert_eq!(
        provider(&server, "g").chat(&ask("hi")).unwrap().text,
        "visible answer"
    );
}

#[test]
fn blocked_prompt_is_a_non_retryable_error() {
    let body = json!({"promptFeedback": {"blockReason": "SAFETY"}}).to_string();
    let server = Server::start(vec![Reply::Json(200, body)]);
    let error = provider(&server, "g").chat(&ask("hi")).unwrap_err();
    assert!(error.reason.contains("blocked") && error.reason.contains("SAFETY"));
    assert!(!error.retryable);
    let server = Server::start(vec![Reply::Sse(sse(&[
        json!({"promptFeedback": {"blockReason": "OTHER"}}),
    ]))]);
    let results = items(&provider(&server, "g"), &ask("hi"));
    assert!(!results[0].as_ref().unwrap_err().retryable);
    let server = Server::start(vec![Reply::Json(200, json!({}).to_string())]);
    assert!(
        provider(&server, "g")
            .chat(&ask("hi"))
            .unwrap_err()
            .reason
            .contains("no candidates")
    );
}

#[test]
fn http_error_and_key_scrubbing() {
    let message = format!("API key not valid: {KEY}\u{1b}[0m");
    let body = json!({"error": {"code": 400, "message": message}}).to_string();
    let server = Server::start(vec![Reply::Json(400, body)]);
    let error = provider(&server, "g").chat(&ask("hi")).unwrap_err();
    assert_eq!((error.status, error.retryable), (400, false));
    assert!(!error.reason.contains(KEY) && !error.reason.contains('\u{1b}'));
    assert!(error.reason.contains("***"));
    let server = Server::start(vec![Reply::Json(503, "{}".to_owned())]);
    let error = provider(&server, "g").chat(&ask("hi")).unwrap_err();
    assert_eq!((error.status, error.retryable), (503, true));
}

#[test]
fn stream_uses_sse_endpoint_and_assembles_the_answer() {
    let last = json!({
        "candidates": [{"content": {"parts": [{"text": "lo"}]}, "finishReason": "STOP"}],
        "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 2},
    });
    let server = Server::start(vec![Reply::Sse(sse(&[chunk("Hel"), last]))]);
    let provider = provider(&server, "gemini-flash-latest");
    let mut stream = provider.stream(&ask("hi"));
    assert!(server.requests().is_empty());
    let events: Vec<_> = stream.by_ref().map(Result::unwrap).collect();
    assert_eq!(events[0], StreamEvent::Text("Hel".to_owned()));
    assert_eq!(events[1], StreamEvent::Text("lo".to_owned()));
    let StreamEvent::Done(response) = &events[2] else {
        panic!("{events:?}")
    };
    assert_eq!(
        (response.text.as_str(), response.finish_reason.as_deref()),
        ("Hello", Some("STOP"))
    );
    assert_eq!(response.usage.unwrap().completion_tokens, Some(2));
    assert_eq!(events.len(), 3);
    let seen = &server.requests()[0];
    assert_eq!(
        seen.path,
        "/v1beta/models/gemini-flash-latest:streamGenerateContent?alt=sse"
    );
    assert_eq!(seen.headers["x-goog-api-key"], KEY);
    assert_eq!(seen.headers["accept"], "text/event-stream");
    assert!(!seen.path.contains(KEY));
}

#[test]
fn empty_stream_is_an_error() {
    let server = Server::start(vec![Reply::Sse(String::new())]);
    let results = items(&provider(&server, "g"), &ask("hi"));
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].as_ref().unwrap_err().reason,
        "stream ended without data"
    );
}

#[test]
fn stream_error_event_raises() {
    let error_event = json!({"error": {"code": 429, "message": format!("quota for {KEY}")}});
    let server = Server::start(vec![Reply::Sse(sse(&[chunk("a"), error_event]))]);
    let results = items(&provider(&server, "g"), &ask("hi"));
    assert_eq!(results[0], Ok(StreamEvent::Text("a".to_owned())));
    let error = results[1].as_ref().unwrap_err();
    assert_eq!((error.status, error.retryable), (429, true));
    assert!(!error.reason.contains(KEY));
    let server = Server::start(vec![Reply::Sse("data: nope\n\n".to_owned())]);
    let results = items(&provider(&server, "g"), &ask("hi"));
    assert!(
        results[0]
            .as_ref()
            .unwrap_err()
            .reason
            .starts_with("malformed response")
    );
}

#[test]
fn stream_http_error_is_raised_when_iterating() {
    let server = Server::start(vec![Reply::Json(500, "{}".to_owned())]);
    let provider = provider(&server, "g");
    let mut stream = provider.stream(&ask("hi"));
    assert!(stream.next().unwrap().unwrap_err().retryable);
    assert!(stream.next().is_none());
}
