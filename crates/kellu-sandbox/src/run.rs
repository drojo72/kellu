//! Running one command in the sandbox.

use std::ffi::OsString;
use std::io::Read;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use kellu_common::{KelluError, Result};

use crate::bwrap::{self, BwrapVersion, Spec};
use crate::policy::{Policy, SystemLayout};
use crate::probe::probe;
use crate::staging::Staging;

/// How long the supervisor sleeps between checks of the child.
const POLL: Duration = Duration::from_millis(5);

/// How a sandboxed command ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// It exited with this status code.
    Exited(i32),
    /// A signal ended it (a resource limit shows up here: `SIGXCPU`, `SIGXFSZ`, `SIGKILL`).
    Signaled(i32),
    /// The wall-clock timeout expired and the process group was killed.
    TimedOut,
    /// It wrote more than the output cap and the process group was killed.
    OutputLimit,
}

impl Outcome {
    /// The signal that ended the command, if one did.
    ///
    /// Bubblewrap reports a command killed by a signal as exit status `128 + signal` (the shell
    /// convention), so a command that really exits with 129 to 159 cannot be told apart from one
    /// killed by a signal. This applies the convention and is meant for resource-limit reports.
    #[must_use]
    pub fn signal(self) -> Option<i32> {
        match self {
            Self::Signaled(signal) => Some(signal),
            Self::Exited(code @ 129..=159) => Some(code - 128),
            Self::Exited(_) | Self::TimedOut | Self::OutputLimit => None,
        }
    }
}

/// What a sandboxed command produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// Standard output, at most the output cap.
    pub stdout: Vec<u8>,
    /// Standard error, at most the output cap.
    pub stderr: Vec<u8>,
    /// How it ended.
    pub outcome: Outcome,
}

impl Output {
    /// Whether the command exited with status 0.
    #[must_use]
    pub fn success(&self) -> bool {
        self.outcome == Outcome::Exited(0)
    }
}

fn outcome_of(status: ExitStatus) -> Outcome {
    match (status.code(), status.signal()) {
        (Some(code), _) => Outcome::Exited(code),
        (None, Some(signal)) => Outcome::Signaled(signal),
        (None, None) => Outcome::Signaled(0),
    }
}

fn kill_group(child: &mut Child) {
    let pid = rustix::process::Pid::from_child(child);
    // The child leads its own process group; killing the group takes bubblewrap down, and with
    // it the whole PID namespace.
    let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    let _ = child.kill();
}

fn read_capped<R: Read>(reader: Option<R>, cap: usize, exceeded: &AtomicBool) -> Vec<u8> {
    let Some(mut reader) = reader else {
        return Vec::new();
    };
    let mut data = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                let room = cap.saturating_sub(data.len());
                data.extend_from_slice(&buffer[..n.min(room)]);
                if n > room {
                    exceeded.store(true, Ordering::SeqCst);
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    data
}

/// Run `argv` (program first) with an empty environment, no stdin, bounded output and a wall-clock
/// timeout, in its own process group.
pub(crate) fn exec(argv: &[OsString], wall_clock: Duration, max_output: usize) -> Result<Output> {
    let (program, rest) = argv
        .split_first()
        .ok_or_else(|| KelluError::security("there is no command to run"))?;
    let mut child = Command::new(program)
        .args(rest)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| KelluError::io(Path::new(program), e))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let exceeded = AtomicBool::new(false);
    let deadline = Instant::now() + wall_clock;
    thread::scope(|scope| {
        let out = scope.spawn(|| read_capped(stdout, max_output, &exceeded));
        let err = scope.spawn(|| read_capped(stderr, max_output, &exceeded));
        let outcome = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(outcome_of(status)),
                Ok(None) => {}
                Err(error) => break Err(KelluError::io(Path::new(program), error)),
            }
            if exceeded.load(Ordering::SeqCst) {
                break Ok(Outcome::OutputLimit);
            }
            if Instant::now() >= deadline {
                break Ok(Outcome::TimedOut);
            }
            thread::sleep(POLL);
        };
        if !matches!(outcome, Ok(Outcome::Exited(_) | Outcome::Signaled(_))) {
            kill_group(&mut child);
        }
        let _ = child.wait();
        let join = |handle: thread::ScopedJoinHandle<'_, Vec<u8>>| {
            handle
                .join()
                .map_err(|_| KelluError::security("reading the command output failed"))
        };
        let (stdout, stderr) = (join(out)?, join(err)?);
        // A command that wrote too much can die of SIGPIPE before the loop above notices, so the
        // flag is read only after both readers are done.
        let outcome = match outcome? {
            Outcome::Exited(_) | Outcome::Signaled(_) if exceeded.load(Ordering::SeqCst) => {
                Outcome::OutputLimit
            }
            other => other,
        };
        Ok(Output {
            stdout,
            stderr,
            outcome,
        })
    })
}

