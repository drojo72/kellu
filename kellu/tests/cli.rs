//! End-to-end tests of the real binary (`CARGO_BIN_EXE_kellu`), no extra dependencies.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

/// Run `kellu` with a controlled environment so the real home and PATH are never involved.
fn kellu(args: &[&str], home: &Path, path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kellu"))
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("PATH", path)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn fake_tool(dir: &Path, name: &str) {
    let path = dir.join(name);
    fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn version_flag() {
    let home = TempDir::new().unwrap();
    let out = kellu(&["--version"], home.path(), home.path());
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        text(&out.stdout).trim(),
        format!("kellu {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn no_command_prints_help() {
    let home = TempDir::new().unwrap();
    let out = kellu(&[], home.path(), home.path());
    assert_eq!(out.status.code(), Some(0));
    assert!(text(&out.stdout).contains("doctor"));
    assert!(text(&out.stderr).contains("M6"));
}

#[test]
fn unknown_command_is_a_usage_error() {
    let home = TempDir::new().unwrap();
    let out = kellu(&["frobnicate"], home.path(), home.path());
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn planned_commands_report_not_implemented() {
    let home = TempDir::new().unwrap();
    for (command, milestone) in [
        ("config", "M1"),
        ("keys", "M3"),
        ("tools", "M5"),
        ("lsp", "M6"),
    ] {
        let out = kellu(&[command], home.path(), home.path());
        assert_eq!(out.status.code(), Some(3), "{command}");
        let stderr = text(&out.stderr);
        assert!(
            stderr.contains("not implemented yet") && stderr.contains(milestone),
            "{stderr}"
        );
    }
}

#[test]
fn doctor_fails_without_the_required_tool_and_creates_nothing() {
    let home = TempDir::new().unwrap();
    let empty_path = TempDir::new().unwrap();
    let out = kellu(&["doctor"], home.path(), empty_path.path());
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stdout).contains("[FAIL] secret-tool"));
    assert_eq!(
        fs::read_dir(home.path()).unwrap().count(),
        0,
        "doctor must not create files"
    );
}

#[test]
fn doctor_passes_with_the_required_tool_and_only_warns_about_the_rest() {
    let home = TempDir::new().unwrap();
    let bin = TempDir::new().unwrap();
    fake_tool(bin.path(), "secret-tool");
    let out = kellu(&["doctor"], home.path(), bin.path());
    assert_eq!(out.status.code(), Some(0));
    let stdout = text(&out.stdout);
    assert!(stdout.contains("[  ok] secret-tool"), "{stdout}");
    assert!(stdout.contains("[warn] bwrap"), "{stdout}");
    assert!(stdout.contains(".config/kellu"), "{stdout}");
}

#[test]
fn doctor_ignores_a_non_executable_file_named_like_the_tool() {
    let home = TempDir::new().unwrap();
    let bin = TempDir::new().unwrap();
    fs::write(bin.path().join("secret-tool"), "not executable").unwrap();
    let out = kellu(&["doctor"], home.path(), bin.path());
    assert_eq!(out.status.code(), Some(1));
}
