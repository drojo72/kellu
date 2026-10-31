//! The process-wide logger can only be tested in its own process (one global per binary).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;
use std::sync::Arc;

use kellu_common::text::Redactor;
use kellu_core::log::{LOG_FILENAME, LogLevel, LogSettings, configure_logging, no_redaction};
use tempfile::TempDir;

fn lines(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join(LOG_FILENAME))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn configuring_twice_replaces_the_sink_and_does_not_duplicate_output() {
    let root = TempDir::new().unwrap();
    let first = root.path().join("first");
    let second = root.path().join("second");

    configure_logging(&first, LogSettings::default(), no_redaction()).unwrap();
    log::info!(target: "kellu_core::test", "before");

    let redact: Arc<dyn Redactor> = Arc::new(|text: &str| text.replace("hunter2", "***"));
    let settings = LogSettings {
        level: LogLevel::Warning,
        ..LogSettings::default()
    };
    configure_logging(&second, settings, redact).unwrap();
    log::info!(target: "kellu_core::test", "dropped by the new level");
    log::warn!(target: "kellu_core::test", "after hunter2");
    log::error!(target: "some_dependency", "not ours");

    assert_eq!(lines(&first).len(), 1);
    assert!(lines(&first)[0].ends_with("INFO kellu_core::test: before"));
    let second_lines = lines(&second);
    assert_eq!(second_lines.len(), 1, "{second_lines:?}");
    assert!(second_lines[0].ends_with("WARNING kellu_core::test: after ***"));
}
