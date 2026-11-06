//! The staging copy: the only part of a project a sandbox ever sees.
//!
//! The real project is never mounted. The files the grant allows the session to read are copied
//! (through [`Scope`], so with `openat2` and without following symlinks) into a private directory
//! that is mounted at [`crate::policy::WORK_DIR`]. Afterwards [`Staging::changes`] compares what is
//! there with what was copied, so the core can turn the difference into a proposal; this crate
//! never writes anything back.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, DirBuilder, OpenOptions, Permissions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use kellu_common::paths::{self, PRIVATE_DIR_MODE};
use kellu_common::{KelluError, Result};
use kellu_security::Scope;
use kellu_security::hash::sha256_hex;
use rustix::fs::OFlags;

use crate::policy::Policy;

const EXEC_FILE_MODE: u32 = 0o700;
const PLAIN_FILE_MODE: u32 = 0o600;
const NOFOLLOW_FLAGS: i32 = (OFlags::NOFOLLOW.bits() | OFlags::CLOEXEC.bits()) as i32;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Size caps for a staging copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagingLimits {
    /// Most files copied.
    pub max_files: usize,
    /// Largest single file copied (larger ones are skipped and reported).
    pub max_file_bytes: u64,
    /// Most bytes copied in total (exceeding it is an error).
    pub max_total_bytes: u64,
}

impl Default for StagingLimits {
    fn default() -> Self {
        Self {
            max_files: 20_000,
            max_file_bytes: 8 << 20,
            max_total_bytes: 256 << 20,
        }
    }
}

/// Why a granted file is not in the staging copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The scope refused it: a symlink, a special file, outside the root or denied.
    Refused,
    /// It is larger than [`StagingLimits::max_file_bytes`].
    TooLarge,
}

/// A file of the grant that was left out of the copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// Path relative to the project root.
    pub path: PathBuf,
    /// Why it was left out.
    pub reason: SkipReason,
}

/// What happened to one path while a tool ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// A copied file now has different content or a different executable bit.
    Modified {
        /// Path relative to the project root.
        path: PathBuf,
        /// SHA-256 of the file as it was copied (what the user saw).
        base_hash: String,
        /// SHA-256 of the file now.
        new_hash: String,
    },
    /// A regular file that was not in the copy.
    Created {
        /// Path relative to the project root.
        path: PathBuf,
        /// SHA-256 of the new file.
        new_hash: String,
    },
    /// A copied file that is gone.
    Deleted {
        /// Path relative to the project root.
        path: PathBuf,
        /// SHA-256 of the file as it was copied.
        base_hash: String,
    },
    /// Something that can never become a proposal: a symlink, a device, a pipe, a file that is
    /// too large. It is reported and never applied.
    Unsafe {
        /// Path relative to the project root.
        path: PathBuf,
        /// What it is.
        what: &'static str,
    },
}

impl Change {
    /// The path the change is about.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Modified { path, .. }
            | Self::Created { path, .. }
            | Self::Deleted { path, .. }
            | Self::Unsafe { path, .. } => path,
        }
    }
}

#[derive(Debug, Clone)]
struct Copied {
    hash: String,
    executable: bool,
}

/// A staging copy on disk. It is deleted when dropped.
#[derive(Debug)]
pub struct Staging {
    parent: PathBuf,
    work: PathBuf,
    root: PathBuf,
    copied: BTreeMap<PathBuf, Copied>,
    skipped: Vec<Skipped>,
    limits: StagingLimits,
}

/// A relative path made only of normal components, written canonically (no `.`, no trailing or
/// doubled slash), so two spellings can never name the same file.
fn is_plain_relative(path: &Path) -> bool {
    path.components().next().is_some()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && path.components().collect::<PathBuf>().as_os_str() == path.as_os_str()
}

fn unique_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "run-{}-{nanos}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

