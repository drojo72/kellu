//! What a sandboxed run may see and do, derived from one [`Grant`].
//!
//! The policy records the files the grant lets the session read, whether the grant allows the
//! network, the profile and the limits. Nothing else feeds the sandbox: the staging copy is built
//! from [`Policy::files`] and the network flag comes from the grant, so the path policy and the
//! mounts cannot drift apart.
//!
//! Mount invariant (what keeps the bubblewrap setup-time symlink flaw, GHSA-pxhw-h44j-8pfx, from
//! applying): bubblewrap never creates a mount point inside a project tree. The project is always
//! a **leaf** mount at the fixed destination [`WORK_DIR`] on a fresh tmpfs root, with no nested
//! binds inside it and no symlinks in destination paths.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use kellu_common::{KelluError, Result};
use kellu_security::Grant;
use kellu_security::env::{TOOL_ALLOWLIST, scrub};

use crate::limits::Limits;

/// Where the staging copy of the project appears inside the sandbox.
pub const WORK_DIR: &str = "/work";
/// The sandbox's `HOME`: an empty tmpfs, so no host home directory is ever visible.
pub const SANDBOX_HOME: &str = "/home";
/// The sandbox's `TMPDIR`: an empty tmpfs.
pub const SANDBOX_TMP: &str = "/tmp";
/// The sandbox's `PATH`.
pub const SANDBOX_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// How a path is mounted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Writes and new files fail.
    ReadOnly,
    /// The sandboxed process may change the mounted directory.
    ReadWrite,
}

/// What the sandboxed process may do with the project.
///
/// Either way it works on a **staging copy**: the real project is never mounted. Network access
/// is not a profile: it comes from the grant ([`Grant::allows_network`]). The `unsafe` profile of
/// the design is deliberately not implemented yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// The copy is mounted read-only (tests, linters, searches).
    ReadOnly,
    /// The copy is mounted writable (formatters, generators); the result comes back as a diff.
    WorkspaceWrite,
}

impl Profile {
    /// The access the staging copy gets under this profile.
    #[must_use]
    pub fn access(self) -> Access {
        match self {
            Self::ReadOnly => Access::ReadOnly,
            Self::WorkspaceWrite => Access::ReadWrite,
        }
    }
}

/// One piece of the read-only system the sandbox needs to run programs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemEntry {
    /// A host directory mounted read-only at the same path.
    Bind(PathBuf),
    /// Like [`SystemEntry::Bind`], but skipped when the path does not exist.
    BindTry(PathBuf),
    /// A symlink created in the sandbox (merged-`/usr` systems link `/bin` to `usr/bin`).
    Symlink {
        /// What the link points to.
        target: PathBuf,
        /// Where the link appears.
        dest: PathBuf,
    },
}

/// Host directories that are mounted read-only, or linked when the host links them.
const SYSTEM_DIRS: &[&str] = &[
    "/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32",
];
/// Files the dynamic linker and `alternatives` symlinks need; nothing else from `/etc`.
const SYSTEM_ETC: &[&str] = &["/etc/ld.so.cache", "/etc/alternatives"];

/// The read-only system part of the sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemLayout {
    entries: Vec<SystemEntry>,
}

impl SystemLayout {
    /// A layout made of explicit `entries` (tests and unusual hosts).
    #[must_use]
    pub fn new(entries: Vec<SystemEntry>) -> Self {
        Self { entries }
    }

    /// Inspect the host. Only the standard system directories are included: `/usr` and the
    /// directories that merged-`/usr` systems link to it, plus the linker cache. Never `/home`,
    /// `/root`, `/var`, `/run` or the rest of `/etc`.
    #[must_use]
    pub fn host() -> Self {
        let mut entries = Vec::new();
        for dir in SYSTEM_DIRS {
            let path = Path::new(dir);
            let Ok(meta) = fs::symlink_metadata(path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                // Only links that stay inside the system tree are recreated.
                if let Ok(target) = fs::read_link(path)
                    && link_target_is_system(&target)
                {
                    entries.push(SystemEntry::Symlink {
                        target,
                        dest: path.to_path_buf(),
                    });
                }
            } else if meta.is_dir() {
                entries.push(SystemEntry::Bind(path.to_path_buf()));
            }
        }
        entries.extend(
            SYSTEM_ETC
                .iter()
                .map(|path| SystemEntry::BindTry(PathBuf::from(path))),
        );
        Self { entries }
    }

    /// The entries, in mount order.
    #[must_use]
    pub fn entries(&self) -> &[SystemEntry] {
        &self.entries
    }

