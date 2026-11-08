//! Running git and `gh` on the host, outside the sandbox.
//!
//! These two cannot run in the sandbox (the staging copy has no `.git`, and they need the user's
//! credentials), so they get a hardened environment instead: a fixed `PATH`, no variables from the
//! parent process except a short allowlist, a bounded runtime and bounded output. Programs are
//! found in fixed system directories, never through the caller's `PATH`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kellu_common::text::{strip_control_chars, truncate};
use kellu_sandbox::{HostCommand, Outcome, Output, run_host};

use crate::spec::ToolError;

/// Where git and `gh` are looked for.
const SEARCH_DIRS: &[&str] = &["/usr/bin", "/usr/local/bin", "/bin"];

/// The `PATH` host commands run with.
pub const HOST_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// How long a host command may run.
pub const HOST_TIMEOUT: Duration = Duration::from_secs(120);

/// How much of each of stdout and stderr is kept.
pub const HOST_MAX_OUTPUT: usize = 1 << 20;

/// Variables of the user's environment that git and `gh` need to find their configuration and
/// credentials. Tokens in environment variables are deliberately not passed.
const WRITE_ENV_ALLOWLIST: &[&str] = &[
    "HOME",
    "XDG_CONFIG_HOME",
    "XDG_RUNTIME_DIR",
    "SSH_AUTH_SOCK",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
];

/// The host programs, when they exist.
#[derive(Debug, Clone, Default)]
pub struct HostTools {
    git: Option<PathBuf>,
    gh: Option<PathBuf>,
}

impl HostTools {
    /// Look for `git` and `gh` in the fixed system directories.
    #[must_use]
    pub fn discover() -> Self {
        Self {
            git: find("git"),
            gh: find("gh"),
        }
    }

    /// Use these exact programs (tests, or a user-chosen location).
    #[must_use]
    pub fn with(git: Option<PathBuf>, gh: Option<PathBuf>) -> Self {
        Self { git, gh }
    }

    /// Neither program is available.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// The git program.
    ///
    /// # Errors
    /// Git was not found.
    pub fn git(&self) -> Result<&Path, ToolError> {
        self.git
            .as_deref()
            .ok_or_else(|| ToolError::Failed("git was not found on this machine".into()))
    }

    /// The `gh` program.
    ///
    /// # Errors
    /// `gh` was not found.
    pub fn gh(&self) -> Result<&Path, ToolError> {
        self.gh
            .as_deref()
            .ok_or_else(|| ToolError::Failed("gh was not found on this machine".into()))
    }

    /// Whether git was found.
    #[must_use]
    pub fn has_git(&self) -> bool {
        self.git.is_some()
    }

    /// Whether `gh` was found.
    #[must_use]
    pub fn has_gh(&self) -> bool {
        self.gh.is_some()
    }
}

fn find(name: &str) -> Option<PathBuf> {
    SEARCH_DIRS
        .iter()
        .map(|dir| Path::new(dir).join(name))
        .find(|path| path.is_file())
}

fn pair(key: &str, value: &str) -> (OsString, OsString) {
    (OsString::from(key), OsString::from(value))
}

/// The environment for **reading** with git. Nothing is inherited, and every configuration file
/// outside the repository is ignored, so a user's or system's alias, hook path or credential
/// helper cannot change what a read does.
#[must_use]
pub fn read_env() -> Vec<(OsString, OsString)> {
    vec![
        pair("PATH", HOST_PATH),
        pair("HOME", "/nonexistent"),
        pair("LANG", "C.UTF-8"),
        pair("GIT_CONFIG_GLOBAL", "/dev/null"),
        pair("GIT_CONFIG_SYSTEM", "/dev/null"),
        pair("GIT_CONFIG_NOSYSTEM", "1"),
        pair("GIT_ATTR_NOSYSTEM", "1"),
        pair("GIT_OPTIONAL_LOCKS", "0"),
        pair("GIT_TERMINAL_PROMPT", "0"),
        pair("GIT_NO_REPLACE_OBJECTS", "1"),
        pair("GIT_PAGER", "cat"),
        pair("PAGER", "cat"),
    ]
}

/// The environment for **writing** with git or `gh`: the allowlisted variables of `source` plus a
/// fixed `PATH`. The user's own git configuration applies (it holds their name, e-mail and
/// credentials), which is why every write needs confirmation.
#[must_use]
pub fn write_env<I>(source: I) -> Vec<(OsString, OsString)>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let mut env: Vec<(OsString, OsString)> = source
        .into_iter()
        .filter(|(key, _)| {
            key.to_str()
                .is_some_and(|name| WRITE_ENV_ALLOWLIST.contains(&name))
        })
        .collect();
    env.push(pair("PATH", HOST_PATH));
    env.push(pair("GIT_TERMINAL_PROMPT", "0"));
    env.push(pair("GH_PROMPT_DISABLED", "1"));
    env.push(pair("GH_NO_UPDATE_NOTIFIER", "1"));
    env
}

