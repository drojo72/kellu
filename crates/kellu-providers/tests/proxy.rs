//! Plain-http endpoints (localhost only) must never go through a proxy from the environment:
//! a proxy would see the API key and the prompt in cleartext.
//!
//! Setting environment variables needs `unsafe`, which the workspace forbids, so the test
//! re-runs this test binary as a child process with a dead proxy in its environment.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::process::Command;

use kellu_providers::openai_compat::OpenAiCompatProvider;
use kellu_providers::{Provider, ProviderKind};
use serde_json::json;
use support::{KEY, Reply, Server, ask, config, lookup};

const CHILD_FLAG: &str = "KELLU_PROXY_TEST_CHILD";

#[test]
fn plain_http_ignores_the_proxy_environment() {
    if std::env::var_os(CHILD_FLAG).is_none() {
        let dead_proxy = "http://127.0.0.1:9";
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "plain_http_ignores_the_proxy_environment",
                "--nocapture",
            ])
            .env(CHILD_FLAG, "1")
            .env("HTTP_PROXY", dead_proxy)
            .env("http_proxy", dead_proxy)
            .env("ALL_PROXY", dead_proxy)
            .env("all_proxy", dead_proxy)
            .env_remove("NO_PROXY")
            .env_remove("no_proxy")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "the child run failed (the request was probably sent to the proxy):\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    }
    // Child: the dead proxy is in the environment; a direct connection must still work.
    let body = json!({"choices": [{"message": {"content": "direct"}}]}).to_string();
    let server = Server::start(vec![Reply::Json(200, body)]);
    let config = config(
        ProviderKind::OpenAiCompat,
        &server.url("/v1"),
        "m",
        Some("test"),
    );
    let provider = OpenAiCompatProvider::new(config, lookup(Some(KEY)));
    assert_eq!(provider.chat(&ask("hi")).unwrap().text, "direct");
    assert_eq!(server.requests().len(), 1);
}
