//! `run_command` in a real bubblewrap sandbox, and the road from a command's changes to the
//! project: command -> proposal -> approval -> apply.
//!
//! They need `bwrap` and user namespaces. Where those are missing the tests print `SKIPPED` and
//! pass, unless `KELLU_REQUIRE_SANDBOX_TESTS=1` is set, which turns a skip into a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use common::{Harness, Scripted};
use kellu_sandbox::{Sandbox, SystemLayout};
use kellu_tools::host::HostTools;
use kellu_tools::{Approval, Choice, Confirmation, Operation, apply, approve};
use serde_json::json;

fn find(binary: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(binary))
        .find(|path| path.is_file())
}

fn skip(reason: &str) {
    assert!(
        std::env::var_os("KELLU_REQUIRE_SANDBOX_TESTS").is_none(),
        "the sandbox is required but unavailable: {reason}"
    );
    eprintln!("SKIPPED: {reason}");
}

fn sandbox() -> Option<Sandbox> {
    let Some(bwrap) = find("bwrap") else {
        skip("bwrap is not installed");
        return None;
    };
    let prlimit = find("prlimit");
    match Sandbox::open(&bwrap, prlimit.as_deref(), SystemLayout::host(), false) {
        Ok(sandbox) => Some(sandbox),
        Err(error) => {
            skip(&error.to_string());
            None
        }
    }
}

const FILES: &[&str] = &["src/main.rs", "src/lib.rs", "README.md", ".env"];

fn sh(script: &str) -> serde_json::Value {
    json!({"argv": ["sh", "-c", script]})
}

fn sh_write(script: &str) -> serde_json::Value {
    json!({"argv": ["sh", "-c", script], "write_back": true})
}

fn run(
    h: &Harness,
    sb: &Sandbox,
    grant: &kellu_security::Grant,
    args: serde_json::Value,
) -> kellu_tools::CallResult {
    h.call(
        grant,
        Instant::now(),
        Some(sb),
        &Scripted::new(Confirmation::Denied),
        "run_command",
        args,
    )
}

#[test]
fn a_command_runs_in_the_sandbox_and_its_output_comes_back_clean() {
    let Some(sb) = sandbox() else { return };
    let h = Harness::new(HostTools::none());
    let grant = h.p.grant(FILES, Instant::now());
    let echo = run(&h, &sb, &grant, json!({"argv": ["echo", "hola"]}));
    assert!(!echo.is_error, "{}", echo.text);
    assert!(
        echo.text.contains("$ echo hola")
            && echo.text.contains("exit status 0")
            && echo.text.contains("hola"),
        "{}",
        echo.text
    );
    let escapes = run(&h, &sb, &grant, sh("printf 'a\\033[31mred\\033[0m\\r\\n'"));
    assert!(
        !escapes.text.contains('\u{1b}') && !escapes.text.contains('\r'),
        "{:?}",
        escapes.text
    );
    let failing = run(&h, &sb, &grant, sh("echo oops >&2; exit 3"));
    assert!(
        !failing.is_error
            && failing.text.contains("exit status 3")
            && failing.text.contains("oops"),
        "{}",
        failing.text
    );
}

#[test]
fn without_write_back_the_project_is_read_only_and_never_touched() {
    let Some(sb) = sandbox() else { return };
    let h = Harness::new(HostTools::none());
    let grant = h.p.grant(FILES, Instant::now());
    let result = run(
        &h,
        &sb,
        &grant,
        sh("echo x > created.txt; echo y >> README.md; rm src/lib.rs"),
    );
    assert!(result.proposal.is_none());
    assert!(!h.p.root.join("created.txt").exists());
    assert_eq!(h.p.read("README.md"), "# demo\n");
    assert!(h.p.root.join("src/lib.rs").exists());
}

#[test]
fn what_a_writable_command_changes_becomes_a_proposal_and_not_a_write() {
    let Some(sb) = sandbox() else { return };
    let h = Harness::new(HostTools::none());
    let now = Instant::now();
    let mut grant = h.p.grant(FILES, now);
    let script =
        "echo added >> README.md; echo new > created.txt; rm src/lib.rs; echo TOKEN=x > .env.local";
    let result = run(&h, &sb, &grant, sh_write(script));
    assert!(!result.is_error, "{}", result.text);
    let proposal = result.proposal.clone().expect("a proposal");
    // Nothing happened to the real project yet.
    assert_eq!(h.p.read("README.md"), "# demo\n");
    assert!(!h.p.root.join("created.txt").exists() && h.p.root.join("src/lib.rs").exists());

    let by_path = |name: &str| {
        proposal
            .items()
            .iter()
            .find(|i| i.path().to_str() == Some(name))
            .unwrap_or_else(|| panic!("{name} missing: {}", result.text))
    };
    assert_eq!(by_path("README.md").operation(), Operation::Modify);
    assert_eq!(by_path("created.txt").operation(), Operation::Create);
    assert_eq!(by_path("src/lib.rs").operation(), Operation::Delete);
    assert!(
        proposal
            .items()
            .iter()
            .all(|i| i.path().to_str() != Some(".env.local")),
        "a denied path must not be proposed"
    );
    assert!(
        result.text.contains(".env.local") && result.text.contains("not proposed"),
        "{}",
        result.text
    );

    // The user approves everything and the changes reach the project.
    let session = h.p.session(now);
    let mut approval = Approval::default();
    for item in proposal.items() {
        approval.items.insert(item.id().to_owned(), Choice::Full);
        if item.operation() == Operation::Delete {
            approval
                .typed
                .insert(item.id().to_owned(), item.path().display().to_string());
        }
    }
    let approved = approve(&proposal, &approval, &session, &mut grant).unwrap();
    let report = apply(&proposal, &approved, &session, &grant, &h.p.backups()).unwrap();
    assert!(report.audit_complete);
    assert_eq!(h.p.read("README.md"), "# demo\nadded\n");
    assert_eq!(h.p.read("created.txt"), "new\n");
    assert!(!h.p.root.join("src/lib.rs").exists());
    assert!(!h.p.root.join(".env.local").exists());
}

