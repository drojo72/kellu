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
}

#[test]
fn unknown_command_is_a_usage_error() {
    let home = TempDir::new().unwrap();
    let out = kellu(&["frobnicate"], home.path(), home.path());
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn the_real_lsp_binary_speaks_only_lsp_on_stdout() {
    use std::io::Write;
    use std::process::Stdio;

    let home = TempDir::new().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_kellu"))
        .arg("lsp")
        .env_clear()
        .env("HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("PATH", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let frame = |body: &str| format!("Content-Length: {}\r\n\r\n{body}", body.len());
    let mut stdin = child.stdin.take().unwrap();
    for body in [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#,
        r#"{"jsonrpc":"2.0","method":"exit"}"#,
    ] {
        stdin.write_all(frame(body).as_bytes()).unwrap();
    }
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(stdout.starts_with("Content-Length: "), "{stdout}");
    assert_eq!(stdout.matches("Content-Length: ").count(), 2, "{stdout}");
    assert!(
        !stdout.contains("hoverProvider"),
        "features are off by default"
    );
    assert!(text(&out.stderr).contains("keyring unavailable"));
}

#[test]
fn the_real_chat_binary_leaves_at_the_end_of_input() {
    use std::io::Write;
    use std::process::Stdio;

    let home = TempDir::new().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_kellu"))
        .arg("chat")
        .env_clear()
        .env("HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("PATH", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"/help\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(
        stdout.contains("<*> Kellu.") && stdout.contains("/clear"),
        "{stdout}"
    );
    assert!(
        !stdout.contains('\u{1b}'),
        "output is not a terminal: no colors"
    );
}

#[test]
fn tools_lists_every_tool_with_its_risk_and_creates_nothing() {
    let home = TempDir::new().unwrap();
    let out = kellu(&["tools"], home.path(), home.path());
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let listing = text(&out.stdout);
    for (name, risk) in [
        ("list_files", "read"),
        ("run_command", "read"),
        ("git_commit", "write"),
        ("git_push", "network"),
        ("gh_pr_create", "network"),
    ] {
        let line = listing
            .lines()
            .find(|l| l.starts_with(name))
            .unwrap_or_else(|| panic!("{name}: {listing}"));
        assert!(line.contains(risk), "{line}");
    }
    assert!(
        listing.contains("only with network permission"),
        "{listing}"
    );
    assert!(listing.contains("never written by a tool"), "{listing}");
    assert_eq!(
        std::fs::read_dir(home.path()).unwrap().count(),
        0,
        "listing the tools must not create anything"
    );
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
        "ok: 6 providers, 3 routes (packaged defaults only)"
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
        stdout.contains("general       gemma_direct -> groq -> openrouter"),
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

// ---- kellu keys, against a fake `secret-tool` that keeps its state in files ----

/// A fake `secret-tool`. State lives next to the script: `value`, `argv` and the JSON that the
/// fake `busctl` answers with (listing goes through D-Bus, never through `secret-tool`).
const FAKE_SECRET_TOOL: &str = r#"#!/bin/sh
PATH=/usr/bin:/bin
D="${0%/*}"
printf '%s\n' "$*" >> "$D/argv"
case "$1" in
  lookup) [ -f "$D/value" ] && cat "$D/value" || exit 1 ;;
  store)
    V=$(cat)
    printf '%s' "$V" > "$D/value"
    printf '{"type":"aoao","data":[["/org/freedesktop/secrets/collection/login/1"],[]]}' > "$D/items.json"
    printf '{"type":"a{ss}","data":{"kellu":"%s"}}' "$5" > "$D/attrs-1" ;;
  clear) rm -f "$D/value" "$D/items.json" "$D/attrs-1" ;;
esac
"#;

/// A fake `busctl` that answers `SearchItems` and `get-property` from the files above.
const FAKE_BUSCTL: &str = r#"#!/bin/sh
PATH=/usr/bin:/bin
D="${0%/*}"
printf '%s\n' "$*" >> "$D/busctl_argv"
case "$3" in
  call) cat "$D/items.json" 2>/dev/null || printf '{"type":"aoao","data":[[],[]]}' ;;
  get-property) cat "$D/attrs-$(basename "$5")" ;;
esac
"#;

struct Keyring {
    home: TempDir,
    bin: TempDir,
}

