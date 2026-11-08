//! Git and `gh` on the host, against real repositories (including hostile ones).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use common::{Harness, Scripted};
use kellu_tools::Confirmation;
use kellu_tools::host::HostTools;
use serde_json::json;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("/usr/bin/git")
        .args(args)
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/nonexistent")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A harness whose project is a real repository with one commit that includes a tracked `.env`.
fn repo() -> Harness {
    let h = Harness::new(HostTools::with(Some("/usr/bin/git".into()), None));
    let root = h.p.root.clone();
    fs::remove_dir_all(root.join(".git")).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "t@example.com"),
        ("commit.gpgsign", "false"),
    ] {
        git(&root, &["config", key, value]);
    }
    git(&root, &["add", "-f", "."]);
    git(&root, &["commit", "-q", "-m", "initial"]);
    h
}

const FILES: &[&str] = &["src/main.rs", "src/lib.rs", "README.md"];

fn run(
    h: &Harness,
    grant: &kellu_security::Grant,
    name: &str,
    args: serde_json::Value,
    reply: Confirmation,
) -> kellu_tools::CallResult {
    h.call(
        grant,
        Instant::now(),
        None,
        &Scripted::new(reply),
        name,
        args,
    )
}

#[test]
fn status_diff_and_log_work_and_hide_what_the_deny_list_hides() {
    let h = repo();
    let now = Instant::now();
    let grant = h.p.grant(FILES, now);
    fs::write(h.p.root.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(h.p.root.join(".env"), "TOKEN=leaked-value-123\n").unwrap();
    fs::write(h.p.root.join("untracked.txt"), "new\n").unwrap();

    let status = run(&h, &grant, "git_status", json!({}), Confirmation::Denied);
    assert!(!status.is_error, "{}", status.text);
    assert!(
        status.text.contains("branch: main")
            && status.text.contains(" M src/main.rs")
            && status.text.contains("?? untracked.txt"),
        "{}",
        status.text
    );
    assert!(
        !status.text.contains(".env") && status.text.contains("hidden by policy"),
        "{}",
        status.text
    );

    let diff = run(&h, &grant, "git_diff", json!({}), Confirmation::Denied);
    assert!(
        diff.text.contains("-    println!") && diff.text.contains("src/main.rs"),
        "{}",
        diff.text
    );
    assert!(
        !diff.text.contains("leaked-value-123") && !diff.text.contains(".env"),
        "{}",
        diff.text
    );
    let only_env = run(
        &h,
        &grant,
        "git_diff",
        json!({"paths": [".env"]}),
        Confirmation::Denied,
    );
    assert!(only_env.is_error && !only_env.text.contains("leaked-value-123"));

    let log = run(
        &h,
        &grant,
        "git_log",
        json!({"max": 5}),
        Confirmation::Denied,
    );
    assert!(log.text.contains("Test initial"), "{}", log.text);
    assert!(
        run(
            &h,
            &grant,
            "git_log",
            json!({"path": "../x"}),
            Confirmation::Denied
        )
        .is_error
    );
    assert!(
        run(
            &h,
            &grant,
            "git_status",
            json!({"x": 1}),
            Confirmation::Denied
        )
        .is_error
    );
}

#[test]
fn staged_diffs_and_path_filters_work() {
    let h = repo();
    let now = Instant::now();
    let grant = h.p.grant(FILES, now);
    fs::write(h.p.root.join("README.md"), "# changed\n").unwrap();
    fs::write(h.p.root.join("src/lib.rs"), "// changed\n").unwrap();
    git(&h.p.root, &["add", "README.md"]);
    let staged = run(
        &h,
        &grant,
        "git_diff",
        json!({"staged": true}),
        Confirmation::Denied,
    );
    assert!(
        staged.text.contains("README.md") && !staged.text.contains("src/lib.rs"),
        "{}",
        staged.text
    );
    let one = run(
        &h,
        &grant,
        "git_diff",
        json!({"paths": ["src/lib.rs"]}),
        Confirmation::Denied,
    );
    assert!(
        one.text.contains("src/lib.rs") && !one.text.contains("README.md"),
        "{}",
        one.text
    );
}

/// Everything a repository's own configuration can name to make git run something.
fn plant(root: &Path, marker_dir: &Path) {
    let m = |name: &str| marker_dir.join(name).display().to_string();
    fs::write(
        root.join(".gitattributes"),
        "*.rs diff=evil\n*.md filter=evil\n",
    )
    .unwrap();
    for (key, value) in [
        ("core.fsmonitor", format!("touch {}; echo", m("FSMONITOR"))),
        ("core.pager", format!("touch {}; cat", m("PAGER"))),
        ("diff.external", format!("touch {}", m("EXTERNAL"))),
        (
            "diff.evil.textconv",
            format!("touch {}; cat", m("TEXTCONV")),
        ),
    ] {
        git(root, &["config", key, &value]);
    }
}

#[test]
fn a_repository_that_names_programs_for_git_to_run_does_not_get_them_run() {
    let h = repo();
    let now = Instant::now();
    let grant = h.p.grant(FILES, now);
    let markers = h.p.state.join("markers");
    fs::create_dir_all(&markers).unwrap();
    plant(&h.p.root, &markers);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    fs::write(
        h.p.root.join("src/main.rs"),
        "fn main() { /* changed */ }\n",
    )
    .unwrap();
    fs::write(h.p.root.join("src/lib.rs"), "// changed\n").unwrap();

    // The settings that `-c` can switch off are switched off, and the tools still work.
    for (name, args) in [
        ("git_status", json!({})),
        ("git_diff", json!({})),
        ("git_log", json!({})),
    ] {
        let result = run(&h, &grant, name, args, Confirmation::Denied);
        assert!(!result.is_error, "{name}: {}", result.text);
    }
    let ran: Vec<String> = fs::read_dir(&markers)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        ran.is_empty(),
        "git ran a program named by the repository: {ran:?}"
    );
}