#[test]
fn links_and_odd_files_made_by_a_command_are_reported_and_never_proposed() {
    let Some(sb) = sandbox() else { return };
    let h = Harness::new(HostTools::none());
    let grant = h.p.grant(FILES, Instant::now());
    let result = run(
        &h,
        &sb,
        &grant,
        sh_write("ln -s /etc/passwd evil; mkfifo pipe; echo ok > fine.txt"),
    );
    let proposal = result.proposal.expect("fine.txt is still proposed");
    let paths: Vec<_> = proposal
        .items()
        .iter()
        .map(|i| i.path().display().to_string())
        .collect();
    assert_eq!(paths, ["fine.txt"], "{}", result.text);
    assert!(
        result.text.contains("evil") && result.text.contains("symlink"),
        "{}",
        result.text
    );
    assert!(result.text.contains("pipe"), "{}", result.text);
}

#[test]
fn a_file_that_changed_in_the_project_while_the_command_ran_is_not_proposed() {
    let Some(sb) = sandbox() else { return };
    let h = Harness::new(HostTools::none());
    let grant = h.p.grant(FILES, Instant::now());
    let target = h.p.root.join("README.md");
    // The command waits; the "user" edits the real file meanwhile.
    let editor = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(600));
        fs::write(target, "# edited by the user\n").unwrap();
    });
    let result = run(
        &h,
        &sb,
        &grant,
        sh_write("echo mine >> README.md; sleep 1.5"),
    );
    editor.join().unwrap();
    assert!(result.proposal.is_none(), "{}", result.text);
    assert!(
        result.text.contains("README.md") && result.text.contains("not proposed"),
        "{}",
        result.text
    );
    assert_eq!(h.p.read("README.md"), "# edited by the user\n");
}

#[test]
fn the_command_never_gets_the_network_even_when_the_session_allows_it() {
    let Some(sb) = sandbox() else { return };
    let h = Harness::new(HostTools::none());
    let mut grant = h.p.grant(FILES, Instant::now());
    grant.allow_network();
    let result = run(&h, &sb, &grant, sh("cat /proc/net/dev"));
    assert!(!result.is_error, "{}", result.text);
    let interfaces: Vec<&str> = result
        .text
        .lines()
        .filter_map(|l| l.split(':').next())
        .map(str::trim)
        .filter(|name| {
            !name.is_empty()
                && !name.starts_with('$')
                && !name.contains(' ')
                && !name.contains('|')
                && !name.contains("---")
                && !name.contains("exit")
        })
        .collect();
    assert_eq!(
        interfaces,
        ["lo"],
        "only loopback may exist: {}",
        result.text
    );
}

#[test]
fn secrets_in_the_callers_environment_do_not_reach_the_command() {
    let Some(sb) = sandbox() else { return };
    let h = Harness::new(HostTools::none());
    let grant = h.p.grant(FILES, Instant::now());
    let result = run(&h, &sb, &grant, sh("env"));
    for leaked in [
        "KELLU",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "SSH_AUTH_SOCK",
        "AWS_",
        "OPENAI",
        "ANTHROPIC",
    ] {
        assert!(
            !result.text.contains(leaked),
            "{leaked} reached the command:\n{}",
            result.text
        );
    }
    assert!(
        !result.text.contains(".env"),
        "denied files are not even copied"
    );
    let cat = run(&h, &sb, &grant, sh("cat .env 2>&1; ls -a"));
    assert!(!cat.text.contains("do-not-read"), "{}", cat.text);
}

#[test]
fn the_timeout_is_honoured_and_never_raised_by_the_model() {
    let Some(sb) = sandbox() else { return };
    let h = Harness::new(HostTools::none());
    let grant = h.p.grant(FILES, Instant::now());
    let started = Instant::now();
    let result = run(
        &h,
        &sb,
        &grant,
        json!({"argv": ["sleep", "60"], "timeout_s": 1}),
    );
    assert!(result.text.contains("timed out"), "{}", result.text);
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
}

#[test]
fn commands_are_argument_lists_and_bad_arguments_are_refused() {
    let Some(sb) = sandbox() else { return };
    let h = Harness::new(HostTools::none());
    let grant = h.p.grant(FILES, Instant::now());
    for args in [
        json!({}),
        json!({"argv": []}),
        json!({"argv": "ls -la"}),
        json!({"argv": [1, 2]}),
        json!({"argv": ["echo", "a\u{0}b"]}),
        json!({"argv": ["echo"], "network": true}),
        json!({"argv": ["echo"], "write_back": "yes"}),
    ] {
        assert!(run(&h, &sb, &grant, args.clone()).is_error, "{args}");
    }
    // A `;` is just a character: there is no shell to interpret it.
    let result = run(&h, &sb, &grant, json!({"argv": ["echo", "a; touch pwned"]}));
    assert!(result.text.contains("a; touch pwned") && !h.p.root.join("pwned").exists());
}

#[test]
fn without_a_working_sandbox_commands_do_not_run_at_all() {
    let h = Harness::new(HostTools::none());
    let grant = h.p.grant(FILES, Instant::now());
    let result = h.call(
        &grant,
        Instant::now(),
        None,
        &Scripted::new(Confirmation::Denied),
        "run_command",
        json!({"argv": ["echo", "x"]}),
    );
    assert!(
        result.is_error && result.text.contains("no working sandbox"),
        "{}",
        result.text
    );
}