impl Keyring {
    fn new() -> Self {
        let bin = TempDir::new().unwrap();
        for (name, body) in [("secret-tool", FAKE_SECRET_TOOL), ("busctl", FAKE_BUSCTL)] {
            let path = bin.path().join(name);
            fs::write(&path, body).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self {
            home: TempDir::new().unwrap(),
            bin,
        }
    }

    fn run(&self, args: &[&str], stdin: &str) -> Output {
        use std::io::Write;
        use std::process::Stdio;
        let mut child = Command::new(env!("CARGO_BIN_EXE_kellu"))
            .args(args)
            .env_clear()
            .env("HOME", self.home.path())
            .env("PATH", self.bin.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // The binary may exit without reading stdin (that is the point of some tests), so a
        // broken pipe here is expected and not an error. Dropping the handle sends EOF.
        let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
        child.wait_with_output().unwrap()
    }

    fn file(&self, name: &str) -> String {
        fs::read_to_string(self.bin.path().join(name)).unwrap_or_default()
    }
}

const SECRET: &str = "sk-or-test-0123456789abcdef";

#[test]
fn keys_set_reads_the_secret_from_stdin_and_never_prints_or_passes_it() {
    let keyring = Keyring::new();
    let out = keyring.run(&["keys", "set", "openrouter"], &format!("{SECRET}\n"));
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("stored ai_openrouter_api_key (provider)"));
    for stream in [&out.stdout, &out.stderr] {
        assert!(!text(stream).contains(SECRET));
    }
    assert!(
        !keyring.file("argv").contains(SECRET),
        "the secret reached argv"
    );
    assert_eq!(keyring.file("value"), SECRET);
}

#[test]
fn keys_list_shows_names_and_tiers_but_never_values() {
    let keyring = Keyring::new();
    keyring.run(&["keys", "set", "openrouter"], &format!("{SECRET}\n"));
    let out = keyring.run(&["keys", "list"], "");
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(
        stdout.contains("provider  ai_openrouter_api_key"),
        "{stdout}"
    );
    assert!(!stdout.contains(SECRET) && !text(&out.stderr).contains(SECRET));
    // Listing used D-Bus only: `secret-tool` ran once (the store), never a search.
    assert_eq!(
        keyring.file("argv").lines().count(),
        1,
        "{}",
        keyring.file("argv")
    );
    assert!(!keyring.file("busctl_argv").contains("GetSecret"));
}

#[test]
fn keys_list_explains_how_to_get_busctl_when_it_is_missing() {
    let keyring = Keyring::new();
    fs::remove_file(keyring.bin.path().join("busctl")).unwrap();
    let out = keyring.run(&["keys", "list"], "");
    assert_eq!(out.status.code(), Some(1));
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("busctl") && stderr.contains("systemd"),
        "{stderr}"
    );
    // Everything else still works without it.
    let set = keyring.run(&["keys", "set", "openrouter"], &format!("{SECRET}\n"));
    assert_eq!(set.status.code(), Some(0), "{}", text(&set.stderr));
}

#[test]
fn keys_rm_removes_the_entry() {
    let keyring = Keyring::new();
    keyring.run(&["keys", "set", "openrouter"], &format!("{SECRET}\n"));
    let out = keyring.run(&["keys", "rm", "openrouter"], "");
    assert_eq!(out.status.code(), Some(0));
    assert!(text(&out.stdout).contains("removed ai_openrouter_api_key"));
    assert_eq!(keyring.file("value"), "");
}

#[test]
fn keys_set_rejects_bad_names_before_reading_anything() {
    let keyring = Keyring::new();
    for bad in ["Bad Name", "../x", "x;rm"] {
        let out = keyring.run(&["keys", "set", bad], &format!("{SECRET}\n"));
        assert_eq!(out.status.code(), Some(1), "{bad}");
        assert!(!text(&out.stderr).contains(SECRET));
    }
    assert_eq!(keyring.file("argv"), "", "the tool must not have been run");
}

#[test]
fn keys_set_refuses_an_empty_or_unsafe_secret() {
    let keyring = Keyring::new();
    for bad in ["", "\n", "two\tparts\n", "esc\u{1b}[0m\n"] {
        let out = keyring.run(&["keys", "set", "openrouter"], bad);
        assert_eq!(out.status.code(), Some(1), "{bad:?}");
    }
    assert_eq!(keyring.file("value"), "");
}

#[test]
fn a_secret_given_as_an_argument_is_a_usage_error_and_is_not_stored() {
    let keyring = Keyring::new();
    let out = keyring.run(&["keys", "set", "openrouter", SECRET], "");
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(keyring.file("value"), "");
    assert_eq!(keyring.file("argv"), "");
}

#[test]
fn keys_without_a_subcommand_is_a_usage_error() {
    let keyring = Keyring::new();
    assert_eq!(keyring.run(&["keys"], "").status.code(), Some(2));
}

#[test]
fn keys_fail_clearly_without_secret_tool() {
    let home = TempDir::new().unwrap();
    let empty = TempDir::new().unwrap();
    let out = kellu(&["keys", "list"], home.path(), empty.path());
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("secret-tool not found"),
        "{}",
        text(&out.stderr)
    );
}
