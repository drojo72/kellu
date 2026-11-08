//! Proposed changes.
//!
//! A [`Proposal`] is built from items that were validated against the project *when they were
//! made* and are immutable afterwards. A modify item is a list of [`Hunk`]s already resolved to a
//! byte offset in the exact file the user will be shown (identified by its SHA-256), so approving
//! a subset of hunks is a plain selection and nothing is searched for again at apply time.
//!
//! Nothing in this module writes anything.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use kellu_common::text::strip_control_chars;
use kellu_security::hash::sha256_hex;
use kellu_security::{Grant, Risk, Scope};
use rustix::fs::{AtFlags, statat};
use rustix::io::Errno;

use crate::spec::ToolError;

/// The largest existing file a proposal may modify or delete.
pub const MAX_FILE_BYTES: u64 = 1 << 20;
/// The largest file a proposal may create.
pub const MAX_CREATE_BYTES: usize = 1 << 20;
/// Most items in one proposal.
pub const MAX_ITEMS: usize = 50;
/// Most hunks in one item.
pub const MAX_HUNKS: usize = 100;
/// Longest rationale kept.
const MAX_RATIONALE_CHARS: usize = 500;
/// Prefix of the temporary files the core creates while applying; the model may not use it.
pub(crate) const TEMP_PREFIX: &str = ".kellu-tmp-";

/// What an item does to its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Replace parts of an existing text file.
    Modify,
    /// Create a file that does not exist.
    Create,
    /// Delete an existing file.
    Delete,
}

impl Operation {
    /// Lowercase name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Modify => "modify",
            Self::Create => "create",
            Self::Delete => "delete",
        }
    }

    /// The risk of applying it: deleting is destructive, the rest writes.
    #[must_use]
    pub fn risk(self) -> Risk {
        match self {
            Self::Delete => Risk::Destructive,
            Self::Modify | Self::Create => Risk::Write,
        }
    }
}

/// One replacement inside a file, at a fixed position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    offset: usize,
    line: usize,
    old: String,
    new: String,
}

impl Hunk {
    /// Byte offset of `old` in the base file.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// 1-based line where the hunk starts.
    #[must_use]
    pub fn line(&self) -> usize {
        self.line
    }

    /// The text that is replaced.
    #[must_use]
    pub fn old(&self) -> &str {
        &self.old
    }

    /// The replacement.
    #[must_use]
    pub fn new_text(&self) -> &str {
        &self.new
    }
}

/// One file the proposal would change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    id: String,
    path: PathBuf,
    operation: Operation,
    base_hash: Option<String>,
    hunks: Vec<Hunk>,
    content: Option<String>,
    rationale: String,
}

/// A path made only of normal components, written canonically.
pub(crate) fn plain_relative(path: &Path) -> bool {
    path.components().next().is_some()
        && path.components().all(|c| matches!(c, Component::Normal(_)))
        && path.components().collect::<PathBuf>().as_os_str() == path.as_os_str()
}

fn check_path(scope: &Scope, path: &Path) -> Result<(), ToolError> {
    if !plain_relative(path) {
        return Err(ToolError::Refused(
            "the path must be relative, inside the project, without `..` or `.`".into(),
        ));
    }
    if scope.is_denied(path) {
        return Err(ToolError::Refused(format!(
            "{} is denied by policy",
            path.display()
        )));
    }
    let name = path.file_name().unwrap_or_default();
    if name.to_string_lossy().starts_with(TEMP_PREFIX) {
        return Err(ToolError::Refused(format!(
            "names starting with {TEMP_PREFIX} are reserved"
        )));
    }
    Ok(())
}

fn clean_rationale(text: &str) -> String {
    strip_control_chars(text)
        .chars()
        .take(MAX_RATIONALE_CHARS)
        .collect()
}

