//! Append-only audit log with a hash chain.
//!
//! One JSON object per line in a private file (mode 0600). Every line carries the hash of the
//! previous one, so removing, reordering or editing a line is detectable by
//! [`verify`]. The chain is *tamper evidence*, not tamper proofing: someone who can rewrite the
//! whole file can rebuild it, which is why the file is private and why the design keeps the head
//! hash in memory for the session (see `docs/design/permissions-and-sandbox.md`).
//!
//! Lines are written by hand in a fixed field order, so the bytes that are hashed do not depend
//! on the JSON library's map ordering. Appends take an exclusive lock, because several `kellu
//! lsp` processes may write to the same file; a log that fails to parse is never appended to.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use kellu_common::paths::{open_private_append, read_trusted_file};
use kellu_common::text::{strip_control_chars, truncate};
use kellu_common::utc::UtcStamp;
use kellu_common::{KelluError, Result};
use rustix::fs::{FlockOperation, OFlags, flock};

use crate::hash::{SHA256_HEX_LEN, is_sha256_hex, sha256_hex};
use crate::permission::Risk;

/// The `prev` of the first line.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const MAX_LOG_BYTES: u64 = 64 * 1024 * 1024;
const TAIL_BYTES: u64 = 16 * 1024;
const LOCK_WAIT: Duration = Duration::from_secs(5);
const CHAIN_FIELD: &str = ",\"chain\":\"";

/// A transition in the life of a proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditEvent {
    /// The model proposed a change.
    Proposed,
    /// The user approved it (fully or in part).
    Approved,
    /// The user denied it.
    Rejected,
    /// The core applied it.
    Applied,
    /// Applying it failed; nothing was applied.
    Failed,
}

impl AuditEvent {
    /// Lowercase name, as written in the log.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Applied => "applied",
            Self::Failed => "failed",
        }
    }
}

/// What to record for one transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    /// Proposal id.
    pub proposal: String,
    /// The transition.
    pub event: AuditEvent,
    /// Resolved path inside the project.
    pub path: String,
    /// The permission involved.
    pub permission: Risk,
    /// Where the request came from (for example `terminal`, `lsp`).
    pub origin: String,
    /// Outcome of the validation (for example `ok` or the reason it failed).
    pub validation: String,
    /// SHA-256 of the content involved, when there is one.
    pub hash: Option<String>,
}

/// The audit log at one path. Opening nothing until a method is called.
#[derive(Debug, Clone)]
pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    /// The log stored at `path`.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Where the log lives.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one record stamped `now`; returns its sequence number.
    ///
    /// # Errors
    /// The file is unsafe (symlink, wrong owner), busy for too long, or already corrupt.
    pub fn append(&self, record: &AuditRecord, now: UtcStamp) -> Result<u64> {
        let file = open_private_append(&self.path, false)?;
        lock(&file, &self.path)?;
        let (last_seq, prev) = self.head(&file)?;
        let seq = last_seq + 1;
        let line = format_line(seq, &prev, record, now);
        // One write on an O_APPEND descriptor: concurrent writers cannot interleave inside it.
        let mut writer = &file;
        writer
            .write_all(line.as_bytes())
            .map_err(|e| KelluError::io(&self.path, e))?;
        writer.flush().map_err(|e| KelluError::io(&self.path, e))?;
        Ok(seq)
    }

    /// `(sequence, chain hash)` of the last line, or `(0, GENESIS)` for an empty log.
    fn head(&self, locked: &std::fs::File) -> Result<(u64, String)> {
        let flags = i32::try_from(OFlags::NOFOLLOW.bits()).unwrap_or(0);
        let reader = OpenOptions::new()
            .read(true)
            .custom_flags(flags)
            .open(&self.path)
            .map_err(|e| KelluError::io(&self.path, e))?;
        let (meta_w, meta_r) = (
            locked
                .metadata()
                .map_err(|e| KelluError::io(&self.path, e))?,
            reader
                .metadata()
                .map_err(|e| KelluError::io(&self.path, e))?,
        );
        if (meta_w.dev(), meta_w.ino()) != (meta_r.dev(), meta_r.ino()) {
            return Err(KelluError::security(
                "the audit log changed while it was being opened",
            ));
        }
        let len = meta_r.len();
        if len == 0 {
            return Ok((0, GENESIS.to_owned()));
        }
        let start = len.saturating_sub(TAIL_BYTES);
        let size = usize::try_from(len - start).map_err(|_| corrupt())?;
        let mut tail = vec![0_u8; size];
        reader
            .read_exact_at(&mut tail, start)
            .map_err(|e| KelluError::io(&self.path, e))?;
        let text = std::str::from_utf8(&tail).map_err(|_| corrupt())?;
        let text = text.strip_suffix('\n').ok_or_else(corrupt)?;
        let line = match text.rfind('\n') {
            Some(i) => &text[i + 1..],
            None if start == 0 => text,
            None => return Err(corrupt()),
        };
        let parsed = parse_line(line).ok_or_else(corrupt)?;
        if sha256_hex(format!("{}\n{}", parsed.prev, parsed.body).as_bytes()) != parsed.chain {
            return Err(corrupt());
        }
        Ok((parsed.seq, parsed.chain.to_owned()))
    }
}