    /// Whether `path` (absolute, as on the host) is also visible at the same path inside the
    /// sandbox, because it lies under a directory or link this layout provides.
    #[must_use]
    pub fn provides(&self, path: &Path) -> bool {
        self.entries.iter().any(|entry| match entry {
            SystemEntry::Bind(dir) | SystemEntry::BindTry(dir) => path.starts_with(dir),
            SystemEntry::Symlink { dest, .. } => path.starts_with(dest),
        })
    }
}

/// A link target is acceptable when it is relative without `..`, or absolute under `/usr`.
fn link_target_is_system(target: &Path) -> bool {
    if target.is_absolute() {
        target.starts_with("/usr")
            && target
                .components()
                .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
    } else {
        target
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
            && target.components().next().is_some()
    }
}

/// What one sandboxed run may do. Built from a [`Grant`]; immutable afterwards.
#[derive(Debug, Clone)]
pub struct Policy {
    grant: Grant,
    profile: Profile,
    network: bool,
    root: PathBuf,
    files: BTreeSet<PathBuf>,
    limits: Limits,
    env: Vec<(String, String)>,
}

impl Policy {
    /// Derive the policy from `grant` as of `now`.
    ///
    /// # Errors
    /// The grant has expired, or the limits are invalid.
    pub fn from_grant(
        grant: &Grant,
        profile: Profile,
        limits: Limits,
        now: Instant,
    ) -> Result<Self> {
        if grant.is_expired(now) {
            return Err(KelluError::security(
                "the grant has expired; nothing runs in the sandbox",
            ));
        }
        limits.validate()?;
        Ok(Self {
            grant: grant.clone(),
            profile,
            network: grant.allows_network(now),
            root: grant.root().to_path_buf(),
            files: grant.readable().map(Path::to_path_buf).collect(),
            limits,
            env: Vec::new(),
        })
    }

    /// Pass the variables of `source` that the tool allowlist permits (locale, terminal, time
    /// zone). Anything else, and anything that looks like a secret, is dropped.
    #[must_use]
    pub fn with_env<I>(mut self, source: I) -> Self
    where
        I: IntoIterator<Item = (OsString, OsString)>,
    {
        self.env = scrub(TOOL_ALLOWLIST, source);
        self
    }

    /// Whether the grant this policy came from has expired at `now`. Staging and running check
    /// it again, so a policy kept past its grant does nothing.
    #[must_use]
    pub fn is_expired(&self, now: Instant) -> bool {
        self.grant.is_expired(now)
    }

    /// The profile.
    #[must_use]
    pub fn profile(&self) -> Profile {
        self.profile
    }

    /// Whether the sandbox shares the host network (only when the grant allows it).
    #[must_use]
    pub fn network(&self) -> bool {
        self.network
    }

    /// The project root the grant is about. It is never mounted.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The files the sandbox may see, relative to the root: exactly what the grant lets the
    /// session read.
    #[must_use]
    pub fn files(&self) -> &BTreeSet<PathBuf> {
        &self.files
    }

