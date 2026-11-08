//! The host runner used by the few tools that cannot run in the sandbox (git, gh).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, Instant};

use kellu_sandbox::{HostCommand, Outcome, run_host};

fn argv(parts: &[&str]) -> Vec<OsString> {
    parts.iter().map(OsString::from).collect()
}

fn run(
    parts: &[&str],
    env: &[(OsString, OsString)],
    stdin: Option<&[u8]>,
    wall: Duration,
    cap: usize,
) -> kellu_sandbox::Output {
    let argv = argv(parts);
    run_host(&HostCommand {
        argv: &argv,
        env,
        cwd: Path::new("/"),
        stdin,
        wall_clock: wall,
        max_output_bytes: cap,
    })
    .unwrap()
}

#[test]
fn the_environment_is_exactly_what_was_given() {
    let env = [(OsString::from("ONLY"), OsString::from("this"))];
    let out = run(&["/usr/bin/env"], &env, None, Duration::from_secs(10), 4096);
    assert!(out.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ONLY=this");
}

#[test]
fn the_working_directory_and_stdin_are_honoured() {
    let out = run(&["/bin/pwd"], &[], None, Duration::from_secs(10), 4096);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "/");
    let out = run(
        &["/bin/cat"],
        &[],
        Some(b"from stdin"),
        Duration::from_secs(10),
        4096,
    );
    assert_eq!(out.stdout, b"from stdin");
}

#[test]
fn a_command_that_ignores_a_large_stdin_does_not_hang_the_caller() {
    let big = vec![b'x'; 4 << 20];
    let started = Instant::now();
    let out = run(
        &["/bin/true"],
        &[],
        Some(&big),
        Duration::from_secs(10),
        4096,
    );
    assert!(out.success());
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn the_timeout_kills_the_command_and_its_children() {
    let started = Instant::now();
    let out = run(
        &["/bin/sh", "-c", "sleep 30 & sleep 30"],
        &[],
        None,
        Duration::from_millis(300),
        4096,
    );
    assert_eq!(out.outcome, Outcome::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn output_above_the_cap_stops_the_command() {
    let out = run(
        &["/bin/sh", "-c", "yes"],
        &[],
        None,
        Duration::from_secs(10),
        1024,
    );
    assert_eq!(out.outcome, Outcome::OutputLimit);
    assert!(out.stdout.len() <= 1024);
}

#[test]
fn an_empty_command_and_zero_limits_are_refused() {
    let empty: Vec<OsString> = Vec::new();
    let call = |argv: &[OsString], wall, cap| {
        run_host(&HostCommand {
            argv,
            env: &[],
            cwd: Path::new("/"),
            stdin: None,
            wall_clock: wall,
            max_output_bytes: cap,
        })
    };
    assert!(call(&empty, Duration::from_secs(1), 10).is_err());
    let echo = argv(&["/bin/true"]);
    assert!(call(&echo, Duration::ZERO, 10).is_err());
    assert!(call(&echo, Duration::from_secs(1), 0).is_err());
}
