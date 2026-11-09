//! The terminal chat, driven through in-memory streams.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::io::Cursor;
use std::sync::atomic::Ordering;

use kellu_core::session::SessionLog;
use kellu_frontends::repl::{MAX_LINE_BYTES, Repl};
use kellu_frontends::theme::{Level, Theme};

struct Run {
    out: String,
    err: String,
    code: u8,
}

fn run(repl: &mut Repl<'_>, input: &[u8]) -> Run {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = repl.run(&mut Cursor::new(input.to_vec()), &mut out, &mut err);
    Run {
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
        code,
    }
}

const KEY: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";

#[test]
fn a_conversation_streams_answers_and_keeps_context_until_cleared() {
    let (assistant, probe) = common::assistant("hello there");
    let mut repl = Repl::new(&assistant, Theme::at(Level::Ascii), None);
    let result = run(&mut repl, b"hi\nand again\n/clear\nfresh\n/exit\nignored\n");
    assert_eq!(result.code, 0);
    assert!(result.out.starts_with("<*> Kellu."), "{}", result.out);
    assert_eq!(result.out.matches("hello there").count(), 3);
    let requests = probe.seen.lock().unwrap();
    assert_eq!(requests.len(), 3, "/exit stops reading");
    // System prompt + user + assistant + user on the second turn; one user message after /clear.
    assert_eq!(requests[1].messages.len(), 4);
    assert_eq!(requests[2].messages.len(), 2);
}

#[test]
fn what_is_typed_is_redacted_on_the_way_out_and_never_echoed_on_failure() {
    let (assistant, probe) = common::assistant("ok");
    let mut repl = Repl::new(&assistant, Theme::at(Level::Ascii), None);
    run(&mut repl, format!("my key is {KEY}\n").as_bytes());
    assert!(!probe.everything_sent().contains(KEY));

    probe.fail.store(true, Ordering::SeqCst);
    let failed = run(&mut repl, b"anything\n");
    assert_eq!(failed.code, 0, "a failed answer is reported, not fatal");
    assert!(failed.err.contains("error:"), "{}", failed.err);
    assert!(!failed.err.contains(KEY) && !failed.out.contains(KEY));
}

#[test]
fn escape_sequences_in_answers_and_input_never_reach_the_terminal_or_the_model() {
    let (assistant, probe) = common::assistant("safe\u{1b}[2J\u{1b}]0;pwned\u{7} text\r");
    let mut repl = Repl::new(&assistant, Theme::at(Level::Ascii), None);
    let result = run(&mut repl, b"he\x1b[31mllo\x08\n");
    assert!(!result.out.contains('\u{1b}') && !result.out.contains('\u{7}'));
    assert!(!result.out.contains('\r'));
    assert!(result.out.contains("safe[2J]0;pwned text"));
    assert!(!probe.everything_sent().contains('\u{1b}'));
}

#[test]
fn only_the_color_level_paints_and_the_rest_is_plain() {
    let (assistant, _) = common::assistant("x");
    let plain = run(
        &mut Repl::new(&assistant, Theme::at(Level::Unicode), None),
        b"/nope\n",
    );
    assert!(!plain.out.contains('\u{1b}') && !plain.err.contains('\u{1b}'));
    assert!(plain.out.contains("\u{2039} \u{25c6} \u{203a}"));
    let color = run(
        &mut Repl::new(&assistant, Theme::at(Level::Color), None),
        b"/nope\n",
    );
    assert!(color.err.contains('\u{1b}'));
}

#[test]
fn commands_and_bad_input_are_handled_without_calling_the_model() {
    let (assistant, probe) = common::assistant("x");
    let mut repl = Repl::new(&assistant, Theme::at(Level::Ascii), None);
    let mut input = b"/help\n\n   \n/frobnicate\x1b[31m now\n".to_vec();
    input.extend(vec![b'z'; MAX_LINE_BYTES * 2]);
    input.extend_from_slice(b"\n/quit\n");
    let result = run(&mut repl, &input);
    assert!(result.out.contains("/clear"));
    assert!(result.err.contains("unknown command /frobnicate"));
    assert!(!result.err.contains('\u{1b}'));
    assert!(result.err.contains("line too long"));
    assert_eq!(probe.calls(), 0);
}

#[test]
fn end_of_input_ends_the_chat_cleanly() {
    let (assistant, _) = common::assistant("x");
    let mut repl = Repl::new(&assistant, Theme::at(Level::Ascii), None);
    assert_eq!(run(&mut repl, b"").code, 0);
    assert_eq!(run(&mut repl, b"no newline at the end").code, 0);
}

#[test]
fn the_session_log_records_the_conversation_without_secrets() {
    let tmp = tempfile::tempdir().unwrap();
    let (assistant, _) = common::assistant("sure");
    let session = SessionLog::new(tmp.path().join("sessions")).label("repl");
    let mut repl = Repl::new(&assistant, Theme::at(Level::Ascii), Some(session));
    run(&mut repl, format!("token {KEY}\n").as_bytes());
    let dir = tmp.path().join("sessions");
    let files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
    assert_eq!(files.len(), 1);
    let text = std::fs::read_to_string(files[0].path()).unwrap();
    assert!(text.contains("sure") && text.contains("token"));
    assert!(!text.contains(KEY), "{text}");
}

#[test]
fn a_session_log_that_cannot_start_is_reported_and_the_chat_goes_on() {
    let tmp = tempfile::tempdir().unwrap();
    let blocked = tmp.path().join("file");
    std::fs::write(&blocked, "x").unwrap();
    let (assistant, probe) = common::assistant("sure");
    let session = SessionLog::new(blocked.join("sessions"));
    let mut repl = Repl::new(&assistant, Theme::at(Level::Ascii), Some(session));
    let result = run(&mut repl, b"hello\n");
    assert!(
        result.err.contains("session log disabled"),
        "{}",
        result.err
    );
    assert_eq!(probe.calls(), 1);
}
