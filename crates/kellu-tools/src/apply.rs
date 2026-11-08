//! Approving and applying a proposal. This is the only code in the workspace that writes to the
//! real project.
//!
//! * [`approve`] turns the user's decision into a frozen list inside the [`Grant`] (approved items
//!   only, each with the hash the user saw) and records it in the audit log.
//! * [`apply`] re-reads every file through a directory descriptor opened beneath the project root,
//!   refuses anything that changed since the user saw it, keeps a private backup of the originals,
//!   writes through a temporary file and an atomic rename, and undoes what it did if a later
//!   step fails. If anything fails, nothing stays applied.
//!
//! What it does not do: hold a lock against another process. A file edited between the final hash
//! check and the rename of that same file can still be replaced; the window is a few
//! microseconds and the backup holds the original. Extended attributes and ACLs of a replaced
//! file are not carried over. It refuses files owned by another user or with several hard links.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions, Permissions};
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use kellu_common::paths::ensure_private_dir;
use kellu_common::utc::UtcStamp;
use kellu_security::hash::sha256_hex;
use kellu_security::{AuditEvent, AuditLog, AuditRecord, Grant, Scope};
use rustix::fs::{AtFlags, Mode, OFlags, fsync, openat, renameat, statat, unlinkat};
use rustix::io::Errno;

use crate::proposal::{
    Item, MAX_CREATE_BYTES, MAX_FILE_BYTES, Operation, Proposal, TEMP_PREFIX, splice,
};
use crate::spec::ToolError;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// What the user chose for one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// All of it.
    Full,
    /// Only these hunks of a modify item (1-based, as shown).
    Hunks(BTreeSet<usize>),
}

/// The user's decision on a proposal. An item that is not listed is rejected; an empty decision
/// rejects everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Approval {
    /// Item id to choice.
    pub items: BTreeMap<String, Choice>,
    /// For each item that deletes a file: the path, typed by the user.
    pub typed: BTreeMap<String, String>,
}

/// The frozen result of [`approve`]. Only [`apply`] consumes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approved {
    proposal_id: String,
    items: Vec<ApprovedItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApprovedItem {
    index: usize,
    /// 0-based hunk indexes (empty for create and delete).
    hunks: Vec<usize>,
}

impl Approved {
    /// The proposal this approval is for.
    #[must_use]
    pub fn proposal_id(&self) -> &str {
        &self.proposal_id
    }

    /// How many items were approved.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether nothing was approved.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// A SHA-256 of the frozen list, for the audit log.
    #[must_use]
    pub fn digest(&self, proposal: &Proposal) -> String {
        let mut text = format!("{}\n", self.proposal_id);
        for approved in &self.items {
            if let Some(item) = proposal.items().get(approved.index) {
                text.push_str(&format!(
                    "{}|{}|{}|{:?}\n",
                    item.id(),
                    item.path().display(),
                    item.base_hash().unwrap_or("-"),
                    approved.hunks
                ));
            }
        }
        sha256_hex(text.as_bytes())
    }
}

/// One item that was applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// Path relative to the project root.
    pub path: PathBuf,
    /// What was done.
    pub operation: Operation,
    /// SHA-256 of the file now; `None` after a delete.
    pub new_hash: Option<String>,
}

/// What [`apply`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The items that were written, in order.
    pub applied: Vec<Applied>,
    /// Where the originals were kept (empty directory when nothing was replaced or deleted).
    pub backups: PathBuf,
    /// Whether every audit entry could be written. When `false` the changes are applied but the
    /// log is incomplete, and the caller must say so.
    pub audit_complete: bool,
}

/// What [`approve`] and [`apply`] need besides the proposal: the project, the audit log, where the
/// request came from and the current time.
#[derive(Debug, Clone, Copy)]
pub struct Session<'a> {
    /// The project.
    pub scope: &'a Scope,
    /// Where every transition is recorded.
    pub audit: &'a AuditLog,
    /// Where the request came from (`terminal`, `lsp`).
    pub origin: &'a str,
    /// The current monotonic time, checked against the grant.
    pub now: Instant,
}

