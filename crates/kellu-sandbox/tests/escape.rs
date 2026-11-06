//! Escape tests: real commands in a real bubblewrap sandbox.
//!
//! They need `bwrap` and user namespaces. Where those are missing the tests print `SKIPPED` and
//! pass, unless `KELLU_REQUIRE_SANDBOX_TESTS=1` is set (CI does that), which turns a skip into a
//! failure. Tests that depend on running as a normal user say so and skip as root.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kellu_sandbox::{
    Change, Limits, Outcome, Output, Policy, Profile, Sandbox, Staging, StagingLimits,
    SystemLayout, probe,
};
use kellu_security::{DenyGlobs, Grant, Scope};

const TTL: Duration = Duration::from_secs(600);

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

struct Project {
    _tmp: tempfile::TempDir,
    outside: PathBuf,
    root: PathBuf,
    cache: PathBuf,
    scope: Scope,
}

fn project() -> Project {
    let tmp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(tmp.path()).unwrap();
    let home = base.join("home");
    let root = home.join("project");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(root.join("README.md"), "hello\n").unwrap();
    fs::write(root.join("run.sh"), "#!/bin/sh\n").unwrap();
    fs::write(root.join(".env"), "TOKEN=hunter2\n").unwrap();
    fs::write(root.join("id_rsa"), "PRIVATE\n").unwrap();
    fs::write(base.join("secret.txt"), "outside\n").unwrap();
    symlink(base.join("secret.txt"), root.join("leak.txt")).unwrap();
    let scope = Scope::open(&root, &home, DenyGlobs::new(), false).unwrap();
    Project {
        outside: base.join("secret.txt"),
        cache: base.join("cache"),
        _tmp: tmp,
        root,
        scope,
    }
}

const GRANTED: &[&str] = &[
    "src/main.rs",
    "README.md",
    "run.sh",
    ".env",
    "id_rsa",
    "leak.txt",
];

fn setup(
    p: &Project,
    profile: Profile,
    limits: Limits,
    network: bool,
) -> (Policy, Staging, Instant) {
    let now = Instant::now();
    let mut grant = Grant::read_only(&p.scope, GRANTED.iter().map(PathBuf::from), now, TTL);
    if network {
        grant.allow_network();
    }
    let hostile_env = [
        ("LANG", "C.UTF-8"),
        ("GITHUB_TOKEN", "ghp_secret"),
        ("HOME", "/home/real-user"),
        ("OPENAI_API_KEY", "sk-secret"),
        ("SSH_AUTH_SOCK", "/run/user/1000/ssh"),
    ]
    .map(|(k, v)| (OsString::from(k), OsString::from(v)));
    let policy = Policy::from_grant(&grant, profile, limits, now)
        .unwrap()
        .with_env(hostile_env);
    let staging =
        Staging::create(&p.cache, &p.scope, &policy, StagingLimits::default(), now).unwrap();
    (policy, staging, now)
}