#[test]
fn a_filter_driver_in_the_repository_config_stops_every_git_tool() {
    // `-c` cannot switch off a filter whose name the repository chooses, so the tools refuse
    // before running git at all. Without this guard `git add` and `git status` ran it.
    let h = repo();
    let now = Instant::now();
    let grant = h.p.grant(FILES, now);
    let marker = h.p.state.join("FILTER_RAN");
    fs::write(h.p.root.join(".gitattributes"), "*.md filter=evil\n").unwrap();
    git(
        &h.p.root,
        &[
            "config",
            "filter.evil.clean",
            &format!("touch {}; cat", marker.display()),
        ],
    );
    git(&h.p.root, &["config", "filter.evil.smudge", "cat"]);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    fs::write(h.p.root.join("README.md"), "# changed\n").unwrap();

    for (name, args, reply) in [
        ("git_status", json!({}), Confirmation::Denied),
        ("git_diff", json!({}), Confirmation::Denied),
        ("git_log", json!({}), Confirmation::Denied),
        (
            "git_commit",
            json!({"message": "m", "paths": ["README.md"]}),
            Confirmation::Confirmed,
        ),
    ] {
        let result = run(&h, &grant, name, args, reply);
        assert!(
            result.is_error && result.text.contains("filter.evil.clean"),
            "{name}: {}",
            result.text
        );
    }
    assert!(!marker.exists(), "the filter program ran");
}

#[test]
fn includes_and_other_program_hooks_in_the_repository_config_are_refused_too() {
    for (key, value) in [
        ("include.path", "/tmp/whatever"),
        ("core.sshCommand", "ssh -v"),
        ("gpg.program", "/tmp/gpg"),
        ("core.askPass", "/tmp/ask"),
    ] {
        let h = repo();
        let grant = h.p.grant(FILES, Instant::now());
        git(&h.p.root, &["config", key, value]);
        let result = run(&h, &grant, "git_status", json!({}), Confirmation::Denied);
        assert!(
            result.is_error && result.text.to_lowercase().contains(&key.to_lowercase()),
            "{key}: {}",
            result.text
        );
    }
}

#[test]
fn a_directory_that_is_not_a_repository_is_an_error_not_a_crash() {
    let h = repo();
    fs::remove_dir_all(h.p.root.join(".git")).unwrap();
    let grant = h.p.grant(FILES, Instant::now());
    let result = run(&h, &grant, "git_status", json!({}), Confirmation::Denied);
    assert!(
        result.is_error && result.text.contains("not a usable git repository"),
        "{}",
        result.text
    );
}