impl Session<'_> {
    fn record(
        &self,
        proposal: &str,
        event: AuditEvent,
        item: &Item,
        validation: &str,
        hash: Option<String>,
    ) -> Result<(), ToolError> {
        self.audit
            .append(
                &AuditRecord {
                    proposal: proposal.to_owned(),
                    event,
                    path: item.path().display().to_string(),
                    permission: item.risk(),
                    origin: self.origin.to_owned(),
                    validation: validation.to_owned(),
                    hash,
                },
                UtcStamp::now(),
            )
            .map(|_| ())
            .map_err(|_| {
                ToolError::Failed("the audit log cannot be written; nothing was done".into())
            })
    }
}

/// Record the user's decision and freeze it into `grant`.
///
/// # Errors
/// The grant expired, the decision names an unknown item or hunk, a delete was not confirmed by
/// typing its path, a path is denied, or the audit log cannot be written. On any error the grant
/// is unchanged.
pub fn approve(
    proposal: &Proposal,
    approval: &Approval,
    session: &Session<'_>,
    grant: &mut Grant,
) -> Result<Approved, ToolError> {
    let (scope, now) = (session.scope, session.now);
    if grant.is_expired(now) {
        return Err(ToolError::Refused("the grant has expired".into()));
    }
    if proposal.root() != scope.root() {
        return Err(ToolError::Refused(
            "the proposal is for another project".into(),
        ));
    }
    let known: BTreeSet<&str> = proposal.items().iter().map(Item::id).collect();
    if approval.items.keys().any(|id| !known.contains(id.as_str()))
        || approval.typed.keys().any(|id| !known.contains(id.as_str()))
    {
        return Err(ToolError::Invalid(
            "the decision names an unknown item".into(),
        ));
    }
    let mut chosen = Vec::new();
    for (index, item) in proposal.items().iter().enumerate() {
        let Some(choice) = approval.items.get(item.id()) else {
            continue;
        };
        let hunks = match (item.operation(), choice) {
            (Operation::Modify, Choice::Full) => (0..item.hunks().len()).collect(),
            (Operation::Modify, Choice::Hunks(picked)) => {
                if picked.is_empty() || picked.iter().any(|n| *n == 0 || *n > item.hunks().len()) {
                    return Err(ToolError::Invalid(format!(
                        "item {}: hunks must be between 1 and {}",
                        item.id(),
                        item.hunks().len()
                    )));
                }
                picked.iter().map(|n| n - 1).collect()
            }
            (Operation::Create | Operation::Delete, Choice::Full) => Vec::new(),
            (Operation::Create | Operation::Delete, Choice::Hunks(_)) => {
                return Err(ToolError::Invalid(format!(
                    "item {}: only a modify item has hunks",
                    item.id()
                )));
            }
        };
        if item.operation() == Operation::Delete {
            let expected = item.path().display().to_string();
            if approval.typed.get(item.id()) != Some(&expected) {
                return Err(ToolError::Refused(format!(
                    "item {}: deleting needs the path typed exactly",
                    item.id()
                )));
            }
        }
        chosen.push(ApprovedItem { index, hunks });
    }
    // Dry run on a copy: a denied path or a bad hash must not leave a half-built grant.
    let mut next = grant.clone();
    for approved in &chosen {
        let item = &proposal.items()[approved.index];
        next.approve_write(
            scope,
            item.path().to_path_buf(),
            item.base_hash().map(str::to_owned),
        )?;
    }
    let approved_ids: BTreeSet<usize> = chosen.iter().map(|a| a.index).collect();
    for (index, item) in proposal.items().iter().enumerate() {
        let (event, validation) = if approved_ids.contains(&index) {
            (AuditEvent::Approved, "ok")
        } else {
            (AuditEvent::Rejected, "not selected")
        };
        session.record(
            proposal.id(),
            event,
            item,
            validation,
            item.base_hash().map(str::to_owned),
        )?;
    }
    *grant = next;
    Ok(Approved {
        proposal_id: proposal.id().to_owned(),
        items: chosen,
    })
}