/// The existing file as the user would see it: readable by the grant, valid UTF-8, small.
fn read_base(
    scope: &Scope,
    grant: &Grant,
    now: Instant,
    path: &Path,
) -> Result<(String, String), ToolError> {
    if !grant.can_read(path, now) {
        return Err(ToolError::Refused(format!(
            "{} is not a file this session may read, so it cannot be changed",
            path.display()
        )));
    }
    let bytes = scope.read_file(path, MAX_FILE_BYTES)?;
    let hash = sha256_hex(&bytes);
    let text = String::from_utf8(bytes)
        .map_err(|_| ToolError::Invalid(format!("{} is not UTF-8 text", path.display())))?;
    Ok((text, hash))
}

fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}

/// Where `needle` occurs in `haystack`, counting overlapping matches.
fn occurrences(haystack: &str, needle: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(i) = haystack[from..].find(needle) {
        let at = from + i;
        found.push(at);
        from = at + haystack[at..].chars().next().map_or(1, char::len_utf8);
        if found.len() > 1 {
            break;
        }
    }
    found
}

impl Item {
    /// Replace exact text in an existing file. Each `old` must occur exactly once; ambiguity is
    /// refused rather than guessed.
    ///
    /// # Errors
    /// The path is not allowed, the file cannot be read, an edit does not match exactly once,
    /// edits overlap, or something is out of bounds.
    pub fn modify(
        scope: &Scope,
        grant: &Grant,
        now: Instant,
        path: &Path,
        edits: &[(String, String)],
        rationale: &str,
    ) -> Result<Self, ToolError> {
        check_path(scope, path)?;
        if edits.is_empty() || edits.len() > MAX_HUNKS {
            return Err(ToolError::Invalid(format!(
                "edits: between 1 and {MAX_HUNKS} are required"
            )));
        }
        let (base, hash) = read_base(scope, grant, now, path)?;
        let mut hunks = Vec::with_capacity(edits.len());
        for (index, (old, new)) in edits.iter().enumerate() {
            let number = index + 1;
            if old.is_empty() {
                return Err(ToolError::Invalid(format!("edit {number}: `old` is empty")));
            }
            if old == new {
                return Err(ToolError::Invalid(format!(
                    "edit {number}: `old` and `new` are the same"
                )));
            }
            match occurrences(&base, old).as_slice() {
                [] => {
                    return Err(ToolError::Invalid(format!(
                        "edit {number}: `old` does not occur in the file"
                    )));
                }
                [offset] => hunks.push(Hunk {
                    offset: *offset,
                    line: line_of(&base, *offset),
                    old: old.clone(),
                    new: new.clone(),
                }),
                _ => {
                    return Err(ToolError::Invalid(format!(
                        "edit {number}: `old` occurs more than once; include more context"
                    )));
                }
            }
        }
        hunks.sort_by_key(|h| h.offset);
        if hunks
            .windows(2)
            .any(|pair| pair[0].offset + pair[0].old.len() > pair[1].offset)
        {
            return Err(ToolError::Invalid("edits overlap".into()));
        }
        Ok(Self {
            id: String::new(),
            path: path.to_path_buf(),
            operation: Operation::Modify,
            base_hash: Some(hash),
            hunks,
            content: None,
            rationale: clean_rationale(rationale),
        })
    }

    /// Change an existing file into `new_content` as one hunk (the common prefix and suffix stay
    /// outside it). Used for the result of a command run on a staging copy.
    ///
    /// # Errors
    /// As [`Item::modify`], or the content is unchanged or too large.
    pub fn modify_to(
        scope: &Scope,
        grant: &Grant,
        now: Instant,
        path: &Path,
        new_content: &str,
        rationale: &str,
    ) -> Result<Self, ToolError> {
        check_path(scope, path)?;
        if new_content.len() > MAX_CREATE_BYTES {
            return Err(ToolError::Invalid("the new content is too large".into()));
        }
        let (base, hash) = read_base(scope, grant, now, path)?;
        if base == new_content {
            return Err(ToolError::Invalid("the content is unchanged".into()));
        }
        let (a, b) = (base.as_bytes(), new_content.as_bytes());
        let mut start = a.iter().zip(b).take_while(|(x, y)| x == y).count();
        while !base.is_char_boundary(start) || !new_content.is_char_boundary(start) {
            start -= 1;
        }
        let max_tail = a.len().min(b.len()) - start;
        let mut tail = a
            .iter()
            .rev()
            .zip(b.iter().rev())
            .take(max_tail)
            .take_while(|(x, y)| x == y)
            .count();
        while !base.is_char_boundary(a.len() - tail)
            || !new_content.is_char_boundary(b.len() - tail)
        {
            tail -= 1;
        }
        Ok(Self {
            id: String::new(),
            path: path.to_path_buf(),
            operation: Operation::Modify,
            base_hash: Some(hash),
            hunks: vec![Hunk {
                offset: start,
                line: line_of(&base, start),
                old: base[start..a.len() - tail].to_owned(),
                new: new_content[start..b.len() - tail].to_owned(),
            }],
            content: None,
            rationale: clean_rationale(rationale),
        })
    }