#[test]
fn a_missing_git_is_reported() {
    let h = Harness::new(HostTools::none());
    let grant = h.p.grant(FILES, Instant::now());
    let result = run(&h, &grant, "git_status", json!({}), Confirmation::Denied);
    assert!(
        result.is_error && result.text.contains("git was not found"),
        "{}",
        result.text
    );
}

#[test]
fn a_commit_takes_only_the_named_paths_asks_first_and_never_runs_hooks() {
    let h = repo();
    let now = Instant::now();
    let grant = h.p.grant(FILES, now);
    let hook = h.p.root.join(".git/hooks/pre-commit");
    let hook_marker = h.p.state.join("HOOK_RAN");
    fs::write(
        &hook,
        format!("#!/bin/sh\ntouch {}\n", hook_marker.display()),
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(h.p.root.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(h.p.root.join("src/lib.rs"), "// not part of this commit\n").unwrap();
    fs::write(h.p.root.join("src/added.rs"), "pub fn added() {}\n").unwrap();

    let args =
        json!({"message": "-n tricky subject\n\nbody", "paths": ["src/main.rs", "src/added.rs"]});
    let no = Scripted::new(Confirmation::Denied);
    let refused = h.call(&grant, now, None, &no, "git_commit", args.clone());
    assert!(refused.is_error);
    let shown = no.asked.borrow()[0].clone();
    assert!(
        shown.contains("src/main.rs src/added.rs")
            && shown.contains("hooks disabled")
            && shown.contains("-n tricky subject"),
        "{shown}"
    );
    assert!(
        git(&h.p.root, &["log", "--oneline"]).lines().count() == 1,
        "declined means no commit"
    );

    let yes = Scripted::new(Confirmation::Confirmed);
    let done = h.call(&grant, now, None, &yes, "git_commit", args);
    assert!(!done.is_error, "{}", done.text);
    assert_eq!(
        git(&h.p.root, &["log", "-1", "--format=%s"]).trim(),
        "-n tricky subject"
    );
    let committed = git(&h.p.root, &["show", "--name-only", "--format=", "HEAD"]);
    assert!(
        committed.contains("src/main.rs")
            && committed.contains("src/added.rs")
            && !committed.contains("src/lib.rs"),
        "{committed}"
    );
    assert!(git(&h.p.root, &["status", "--porcelain"]).contains("src/lib.rs"));
    assert!(!hook_marker.exists(), "hooks must not run");
}

#[test]
fn a_commit_refuses_denied_or_escaping_paths_and_blank_messages_before_asking() {
    let h = repo();
    let now = Instant::now();
    let grant = h.p.grant(FILES, now);
    fs::write(h.p.root.join(".env"), "TOKEN=changed\n").unwrap();
    let ask = Scripted::new(Confirmation::Confirmed);
    for args in [
        json!({"message": "m", "paths": [".env"]}),
        json!({"message": "m", "paths": ["../outside/secret.txt"]}),
        json!({"message": "m", "paths": ["/etc/passwd"]}),
        json!({"message": "m", "paths": [".git/config"]}),
        json!({"message": "m", "paths": [":(top)README.md"]}),
        json!({"message": "m", "paths": []}),
        json!({"message": "   ", "paths": ["README.md"]}),
        json!({"message": "m", "paths": ["README.md"], "amend": true}),
    ] {
        let result = h.call(&grant, now, None, &ask, "git_commit", args.clone());
        assert!(result.is_error, "{args}: {}", result.text);
    }
    assert_eq!(git(&h.p.root, &["log", "--oneline"]).lines().count(), 1);
    assert!(
        git(&h.p.root, &["status", "--porcelain"]).contains(".env"),
        ".env must stay uncommitted"
    );
}

fn remote(h: &Harness) -> PathBuf {
    let bare = h.p.state.join("remote.git");
    git(
        &h.p.state,
        &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()],
    );
    git(
        &h.p.root,
        &["remote", "add", "origin", bare.to_str().unwrap()],
    );
    bare
}

#[test]
fn a_push_needs_network_permission_then_confirmation_and_never_forces() {
    let h = repo();
    let now = Instant::now();
    let bare = remote(&h);
    let mut grant = h.p.grant(FILES, now);
    let yes = Scripted::new(Confirmation::Confirmed);

    let denied = h.call(
        &grant,
        now,
        None,
        &yes,
        "git_push",
        json!({"branch": "main"}),
    );
    assert!(denied.is_error && denied.text.contains("network") && yes.asked.borrow().is_empty());
    assert!(git(&bare, &["branch", "--list"]).trim().is_empty());

    grant.allow_network();
    let no = Scripted::new(Confirmation::Denied);
    assert!(
        h.call(
            &grant,
            now,
            None,
            &no,
            "git_push",
            json!({"branch": "main"})
        )
        .is_error
    );
    assert!(no.asked.borrow()[0].contains("HEAD:refs/heads/main (no force)"));
    assert!(git(&bare, &["branch", "--list"]).trim().is_empty());

    let pushed = h.call(
        &grant,
        now,
        None,
        &yes,
        "git_push",
        json!({"branch": "main"}),
    );
    assert!(!pushed.is_error, "{}", pushed.text);
    assert!(git(&bare, &["branch", "--list"]).contains("main"));

    // History that diverged from the remote is rejected: there is no way to force it.
    git(&h.p.root, &["commit", "-q", "--amend", "-m", "rewritten"]);
    let rewritten = h.call(
        &grant,
        now,
        None,
        &yes,
        "git_push",
        json!({"branch": "main"}),
    );
    assert!(
        rewritten.is_error,
        "a non-fast-forward push must fail: {}",
        rewritten.text
    );
    assert_eq!(
        git(&bare, &["log", "-1", "--format=%s", "main"]).trim(),
        "initial"
    );

    for bad in [
        "--force",
        "+main",
        "main:other",
        "-f",
        "a..b",
        "x.lock",
        "refs/heads/../x",
        "",
    ] {
        let result = h.call(&grant, now, None, &yes, "git_push", json!({"branch": bad}));
        assert!(result.is_error, "{bad:?}");
    }
    assert!(
        h.call(
            &grant,
            now,
            None,
            &yes,
            "git_push",
            json!({"branch": "main", "force": true})
        )
        .is_error
    );
}

/// A stand-in for `gh` that records how it was called.
fn fake_gh(h: &Harness) -> (PathBuf, PathBuf) {
    let record = h.p.state.join("gh-record");
    let script = h.p.state.join("gh");
    fs::write(
        &script,
        format!("#!/bin/sh\n{{ printf 'ARG:%s\\n' \"$@\"; echo STDIN:; cat; echo ENV:; env; }} > {}\necho https://example.invalid/pr/1\n", record.display()),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    (script, record)
}

#[test]
fn a_pull_request_is_opened_with_the_body_on_stdin_and_a_scrubbed_environment() {
    let h = repo();
    let (script, record) = fake_gh(&h);
    let h = Harness {
        host: HostTools::with(Some("/usr/bin/git".into()), Some(script)),
        ..h
    };
    let now = Instant::now();
    let mut grant = h.p.grant(FILES, now);
    let yes = Scripted::new(Confirmation::Confirmed);
    let args = json!({"title": "--web is just a title", "body": "line one\n--draft\n", "base": "main", "draft": true});

    let denied = h.call(&grant, now, None, &yes, "gh_pr_create", args.clone());
    assert!(denied.is_error && !record.exists());
    grant.allow_network();
    let made = h.call(&grant, now, None, &yes, "gh_pr_create", args);
    assert!(!made.is_error, "{}", made.text);
    assert!(made.text.contains("https://example.invalid/pr/1"));

    let text = fs::read_to_string(&record).unwrap();
    for expected in [
        "ARG:pr",
        "ARG:create",
        "ARG:--title=--web is just a title",
        "ARG:--body-file=-",
        "ARG:--base=main",
        "ARG:--draft",
    ] {
        assert!(text.contains(expected), "missing {expected}:\n{text}");
    }
    assert!(text.contains("STDIN:\nline one\n--draft\n"), "{text}");
    let env = text.split("ENV:\n").nth(1).unwrap();
    assert!(env.contains("PATH=/usr/local/bin:/usr/bin:/bin"));
    for leaked in ["GH_TOKEN", "GITHUB_TOKEN", "KELLU", "CARGO"] {
        assert!(!env.contains(leaked), "{leaked} reached gh:\n{env}");
    }

    for bad in [
        json!({"title": "a\nb"}),
        json!({"title": ""}),
        json!({"title": "t", "base": "--x"}),
        json!({"title": "t", "head": "x"}),
    ] {
        assert!(
            h.call(&grant, now, None, &yes, "gh_pr_create", bad)
                .is_error
        );
    }
}
