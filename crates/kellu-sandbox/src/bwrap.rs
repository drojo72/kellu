//! The bubblewrap command line.
//!
//! The command is always an `argv` **list**: no shell is involved at any point. Everything that
//! varies (the staging directory, the user's command) is passed as a separate element, and the
//! user's command follows a `--`, so it can never be read as a bubblewrap option.

use std::ffi::OsString;
use std::fmt;
use std::path::Path;

use crate::policy::{
    Access, SANDBOX_HOME, SANDBOX_PATH, SANDBOX_TMP, SystemEntry, SystemLayout, WORK_DIR,
};

/// The first bubblewrap release that fixes GHSA-pxhw-h44j-8pfx (setup-time symlink traversal).
pub const FIRST_PATCHED: BwrapVersion = BwrapVersion {
    major: 0,
    minor: 12,
    patch: 0,
};

/// Host files that a sandbox with network access needs to resolve names and check certificates.
/// `resolv.conf` is often a link into `/run`, which is not mounted: name resolution may still fail.
const NETWORK_ETC: &[&str] = &[
    "/etc/resolv.conf",
    "/etc/hosts",
    "/etc/nsswitch.conf",
    "/etc/ssl",
    "/etc/ca-certificates",
];

/// A bubblewrap version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BwrapVersion {
    /// Major number.
    pub major: u32,
    /// Minor number.
    pub minor: u32,
    /// Patch number.
    pub patch: u32,
}

impl BwrapVersion {
    /// Parse the output of `bwrap --version` (`bubblewrap 0.9.0`).
    ///
    /// Returns `None` for anything that is not `name MAJOR.MINOR[.PATCH][suffix]`.
    #[must_use]
    pub fn parse(output: &str) -> Option<Self> {
        let line = output.lines().next()?.trim();
        let (name, number) = line.split_once(' ')?;
        if !name.eq_ignore_ascii_case("bubblewrap") {
            return None;
        }
        let mut parts = number.trim().split('.');
        let leading = |part: &str| -> Option<u32> {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        };
        let major = leading(parts.next()?)?;
        let minor = leading(parts.next()?)?;
        let patch = parts.next().map_or(Some(0), leading)?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }

    /// Whether this release contains the fix for GHSA-pxhw-h44j-8pfx.
    #[must_use]
    pub fn is_patched(self) -> bool {
        self >= FIRST_PATCHED
    }
}

impl fmt::Display for BwrapVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Everything that shapes one bubblewrap invocation.
pub(crate) struct Spec<'a> {
    pub system: &'a SystemLayout,
    pub network: bool,
    /// The staging directory and how it is mounted at [`WORK_DIR`]; `None` mounts nothing.
    pub work: Option<(&'a Path, Access)>,
    pub env: &'a [(String, String)],
    pub command: &'a [OsString],
}

fn push(args: &mut Vec<OsString>, items: &[&str]) {
    args.extend(items.iter().map(OsString::from));
}

fn push_path(args: &mut Vec<OsString>, flag: &str, source: &Path, dest: &Path) {
    args.push(flag.into());
    args.push(source.into());
    args.push(dest.into());
}

