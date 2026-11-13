//! An editor sees only an empty answer, so the server must say why in the private log.
//! This file has its own test binary because it installs the process-wide logger.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::fs;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use kellu_frontends::lsp::{Features, Server};
use serde_json::{Value, json};

static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        LINES.lock().unwrap().push(format!("{}", record.args()));
    }
    fn flush(&self) {}
}

fn lines() -> Vec<String> {
    LINES.lock().unwrap().clone()
}

fn uri(path: &std::path::Path) -> String {
    format!("file://{}", path.display())
}

#[test]
fn every_silent_empty_answer_leaves_a_reason_once() {
    log::set_logger(&Capture).unwrap();
    log::set_max_level(log::LevelFilter::Warn);

    let tmp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(tmp.path()).unwrap();
    let home = base.join("home");
    let proj = base.join("proj");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(proj.join(".git")).unwrap();
    let file = proj.join("a.rs");
    fs::write(&file, "fn alpha() {}\n").unwrap();
    let outside = home.join("n.rs");
    fs::write(&outside, "fn beta() {}\n").unwrap();

    let (assistant, probe) = common::assistant("");
    let mut server = Server::new(
        &assistant,
        Features {
            hover: true,
            completion: true,
        },
        home,
    );
    server.handle(
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri": uri(&proj)}}),
    );
    let open = |server: &mut Server<'_>, path: &std::path::Path, text: &str| {
        server.handle(
            &json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
            "textDocument":{"uri": uri(path),"languageId":"rust","version":1,"text":text}}}),
        );
    };
    let ask = |server: &mut Server<'_>, method: &str, path: &std::path::Path| -> Value {
        server
            .handle(&json!({"jsonrpc":"2.0","id":9,"method":method,"params":{
                "textDocument":{"uri": uri(path)},"position":{"line":0,"character":4}}}))
            .0
            .remove(0)
    };

    // A document outside the project: refused, and the reason names it. Asked three times, written once.
    open(&mut server, &outside, "fn beta() {}\n");
    for _ in 0..3 {
        ask(&mut server, "textDocument/hover", &outside);
    }
    let refused: Vec<_> = lines()
        .into_iter()
        .filter(|l| l.contains("refused"))
        .collect();
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(
        refused[0].contains("hover") && refused[0].contains("n.rs"),
        "{refused:?}"
    );

    // An allowed document with a model that says nothing.
    open(&mut server, &file, "fn alpha() {}\n");
    ask(&mut server, "textDocument/hover", &file);
    ask(&mut server, "textDocument/completion", &file);
    let all = lines().join("\n");
    assert!(
        all.contains("hover: the model returned an empty answer"),
        "{all}"
    );
    assert!(
        all.contains("completion: the model returned an empty answer"),
        "{all}"
    );

    // A model that fails.
    probe.fail.store(true, Ordering::SeqCst);
    server.handle(&json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{
        "textDocument":{"uri": uri(&file),"version":2},"contentChanges":[{"text":"fn gamma() {}\n"}]}}));
    ask(&mut server, "textDocument/hover", &file);
    assert!(
        lines()
            .join("\n")
            .contains("hover: the model request failed")
    );

    // Nothing in the log carries the document text.
    assert!(!lines().join("\n").contains("fn alpha"));
}