/// A bubblewrap sandbox that passed its probe.
#[derive(Debug, Clone)]
pub struct Sandbox {
    bwrap: PathBuf,
    prlimit: Option<PathBuf>,
    system: SystemLayout,
    version: BwrapVersion,
}

impl Sandbox {
    /// Probe bubblewrap and open a sandbox, or refuse (fail closed).
    ///
    /// Version policy B: a bubblewrap older than 0.12.0 is accepted, because the mount invariant
    /// keeps its setup-time symlink flaw from applying; with `require_patched` it is refused.
    ///
    /// # Errors
    /// Namespaces cannot be created with these exact options, the version cannot be read, or the
    /// version is older than 0.12.0 and `require_patched` is set.
    pub fn open(
        bwrap: &Path,
        prlimit: Option<&Path>,
        system: SystemLayout,
        require_patched: bool,
    ) -> Result<Self> {
        let report = probe(bwrap, prlimit, &system);
        if let Err(reason) = &report.isolation {
            return Err(KelluError::security(format!(
                "the sandbox cannot isolate processes here: {reason}"
            )));
        }
        let version = report
            .version
            .ok_or_else(|| KelluError::security("cannot read the bubblewrap version"))?;
        if require_patched && !version.is_patched() {
            return Err(KelluError::security(format!(
                "bubblewrap {version} is older than {}; sandbox.require_patched_bwrap refuses it",
                bwrap::FIRST_PATCHED
            )));
        }
        Ok(Self {
            bwrap: bwrap.to_path_buf(),
            prlimit: if report.prlimit {
                prlimit.map(Path::to_path_buf)
            } else {
                None
            },
            system,
            version,
        })
    }

    /// The bubblewrap version in use.
    #[must_use]
    pub fn version(&self) -> BwrapVersion {
        self.version
    }

    /// Whether this bubblewrap contains the fix for GHSA-pxhw-h44j-8pfx.
    #[must_use]
    pub fn is_patched(&self) -> bool {
        self.version.is_patched()
    }

    /// Run `command` (program and arguments, no shell) on `staging` under `policy`.
    ///
    /// # Errors
    /// The grant expired, `staging` is not a copy of the policy's project, the command is empty,
    /// resource limits are requested but `prlimit` is missing, or the process cannot be started.
    /// A command that fails, times out or writes too much is not an error: see [`Outcome`].
    pub fn run(
        &self,
        policy: &Policy,
        staging: &Staging,
        command: &[OsString],
        now: Instant,
    ) -> Result<Output> {
        if policy.is_expired(now) {
            return Err(KelluError::security(
                "the grant has expired; nothing runs in the sandbox",
            ));
        }
        if staging.root() != policy.root() {
            return Err(KelluError::security(
                "the staging copy is not of this policy's project",
            ));
        }
        if command.first().is_none_or(|program| program.is_empty()) {
            return Err(KelluError::security("there is no command to run"));
        }
        let limits = policy.limits();
        // The limits wrap only the user's command, inside the sandbox (see `limits.rs`).
        let mut inner: Vec<OsString> = Vec::new();
        if limits.has_rlimits() {
            let prlimit = self.prlimit.as_ref().ok_or_else(|| {
                KelluError::security(
                    "resource limits are required but `prlimit` is not available inside the \
                     sandbox; refusing to run without them",
                )
            })?;
            inner.push(prlimit.into());
            inner.extend(limits.prlimit_args().into_iter().map(OsString::from));
        }
        inner.extend(command.iter().cloned());
        let mut argv: Vec<OsString> = vec![(&self.bwrap).into()];
        argv.extend(bwrap::args(&Spec {
            system: &self.system,
            network: policy.network(),
            work: Some((staging.path(), policy.profile().access())),
            env: policy.env(),
            command: &inner,
        }));
        exec(&argv, limits.wall_clock, limits.max_output_bytes)
    }
}