/// Arguments placed before the subcommand of every git call.
///
/// They switch off the ways a repository's own configuration can make git run a program while it
/// only reads: the filesystem monitor, a pager, hooks and external transports. Paths are literal.
#[must_use]
pub fn git_prefix() -> Vec<OsString> {
    [
        "--no-pager",
        "--literal-pathspecs",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.pager=cat",
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "protocol.ext.allow=never",
        "-c",
        "core.untrackedCache=false",
    ]
    .iter()
    .map(OsString::from)
    .collect()
}

/// Run `program` with `args` in `cwd`.
///
/// # Errors
/// The process cannot be started.
pub fn run(
    program: &Path,
    args: &[OsString],
    env: &[(OsString, OsString)],
    cwd: &Path,
    stdin: Option<&[u8]>,
) -> Result<Output, ToolError> {
    let mut argv: Vec<OsString> = vec![program.into()];
    argv.extend(args.iter().cloned());
    run_host(&HostCommand {
        argv: &argv,
        env,
        cwd,
        stdin,
        wall_clock: HOST_TIMEOUT,
        max_output_bytes: HOST_MAX_OUTPUT,
    })
    .map_err(ToolError::from)
}

/// Make command output safe to show and bounded: lossy UTF-8, no control characters.
#[must_use]
pub fn clean(bytes: &[u8], max_chars: usize) -> String {
    let text = strip_control_chars(&String::from_utf8_lossy(bytes));
    match std::num::NonZeroUsize::new(max_chars) {
        Some(limit) => truncate(&text, limit),
        None => String::new(),
    }
}

/// How a command ended, in words.
#[must_use]
pub fn describe(outcome: Outcome) -> String {
    match outcome {
        Outcome::Exited(0) => "exit status 0".into(),
        Outcome::Exited(code) => match outcome.signal() {
            Some(signal) => format!("exit status {code} (probably signal {signal})"),
            None => format!("exit status {code}"),
        },
        Outcome::Signaled(signal) => format!("killed by signal {signal}"),
        Outcome::TimedOut => "timed out and was killed".into(),
        Outcome::OutputLimit => "wrote too much output and was killed".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_set(env: &[(OsString, OsString)]) -> Vec<String> {
        env.iter()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_read_environment_inherits_nothing() {
        let env = read_env();
        assert!(
            env.iter()
                .any(|(k, v)| k == "GIT_CONFIG_GLOBAL" && v == "/dev/null")
        );
        assert!(
            env.iter()
                .any(|(k, v)| k == "GIT_CONFIG_SYSTEM" && v == "/dev/null")
        );
        assert!(!key_set(&env).contains(&"GIT_DIR".to_owned()));
    }

    #[test]
    fn the_write_environment_keeps_only_the_allowlist() {
        let source = [
            ("HOME", "/home/u"),
            ("GH_TOKEN", "ghp_should_never_pass"),
            ("GITHUB_TOKEN", "x"),
            ("GIT_SSH_COMMAND", "evil"),
            ("GIT_EXTERNAL_DIFF", "evil"),
            ("LD_PRELOAD", "/tmp/evil.so"),
            ("PATH", "/tmp/evil"),
            ("SSH_AUTH_SOCK", "/run/ssh"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let env = write_env(source);
        let keys = key_set(&env);
        for bad in [
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "GIT_SSH_COMMAND",
            "GIT_EXTERNAL_DIFF",
            "LD_PRELOAD",
        ] {
            assert!(!keys.contains(&bad.to_owned()), "{bad} leaked");
        }
        assert!(keys.contains(&"HOME".to_owned()));
        assert!(keys.contains(&"SSH_AUTH_SOCK".to_owned()));
        let path: Vec<_> = env.iter().filter(|(k, _)| k == "PATH").collect();
        assert_eq!(path.len(), 1, "exactly one PATH, ours");
        assert_eq!(path[0].1, HOST_PATH);
    }

    #[test]
    fn the_git_prefix_switches_off_programs_a_repository_could_name() {
        let prefix: Vec<String> = git_prefix()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        for needed in [
            "core.fsmonitor=false",
            "core.hooksPath=/dev/null",
            "--literal-pathspecs",
        ] {
            assert!(prefix.iter().any(|a| a == needed), "{needed}");
        }
    }

    #[test]
    fn output_is_cleaned_and_bounded() {
        assert_eq!(clean(b"a\x1b[31mred\x1b[0m\r\n", 100), "a[31mred[0m\n");
        assert!(clean(&[b'x'; 500], 100).contains("truncated"));
    }
}
