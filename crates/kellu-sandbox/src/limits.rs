//! Resource limits for one sandboxed run.
//!
//! bubblewrap does not limit resources, and applying `setrlimit` in a child process needs `unsafe`
//! (a `pre_exec` hook), which Kellu forbids. The limits are therefore applied by `prlimit` from
//! util-linux, which runs **inside** the sandbox and wraps only the user's command.
//!
//! Applying them around `bwrap` instead does not work: `RLIMIT_NPROC` counts every process of the
//! user, so `--nproc=256` stops bubblewrap from creating its namespaces on any desktop session with
//! more than 256 processes (verified: 306 processes, limit 256, `Creating new namespace failed`).
//! Inside the new user namespace only the sandbox's own processes are counted (verified with 304
//! outside processes: 100 children at limit 256 work, and at limit 20 forking fails).
//!
//! If limits are requested and `prlimit` is missing or not visible inside the sandbox, the run is
//! refused instead of going ahead unlimited (fail closed).

use std::time::Duration;

use kellu_common::{KelluError, Result};

/// The longest wall-clock timeout a run may ask for.
const MAX_WALL_CLOCK: Duration = Duration::from_secs(3600);
/// The largest per-stream output cap a run may ask for.
const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

/// Limits applied to one sandboxed run.
///
/// An `rlimit` set to `None` is not applied. The wall-clock timeout and the output cap always
/// apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// CPU time in seconds (`RLIMIT_CPU`).
    pub cpu_seconds: Option<u64>,
    /// Address space in bytes (`RLIMIT_AS`). Runtimes that reserve a lot of virtual memory need more.
    pub address_space_bytes: Option<u64>,
    /// Processes and threads inside the sandbox (`RLIMIT_NPROC`).
    pub processes: Option<u64>,
    /// Size of any file the process writes, in bytes (`RLIMIT_FSIZE`).
    pub file_size_bytes: Option<u64>,
    /// Open file descriptors (`RLIMIT_NOFILE`).
    pub open_files: Option<u64>,
    /// Wall-clock time after which the process group is killed.
    pub wall_clock: Duration,
    /// Bytes kept from each of stdout and stderr; more than this kills the process.
    pub max_output_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            cpu_seconds: Some(60),
            address_space_bytes: Some(4 << 30),
            processes: Some(256),
            file_size_bytes: Some(64 << 20),
            open_files: Some(1024),
            wall_clock: Duration::from_secs(120),
            max_output_bytes: 1 << 20,
        }
    }
}

impl Limits {
    /// Check that the limits are usable.
    ///
    /// # Errors
    /// A limit is zero, or the timeout or output cap is above its ceiling.
    pub fn validate(&self) -> Result<()> {
        let rlimits = [
            ("cpu_seconds", self.cpu_seconds),
            ("address_space_bytes", self.address_space_bytes),
            ("processes", self.processes),
            ("file_size_bytes", self.file_size_bytes),
            ("open_files", self.open_files),
        ];
        for (name, value) in rlimits {
            if value == Some(0) {
                return Err(KelluError::config(format!("limits.{name}: must not be 0")));
            }
        }
        if self.wall_clock.is_zero() || self.wall_clock > MAX_WALL_CLOCK {
            return Err(KelluError::config(
                "limits.wall_clock: expected between 1 millisecond and 1 hour",
            ));
        }
        if self.max_output_bytes == 0 || self.max_output_bytes > MAX_OUTPUT_BYTES {
            return Err(KelluError::config(
                "limits.max_output_bytes: expected between 1 byte and 64 MiB",
            ));
        }
        Ok(())
    }

    /// Whether any `rlimit` is set, which means `prlimit` is required.
    #[must_use]
    pub fn has_rlimits(&self) -> bool {
        self.cpu_seconds.is_some()
            || self.address_space_bytes.is_some()
            || self.processes.is_some()
            || self.file_size_bytes.is_some()
            || self.open_files.is_some()
    }

    /// The `prlimit` options for the limits that are set (a single value sets soft and hard).
    #[must_use]
    pub fn prlimit_args(&self) -> Vec<String> {
        [
            ("cpu", self.cpu_seconds),
            ("as", self.address_space_bytes),
            ("nproc", self.processes),
            ("fsize", self.file_size_bytes),
            ("nofile", self.open_files),
        ]
        .into_iter()
        .filter_map(|(name, value)| value.map(|v| format!("--{name}={v}")))
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_valid_and_limit_everything() {
        let limits = Limits::default();
        limits.validate().unwrap();
        assert!(limits.has_rlimits());
        assert_eq!(
            limits.prlimit_args(),
            [
                "--cpu=60",
                "--as=4294967296",
                "--nproc=256",
                "--fsize=67108864",
                "--nofile=1024"
            ]
        );
    }

    #[test]
    fn unset_limits_are_not_passed_to_prlimit() {
        let limits = Limits {
            cpu_seconds: None,
            address_space_bytes: None,
            processes: Some(10),
            file_size_bytes: None,
            open_files: None,
            ..Limits::default()
        };
        assert_eq!(limits.prlimit_args(), ["--nproc=10"]);
        let none = Limits {
            processes: None,
            ..limits
        };
        assert!(!none.has_rlimits());
        assert!(none.prlimit_args().is_empty());
    }

    #[test]
    fn nonsense_limits_are_refused() {
        let base = Limits::default();
        let bad = [
            Limits {
                cpu_seconds: Some(0),
                ..base
            },
            Limits {
                processes: Some(0),
                ..base
            },
            Limits {
                wall_clock: Duration::ZERO,
                ..base
            },
            Limits {
                wall_clock: MAX_WALL_CLOCK + Duration::from_secs(1),
                ..base
            },
            Limits {
                max_output_bytes: 0,
                ..base
            },
            Limits {
                max_output_bytes: MAX_OUTPUT_BYTES + 1,
                ..base
            },
        ];
        for limits in bad {
            assert!(limits.validate().is_err(), "{limits:?}");
        }
    }
}