    /// Create a file that does not exist yet, in a directory that does.
    ///
    /// # Errors
    /// The path is not allowed, the directory is missing or outside the project, the file exists,
    /// or the content is too large.
    pub fn create(
        scope: &Scope,
        path: &Path,
        content: &str,
        rationale: &str,
    ) -> Result<Self, ToolError> {
        check_path(scope, path)?;
        if content.len() > MAX_CREATE_BYTES {
            return Err(ToolError::Invalid("the content is too large".into()));
        }
        let parent = path.parent().unwrap_or(Path::new(""));
        let dir = scope.open_dir(parent)?;
        let name: &OsStr = path.file_name().unwrap_or_default();
        match statat(&dir, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => Err(ToolError::Refused(format!(
                "{} already exists",
                path.display()
            ))),
            Err(Errno::NOENT) => Ok(Self {
                id: String::new(),
                path: path.to_path_buf(),
                operation: Operation::Create,
                base_hash: None,
                hunks: Vec::new(),
                content: Some(content.to_owned()),
                rationale: clean_rationale(rationale),
            }),
            Err(errno) => Err(ToolError::Failed(format!(
                "cannot check {}: {}",
                path.display(),
                std::io::Error::from(errno).kind()
            ))),
        }
    }

    /// Delete an existing file.
    ///
    /// # Errors
    /// The path is not allowed or the file cannot be read by this grant.
    pub fn delete(
        scope: &Scope,
        grant: &Grant,
        now: Instant,
        path: &Path,
        rationale: &str,
    ) -> Result<Self, ToolError> {
        check_path(scope, path)?;
        if !grant.can_read(path, now) {
            return Err(ToolError::Refused(format!(
                "{} is not a file this session may read, so it cannot be deleted",
                path.display()
            )));
        }
        let bytes = scope.read_file(path, MAX_FILE_BYTES)?;
        Ok(Self {
            id: String::new(),
            path: path.to_path_buf(),
            operation: Operation::Delete,
            base_hash: Some(sha256_hex(&bytes)),
            hunks: Vec::new(),
            content: None,
            rationale: clean_rationale(rationale),
        })
    }

    /// The item id within its proposal (`1`, `2`, ...).
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Path relative to the project root.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What the item does.
    #[must_use]
    pub fn operation(&self) -> Operation {
        self.operation
    }

    /// SHA-256 of the file as the user saw it; `None` for a new file.
    #[must_use]
    pub fn base_hash(&self) -> Option<&str> {
        self.base_hash.as_deref()
    }

    /// The hunks of a modify item, in file order.
    #[must_use]
    pub fn hunks(&self) -> &[Hunk] {
        &self.hunks
    }

    /// The content of a create item.
    #[must_use]
    pub fn content(&self) -> Option<&str> {
        self.content.as_deref()
    }

    /// Why the change is proposed (informational only, never trusted).
    #[must_use]
    pub fn rationale(&self) -> &str {
        &self.rationale
    }

    /// The risk of applying this item.
    #[must_use]
    pub fn risk(&self) -> Risk {
        self.operation.risk()
    }
}

/// A set of changes the user may approve, entirely or in part. Immutable once built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    id: String,
    root: PathBuf,
    items: Vec<Item>,
}