impl Staging {
    /// Copy the files `policy` allows into a new private directory under `base`.
    ///
    /// `base` is created with mode 0700 when missing and must be a real directory of this user.
    /// Files that the scope refuses or that are too large are skipped and listed in
    /// [`Staging::skipped`]; files that vanished are ignored.
    ///
    /// # Errors
    /// The policy expired, `scope` is not the project of the policy, a size cap is exceeded, or the
    /// copy cannot be written. A partial copy is deleted.
    pub fn create(
        base: &Path,
        scope: &Scope,
        policy: &Policy,
        limits: StagingLimits,
        now: Instant,
    ) -> Result<Self> {
        if policy.is_expired(now) {
            return Err(KelluError::security(
                "the grant has expired; nothing is staged",
            ));
        }
        if scope.root() != policy.root() {
            return Err(KelluError::security(
                "the scope is not the project of this policy",
            ));
        }
        paths::ensure_private_dir(base)?;
        let parent = base.join(unique_name());
        let work = parent.join("work");
        for dir in [&parent, &work] {
            DirBuilder::new()
                .mode(PRIVATE_DIR_MODE)
                .create(dir)
                .map_err(|e| KelluError::io(dir, e))?;
        }
        let mut staging = Self {
            parent,
            work,
            root: policy.root().to_path_buf(),
            copied: BTreeMap::new(),
            skipped: Vec::new(),
            limits,
        };
        // On error `staging` is dropped here, which deletes the partial copy.
        staging.copy_files(scope, policy.files())?;
        Ok(staging)
    }

    fn copy_files(&mut self, scope: &Scope, files: &BTreeSet<PathBuf>) -> Result<()> {
        let mut total: u64 = 0;
        for relative in files {
            if !is_plain_relative(relative) {
                self.skipped.push(Skipped {
                    path: relative.clone(),
                    reason: SkipReason::Refused,
                });
                continue;
            }
            let mut file = match scope.open_file(relative) {
                Ok(file) => file,
                Err(error) if error.io_kind() == Some(std::io::ErrorKind::NotFound) => continue,
                Err(_) => {
                    self.skipped.push(Skipped {
                        path: relative.clone(),
                        reason: SkipReason::Refused,
                    });
                    continue;
                }
            };
            let meta = file.metadata().map_err(|e| KelluError::io(relative, e))?;
            let mut data = Vec::new();
            (&mut file)
                .take(self.limits.max_file_bytes.saturating_add(1))
                .read_to_end(&mut data)
                .map_err(|e| KelluError::io(relative, e))?;
            if data.len() as u64 > self.limits.max_file_bytes {
                self.skipped.push(Skipped {
                    path: relative.clone(),
                    reason: SkipReason::TooLarge,
                });
                continue;
            }
            if self.copied.len() >= self.limits.max_files {
                return Err(KelluError::security(format!(
                    "the grant has more than {} files; narrow it before running a tool",
                    self.limits.max_files
                )));
            }
            total = total.saturating_add(data.len() as u64);
            if total > self.limits.max_total_bytes {
                return Err(KelluError::security(format!(
                    "the granted files are larger than {} bytes in total; narrow the grant",
                    self.limits.max_total_bytes
                )));
            }
            let executable = meta.mode() & 0o111 != 0;
            self.write_file(relative, &data, executable)?;
            self.copied.insert(
                relative.clone(),
                Copied {
                    hash: sha256_hex(&data),
                    executable,
                },
            );
        }
        Ok(())
    }

    fn write_file(&self, relative: &Path, data: &[u8], executable: bool) -> Result<()> {
        let target = self.work.join(relative);
        if let Some(dir) = target.parent() {
            DirBuilder::new()
                .recursive(true)
                .mode(PRIVATE_DIR_MODE)
                .create(dir)
                .map_err(|e| KelluError::io(dir, e))?;
        }
        let mode = if executable {
            EXEC_FILE_MODE
        } else {
            PLAIN_FILE_MODE
        };
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .custom_flags(NOFOLLOW_FLAGS)
            .open(&target)
            .map_err(|e| KelluError::io(&target, e))?;
        out.write_all(data)
            .map_err(|e| KelluError::io(&target, e))?;
        // `create_new` applies the umask; set the exact mode afterwards.
        out.set_permissions(Permissions::from_mode(mode))
            .map_err(|e| KelluError::io(&target, e))
    }

