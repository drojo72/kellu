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
    for (command, milestone) in [("keys", "M3"), ("tools", "M5"), ("lsp", "M6")] {
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

// ---------------------------------------------------------------- config (M1) ----

/// Write `~/.config/kellu/config.toml` under `home` with the given mode.
fn user_config(home: &Path, text: &str, mode: u32) -> std::path::PathBuf {
    let dir = home.join(".config/kellu");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    fs::write(&path, text).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    path
}

#[test]
fn config_requires_an_action() {
    let home = TempDir::new().unwrap();
    let out = kellu(&["config"], home.path(), home.path());
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn config_path_prints_the_user_file_location() {
    let home = TempDir::new().unwrap();
    let out = kellu(&["config", "path"], home.path(), home.path());
    assert_eq!(out.status.code(), Some(0));
    let expected = home.path().join(".config/kellu/config.toml");
    assert_eq!(text(&out.stdout).trim(), expected.display().to_string());
    assert!(text(&out.stderr).contains("does not exist"));
}

#[test]
fn config_path_honors_xdg_config_home() {
    let home = TempDir::new().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_kellu"))
        .args(["config", "path"])
        .env_clear()
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", "/custom/xdg")
        .output()
        .unwrap();
    assert_eq!(text(&out.stdout).trim(), "/custom/xdg/kellu/config.toml");
}

#[test]
fn config_check_with_defaults_succeeds_and_creates_nothing() {
    let home = TempDir::new().unwrap();
    let out = kellu(&["config", "check"], home.path(), home.path());
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        text(&out.stdout).trim(),
        "ok: 5 providers, 3 routes (packaged defaults only)"
    );
    assert_eq!(fs::read_dir(home.path()).unwrap().count(), 0);
}

#[test]
fn config_check_reports_the_user_file_when_it_is_applied() {
    let home = TempDir::new().unwrap();
    let path = user_config(home.path(), "[router]\ntotal_timeout_s = 30\n", 0o600);
    let out = kellu(&["config", "check"], home.path(), home.path());
    assert_eq!(out.status.code(), Some(0));
    assert!(text(&out.stdout).contains(&path.display().to_string()));
}

#[test]
fn config_check_reports_a_rejected_secret_without_echoing_it() {
    let home = TempDir::new().unwrap();
    user_config(
        home.path(),
        "[providers.grok]\napi_key = \"sk-live-0123456789\"\n",
        0o600,
    );
    let out = kellu(&["config", "check"], home.path(), home.path());
    assert_eq!(out.status.code(), Some(1));
    let stderr = text(&out.stderr);
    assert!(stderr.contains("kellu keys set"), "{stderr}");
    assert!(!stderr.contains("sk-live") && !text(&out.stdout).contains("sk-live"));
}

#[test]
fn config_check_rejects_an_unsafe_file() {
    let home = TempDir::new().unwrap();
    user_config(home.path(), "", 0o666);
    let out = kellu(&["config", "check"], home.path(), home.path());
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("writable by group or others"));
}

#[test]
fn config_show_lists_providers_and_routes_without_secrets() {
    let home = TempDir::new().unwrap();
    let out = kellu(&["config", "show"], home.path(), home.path());
    assert_eq!(out.status.code(), Some(0));
    let stdout = text(&out.stdout);
    assert!(
        stdout.contains("providers:") && stdout.contains("routes:"),
        "{stdout}"
    );
    assert!(stdout.contains("deepseek      openai_compat  deepseek-chat  https://api.deepseek.com/v1  (keyring: ai_deepseek_api_key)"), "{stdout}");
    assert!(
        stdout.contains("general       gemma_direct -> google -> deepseek -> openrouter"),
        "{stdout}"
    );
    assert!(
        stdout.contains("router: total time budget 120s"),
        "{stdout}"
    );
}

#[test]
fn config_show_marks_providers_without_a_key() {
    let home = TempDir::new().unwrap();
    user_config(
        home.path(),
        "[providers.local]\nkind = \"openai_compat\"\nbase_url = \"http://localhost:11434/v1\"\nmodel = \"llama\"\ntimeout_s = 120\n",
        0o600,
    );
    let out = kellu(&["config", "show"], home.path(), home.path());
    assert!(text(&out.stdout).contains("(no key)"));
}

#[test]
fn doctor_fails_on_an_invalid_config() {
    let home = TempDir::new().unwrap();
    let bin = TempDir::new().unwrap();
    fake_tool(bin.path(), "secret-tool");
    user_config(home.path(), "mystery = 1\n", 0o600);
    let out = kellu(&["doctor"], home.path(), bin.path());
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stdout).contains("[FAIL] config: config: unknown key(s): mystery"));
}