fn corrupt() -> KelluError {
    KelluError::security(
        "the audit log is corrupt; refusing to append (move it aside to start a new one)",
    )
}

fn lock(file: &std::fs::File, path: &Path) -> Result<()> {
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        match flock(file, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => return Ok(()),
            Err(rustix::io::Errno::WOULDBLOCK) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(rustix::io::Errno::WOULDBLOCK) => {
                return Err(KelluError::security("the audit log is busy"));
            }
            Err(errno) => return Err(KelluError::io(path, errno.into())),
        }
    }
}

fn json_string(text: &str, max_chars: usize) -> String {
    let limit = std::num::NonZeroUsize::new(max_chars).unwrap_or(std::num::NonZeroUsize::MIN);
    serde_json::Value::String(truncate(&strip_control_chars(text), limit)).to_string()
}

fn format_line(seq: u64, prev: &str, record: &AuditRecord, now: UtcStamp) -> String {
    let hash = record
        .hash
        .as_deref()
        .filter(|h| is_sha256_hex(h))
        .map_or_else(|| "null".to_owned(), |h| format!("\"{h}\""));
    let body = format!(
        "{{\"seq\":{seq},\"ts\":\"{}T{}Z\",\"proposal\":{},\"event\":\"{}\",\"path\":{},\"permission\":\"{}\",\"origin\":{},\"validation\":{},\"hash\":{hash},\"prev\":\"{prev}\"",
        now.date(),
        now.time(),
        json_string(&record.proposal, 128),
        record.event.as_str(),
        json_string(&record.path, 1024),
        record.permission.as_str(),
        json_string(&record.origin, 128),
        json_string(&record.validation, 256),
    );
    let chain = sha256_hex(format!("{prev}\n{body}").as_bytes());
    format!("{body}{CHAIN_FIELD}{chain}\"}}\n")
}

struct Parsed<'a> {
    seq: u64,
    prev: &'a str,
    chain: &'a str,
    body: &'a str,
}

fn parse_line(line: &str) -> Option<Parsed<'_>> {
    let tail_len = CHAIN_FIELD.len() + SHA256_HEX_LEN + 2;
    let split = line.len().checked_sub(tail_len)?;
    let tail = line.get(split..)?;
    let chain = tail.strip_prefix(CHAIN_FIELD)?.strip_suffix("\"}")?;
    let body = &line[..split];
    let digits = body.strip_prefix("{\"seq\":")?;
    let seq: u64 = digits[..digits.find(',')?].parse().ok()?;
    let marker = ",\"prev\":\"";
    let at = body.rfind(marker)?;
    let prev = body[at + marker.len()..].strip_suffix('"')?;
    (is_sha256_hex(chain) && is_sha256_hex(prev)).then_some(Parsed {
        seq,
        prev,
        chain,
        body,
    })
}