    /// The directory that is mounted into the sandbox.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.work
    }

    /// The project root this copy came from.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The files that were copied, relative to the project root.
    pub fn files(&self) -> impl Iterator<Item = &Path> {
        self.copied.keys().map(PathBuf::as_path)
    }

    /// The granted files that were left out of the copy.
    #[must_use]
    pub fn skipped(&self) -> &[Skipped] {
        &self.skipped
    }

    /// Compare the directory with what was copied. Nothing is followed, written or applied.
    ///
    /// # Errors
    /// The directory cannot be listed, or holds more entries than the copy limits allow.
    pub fn changes(&self) -> Result<Vec<Change>> {
        let mut changes = Vec::new();
        let mut seen = BTreeSet::new();
        let mut budget = self.limits.max_files.saturating_mul(2).saturating_add(64);
        let mut pending = vec![PathBuf::new()];
        while let Some(relative) = pending.pop() {
            let dir = self.work.join(&relative);
            let entries = fs::read_dir(&dir).map_err(|e| KelluError::io(&dir, e))?;
            for entry in entries {
                let entry = entry.map_err(|e| KelluError::io(&dir, e))?;
                budget = budget.checked_sub(1).ok_or_else(|| {
                    KelluError::security("the staging directory holds too many entries")
                })?;
                let path = relative.join(entry.file_name());
                let file_type = entry.file_type().map_err(|e| KelluError::io(&dir, e))?;
                if file_type.is_dir() {
                    pending.push(path);
                } else if file_type.is_file() {
                    self.compare_file(path, &mut seen, &mut changes)?;
                } else {
                    let what = if file_type.is_symlink() {
                        "a symlink"
                    } else {
                        "a special file"
                    };
                    changes.push(Change::Unsafe { path, what });
                }
            }
        }
        for (path, copied) in &self.copied {
            if !seen.contains(path)
                && !changes
                    .iter()
                    .any(|change| matches!(change, Change::Unsafe { path: p, .. } if p == path))
            {
                changes.push(Change::Deleted {
                    path: path.clone(),
                    base_hash: copied.hash.clone(),
                });
            }
        }
        changes.sort_by(|a, b| a.path().cmp(b.path()));
        Ok(changes)
    }

    fn compare_file(
        &self,
        path: PathBuf,
        seen: &mut BTreeSet<PathBuf>,
        changes: &mut Vec<Change>,
    ) -> Result<()> {
        let full = self.work.join(&path);
        // Opened without following links, so a file swapped for a symlink cannot be read through.
        let mut file = match OpenOptions::new()
            .read(true)
            .custom_flags(NOFOLLOW_FLAGS | OFlags::NONBLOCK.bits() as i32)
            .open(&full)
        {
            Ok(file) => file,
            Err(_) => {
                changes.push(Change::Unsafe {
                    path,
                    what: "a file that cannot be read safely",
                });
                return Ok(());
            }
        };
        let meta = file.metadata().map_err(|e| KelluError::io(&full, e))?;
        let mut data = Vec::new();
        (&mut file)
            .take(self.limits.max_file_bytes.saturating_add(1))
            .read_to_end(&mut data)
            .map_err(|e| KelluError::io(&full, e))?;
        if !meta.is_file() || data.len() as u64 > self.limits.max_file_bytes {
            changes.push(Change::Unsafe {
                path,
                what: "a file that is too large or not regular",
            });
            return Ok(());
        }
        let new_hash = sha256_hex(&data);
        let executable = meta.mode() & 0o111 != 0;
        match self.copied.get(&path) {
            None => changes.push(Change::Created {
                path: path.clone(),
                new_hash,
            }),
            Some(copied) => {
                if copied.hash != new_hash || copied.executable != executable {
                    changes.push(Change::Modified {
                        path: path.clone(),
                        base_hash: copied.hash.clone(),
                        new_hash,
                    });
                }
            }
        }
        seen.insert(path);
        Ok(())
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        // Best effort: `remove_dir_all` does not follow symlinks the sandbox may have created.
        let _ = fs::remove_dir_all(&self.parent);
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;
    use std::time::Duration;

    use kellu_security::{DenyGlobs, Grant};

    use super::*;
    use crate::limits::Limits;
    use crate::policy::Profile;

    const TTL: Duration = Duration::from_secs(60);

    struct Fixture {
        _tmp: tempfile::TempDir,
        project: PathBuf,
        base: PathBuf,
        scope: Scope,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let base_dir = fs::canonicalize(tmp.path()).unwrap();
        let home = base_dir.join("home");
        let project = home.join("project");
        fs::create_dir_all(project.join(".git")).unwrap();
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(project.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(project.join("README.md"), "hello\n").unwrap();
        fs::write(project.join("run.sh"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(
            project.join("run.sh"),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        fs::write(project.join(".env"), "TOKEN=abc\n").unwrap();
        fs::write(base_dir.join("secret.txt"), "outside\n").unwrap();
        let scope = Scope::open(&project, &home, DenyGlobs::new(), false).unwrap();
        Fixture {
            base: base_dir.join("cache"),
            _tmp: tmp,
            project,
            scope,
        }
    }

    fn policy(f: &Fixture, names: &[&str], now: Instant) -> Policy {
        let grant = Grant::read_only(
            &f.scope,
            names.iter().map(PathBuf::from),
            now,
            Duration::from_secs(60),
        );
        Policy::from_grant(&grant, Profile::WorkspaceWrite, Limits::default(), now).unwrap()
    }

    fn stage(f: &Fixture, names: &[&str]) -> Staging {
        let now = Instant::now();
        Staging::create(
            &f.base,
            &f.scope,
            &policy(f, names, now),
            StagingLimits::default(),
            now,
        )
        .unwrap()
    }

    const ALL: &[&str] = &["src/main.rs", "README.md", "run.sh", ".env"];

    #[test]
    fn only_granted_and_not_denied_files_are_copied() {
        let f = fixture();
        let staging = stage(&f, ALL);
        let files: Vec<_> = staging.files().collect();
        assert_eq!(
            files,
            [
                Path::new("README.md"),
                Path::new("run.sh"),
                Path::new("src/main.rs")
            ]
        );
        assert!(!staging.path().join(".env").exists());
        assert!(!staging.path().join(".git").exists());
        assert_eq!(
            fs::read_to_string(staging.path().join("src/main.rs")).unwrap(),
            "fn main() {}\n"
        );
    }

    #[test]
    fn the_copy_is_private_and_keeps_only_the_executable_bit() {
        let f = fixture();
        let staging = stage(&f, ALL);
        let mode = |p: &str| fs::metadata(staging.path().join(p)).unwrap().mode() & 0o7777;
        assert_eq!(mode("run.sh"), 0o700);
        assert_eq!(mode("README.md"), 0o600);
        assert_eq!(mode("src"), 0o700);
        let parent = staging.path().parent().unwrap();
        assert_eq!(fs::metadata(parent).unwrap().mode() & 0o7777, 0o700);
        assert_eq!(fs::metadata(&f.base).unwrap().mode() & 0o7777, 0o700);
    }

    #[test]
    fn symlinks_and_special_files_are_skipped_not_followed() {
        let f = fixture();
        symlink(
            f.project
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("secret.txt"),
            f.project.join("leak.txt"),
        )
        .unwrap();
        symlink("README.md", f.project.join("alias.md")).unwrap();
        rustix::fs::mknodat(
            rustix::fs::CWD,
            f.project.join("pipe"),
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        let staging = stage(&f, &["README.md", "leak.txt", "alias.md", "pipe"]);
        assert_eq!(
            staging.files().collect::<Vec<_>>(),
            [Path::new("README.md")]
        );
        let skipped: Vec<_> = staging
            .skipped()
            .iter()
            .map(|s| (s.path.as_path(), s.reason))
            .collect();
        assert_eq!(
            skipped,
            [
                (Path::new("alias.md"), SkipReason::Refused),
                (Path::new("leak.txt"), SkipReason::Refused),
                (Path::new("pipe"), SkipReason::Refused),
            ]
        );
        assert!(!staging.path().join("leak.txt").exists());
    }

    #[test]
    fn a_path_through_a_symlinked_directory_is_skipped() {
        let f = fixture();
        let outside = f.project.parent().unwrap().parent().unwrap().to_path_buf();
        symlink(&outside, f.project.join("ext")).unwrap();
        let staging = stage(&f, &["ext/secret.txt", "README.md"]);
        assert_eq!(staging.files().count(), 1);
        assert_eq!(staging.skipped().len(), 1);
    }

    #[test]
    fn files_that_vanished_are_ignored() {
        let f = fixture();
        let staging = stage(&f, &["README.md", "gone.rs"]);
        assert_eq!(staging.files().count(), 1);
        assert!(staging.skipped().is_empty());
    }

    #[test]
    fn oversized_files_are_skipped_and_the_total_is_capped() {
        let f = fixture();
        let now = Instant::now();
        let p = policy(&f, &["README.md", "src/main.rs"], now);
        let small = StagingLimits {
            max_file_bytes: 8,
            ..StagingLimits::default()
        };
        let staging = Staging::create(&f.base, &f.scope, &p, small, now).unwrap();
        assert_eq!(
            staging.files().collect::<Vec<_>>(),
            [Path::new("README.md")]
        );
        assert_eq!(staging.skipped()[0].reason, SkipReason::TooLarge);

        let tight = StagingLimits {
            max_total_bytes: 10,
            ..StagingLimits::default()
        };
        let error = Staging::create(&f.base, &f.scope, &p, tight, now).unwrap_err();
        assert!(error.to_string().contains("in total"), "{error}");
        let few = StagingLimits {
            max_files: 1,
            ..StagingLimits::default()
        };
        let error = Staging::create(&f.base, &f.scope, &p, few, now).unwrap_err();
        assert!(error.to_string().contains("more than 1 files"), "{error}");
    }

    #[test]
    fn a_failed_copy_leaves_nothing_behind() {
        let f = fixture();
        let now = Instant::now();
        let p = policy(&f, &["README.md", "src/main.rs"], now);
        let tight = StagingLimits {
            max_total_bytes: 10,
            ..StagingLimits::default()
        };
        assert!(Staging::create(&f.base, &f.scope, &p, tight, now).is_err());
        assert_eq!(fs::read_dir(&f.base).unwrap().count(), 0);
    }

    #[test]
    fn dropping_the_staging_deletes_it_even_with_symlinks_inside() {
        let f = fixture();
        let staging = stage(&f, ALL);
        let parent = staging.path().parent().unwrap().to_path_buf();
        let outside = f.project.parent().unwrap().parent().unwrap().to_path_buf();
        symlink(&outside, staging.path().join("trap")).unwrap();
        drop(staging);
        assert!(!parent.exists());
        assert!(
            outside.join("secret.txt").exists(),
            "a link target was deleted"
        );
        assert!(f.project.join("README.md").exists());
    }

    #[test]
    fn an_expired_grant_or_a_foreign_scope_stages_nothing() {
        let f = fixture();
        let now = Instant::now();
        let p = policy(&f, ALL, now);
        let later = now + Duration::from_secs(60);
        assert!(Staging::create(&f.base, &f.scope, &p, StagingLimits::default(), later).is_err());
        let other = fixture();
        let error =
            Staging::create(&f.base, &other.scope, &p, StagingLimits::default(), now).unwrap_err();
        assert!(error.to_string().contains("not the project"), "{error}");
    }

    #[test]
    fn plain_relative_paths_only() {
        for ok in ["a", "a/b.rs", "-weird name", "a\nb"] {
            assert!(is_plain_relative(Path::new(ok)), "{ok:?}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../b",
            "./a",
            "a/./b",
            "a/",
            "a//b",
        ] {
            assert!(!is_plain_relative(Path::new(bad)), "{bad:?}");
        }
    }

    /// The design asks for a property test that the sandbox contents equal the grant. `proptest`
    /// is a large development dependency for one test, so this generates hostile names with a
    /// fixed-seed xorshift: the same cases on every run.
    #[test]
    fn the_copy_is_exactly_the_grant_for_generated_names() {
        let f = fixture();
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let pieces = [
            "a", "b", "src", "-x", "é", "sp ace", ".env", "id_rsa", "..", ".", " ", "x\ny", ".git",
            "keys.pem", "a.rs",
        ];
        let mut names = BTreeSet::new();
        for _ in 0..400 {
            let depth = 1 + next() % 3;
            let parts: Vec<&str> = (0..depth)
                .map(|_| pieces[usize::try_from(next() % pieces.len() as u64).unwrap()])
                .collect();
            names.insert(parts.join("/"));
        }
        // Only canonical names are created, so a generated `..` can never write outside the
        // fixture; the others still reach the grant and must be refused there or by staging.
        let mut created = BTreeSet::new();
        for name in names.iter().filter(|n| is_plain_relative(Path::new(n))) {
            let path = f.project.join(name);
            let made = path
                .parent()
                .is_some_and(|dir| fs::create_dir_all(dir).is_ok())
                && fs::write(&path, format!("content of {name}")).is_ok();
            if made {
                created.insert(PathBuf::from(name));
            }
        }
        let now = Instant::now();
        let grant = Grant::read_only(&f.scope, names.iter().map(PathBuf::from), now, TTL);
        let policy =
            Policy::from_grant(&grant, Profile::WorkspaceWrite, Limits::default(), now).unwrap();
        let staging =
            Staging::create(&f.base, &f.scope, &policy, StagingLimits::default(), now).unwrap();

        // `PathBuf` compares by components, so `b/.` equals `b`: compare canonical spellings only.
        let granted: BTreeSet<PathBuf> = policy.files().clone();
        let canonical: BTreeSet<PathBuf> = granted
            .iter()
            .filter(|p| is_plain_relative(p))
            .cloned()
            .collect();
        let staged: BTreeSet<PathBuf> = staging.files().map(Path::to_path_buf).collect();
        let expected: BTreeSet<PathBuf> = canonical.intersection(&created).cloned().collect();
        assert!(
            !expected.is_empty(),
            "the generator produced nothing useful"
        );
        assert_eq!(staged, expected);
        // Nothing that exists, is granted and is spelled canonically may be left out.
        for skipped in staging.skipped() {
            assert_eq!(skipped.reason, SkipReason::Refused);
            if is_plain_relative(&skipped.path) {
                assert!(!created.contains(&skipped.path), "{:?}", skipped.path);
            }
        }
        assert!(staged.iter().all(|p| granted.contains(p)));

        // What is on disk is exactly the staged set: nothing extra, nothing outside the grant.
        let mut on_disk = BTreeSet::new();
        let mut pending = vec![PathBuf::new()];
        while let Some(dir) = pending.pop() {
            for entry in fs::read_dir(staging.path().join(&dir)).unwrap() {
                let entry = entry.unwrap();
                let path = dir.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    pending.push(path);
                } else {
                    on_disk.insert(path);
                }
            }
        }
        assert_eq!(on_disk, staged);
        // Denied names never get through, wherever they sit in the tree.
        for path in &staged {
            let name = path.file_name().unwrap().to_string_lossy();
            assert!(
                name != ".env" && name != "id_rsa" && !name.ends_with(".pem"),
                "{path:?}"
            );
            assert!(
                !path.components().any(|c| c.as_os_str() == ".git"),
                "{path:?}"
            );
        }
        assert!(staging.changes().unwrap().is_empty());
    }

    #[test]
    fn an_untouched_copy_has_no_changes() {
        let f = fixture();
        assert!(stage(&f, ALL).changes().unwrap().is_empty());
    }

    #[test]
    fn modified_created_deleted_and_unsafe_entries_are_reported() {
        let f = fixture();
        let staging = stage(&f, ALL);
        let w = staging.path();
        fs::write(w.join("README.md"), "changed\n").unwrap();
        fs::write(w.join("extra.txt"), "new\n").unwrap();
        fs::create_dir(w.join("nested")).unwrap();
        fs::write(w.join("nested/deep.txt"), "deep\n").unwrap();
        fs::remove_file(w.join("src/main.rs")).unwrap();
        symlink("/etc/passwd", w.join("link")).unwrap();
        fs::set_permissions(
            w.join("run.sh"),
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )
        .unwrap();
        let changes = staging.changes().unwrap();
        let kinds: Vec<_> = changes
            .iter()
            .map(|c| {
                let kind = match c {
                    Change::Modified { .. } => "modified",
                    Change::Created { .. } => "created",
                    Change::Deleted { .. } => "deleted",
                    Change::Unsafe { .. } => "unsafe",
                };
                (c.path().to_str().unwrap().to_owned(), kind)
            })
            .collect();
        assert_eq!(
            kinds,
            [
                ("README.md".to_owned(), "modified"),
                ("extra.txt".to_owned(), "created"),
                ("link".to_owned(), "unsafe"),
                ("nested/deep.txt".to_owned(), "created"),
                ("run.sh".to_owned(), "modified"),
                ("src/main.rs".to_owned(), "deleted"),
            ]
        );
        let Change::Modified {
            base_hash,
            new_hash,
            ..
        } = &changes[0]
        else {
            unreachable!("sorted by path");
        };
        assert_eq!(*base_hash, sha256_hex(b"hello\n"));
        assert_eq!(*new_hash, sha256_hex(b"changed\n"));
        // The real project is untouched by anything above.
        assert_eq!(
            fs::read_to_string(f.project.join("README.md")).unwrap(),
            "hello\n"
        );
        assert!(f.project.join("src/main.rs").exists());
    }

    #[test]
    fn a_copied_file_replaced_by_a_symlink_is_unsafe_not_deleted_nor_followed() {
        let f = fixture();
        let staging = stage(&f, ALL);
        let w = staging.path();
        fs::remove_file(w.join("README.md")).unwrap();
        symlink("/etc/passwd", w.join("README.md")).unwrap();
        let changes = staging.changes().unwrap();
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert!(matches!(changes[0], Change::Unsafe { .. }));
    }

    #[test]
    fn the_entry_budget_stops_a_directory_bomb() {
        let f = fixture();
        let now = Instant::now();
        let p = policy(&f, &["README.md"], now);
        let limits = StagingLimits {
            max_files: 1,
            ..StagingLimits::default()
        };
        let staging = Staging::create(&f.base, &f.scope, &p, limits, now).unwrap();
        for i in 0..80 {
            fs::write(staging.path().join(format!("f{i}")), "x").unwrap();
        }
        let error = staging.changes().unwrap_err();
        assert!(error.to_string().contains("too many entries"), "{error}");
    }
}