fn cmd(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn run(sb: &Sandbox, p: &Project, profile: Profile, limits: Limits, args: &[&str]) -> Output {
    let (policy, staging, now) = setup(p, profile, limits, false);
    sb.run(&policy, &staging, &cmd(args), now).unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn sh(script: &str) -> Vec<String> {
    vec!["sh".into(), "-c".into(), script.into()]
}

fn run_sh(sb: &Sandbox, p: &Project, profile: Profile, limits: Limits, script: &str) -> Output {
    let args = sh(script);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run(sb, p, profile, limits, &refs)
}

fn is_root() -> bool {
    rustix::process::getuid().is_root()
}

// ------------------------------------------------------------------------ the project ----

#[test]
fn a_read_only_project_refuses_writes_new_files_and_directories() {
    let Some(sb) = sandbox() else { return };
    let p = project();
    for script in [
        "echo x > README.md",
        "echo x >> src/main.rs",
        "touch brand-new.txt",
        "mkdir newdir",
        "rm README.md",
        "chmod 777 README.md",
    ] {
        let out = run_sh(&sb, &p, Profile::ReadOnly, Limits::default(), script);
        assert!(
            matches!(out.outcome, Outcome::Exited(code) if code != 0 && code != 127),
            "{script}: {:?} {}",
            out.outcome,
            text(&out.stderr)
        );
    }
    assert_eq!(
        fs::read_to_string(p.root.join("README.md")).unwrap(),
        "hello\n"
    );
}

#[test]
fn a_writable_project_changes_only_the_staging_copy() {
    let Some(sb) = sandbox() else { return };
    let p = project();
    let (policy, staging, now) = setup(&p, Profile::WorkspaceWrite, Limits::default(), false);
    let script = sh(
        "echo changed > README.md; echo new > extra.txt; rm src/main.rs; \
                     ln -s /etc/passwd link; mkdir -p deep/er; echo d > deep/er/f.txt",
    );
    let out = sb
        .run(&policy, &staging, &script_args(&script), now)
        .unwrap();
    assert!(out.success(), "{:?} {}", out.outcome, text(&out.stderr));

    let changes = staging.changes().unwrap();
    let summary: Vec<(String, &str)> = changes
        .iter()
        .map(|c| {
            let kind = match c {
                Change::Modified { .. } => "modified",
                Change::Created { .. } => "created",
                Change::Deleted { .. } => "deleted",
                Change::Unsafe { .. } => "unsafe",
            };
            (c.path().display().to_string(), kind)
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("README.md".to_owned(), "modified"),
            ("deep/er/f.txt".to_owned(), "created"),
            ("extra.txt".to_owned(), "created"),
            ("link".to_owned(), "unsafe"),
            ("src/main.rs".to_owned(), "deleted"),
        ]
    );
    // The real project and everything outside it are untouched.
    assert_eq!(
        fs::read_to_string(p.root.join("README.md")).unwrap(),
        "hello\n"
    );
    assert!(p.root.join("src/main.rs").exists());
    assert!(!p.root.join("extra.txt").exists());
    assert!(!p.root.join("link").exists());
    assert!(!p.root.join("deep").exists());
    assert_eq!(fs::read_to_string(&p.outside).unwrap(), "outside\n");
}

fn script_args(script: &[String]) -> Vec<OsString> {
    script.iter().map(OsString::from).collect()
}

#[test]
fn denied_files_and_symlinks_are_not_in_the_sandbox() {
    let Some(sb) = sandbox() else { return };
    let p = project();
    let out = run(
        &sb,
        &p,
        Profile::ReadOnly,
        Limits::default(),
        &["ls", "-A", "/work"],
    );
    assert!(out.success(), "{}", text(&out.stderr));
    let mut seen: Vec<String> = text(&out.stdout).lines().map(str::to_owned).collect();
    seen.sort();
    assert_eq!(seen, ["README.md", "run.sh", "src"]);
    for secret in [".env", "id_rsa", "leak.txt"] {
        let probe = format!("cat /work/{secret}");
        let out = run_sh(&sb, &p, Profile::ReadOnly, Limits::default(), &probe);
        assert!(!out.success(), "{secret} is readable");
        assert!(!text(&out.stdout).contains("hunter2"));
    }
}

#[test]
fn the_host_is_invisible() {
    let Some(sb) = sandbox() else { return };
    let p = project();
    let (policy, staging, now) = setup(&p, Profile::ReadOnly, Limits::default(), false);
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_owned());
    let script = format!(
        "ls -A /home; echo ---; ls -A /etc; echo ---; ls -A /; echo ---; \
         for path in '{}' '{}' '{}' '{}' /etc/passwd /etc/shadow /root /var /run /mnt /media /opt /srv /boot /sys/fs; do \
         test -e \"$path\" && echo \"VISIBLE $path\"; done; true",
        p.root.display(),
        p.outside.display(),
        staging.path().display(),
        home,
    );
    let out = sb
        .run(&policy, &staging, &script_args(&sh(&script)), now)
        .unwrap();
    let report = text(&out.stdout);
    assert!(out.success(), "{} {report}", text(&out.stderr));
    assert!(!report.contains("VISIBLE"), "{report}");
    let parts: Vec<&str> = report.split("---\n").collect();
    assert_eq!(parts[0].trim(), "", "/home is not empty: {report}");
    let etc: Vec<&str> = parts[1].lines().collect();
    assert!(
        etc.iter()
            .all(|e| ["alternatives", "ld.so.cache"].contains(e)),
        "{etc:?}"
    );
    let root: Vec<&str> = parts[2].lines().collect();
    for forbidden in [
        "root", "var", "run", "mnt", "media", "opt", "srv", "boot", "sys",
    ] {
        assert!(
            !root.contains(&forbidden),
            "/{forbidden} is visible: {root:?}"
        );
    }
}

#[test]
fn the_environment_is_built_from_nothing() {
    let Some(sb) = sandbox() else { return };
    let p = project();
    let out = run(&sb, &p, Profile::ReadOnly, Limits::default(), &["env"]);
    assert!(out.success(), "{}", text(&out.stderr));
    let mut vars: Vec<String> = text(&out.stdout).lines().map(str::to_owned).collect();
    vars.sort();
    assert_eq!(
        vars,
        [
            "HOME=/home",
            "LANG=C.UTF-8",
            "PATH=/usr/local/bin:/usr/bin:/bin",
            "PWD=/work",
            "TMPDIR=/tmp",
        ]
    );
}

#[test]
fn only_the_loopback_interface_exists_without_network() {
    let Some(sb) = sandbox() else { return };
    let p = project();
    let out = run(
        &sb,
        &p,
        Profile::ReadOnly,
        Limits::default(),
        &["cat", "/proc/net/dev"],
    );
    assert!(out.success(), "{}", text(&out.stderr));
    let interfaces: Vec<String> = text(&out.stdout)
        .lines()
        .skip(2)
        .filter_map(|l| l.split(':').next().map(|n| n.trim().to_owned()))
        .collect();
    assert_eq!(interfaces, ["lo"]);
}

#[test]
fn a_sandboxed_process_cannot_create_more_namespaces() {
    let Some(sb) = sandbox() else { return };
    let p = project();
    let present = run(
        &sb,
        &p,
        Profile::ReadOnly,
        Limits::default(),
        &["unshare", "--version"],
    );
    if !present.success() {
        eprintln!("SKIPPED: `unshare` is not available inside the sandbox");
        return;
    }
    let out = run(
        &sb,
        &p,
        Profile::ReadOnly,
        Limits::default(),
        &["unshare", "-U", "true"],
    );
    assert!(
        matches!(out.outcome, Outcome::Exited(code) if code != 0),
        "{:?}",
        out.outcome
    );
}

#[test]
fn the_command_is_never_interpreted_by_a_shell_or_by_bubblewrap() {
    let Some(sb) = sandbox() else { return };
    let p = project();
    let (policy, staging, now) = setup(&p, Profile::WorkspaceWrite, Limits::default(), false);
    let args = cmd(&[
        "echo",
        "$(touch /work/pwned)",
        ";",
        "touch /work/pwned2",
        "`touch /work/pwned3`",
        "--bind",
        "/",
        "/mnt",
        "--share-net",
    ]);
    let out = sb.run(&policy, &staging, &args, now).unwrap();
    assert!(out.success(), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout).trim_end(),
        "$(touch /work/pwned) ; touch /work/pwned2 `touch /work/pwned3` --bind / /mnt --share-net"
    );
    assert!(staging.changes().unwrap().is_empty());
}