fn errno_text(path: &Path, errno: Errno) -> ToolError {
    ToolError::Failed(format!(
        "{}: {}",
        path.display(),
        std::io::Error::from(errno).kind()
    ))
}

fn io_text(path: &Path, error: &std::io::Error) -> ToolError {
    ToolError::Failed(format!("{}: {}", path.display(), error.kind()))
}

/// Read the current regular file `name` of `dir`, refusing anything that is not a plain file of
/// this user with a single link.
fn read_current(dir: &OwnedFd, name: &OsStr, shown: &Path) -> Result<(Vec<u8>, u32), ToolError> {
    let fd = openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOCTTY,
        Mode::empty(),
    )
    .map_err(|e| errno_text(shown, e))?;
    let file = File::from(fd);
    let meta = file.metadata().map_err(|e| io_text(shown, &e))?;
    let refuse = |why: &str| ToolError::Refused(format!("{} {why}", shown.display()));
    if !meta.is_file() {
        return Err(refuse("is not a regular file"));
    }
    if meta.uid() != rustix::process::getuid().as_raw() {
        return Err(refuse("belongs to another user"));
    }
    if meta.nlink() != 1 {
        return Err(refuse("has more than one hard link"));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(refuse("is too large"));
    }
    let mut data = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| io_text(shown, &e))?;
    Ok((data, meta.mode() & 0o7777))
}