    /// The resource limits.
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The extra environment variables, already scrubbed.
    #[must_use]
    pub fn env(&self) -> &[(String, String)] {
        &self.env
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use kellu_security::{DenyGlobs, Scope};

    use super::*;

    fn fixture() -> (tempfile::TempDir, Scope) {
        let tmp = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(tmp.path()).unwrap();
        let project = base.join("home/project");
        fs::create_dir_all(project.join(".git")).unwrap();
        let scope = Scope::open(&project, &base.join("home"), DenyGlobs::new(), false).unwrap();
        (tmp, scope)
    }

    fn grant(scope: &Scope, names: &[&str], now: Instant) -> Grant {
        Grant::read_only(
            scope,
            names.iter().map(PathBuf::from),
            now,
            Duration::from_secs(60),
        )
    }

    #[test]
    fn the_policy_is_exactly_what_the_grant_allows() {
        let (_tmp, scope) = fixture();
        let now = Instant::now();
        let g = grant(&scope, &["src/main.rs", ".env", "id_rsa", "a.txt"], now);
        let policy = Policy::from_grant(&g, Profile::ReadOnly, Limits::default(), now).unwrap();
        let from_grant: BTreeSet<PathBuf> = g.readable().map(Path::to_path_buf).collect();
        assert_eq!(policy.files(), &from_grant);
        assert!(!policy.files().contains(Path::new(".env")));
        assert!(!policy.files().contains(Path::new("id_rsa")));
        assert_eq!(policy.root(), scope.root());
    }

    #[test]
    fn network_follows_the_grant_and_is_off_by_default() {
        let (_tmp, scope) = fixture();
        let now = Instant::now();
        let mut g = grant(&scope, &["a.txt"], now);
        let off = Policy::from_grant(&g, Profile::ReadOnly, Limits::default(), now).unwrap();
        assert!(!off.network());
        g.allow_network();
        let on = Policy::from_grant(&g, Profile::ReadOnly, Limits::default(), now).unwrap();
        assert!(on.network());
    }

    #[test]
    fn an_expired_grant_gives_no_policy() {
        let (_tmp, scope) = fixture();
        let now = Instant::now();
        let g = grant(&scope, &["a.txt"], now);
        let later = now + Duration::from_secs(60);
        let error = Policy::from_grant(&g, Profile::ReadOnly, Limits::default(), later)
            .unwrap_err()
            .to_string();
        assert!(error.contains("expired"), "{error}");
    }

    #[test]
    fn invalid_limits_give_no_policy() {
        let (_tmp, scope) = fixture();
        let now = Instant::now();
        let g = grant(&scope, &["a.txt"], now);
        let limits = Limits {
            processes: Some(0),
            ..Limits::default()
        };
        assert!(Policy::from_grant(&g, Profile::ReadOnly, limits, now).is_err());
    }

    #[test]
    fn profiles_map_to_mount_access() {
        assert_eq!(Profile::ReadOnly.access(), Access::ReadOnly);
        assert_eq!(Profile::WorkspaceWrite.access(), Access::ReadWrite);
    }

    #[test]
    fn only_allowlisted_non_secret_variables_reach_the_sandbox() {
        let (_tmp, scope) = fixture();
        let now = Instant::now();
        let g = grant(&scope, &["a.txt"], now);
        let source = [
            ("LANG", "C.UTF-8"),
            ("TERM", "xterm"),
            ("GITHUB_TOKEN", "ghp_secret"),
            ("HOME", "/home/real"),
            ("OPENAI_API_KEY", "sk-secret"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let policy = Policy::from_grant(&g, Profile::ReadOnly, Limits::default(), now)
            .unwrap()
            .with_env(source);
        assert_eq!(
            policy.env(),
            [
                ("LANG".to_owned(), "C.UTF-8".to_owned()),
                ("TERM".to_owned(), "xterm".to_owned())
            ]
        );
    }

    #[test]
    fn the_host_layout_never_includes_user_data() {
        let layout = SystemLayout::host();
        for entry in layout.entries() {
            let path = match entry {
                SystemEntry::Bind(p) | SystemEntry::BindTry(p) => p,
                SystemEntry::Symlink { dest, .. } => dest,
            };
            for forbidden in ["/home", "/root", "/var", "/run", "/mnt", "/media", "/etc"] {
                assert!(
                    path == Path::new("/etc/ld.so.cache")
                        || path == Path::new("/etc/alternatives")
                        || path != Path::new(forbidden),
                    "{path:?}"
                );
            }
            assert!(
                SYSTEM_DIRS.iter().any(|d| path == Path::new(d))
                    || SYSTEM_ETC.iter().any(|d| path == Path::new(d)),
                "{path:?}"
            );
        }
    }

    #[test]
    fn a_path_is_visible_inside_only_if_the_layout_provides_it() {
        let layout = SystemLayout::new(vec![
            SystemEntry::Bind(PathBuf::from("/usr")),
            SystemEntry::Symlink {
                target: PathBuf::from("usr/bin"),
                dest: PathBuf::from("/bin"),
            },
        ]);
        assert!(layout.provides(Path::new("/usr/bin/prlimit")));
        assert!(layout.provides(Path::new("/bin/prlimit")));
        assert!(!layout.provides(Path::new("/opt/tools/prlimit")));
        assert!(!layout.provides(Path::new("/home/user/bin/prlimit")));
        assert!(!layout.provides(Path::new("/usrx/bin/prlimit")));
    }

    #[test]
    fn link_targets_must_stay_inside_the_system_tree() {
        for ok in ["usr/bin", "usr/lib64", "/usr/lib64"] {
            assert!(link_target_is_system(Path::new(ok)), "{ok}");
        }
        for bad in ["", "../etc", "/etc/passwd", "/home/user", "usr/../../x"] {
            assert!(!link_target_is_system(Path::new(bad)), "{bad}");
        }
    }
}