// ------------------------------------------------------------------------- the limits ----

#[test]
fn a_command_that_runs_too_long_is_killed_with_everything_it_started() {
    let Some(sb) = sandbox() else { return };
    let p = project();
    let limits = Limits {
        wall_clock: Duration::from_millis(400),
        ..Limits::default()
    };
    let started = Instant::now();
    let out = run_sh(
        &sb,
        &p,
        Profile::ReadOnly,
        limits,
        "sleep 31.234 & sleep 31.234",
    );
    assert_eq!(out.outcome, Outcome::TimedOut);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    std::thread::sleep(Duration::from_millis(300));
    let leftover = fs::read_dir("/proc")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read(entry.path().join("cmdline")).ok())
        .any(|cmdline| String::from_utf8_lossy(&cmdline).contains("31.234"));
    assert!(!leftover, "a process outlived the timeout");
}

#[test]
fn too_much_output_kills_the_command_and_keeps_only_the_cap() {
    let Some(sb) = sandbox() else { return };
    let p = project();
    let limits = Limits {
        max_output_bytes: 1000,
        wall_clock: Duration::from_secs(30),
        ..Limits::default()
    };
    let started = Instant::now();
    let out = run(&sb, &p, Profile::ReadOnly, limits, &["yes"]);
    assert_eq!(out.outcome, Outcome::OutputLimit);
    assert_eq!(out.stdout.len(), 1000);
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn the_resource_limits_are_set_inside_the_sandbox() {
    let Some(sb) = sandbox() else { return };
    let p = project();
    let limits = Limits {
        cpu_seconds: Some(7),
        address_space_bytes: Some(2 << 30),
        processes: Some(123),
        file_size_bytes: Some(1 << 20),
        open_files: Some(77),
        ..Limits::default()
    };
    let out = run(
        &sb,
        &p,
        Profile::ReadOnly,
        limits,
        &["cat", "/proc/self/limits"],
    );
    assert!(out.success(), "{}", text(&out.stderr));
    let report = text(&out.stdout);
    for (name, value) in [
        ("Max cpu time", "7"),
        ("Max address space", "2147483648"),
        ("Max processes", "123"),
        ("Max file size", "1048576"),
        ("Max open files", "77"),
    ] {
        let line = report.lines().find(|l| l.starts_with(name)).unwrap_or("");
        let fields: Vec<&str> = line[name.len()..].split_whitespace().collect();
        assert_eq!(fields.first(), Some(&value), "{name}: {line:?}");
        assert_eq!(fields.get(1), Some(&value), "{name} hard limit: {line:?}");
    }
}

#[test]
fn a_file_larger_than_the_limit_cannot_be_written() {
    let Some(sb) = sandbox() else { return };
    let p = project();
    let present = run(
        &sb,
        &p,
        Profile::ReadOnly,
        Limits::default(),
        &["dd", "--version"],
    );
    assert!(present.success(), "dd is needed for this test");
    let limits = Limits {
        file_size_bytes: Some(1 << 20),
        ..Limits::default()
    };
    let out = run(
        &sb,
        &p,
        Profile::ReadOnly,
        limits,
        &["dd", "if=/dev/zero", "of=/tmp/big", "bs=1M", "count=8"],
    );
    assert!(
        out.outcome.signal() == Some(25), // SIGXFSZ
        "{:?} {}",
        out.outcome,
        text(&out.stderr)
    );
}

#[test]
fn a_busy_loop_is_stopped_by_the_cpu_limit_not_the_clock() {
    let Some(sb) = sandbox() else { return };
    let p = project();
    let limits = Limits {
        cpu_seconds: Some(1),
        wall_clock: Duration::from_secs(30),
        ..Limits::default()
    };
    let started = Instant::now();
    let out = run_sh(&sb, &p, Profile::ReadOnly, limits, "while :; do :; done");
    assert!(
        // SIGKILL when the soft and hard limits are equal, SIGXCPU otherwise.
        matches!(out.outcome.signal(), Some(9 | 24)),
        "{:?}",
        out.outcome
    );
    assert!(started.elapsed() < Duration::from_secs(15));
}

#[test]
fn the_process_limit_stops_a_fork_bomb() {
    if is_root() {
        eprintln!("SKIPPED: RLIMIT_NPROC is not enforced for root; run as a normal user");
        return;
    }
    let Some(sb) = sandbox() else { return };
    let p = project();
    let limits = Limits {
        processes: Some(20),
        wall_clock: Duration::from_secs(20),
        ..Limits::default()
    };
    let out = run_sh(
        &sb,
        &p,
        Profile::ReadOnly,
        limits,
        "i=0; while [ $i -lt 200 ]; do sleep 20 & i=$((i+1)); done; echo spawned-all",
    );
    assert!(
        !text(&out.stdout).contains("spawned-all"),
        "{:?}",
        out.outcome
    );
    assert!(
        text(&out.stderr).to_lowercase().contains("fork"),
        "{}",
        text(&out.stderr)
    );
}

/// Other processes of the user (a desktop session has hundreds) must not stop the sandbox:
/// `RLIMIT_NPROC` set around bubblewrap itself would count all of them.
struct Sleepers(Vec<std::process::Child>);

impl Drop for Sleepers {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn many_other_processes_of_the_user_do_not_stop_the_sandbox() {
    if is_root() {
        eprintln!("SKIPPED: RLIMIT_NPROC is not enforced for root; run as a normal user");
        return;
    }
    let Some(sb) = sandbox() else { return };
    let p = project();
    let default_limit = Limits::default().processes.unwrap();
    let count = usize::try_from(default_limit).unwrap() + 50;
    let sleepers = Sleepers(
        (0..count)
            .map(|_| {
                std::process::Command::new("sleep")
                    .arg("40")
                    .spawn()
                    .unwrap()
            })
            .collect(),
    );
    let out = run(&sb, &p, Profile::ReadOnly, Limits::default(), &["true"]);
    assert!(out.success(), "{:?} {}", out.outcome, text(&out.stderr));
    drop(sleepers);
}

// ------------------------------------------------------------------- failing closed ----

#[test]
fn an_expired_grant_a_foreign_copy_or_no_command_runs_nothing() {
    let Some(sb) = sandbox() else { return };
    let p = project();
    let (policy, staging, now) = setup(&p, Profile::ReadOnly, Limits::default(), false);
    let later = now + TTL;
    let error = sb
        .run(&policy, &staging, &cmd(&["true"]), later)
        .unwrap_err();
    assert!(error.to_string().contains("expired"), "{error}");
    assert!(sb.run(&policy, &staging, &[], now).is_err());
    assert!(sb.run(&policy, &staging, &cmd(&[""]), now).is_err());

    let other = project();
    let (_, foreign, _) = setup(&other, Profile::ReadOnly, Limits::default(), false);
    let error = sb.run(&policy, &foreign, &cmd(&["true"]), now).unwrap_err();
    assert!(error.to_string().contains("not of this policy"), "{error}");
}

#[test]
fn limits_without_prlimit_refuse_to_run_unlimited() {
    let Some(bwrap) = find("bwrap") else {
        skip("bwrap is not installed");
        return;
    };
    let Ok(sb) = Sandbox::open(&bwrap, None, SystemLayout::host(), false) else {
        skip("the sandbox is unavailable");
        return;
    };
    let p = project();
    let (policy, staging, now) = setup(&p, Profile::ReadOnly, Limits::default(), false);
    let error = sb.run(&policy, &staging, &cmd(&["true"]), now).unwrap_err();
    assert!(error.to_string().contains("prlimit"), "{error}");

    let unlimited = Limits {
        cpu_seconds: None,
        address_space_bytes: None,
        processes: None,
        file_size_bytes: None,
        open_files: None,
        ..Limits::default()
    };
    let (policy, staging, now) = setup(&p, Profile::ReadOnly, unlimited, false);
    assert!(
        sb.run(&policy, &staging, &cmd(&["true"]), now)
            .unwrap()
            .success()
    );
}

#[test]
fn a_binary_that_is_not_a_working_bubblewrap_is_refused() {
    let system = SystemLayout::host();
    assert!(Sandbox::open(Path::new("/nonexistent/bwrap"), None, system.clone(), false).is_err());
    for imposter in ["/usr/bin/true", "/usr/bin/false", "/usr/bin/env"] {
        if Path::new(imposter).exists() {
            let error =
                Sandbox::open(Path::new(imposter), None, system.clone(), false).unwrap_err();
            assert!(!error.to_string().is_empty(), "{imposter}");
        }
    }
}

#[test]
fn a_strict_policy_refuses_a_bubblewrap_without_the_fix() {
    let Some(bwrap) = find("bwrap") else {
        skip("bwrap is not installed");
        return;
    };
    let report = probe(&bwrap, None, &SystemLayout::host());
    let Some(patched) = report.patched() else {
        skip("cannot read the bubblewrap version");
        return;
    };
    let strict = Sandbox::open(&bwrap, None, SystemLayout::host(), true);
    if patched {
        assert!(strict.is_ok() || report.isolation.is_err());
    } else {
        let error = strict.unwrap_err().to_string();
        assert!(error.contains("older than 0.12.0"), "{error}");
        // The default policy (B) still opens it when isolation works.
        assert_eq!(
            Sandbox::open(&bwrap, None, SystemLayout::host(), false).is_ok(),
            report.isolation.is_ok()
        );
    }
}

#[test]
fn the_probe_describes_this_host() {
    let Some(bwrap) = find("bwrap") else {
        skip("bwrap is not installed");
        return;
    };
    let report = probe(&bwrap, find("prlimit").as_deref(), &SystemLayout::host());
    assert!(report.version.is_some(), "{report:?}");
    if let Err(reason) = &report.isolation {
        skip(reason);
        return;
    }
    assert!(report.openat2);
    assert_eq!(report.prlimit, find("prlimit").is_some());
}