fn temp_name() -> OsString {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    OsString::from(format!(
        "{TEMP_PREFIX}{}-{nanos}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Replace (or create) `name` with `data` through a temporary file in the same directory and an
/// atomic rename.
fn write_replacing(
    dir: &OwnedFd,
    name: &OsStr,
    data: &[u8],
    mode: u32,
    shown: &Path,
) -> Result<(), ToolError> {
    let tmp = temp_name();
    let fd = openat(
        dir,
        &tmp,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|e| errno_text(shown, e))?;
    let mut file = File::from(fd);
    let written = file
        .write_all(data)
        .and_then(|()| file.set_permissions(Permissions::from_mode(mode)))
        .and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = written {
        let _ = unlinkat(dir, &tmp, AtFlags::empty());
        return Err(io_text(shown, &error));
    }
    if let Err(errno) = renameat(dir, &tmp, dir, name) {
        let _ = unlinkat(dir, &tmp, AtFlags::empty());
        return Err(errno_text(shown, errno));
    }
    let _ = fsync(dir);
    Ok(())
}

/// Create `name`, which must not exist.
fn create_new(
    dir: &OwnedFd,
    name: &OsStr,
    data: &[u8],
    mode: u32,
    shown: &Path,
) -> Result<(), ToolError> {
    let fd = openat(
        dir,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|e| errno_text(shown, e))?;
    let mut file = File::from(fd);
    let written = file
        .write_all(data)
        .and_then(|()| file.set_permissions(Permissions::from_mode(mode)))
        .and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = written {
        let _ = unlinkat(dir, name, AtFlags::empty());
        return Err(io_text(shown, &error));
    }
    let _ = fsync(dir);
    Ok(())
}

enum Step {
    Modify {
        original: Vec<u8>,
        new: Vec<u8>,
        mode: u32,
    },
    Create {
        content: Vec<u8>,
    },
    Delete {
        original: Vec<u8>,
        mode: u32,
    },
}

struct Prepared<'a> {
    item: &'a Item,
    dir: OwnedFd,
    name: OsString,
    base_hash: Option<String>,
    step: Step,
}

fn split(path: &Path) -> (PathBuf, OsString) {
    (
        path.parent().map(Path::to_path_buf).unwrap_or_default(),
        path.file_name()
            .map(OsStr::to_os_string)
            .unwrap_or_default(),
    )
}

fn backup_name(index: usize, name: &OsStr) -> String {
    let safe: String = name
        .to_string_lossy()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    format!("{index:03}-{safe}")
}

fn save_backup(dir: &Path, file: &str, data: &[u8], mode: u32) -> Result<(), ToolError> {
    let path = dir.join(file);
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(i32::try_from(OFlags::NOFOLLOW.bits()).unwrap_or(0))
        .open(&path)
        .map_err(|e| io_text(&path, &e))?;
    out.write_all(data)
        .and_then(|()| out.set_permissions(Permissions::from_mode(0o600)))
        .and_then(|()| out.sync_all())
        .map_err(|e| io_text(&path, &e))?;
    // The original mode is recorded in the name of the sidecar so a restore can reproduce it.
    let sidecar = dir.join(format!("{file}.mode"));
    std::fs::write(&sidecar, format!("{mode:o}\n")).map_err(|e| io_text(&sidecar, &e))?;
    Ok(())
}

/// Apply what [`approve`] froze.
///
/// # Errors
/// The grant expired or does not match the approval, a file changed since the user saw it, a path
/// is not safe, a backup cannot be kept, or a write fails. After an error nothing stays applied
/// (a failed undo is said so in the message).
pub fn apply(
    proposal: &Proposal,
    approved: &Approved,
    session: &Session<'_>,
    grant: &Grant,
    backups: &Path,
) -> Result<Report, ToolError> {
    let (scope, now) = (session.scope, session.now);
    if grant.is_expired(now) {
        return Err(ToolError::Refused("the grant has expired".into()));
    }
    if approved.proposal_id != proposal.id() || proposal.root() != scope.root() {
        return Err(ToolError::Refused(
            "the approval is not for this proposal".into(),
        ));
    }
    if approved.items.is_empty() {
        return Err(ToolError::Invalid("nothing was approved".into()));
    }
    // 1. Prepare everything in memory. Nothing is written yet.
    let mut prepared = Vec::with_capacity(approved.items.len());
    for chosen in &approved.items {
        let item = proposal
            .items()
            .get(chosen.index)
            .ok_or_else(|| ToolError::Refused("the approval does not match the proposal".into()))?;
        let approval = grant.write_approval(item.path(), now).ok_or_else(|| {
            ToolError::Refused(format!("{} is not approved", item.path().display()))
        })?;
        if approval.base_hash.as_deref() != item.base_hash() {
            return Err(ToolError::Refused(format!(
                "{}: the approved hash is not the one the user saw",
                item.path().display()
            )));
        }
        if scope.is_denied(item.path()) {
            return Err(ToolError::Refused(format!(
                "{} is denied by policy",
                item.path().display()
            )));
        }
        let (parent, name) = split(item.path());
        let dir = scope.open_dir(&parent)?;
        let step = match item.operation() {
            Operation::Modify => {
                let (original, mode) = read_current(&dir, &name, item.path())?;
                if Some(sha256_hex(&original).as_str()) != item.base_hash() {
                    return Err(stale(item));
                }
                let text = String::from_utf8(original.clone())
                    .map_err(|_| ToolError::Refused("the file is no longer UTF-8 text".into()))?;
                let picked: Vec<_> = chosen.hunks.iter().map(|i| &item.hunks()[*i]).collect();
                let new = splice(&text, &picked)?;
                if new.len() > MAX_CREATE_BYTES {
                    return Err(ToolError::Refused("the result would be too large".into()));
                }
                Step::Modify {
                    original,
                    new: new.into_bytes(),
                    mode,
                }
            }
            Operation::Create => {
                match statat(&dir, &name, AtFlags::SYMLINK_NOFOLLOW) {
                    Err(Errno::NOENT) => {}
                    Ok(_) => {
                        return Err(ToolError::Refused(format!(
                            "{} now exists; it was free when proposed",
                            item.path().display()
                        )));
                    }
                    Err(errno) => return Err(errno_text(item.path(), errno)),
                }
                Step::Create {
                    content: item.content().unwrap_or_default().as_bytes().to_vec(),
                }
            }
            Operation::Delete => {
                let (original, mode) = read_current(&dir, &name, item.path())?;
                if Some(sha256_hex(&original).as_str()) != item.base_hash() {
                    return Err(stale(item));
                }
                Step::Delete { original, mode }
            }
        };
        prepared.push(Prepared {
            item,
            dir,
            name,
            base_hash: item.base_hash().map(str::to_owned),
            step,
        });
    }
    // 2. Keep the originals, privately, before touching anything.
    ensure_private_dir(backups).map_err(ToolError::from)?;
    let backup_dir = backups.join(proposal.id());
    ensure_private_dir(&backup_dir).map_err(ToolError::from)?;
    for (index, p) in prepared.iter().enumerate() {
        match &p.step {
            Step::Modify { original, mode, .. } | Step::Delete { original, mode } => {
                save_backup(
                    &backup_dir,
                    &backup_name(index + 1, &p.name),
                    original,
                    *mode,
                )?;
            }
            Step::Create { .. } => {}
        }
    }
    // 3. Write. A failure undoes everything done so far.
    let mut done: Vec<&Prepared<'_>> = Vec::new();
    for p in &prepared {
        match commit(p) {
            Ok(()) => done.push(p),
            Err(error) => {
                let undone = rollback(&done);
                let _ = session.record(
                    proposal.id(),
                    AuditEvent::Failed,
                    p.item,
                    if undone {
                        "write failed, rolled back"
                    } else {
                        "write failed, rollback incomplete"
                    },
                    p.base_hash.clone(),
                );
                let tail = if undone {
                    "nothing was applied"
                } else {
                    "undoing the earlier items also failed; the originals are in the backup folder"
                };
                return Err(ToolError::Failed(format!("{error}; {tail}")));
            }
        }
    }
    // 4. Say what happened.
    let mut audit_complete = true;
    let mut applied = Vec::new();
    for p in &prepared {
        let new_hash = match &p.step {
            Step::Modify { new, .. } => Some(sha256_hex(new)),
            Step::Create { content } => Some(sha256_hex(content)),
            Step::Delete { .. } => None,
        };
        audit_complete &= session
            .record(
                proposal.id(),
                AuditEvent::Applied,
                p.item,
                "ok",
                new_hash.clone().or_else(|| p.base_hash.clone()),
            )
            .is_ok();
        applied.push(Applied {
            path: p.item.path().to_path_buf(),
            operation: p.item.operation(),
            new_hash,
        });
    }
    Ok(Report {
        applied,
        backups: backup_dir,
        audit_complete,
    })
}

fn stale(item: &Item) -> ToolError {
    ToolError::Refused(format!(
        "{} changed after the user saw it; nothing was applied",
        item.path().display()
    ))
}

fn commit(p: &Prepared<'_>) -> Result<(), ToolError> {
    let shown = p.item.path();
    match &p.step {
        Step::Modify { new, mode, .. } => {
            recheck(p)?;
            write_replacing(&p.dir, &p.name, new, *mode, shown)
        }
        Step::Create { content } => create_new(&p.dir, &p.name, content, 0o644, shown),
        Step::Delete { .. } => {
            recheck(p)?;
            unlinkat(&p.dir, &p.name, AtFlags::empty()).map_err(|e| errno_text(shown, e))?;
            let _ = fsync(&p.dir);
            Ok(())
        }
    }
}

/// Last look at the file right before it is replaced or removed.
fn recheck(p: &Prepared<'_>) -> Result<(), ToolError> {
    let (now, _) = read_current(&p.dir, &p.name, p.item.path())?;
    if Some(sha256_hex(&now).as_str()) == p.base_hash.as_deref() {
        Ok(())
    } else {
        Err(stale(p.item))
    }
}

/// Undo `done` in reverse. Returns whether every undo worked.
fn rollback(done: &[&Prepared<'_>]) -> bool {
    let mut all = true;
    for p in done.iter().rev() {
        let shown = p.item.path();
        let result = match &p.step {
            Step::Modify { original, mode, .. } => {
                write_replacing(&p.dir, &p.name, original, *mode, shown)
            }
            Step::Create { .. } => {
                unlinkat(&p.dir, &p.name, AtFlags::empty()).map_err(|e| errno_text(shown, e))
            }
            Step::Delete { original, mode } => create_new(&p.dir, &p.name, original, *mode, shown),
        };
        all &= result.is_ok();
    }
    all
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use kellu_security::DenyGlobs;

    use super::*;

    /// The second item cannot be written (its directory is gone), so the first, already written,
    /// must be put back. This runs as root too, unlike a permission-based failure.
    #[test]
    fn a_failed_second_write_restores_the_first() {
        let tmp = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(tmp.path()).unwrap();
        let (home, root) = (base.join("home"), base.join("home/project"));
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("a.txt"), "original a\n").unwrap();
        fs::write(root.join("sub/b.txt"), "original b\n").unwrap();
        let scope = Scope::open(&root, &home, DenyGlobs::new(), false).unwrap();
        let now = Instant::now();
        let grant = Grant::read_only(
            &scope,
            ["a.txt", "sub/b.txt"].map(PathBuf::from),
            now,
            Duration::from_secs(60),
        );
        let a = Item::modify(
            &scope,
            &grant,
            now,
            Path::new("a.txt"),
            &[("original a".into(), "changed a".into())],
            "t",
        )
        .unwrap();
        let b = Item::modify(
            &scope,
            &grant,
            now,
            Path::new("sub/b.txt"),
            &[("original b".into(), "changed b".into())],
            "t",
        )
        .unwrap();
        let first = Prepared {
            item: &a,
            dir: scope.open_dir(Path::new("")).unwrap(),
            name: "a.txt".into(),
            base_hash: a.base_hash().map(str::to_owned),
            step: Step::Modify {
                original: b"original a\n".to_vec(),
                new: b"changed a\n".to_vec(),
                mode: 0o644,
            },
        };
        let second = Prepared {
            item: &b,
            dir: scope.open_dir(Path::new("sub")).unwrap(),
            name: "b.txt".into(),
            base_hash: b.base_hash().map(str::to_owned),
            step: Step::Modify {
                original: b"original b\n".to_vec(),
                new: b"changed b\n".to_vec(),
                mode: 0o644,
            },
        };
        // The descriptor stays valid but the directory no longer exists.
        fs::remove_dir_all(root.join("sub")).unwrap();

        commit(&first).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("a.txt")).unwrap(),
            "changed a\n"
        );
        assert!(
            commit(&second).is_err(),
            "writing into a removed directory must fail"
        );
        assert!(rollback(&[&first]));
        assert_eq!(
            fs::read_to_string(root.join("a.txt")).unwrap(),
            "original a\n"
        );
        let leftovers = fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(TEMP_PREFIX))
            .count();
        assert_eq!(leftovers, 0, "no temporary file may stay behind");
    }

    #[test]
    fn rolling_back_a_create_removes_it_and_rolling_back_a_delete_restores_it() {
        let tmp = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(tmp.path()).unwrap();
        let (home, root) = (base.join("home"), base.join("home/project"));
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("gone.txt"), "keep me\n").unwrap();
        let scope = Scope::open(&root, &home, DenyGlobs::new(), false).unwrap();
        let now = Instant::now();
        let grant = Grant::read_only(
            &scope,
            [PathBuf::from("gone.txt")],
            now,
            Duration::from_secs(60),
        );
        let created = Item::create(&scope, Path::new("new.txt"), "new\n", "t").unwrap();
        let deleted = Item::delete(&scope, &grant, now, Path::new("gone.txt"), "t").unwrap();
        let make = Prepared {
            item: &created,
            dir: scope.open_dir(Path::new("")).unwrap(),
            name: "new.txt".into(),
            base_hash: None,
            step: Step::Create {
                content: b"new\n".to_vec(),
            },
        };
        let remove = Prepared {
            item: &deleted,
            dir: scope.open_dir(Path::new("")).unwrap(),
            name: "gone.txt".into(),
            base_hash: deleted.base_hash().map(str::to_owned),
            step: Step::Delete {
                original: b"keep me\n".to_vec(),
                mode: 0o640,
            },
        };
        commit(&make).unwrap();
        commit(&remove).unwrap();
        assert!(root.join("new.txt").exists() && !root.join("gone.txt").exists());
        assert!(rollback(&[&make, &remove]));
        assert!(!root.join("new.txt").exists());
        assert_eq!(
            fs::read_to_string(root.join("gone.txt")).unwrap(),
            "keep me\n"
        );
        assert_eq!(
            fs::metadata(root.join("gone.txt")).unwrap().mode() & 0o777,
            0o640
        );
    }
}