/// The arguments for `bwrap` (without the program itself).
///
/// Mount invariant: the only thing mounted from outside the system is the staging directory, as a
/// leaf at [`WORK_DIR`] on a fresh tmpfs root. There are no nested binds and no symlinks in
/// destination paths other than the system links of a merged-`/usr` host.
pub(crate) fn args(spec: &Spec<'_>) -> Vec<OsString> {
    let mut args = Vec::new();
    // `--unshare-user` is explicit so that a missing user namespace is an error and never a
    // silent fallback to running unconfined with a setuid bwrap.
    push(
        &mut args,
        &[
            "--unshare-all",
            "--unshare-user",
            "--disable-userns",
            "--die-with-parent",
            "--new-session",
            "--cap-drop",
            "ALL",
            "--clearenv",
        ],
    );
    if spec.network {
        push(&mut args, &["--share-net"]);
    }
    for entry in spec.system.entries() {
        match entry {
            SystemEntry::Bind(path) => push_path(&mut args, "--ro-bind", path, path),
            SystemEntry::BindTry(path) => push_path(&mut args, "--ro-bind-try", path, path),
            SystemEntry::Symlink { target, dest } => {
                push_path(&mut args, "--symlink", target, dest);
            }
        }
    }
    if spec.network {
        for path in NETWORK_ETC {
            let path = Path::new(path);
            push_path(&mut args, "--ro-bind-try", path, path);
        }
    }
    push(&mut args, &["--proc", "/proc", "--dev", "/dev"]);
    push(
        &mut args,
        &["--tmpfs", SANDBOX_HOME, "--tmpfs", SANDBOX_TMP],
    );
    let chdir = match spec.work {
        Some((source, access)) => {
            let flag = match access {
                Access::ReadOnly => "--ro-bind",
                Access::ReadWrite => "--bind",
            };
            push_path(&mut args, flag, source, Path::new(WORK_DIR));
            WORK_DIR
        }
        None => "/",
    };
    push(&mut args, &["--chdir", chdir]);
    for (name, value) in [
        ("PATH", SANDBOX_PATH),
        ("HOME", SANDBOX_HOME),
        ("TMPDIR", SANDBOX_TMP),
    ] {
        push(&mut args, &["--setenv", name, value]);
    }
    for (name, value) in spec.env {
        push(&mut args, &["--setenv", name, value]);
    }
    args.push("--".into());
    args.extend(spec.command.iter().cloned());
    args
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn layout() -> SystemLayout {
        SystemLayout::new(vec![
            SystemEntry::Bind(PathBuf::from("/usr")),
            SystemEntry::Symlink {
                target: PathBuf::from("usr/bin"),
                dest: PathBuf::from("/bin"),
            },
            SystemEntry::BindTry(PathBuf::from("/etc/ld.so.cache")),
        ])
    }

    fn strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn build(network: bool, access: Option<Access>, command: &[&str]) -> Vec<String> {
        let command: Vec<OsString> = command.iter().map(OsString::from).collect();
        let system = layout();
        let staging = PathBuf::from("/cache/kellu/sandbox/run-1/work");
        let spec = Spec {
            system: &system,
            network,
            work: access.map(|a| (staging.as_path(), a)),
            env: &[],
            command: &command,
        };
        strings(&args(&spec))
    }

    #[test]
    fn versions_are_parsed_and_compared() {
        let parse = BwrapVersion::parse;
        assert_eq!(
            parse("bubblewrap 0.9.0\n"),
            Some(BwrapVersion {
                major: 0,
                minor: 9,
                patch: 0
            })
        );
        assert_eq!(parse("bubblewrap 0.12.0"), Some(FIRST_PATCHED));
        assert_eq!(
            parse("bubblewrap 0.11.1-rc1").map(|v| v.to_string()),
            Some("0.11.1".to_owned())
        );
        assert_eq!(
            parse("bubblewrap 1.0").map(|v| v.to_string()),
            Some("1.0.0".to_owned())
        );
        for bad in [
            "",
            "bwrap 0.9.0",
            "bubblewrap",
            "bubblewrap x.y",
            "bubblewrap 0",
            "bubblewrap 0.9.0.1",
            "0.9.0",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_fix_starts_at_0_12_0() {
        let v = |m, n, p| BwrapVersion {
            major: m,
            minor: n,
            patch: p,
        };
        assert!(!v(0, 9, 0).is_patched());
        assert!(!v(0, 11, 99).is_patched());
        assert!(v(0, 12, 0).is_patched());
        assert!(v(0, 12, 1).is_patched());
        assert!(v(1, 0, 0).is_patched());
    }

    #[test]
    fn the_defaults_isolate_everything_and_share_nothing() {
        let args = build(false, Some(Access::ReadOnly), &["true"]);
        for flag in [
            "--unshare-all",
            "--unshare-user",
            "--disable-userns",
            "--die-with-parent",
            "--new-session",
            "--clearenv",
        ] {
            assert!(args.iter().any(|a| a == flag), "{flag}");
        }
        assert!(!args.iter().any(|a| a == "--share-net"));
        let cap = args.iter().position(|a| a == "--cap-drop").unwrap();
        assert_eq!(args[cap + 1], "ALL");
    }

    #[test]
    fn the_network_is_shared_only_when_asked() {
        let on = build(true, Some(Access::ReadOnly), &["true"]);
        assert!(on.iter().any(|a| a == "--share-net"));
        assert!(on.iter().any(|a| a == "/etc/resolv.conf"));
        let off = build(false, Some(Access::ReadOnly), &["true"]);
        assert!(!off.iter().any(|a| a == "/etc/resolv.conf"));
    }

    #[test]
    fn the_command_follows_a_double_dash_and_is_never_an_option() {
        let args = build(false, None, &["--bind", "/", "/mnt", "-x"]);
        let dashes = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(args[dashes + 1..], ["--bind", "/", "/mnt", "-x"]);
        // Nothing before the separator comes from the command.
        assert!(!args[..dashes].iter().any(|a| a == "/mnt"));
    }

    /// Every bind-like mount as `(flag, source, destination)`.
    fn mounts(args: &[String]) -> Vec<(&str, &str, &str)> {
        args.windows(3)
            .filter(|w| {
                matches!(
                    w[0].as_str(),
                    "--bind" | "--ro-bind" | "--ro-bind-try" | "--bind-try"
                )
            })
            .map(|w| (w[0].as_str(), w[1].as_str(), w[2].as_str()))
            .collect()
    }

    #[test]
    fn the_staging_directory_is_the_only_mount_from_outside_the_system() {
        let staging = "/cache/kellu/sandbox/run-1/work";
        for (access, flag) in [
            (Access::ReadWrite, "--bind"),
            (Access::ReadOnly, "--ro-bind"),
        ] {
            let args = build(false, Some(access), &["true"]);
            let outside: Vec<_> = mounts(&args)
                .into_iter()
                .filter(|(_, source, _)| !source.starts_with("/usr") && !source.starts_with("/etc"))
                .collect();
            assert_eq!(outside, [(flag, staging, WORK_DIR)]);
            // Nothing is mounted below the project mount.
            assert!(
                mounts(&args)
                    .iter()
                    .all(|(_, _, dest)| !dest.starts_with("/work/"))
            );
        }
        let none = build(false, None, &["true"]);
        assert!(mounts(&none).iter().all(|(_, _, dest)| *dest != WORK_DIR));
        assert!(none.windows(2).any(|w| w == ["--chdir", "/"]));
    }

    #[test]
    fn home_and_tmp_are_empty_tmpfs_and_the_environment_is_built_from_nothing() {
        let args = build(false, Some(Access::ReadOnly), &["true"]);
        assert!(args.windows(2).any(|w| w == ["--tmpfs", SANDBOX_HOME]));
        assert!(args.windows(2).any(|w| w == ["--tmpfs", SANDBOX_TMP]));
        let clear = args.iter().position(|a| a == "--clearenv").unwrap();
        let first_setenv = args.iter().position(|a| a == "--setenv").unwrap();
        assert!(clear < first_setenv);
        assert!(
            args.windows(3)
                .any(|w| w == ["--setenv", "HOME", SANDBOX_HOME])
        );
    }
}