/// Check the whole log at `path`: every line parses, links to the previous one, and its hash
/// matches. Returns the number of lines (0 for a missing or empty log).
///
/// # Errors
/// The file is unsafe or too large, or the chain is broken (the message names the line).
pub fn verify(path: &Path) -> Result<u64> {
    let Some(bytes) = read_trusted_file(path, MAX_LOG_BYTES)? else {
        return Ok(0);
    };
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| KelluError::security("the audit log is not UTF-8"))?;
    if text.is_empty() {
        return Ok(0);
    }
    let text = text
        .strip_suffix('\n')
        .ok_or_else(|| KelluError::security("the audit log ends in a partial line"))?;
    let (mut expected_prev, mut expected_seq) = (GENESIS.to_owned(), 1_u64);
    for (number, line) in text.split('\n').enumerate() {
        let broken =
            |why: &str| KelluError::security(format!("audit log line {}: {why}", number + 1));
        let parsed = parse_line(line).ok_or_else(|| broken("malformed"))?;
        if parsed.prev != expected_prev {
            return Err(broken(
                "does not follow the previous line (a line was removed or reordered)",
            ));
        }
        if parsed.seq != expected_seq {
            return Err(broken("unexpected sequence number"));
        }
        if sha256_hex(format!("{}\n{}", parsed.prev, parsed.body).as_bytes()) != parsed.chain {
            return Err(broken("hash mismatch (the line was edited)"));
        }
        expected_prev = parsed.chain.to_owned();
        expected_seq += 1;
    }
    Ok(expected_seq - 1)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::sync::Arc;

    use super::*;

    fn record(proposal: &str, event: AuditEvent) -> AuditRecord {
        AuditRecord {
            proposal: proposal.to_owned(),
            event,
            path: "src/main.rs".to_owned(),
            permission: Risk::Write,
            origin: "terminal".to_owned(),
            validation: "ok".to_owned(),
            hash: Some(sha256_hex(b"content")),
        }
    }

    fn log() -> (tempfile::TempDir, AuditLog) {
        let tmp = tempfile::tempdir().unwrap();
        let log = AuditLog::new(tmp.path().join("audit.jsonl"));
        (tmp, log)
    }

    fn at(secs: i64) -> UtcStamp {
        UtcStamp::from_unix(secs)
    }

    #[test]
    fn records_are_chained_and_verify() {
        let (_tmp, log) = log();
        assert_eq!(
            log.append(&record("p1", AuditEvent::Proposed), at(1_793_000_000))
                .unwrap(),
            1
        );
        assert_eq!(
            log.append(&record("p1", AuditEvent::Approved), at(1_793_000_010))
                .unwrap(),
            2
        );
        assert_eq!(
            log.append(&record("p1", AuditEvent::Applied), at(1_793_000_020))
                .unwrap(),
            3
        );
        assert_eq!(verify(log.path()).unwrap(), 3);
    }

    #[test]
    fn the_file_is_private_and_each_line_is_json() {
        let (_tmp, log) = log();
        log.append(&record("p1", AuditEvent::Proposed), at(0))
            .unwrap();
        let mode = fs::metadata(log.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let text = fs::read_to_string(log.path()).unwrap();
        for line in text.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["event"], "proposed");
            assert_eq!(value["permission"], "write");
            assert_eq!(value["ts"], "1970-01-01T00:00:00Z");
            assert_eq!(value["prev"], GENESIS);
        }
    }

    #[test]
    fn editing_a_line_is_detected() {
        let (_tmp, log) = log();
        log.append(&record("p1", AuditEvent::Proposed), at(1))
            .unwrap();
        log.append(&record("p1", AuditEvent::Rejected), at(2))
            .unwrap();
        let text = fs::read_to_string(log.path())
            .unwrap()
            .replace("rejected", "approved");
        fs::write(log.path(), text).unwrap();
        let error = verify(log.path()).unwrap_err().to_string();
        assert!(
            error.contains("line 2") && error.contains("edited"),
            "{error}"
        );
    }

    #[test]
    fn removing_or_reordering_lines_is_detected() {
        let (_tmp, log) = log();
        for (i, event) in [
            AuditEvent::Proposed,
            AuditEvent::Approved,
            AuditEvent::Applied,
        ]
        .into_iter()
        .enumerate()
        {
            log.append(&record("p1", event), at(i as i64)).unwrap();
        }
        let lines: Vec<String> = fs::read_to_string(log.path())
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        fs::write(log.path(), format!("{}\n{}\n", lines[0], lines[2])).unwrap();
        assert!(
            verify(log.path())
                .unwrap_err()
                .to_string()
                .contains("line 2")
        );
        fs::write(
            log.path(),
            format!("{}\n{}\n{}\n", lines[1], lines[0], lines[2]),
        )
        .unwrap();
        assert!(verify(log.path()).is_err());
        fs::write(log.path(), format!("{}\n{}\n", lines[1], lines[2])).unwrap();
        assert!(verify(log.path()).is_err(), "dropping the first line");
    }

    #[test]
    fn a_truncated_tail_is_detected_and_never_appended_to() {
        let (_tmp, log) = log();
        log.append(&record("p1", AuditEvent::Proposed), at(1))
            .unwrap();
        let text = fs::read_to_string(log.path()).unwrap();
        fs::write(log.path(), &text[..text.len() - 5]).unwrap();
        assert!(verify(log.path()).is_err());
        assert!(
            log.append(&record("p1", AuditEvent::Applied), at(2))
                .is_err()
        );
        fs::write(log.path(), "garbage\n").unwrap();
        assert!(
            log.append(&record("p1", AuditEvent::Applied), at(2))
                .is_err()
        );
    }

    #[test]
    fn hostile_field_values_cannot_forge_structure() {
        let (_tmp, log) = log();
        let mut hostile = record("p\"1\n{\"seq\":99}", AuditEvent::Proposed);
        hostile.path = "a\u{1b}[31m\"b\\c\n\r".to_owned();
        hostile.origin = "\",\"chain\":\"".to_owned();
        hostile.validation = "x".repeat(10_000);
        hostile.hash = Some("not a hash".to_owned());
        log.append(&hostile, at(1)).unwrap();
        log.append(&record("p2", AuditEvent::Applied), at(2))
            .unwrap();
        let text = fs::read_to_string(log.path()).unwrap();
        assert_eq!(text.lines().count(), 2, "one line per record");
        assert!(!text.contains('\u{1b}'));
        let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(first["hash"], serde_json::Value::Null);
        assert_eq!(verify(log.path()).unwrap(), 2);
    }

    #[test]
    fn a_symlinked_log_is_refused() {
        let (tmp, log) = log();
        let target = tmp.path().join("elsewhere");
        fs::write(&target, "").unwrap();
        symlink(&target, log.path()).unwrap();
        assert!(
            log.append(&record("p1", AuditEvent::Proposed), at(1))
                .is_err()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "");
    }

    #[test]
    fn concurrent_writers_keep_one_unbroken_chain() {
        let (_tmp, log) = log();
        let log = Arc::new(log);
        let handles: Vec<_> = (0..8)
            .map(|worker| {
                let log = Arc::clone(&log);
                thread::spawn(move || {
                    for i in 0..10 {
                        log.append(
                            &record(&format!("w{worker}-{i}"), AuditEvent::Proposed),
                            at(i),
                        )
                        .unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(verify(log.path()).unwrap(), 80);
    }

    #[test]
    fn a_missing_or_empty_log_verifies_as_empty() {
        let (_tmp, log) = log();
        assert_eq!(verify(log.path()).unwrap(), 0);
        fs::write(log.path(), "").unwrap();
        fs::set_permissions(log.path(), fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(verify(log.path()).unwrap(), 0);
    }
}
