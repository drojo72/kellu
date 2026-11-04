//! `Grant`: what one session may do, as a single object.
//!
//! Read access is granted when the user opens a project and is read-only. Write access is never
//! standing: it is granted per proposal, only for the approved items and only with the hash the
//! user saw. Network access is off by default. A grant expires.
//!
//! In M4 the sandbox mounts are derived from this same object, so the path policy and the mounts
//! cannot drift apart (a test will assert that they match).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kellu_common::{KelluError, Result};

use crate::hash::is_sha256_hex;
use crate::scope::Scope;

/// The permission one approved write carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteApproval {
    /// SHA-256 of the file as the user saw it; `None` when the file is being created.
    pub base_hash: Option<String>,
}

/// What a session may do, until it expires.
#[derive(Debug, Clone)]
pub struct Grant {
    root: PathBuf,
    read: BTreeSet<PathBuf>,
    write: BTreeMap<PathBuf, WriteApproval>,
    network: bool,
    expires: Instant,
}

impl Grant {
    /// A read-only grant over `files` (paths relative to the scope root) that lasts `ttl`.
    ///
    /// Files that the scope denies are dropped, so a denied file never appears readable.
    #[must_use]
    pub fn read_only<I>(scope: &Scope, files: I, now: Instant, ttl: Duration) -> Self
    where
        I: IntoIterator<Item = PathBuf>,
    {
        let read = files
            .into_iter()
            .filter(|file| !scope.is_denied(file))
            .collect();
        Self {
            root: scope.root().to_path_buf(),
            read,
            write: BTreeMap::new(),
            network: false,
            expires: now + ttl,
        }
    }

    /// The project root this grant is about.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether `relative` may be read at time `now`.
    #[must_use]
    pub fn can_read(&self, relative: &Path, now: Instant) -> bool {
        !self.is_expired(now) && self.read.contains(relative)
    }

    /// The readable files, in order.
    pub fn readable(&self) -> impl Iterator<Item = &Path> {
        self.read.iter().map(PathBuf::as_path)
    }

    /// Approve writing `relative` for the user's decision on one proposal.
    ///
    /// # Errors
    /// The path is denied, or `base_hash` is not a SHA-256 digest.
    pub fn approve_write(
        &mut self,
        scope: &Scope,
        relative: PathBuf,
        base_hash: Option<String>,
    ) -> Result<()> {
        if scope.is_denied(&relative) {
            return Err(KelluError::security(format!(
                "{} is denied by policy",
                relative.display()
            )));
        }
        if base_hash
            .as_deref()
            .is_some_and(|hash| !is_sha256_hex(hash))
        {
            return Err(KelluError::security(
                "the base hash is not a SHA-256 digest",
            ));
        }
        self.write.insert(relative, WriteApproval { base_hash });
        Ok(())
    }

    /// The approval for writing `relative` at time `now`, if there is one and it has not expired.
    #[must_use]
    pub fn write_approval(&self, relative: &Path, now: Instant) -> Option<&WriteApproval> {
        if self.is_expired(now) {
            None
        } else {
            self.write.get(relative)
        }
    }

    /// Whether network access is granted at time `now` (never by default).
    #[must_use]
    pub fn allows_network(&self, now: Instant) -> bool {
        self.network && !self.is_expired(now)
    }

    /// Grant network access (only after the user confirmed it).
    pub fn allow_network(&mut self) {
        self.network = true;
    }

    /// Whether the grant has expired at time `now`.
    #[must_use]
    pub fn is_expired(&self, now: Instant) -> bool {
        now >= self.expires
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::deny::DenyGlobs;
    use crate::hash::sha256_hex;

    fn scope() -> (tempfile::TempDir, Scope) {
        let tmp = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(tmp.path()).unwrap();
        let project = base.join("home/project");
        fs::create_dir_all(project.join(".git")).unwrap();
        let scope = Scope::open(&project, &base.join("home"), DenyGlobs::new(), false).unwrap();
        (tmp, scope)
    }

    fn files(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    const TTL: Duration = Duration::from_secs(60);

    #[test]
    fn a_new_grant_is_read_only_without_network() {
        let (_tmp, scope) = scope();
        let now = Instant::now();
        let grant = Grant::read_only(&scope, files(&["src/main.rs"]), now, TTL);
        assert!(grant.can_read(Path::new("src/main.rs"), now));
        assert!(!grant.can_read(Path::new("src/other.rs"), now));
        assert!(
            grant
                .write_approval(Path::new("src/main.rs"), now)
                .is_none()
        );
        assert!(!grant.allows_network(now));
    }

    #[test]
    fn denied_files_never_become_readable() {
        let (_tmp, scope) = scope();
        let now = Instant::now();
        let grant = Grant::read_only(
            &scope,
            files(&[".env", "id_rsa", "src/main.rs", "../x"]),
            now,
            TTL,
        );
        assert_eq!(
            grant.readable().collect::<Vec<_>>(),
            [Path::new("src/main.rs")]
        );
    }

    #[test]
    fn writes_are_granted_per_file_with_the_hash_the_user_saw() {
        let (_tmp, scope) = scope();
        let now = Instant::now();
        let mut grant = Grant::read_only(&scope, files(&["a.rs"]), now, TTL);
        let hash = sha256_hex(b"old content");
        grant
            .approve_write(&scope, PathBuf::from("a.rs"), Some(hash.clone()))
            .unwrap();
        assert_eq!(
            grant
                .write_approval(Path::new("a.rs"), now)
                .unwrap()
                .base_hash
                .as_deref(),
            Some(hash.as_str())
        );
        assert!(grant.write_approval(Path::new("b.rs"), now).is_none());
    }

    #[test]
    fn a_denied_path_cannot_be_approved_for_writing() {
        let (_tmp, scope) = scope();
        let mut grant = Grant::read_only(&scope, files(&[]), Instant::now(), TTL);
        assert!(
            grant
                .approve_write(&scope, PathBuf::from(".env"), None)
                .is_err()
        );
        assert!(
            grant
                .approve_write(&scope, PathBuf::from("../escape"), None)
                .is_err()
        );
        assert!(
            grant
                .approve_write(&scope, PathBuf::from("/etc/passwd"), None)
                .is_err()
        );
    }

    #[test]
    fn a_malformed_base_hash_is_refused() {
        let (_tmp, scope) = scope();
        let mut grant = Grant::read_only(&scope, files(&[]), Instant::now(), TTL);
        for bad in ["", "abc", "ZZ", &"A".repeat(64)] {
            assert!(
                grant
                    .approve_write(&scope, PathBuf::from("a.rs"), Some(bad.to_owned()))
                    .is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn everything_stops_when_the_grant_expires() {
        let (_tmp, scope) = scope();
        let now = Instant::now();
        let mut grant = Grant::read_only(&scope, files(&["a.rs"]), now, TTL);
        grant
            .approve_write(&scope, PathBuf::from("a.rs"), None)
            .unwrap();
        grant.allow_network();
        let later = now + TTL;
        assert!(grant.is_expired(later));
        assert!(!grant.can_read(Path::new("a.rs"), later));
        assert!(grant.write_approval(Path::new("a.rs"), later).is_none());
        assert!(!grant.allows_network(later));
        assert!(grant.can_read(Path::new("a.rs"), now));
    }
}