static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl Proposal {
    /// Seal `items` into a proposal for the project of `scope`.
    ///
    /// # Errors
    /// There are no items, too many, or two items name the same path.
    pub fn new(scope: &Scope, mut items: Vec<Item>) -> Result<Self, ToolError> {
        if items.is_empty() {
            return Err(ToolError::Invalid(
                "a proposal needs at least one item".into(),
            ));
        }
        if items.len() > MAX_ITEMS {
            return Err(ToolError::Invalid(format!(
                "a proposal holds at most {MAX_ITEMS} items"
            )));
        }
        let mut seen = BTreeSet::new();
        for item in &items {
            if !seen.insert(item.path.clone()) {
                return Err(ToolError::Invalid(
                    "two items change the same path; merge them into one".into(),
                ));
            }
        }
        let mut digest = String::new();
        for (index, item) in items.iter_mut().enumerate() {
            item.id = (index + 1).to_string();
            digest.push_str(&format!(
                "{}|{}|{}|{}|{}|{}\n",
                item.id,
                item.path.display(),
                item.operation.as_str(),
                item.base_hash.as_deref().unwrap_or("-"),
                item.hunks.len(),
                item.content.as_deref().map_or(0, str::len),
            ));
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let salt = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let hash = sha256_hex(
            format!(
                "{}|{digest}|{nanos}|{salt}|{}",
                scope.root().display(),
                std::process::id()
            )
            .as_bytes(),
        );
        Ok(Self {
            id: format!("p-{}", &hash[..12]),
            root: scope.root().to_path_buf(),
            items,
        })
    }

    /// The proposal id, `p-` and 12 hex digits.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The project root the proposal is about.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The items, in order.
    #[must_use]
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Show the proposal as text for the user: no control characters, long hunks cut short.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = format!("Proposal {} ({} item(s))\n", self.id, self.items.len());
        for item in &self.items {
            out.push_str(&format!(
                "\n[{}] {} {}",
                item.id,
                item.operation.as_str(),
                printable(&item.path.display().to_string())
            ));
            if let Some(hash) = &item.base_hash {
                out.push_str(&format!(" (base {})", &hash[..12]));
            }
            out.push('\n');
            if !item.rationale.is_empty() {
                out.push_str(&format!("    why: {}\n", item.rationale.replace('\n', " ")));
            }
            for (index, hunk) in item.hunks.iter().enumerate() {
                out.push_str(&format!("  hunk {} @ line {}\n", index + 1, hunk.line));
                push_lines(&mut out, '-', &hunk.old);
                push_lines(&mut out, '+', &hunk.new);
            }
            if let Some(content) = &item.content {
                push_lines(&mut out, '+', content);
            }
        }
        out
    }
}

const MAX_RENDER_LINES: usize = 40;

fn printable(text: &str) -> String {
    strip_control_chars(text).replace('\n', "\\n")
}

fn push_lines(out: &mut String, sign: char, text: &str) {
    let clean = strip_control_chars(text);
    let total = clean.lines().count();
    for (count, line) in clean.lines().enumerate() {
        if count == MAX_RENDER_LINES {
            out.push_str(&format!(
                "    {sign} ... ({} more line(s))\n",
                total - count
            ));
            return;
        }
        out.push_str(&format!("    {sign} {line}\n"));
    }
}

/// Apply the chosen hunks to `base`, checking each one still sits where it was resolved.
///
/// # Errors
/// A hunk does not match `base` at its offset.
pub(crate) fn splice(base: &str, hunks: &[&Hunk]) -> Result<String, ToolError> {
    let mut ordered: Vec<&Hunk> = hunks.to_vec();
    ordered.sort_by_key(|h| h.offset);
    let mut out = String::with_capacity(base.len());
    let mut cursor = 0;
    for hunk in ordered {
        let end = hunk.offset + hunk.old.len();
        if hunk.offset < cursor
            || end > base.len()
            || !base.is_char_boundary(hunk.offset)
            || !base.is_char_boundary(end)
            || base[hunk.offset..end] != hunk.old
        {
            return Err(ToolError::Refused(
                "the file no longer matches the hunk the user saw".into(),
            ));
        }
        out.push_str(&base[cursor..hunk.offset]);
        out.push_str(&hunk.new);
        cursor = end;
    }
    out.push_str(&base[cursor..]);
    Ok(out)
}
