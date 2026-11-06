//! What this host can do: the facts `kellu doctor` reports and `Sandbox::open` relies on.

use std::ffi::OsString;
use std::num::NonZeroUsize;
use std::path::Path;
use std::time::Duration;

use kellu_common::text::{strip_control_chars, truncate};

use crate::bwrap::{self, BwrapVersion, Spec};
use crate::policy::SystemLayout;
use crate::run::{Outcome, exec};

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_OUTPUT: usize = 4096;
const REASON_CHARS: usize = 200;

/// What the probe found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    /// The bubblewrap version, or `None` if it could not be read.
    pub version: Option<BwrapVersion>,
    /// `Ok` when bubblewrap can build the sandbox with the exact options Kellu uses (user, PID,
    /// network and other namespaces); otherwise why not.
    pub isolation: std::result::Result<(), String>,
    /// Whether `prlimit` is visible and works inside the sandbox, which resource limits need.
    pub prlimit: bool,
    /// Whether the kernel allows `openat2`, which every project read needs.
    pub openat2: bool,
}

impl Probe {
    /// Whether the bubblewrap found contains the fix for GHSA-pxhw-h44j-8pfx.
    #[must_use]
    pub fn patched(&self) -> Option<bool> {
        self.version.map(BwrapVersion::is_patched)
    }
}

fn first_line(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    NonZeroUsize::new(REASON_CHARS).map_or_else(String::new, |limit| {
        truncate(&strip_control_chars(line), limit)
    })
}

/// Probe the host. Runs `bwrap --version`, an empty sandbox with the exact options Kellu uses
/// (`true` inside it) and, when a path is given, `prlimit` inside that sandbox. Nothing is written
/// and nothing is mounted from the user's directories.
#[must_use]
pub fn probe(bwrap_path: &Path, prlimit: Option<&Path>, system: &SystemLayout) -> Probe {
    let run = |argv: &[OsString]| exec(argv, PROBE_TIMEOUT, PROBE_OUTPUT);
    let version = run(&[bwrap_path.into(), "--version".into()])
        .ok()
        .filter(|out| out.success())
        .and_then(|out| BwrapVersion::parse(&String::from_utf8_lossy(&out.stdout)));

    let sandboxed = |command: &[OsString]| {
        let mut argv: Vec<OsString> = vec![bwrap_path.into()];
        argv.extend(bwrap::args(&Spec {
            system,
            network: false,
            work: None,
            env: &[],
            command,
        }));
        run(&argv)
    };
    let isolation = match sandboxed(&[OsString::from("true")]) {
        Ok(out) if out.success() => Ok(()),
        Ok(out) if out.outcome == Outcome::TimedOut => Err("bubblewrap did not finish".to_owned()),
        Ok(out) => {
            let line = first_line(&out.stderr);
            Err(if line.is_empty() {
                "bubblewrap failed without a message".to_owned()
            } else {
                line
            })
        }
        Err(error) => Err(first_line(error.to_string().as_bytes())),
    };

    // The limits run inside the sandbox, so `prlimit` must be visible there and must work there.
    let prlimit = isolation.is_ok()
        && prlimit.is_some_and(|path| {
            system.provides(path)
                && sandboxed(&[path.into(), "--nofile=4096".into(), "true".into()])
                    .is_ok_and(|out| out.success())
        });

    Probe {
        version,
        isolation,
        prlimit,
        openat2: kellu_security::openat2_available(),
    }
}
