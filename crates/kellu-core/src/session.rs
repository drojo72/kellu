//! Session logs: append-only Markdown files, redacted, size-limited and private (mode 0600).
//!
//! Every entry goes through the same pipeline: redact secrets, strip control characters,
//! truncate. Content is written inside a code fence longer than any backtick run it contains, so
//! it cannot forge headings or close the fence. File names come from a sanitized slug, never from
//! raw input.

use std::fs::File;
use std::io::Write;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;

use kellu_common::text::{IdentityRedactor, Redactor, strip_control_chars, truncate};
use kellu_common::utc::UtcStamp;
use kellu_common::{KelluError, Result, paths};

const MAX_SLUG: usize = 40;
const MAX_PROVIDER_CHARS: usize = 40;
const MAX_ATTEMPTS: u32 = 100;
const LABEL_LIMIT: usize = 100;
const STEP_LIMIT: usize = 200;
const DEFAULT_MAX_ENTRY_CHARS: usize = 2000;

/// What kind of event an entry records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// Something the user typed.
    User,
    /// A model answer.
    Assistant,
    /// A tool call or its result.
    Tool,
    /// A decision the assistant took (routing, fallback, confirmation).
    Decision,
    /// A failure.
    Error,
}

impl EntryKind {
    /// Every kind, in the order of the Python prototype.
    pub const ALL: [Self; 5] = [
        Self::User,
        Self::Assistant,
        Self::Tool,
        Self::Decision,
        Self::Error,
    ];

    /// The name written in the session file.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::Decision => "decision",
            Self::Error => "error",
        }
    }

    /// Parse a kind name (exact match).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    New,
    Active,
    Ended,
}

/// One conversation session written to a Markdown file.
///
/// Building a `SessionLog` touches nothing; [`SessionLog::start`] creates the directory and file.
/// Dropping an active session closes its file (every entry is flushed as it is written).
pub struct SessionLog {
    directory: PathBuf,
    label: Option<String>,
    redact: Arc<dyn Redactor>,
    max_entry_chars: NonZeroUsize,
    clock: Box<dyn Fn() -> UtcStamp + Send + Sync>,
    file: Option<File>,
    state: State,
    path: Option<PathBuf>,
}

impl std::fmt::Debug for SessionLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionLog")
            .field("directory", &self.directory)
            .field("state", &self.state)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl SessionLog {
    /// A new session that will be written inside `directory`.
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            label: None,
            redact: Arc::new(IdentityRedactor),
            max_entry_chars: NonZeroUsize::MIN.saturating_add(DEFAULT_MAX_ENTRY_CHARS - 1),
            clock: Box::new(UtcStamp::now),
            file: None,
            state: State::New,
            path: None,
        }
    }

    /// A short description shown in the header and used for the file name.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The redactor applied to every piece of text before it is written.
    #[must_use]
    pub fn redactor(mut self, redact: Arc<dyn Redactor>) -> Self {
        self.redact = redact;
        self
    }

    /// Longest entry written, in characters.
    #[must_use]
    pub fn max_entry_chars(mut self, limit: NonZeroUsize) -> Self {
        self.max_entry_chars = limit;
        self
    }

    /// Replace the clock (tests).
    #[must_use]
    pub fn clock(mut self, clock: impl Fn() -> UtcStamp + Send + Sync + 'static) -> Self {
        self.clock = Box::new(clock);
        self
    }

    /// The session file, once started.
    #[must_use]
    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    // ------------------------------------------------------------- lifecycle ----

    /// Create the session file and write its header.
    ///
    /// # Errors
    ///
    /// [`KelluError::Session`] when already started; [`KelluError::Security`] when the directory
    /// is a symlink or owned by someone else; [`KelluError::Io`] for anything else.
    pub fn start(&mut self) -> Result<PathBuf> {
        if self.state != State::New {
            return Err(KelluError::session("session was already started"));
        }
        paths::ensure_private_dir(&self.directory)?;
        let started = (self.clock)();
        let label = self
            .label
            .as_deref()
            .map(|label| self.clean(label, LABEL_LIMIT, true));
        let stem = format!("{}-{}", started.compact(), slugify(label.as_deref()));
        let (file, path) = self.open_unique(&stem)?;
        self.file = Some(file);
        self.path = Some(path.clone());
        self.state = State::Active;

        let mut header = format!("# Session {started}\n\n");
        if let Some(label) = label.filter(|label| !label.is_empty()) {
            header.push_str(&format!("- **Context**: {label}\n\n"));
        }
        self.write(&header)?;
        Ok(path)
    }

    /// Append the next steps (if any) and close the file.
    ///
    /// # Errors
    ///
    /// [`KelluError::Session`] when the session is not active.
    pub fn end(&mut self, next_steps: &[&str]) -> Result<PathBuf> {
        self.require_active()?;
        if !next_steps.is_empty() {
            let steps: Vec<String> = next_steps
                .iter()
                .map(|step| format!("- [ ] {}", self.clean(step, STEP_LIMIT, true)))
                .collect();
            self.write(&format!("## Next steps\n\n{}\n", steps.join("\n")))?;
        }
        self.file = None;
        self.state = State::Ended;
        self.path
            .clone()
            .ok_or_else(|| KelluError::session("session is not active"))
    }

    // --------------------------------------------------------------- entries ----

    /// Append one entry.
    ///
    /// # Errors
    ///
    /// [`KelluError::Session`] when the session is not active.
    pub fn log(&mut self, kind: EntryKind, content: &str, provider: Option<&str>) -> Result<()> {
        self.require_active()?;
        let body = self.clean(content, self.max_entry_chars.get(), false);
        let who = match provider.filter(|name| !name.is_empty()) {
            Some(name) => format!("{} ({})", kind.as_str(), sanitize_provider(name)),
            None => kind.as_str().to_owned(),
        };
        let fence = fence_for(&body);
        let time = (self.clock)().time();
        self.write(&format!(
            "### {time} · {who}\n\n{fence}text\n{body}\n{fence}\n\n"
        ))
    }

    // --------------------------------------------------------------- helpers ----

    /// Redact first (so a secret cut in half is never exposed), then sanitize and truncate.
    fn clean(&self, text: &str, limit: usize, single_line: bool) -> String {
        let mut cleaned = strip_control_chars(&self.redact.redact(text));
        if single_line {
            cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
        }
        truncate(
            &cleaned,
            NonZeroUsize::new(limit).unwrap_or(NonZeroUsize::MIN),
        )
    }

    fn open_unique(&self, stem: &str) -> Result<(File, PathBuf)> {
        for attempt in 0..MAX_ATTEMPTS {
            let name = if attempt == 0 {
                format!("{stem}.md")
            } else {
                format!("{stem}-{attempt}.md")
            };
            let path = self.directory.join(name);
            match paths::open_private_append(&path, true) {
                Ok(file) => return Ok((file, path)),
                Err(error) if error.io_kind() == Some(std::io::ErrorKind::AlreadyExists) => {}
                Err(error) => return Err(error),
            }
        }
        Err(KelluError::session(
            "could not allocate a unique session file",
        ))
    }

    fn require_active(&self) -> Result<()> {
        if self.state == State::Active {
            Ok(())
        } else {
            Err(KelluError::session("session is not active"))
        }
    }

    fn write(&mut self, text: &str) -> Result<()> {
        let location = self.path.as_deref().unwrap_or(&self.directory);
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| KelluError::session("session is not active"))?;
        file.write_all(text.as_bytes())
            .and_then(|()| file.flush())
            .map_err(|e| KelluError::io(location, e))
    }
}

/// Lower-case ASCII letters and digits separated by single dashes, at most 40 characters.
fn slugify(label: Option<&str>) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for c in label.unwrap_or("").to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(c);
        } else {
            pending_dash = true;
        }
    }
    let cut: String = slug.chars().take(MAX_SLUG).collect();
    let cut = cut.trim_matches('-');
    if cut.is_empty() {
        "session".to_owned()
    } else {
        cut.to_owned()
    }
}

fn sanitize_provider(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '?'
            }
        })
        .take(MAX_PROVIDER_CHARS)
        .collect()
}

/// A backtick run longer than any inside `text` (at least three).
fn fence_for(text: &str) -> String {
    let (mut longest, mut run) = (0usize, 0usize);
    for c in text.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    "`".repeat(longest.saturating_add(1).max(3))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{MetadataExt, symlink};
    use std::path::Path;

    use tempfile::TempDir;

    use super::*;

    const NOON: i64 = 1_700_000_000; // 2023-11-14 22:13:20 UTC

    fn session(dir: &Path) -> SessionLog {
        SessionLog::new(dir).clock(|| UtcStamp::from_unix(NOON))
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().mode() & 0o7777
    }

    fn started(log: SessionLog) -> SessionLog {
        let mut log = log;
        log.start().unwrap();
        log
    }

    #[test]
    fn start_creates_private_file_with_header() {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("sessions");
        let mut log = session(&dir).label("Fix the parser");
        let path = log.start().unwrap();
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&path), 0o600);
        assert_eq!(
            path.file_name().unwrap(),
            "2023-11-14-221320-fix-the-parser.md"
        );
        assert_eq!(
            read(&path),
            "# Session 2023-11-14 22:13:20\n\n- **Context**: Fix the parser\n\n"
        );
    }

    #[test]
    fn empty_label_uses_default_slug() {
        let root = TempDir::new().unwrap();
        for label in ["", "   ", "!!!", "ñ¿?"] {
            let mut log = session(root.path()).label(label);
            let path = log.start().unwrap();
            assert!(
                path.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .ends_with("-session.md")
                    || path.to_str().unwrap().contains("-session-"),
                "{label:?}"
            );
        }
        // A blank label leaves no empty "Context" line; punctuation is real content.
        for label in ["", "   "] {
            let own = TempDir::new().unwrap();
            let mut log = session(own.path()).label(label);
            assert!(
                !read(&log.start().unwrap()).contains("Context"),
                "{label:?}"
            );
        }
        let mut log = session(root.path());
        let path = log.start().unwrap();
        assert!(path.to_str().unwrap().contains("-session"));
    }

    #[test]
    fn hostile_label_cannot_escape_the_directory() {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("sessions");
        for label in [
            "../../etc/passwd",
            "/etc/shadow",
            "a/b\\c",
            "..",
            "x\0y\n--",
        ] {
            let mut log = session(&dir).label(label);
            let path = log.start().unwrap();
            assert_eq!(path.parent().unwrap(), dir, "{label:?}");
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            assert!(
                name.ends_with(".md") && !name.contains(['/', '\\', '\0']),
                "{name}"
            );
        }
    }

    #[test]
    fn a_long_label_is_cut_in_the_name_and_the_header() {
        let root = TempDir::new().unwrap();
        let mut log = session(root.path()).label("word ".repeat(100));
        let path = log.start().unwrap();
        let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
        assert!(
            stem.len() <= "2023-11-14-221320-".len() + MAX_SLUG,
            "{stem}"
        );
        assert!(!stem.ends_with('-'));
        assert!(read(&path).contains("[truncated"));
    }

    #[test]
    fn a_secret_in_the_label_is_redacted_before_the_file_name_is_made() {
        let root = TempDir::new().unwrap();
        let redact: Arc<dyn Redactor> = Arc::new(|text: &str| text.replace("sk-live-abc", "***"));
        let mut log = session(root.path())
            .label("deploy sk-live-abc")
            .redactor(redact);
        let path = log.start().unwrap();
        assert!(
            !path.to_str().unwrap().contains("sk-live"),
            "{}",
            path.display()
        );
        assert!(!read(&path).contains("sk-live"));
    }

    #[test]
    fn same_second_sessions_get_unique_files() {
        let root = TempDir::new().unwrap();
        let mut paths_seen = Vec::new();
        for _ in 0..5 {
            let mut log = session(root.path()).label("same");
            paths_seen.push(log.start().unwrap());
        }
        let mut unique = paths_seen.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 5);
        assert!(paths_seen[1].to_str().unwrap().ends_with("-same-1.md"));
    }

    #[test]
    fn an_existing_dangling_symlink_is_skipped_not_followed() {
        let root = TempDir::new().unwrap();
        let victim = root.path().join("victim");
        symlink(&victim, root.path().join("2023-11-14-221320-session.md")).unwrap();
        let mut log = session(root.path());
        let path = log.start().unwrap();
        assert!(path.to_str().unwrap().ends_with("-session-1.md"));
        assert!(!victim.exists());
    }

    #[test]
    fn a_symlinked_session_directory_is_refused() {
        let root = TempDir::new().unwrap();
        let real = root.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = root.path().join("link");
        symlink(&real, &link).unwrap();
        assert!(matches!(
            session(&link).start(),
            Err(KelluError::Security(_))
        ));
        assert_eq!(fs::read_dir(&real).unwrap().count(), 0);
    }

    #[test]
    fn entries_have_a_heading_and_a_fenced_body() {
        let root = TempDir::new().unwrap();
        let mut log = started(session(root.path()));
        log.log(EntryKind::User, "hello", None).unwrap();
        log.log(EntryKind::Assistant, "hi", Some("deepseek"))
            .unwrap();
        let text = read(log.path().unwrap());
        assert!(
            text.contains("### 22:13:20 · user\n\n```text\nhello\n```\n\n"),
            "{text}"
        );
        assert!(text.contains("### 22:13:20 · assistant (deepseek)\n\n```text\nhi\n```\n\n"));
    }

    #[test]
    fn entries_are_redacted_cleaned_and_truncated() {
        let root = TempDir::new().unwrap();
        let redact: Arc<dyn Redactor> = Arc::new(|text: &str| text.replace("hunter2", "***"));
        let mut log = started(
            session(root.path())
                .redactor(redact)
                .max_entry_chars(NonZeroUsize::new(120).unwrap()),
        );
        log.log(EntryKind::User, "pw hunter2 \u{1b}[31mred\u{202e}", None)
            .unwrap();
        log.log(EntryKind::Tool, &"x".repeat(500), None).unwrap();
        let text = read(log.path().unwrap());
        assert!(
            !text.contains("hunter2") && !text.contains('\u{1b}') && !text.contains('\u{202e}')
        );
        assert!(text.contains("pw *** [31mred"), "{text}");
        assert!(text.contains("... [truncated 380 chars]"), "{text}");
    }

    #[test]
    fn a_secret_cut_by_truncation_is_never_exposed() {
        let root = TempDir::new().unwrap();
        let redact: Arc<dyn Redactor> =
            Arc::new(|text: &str| text.replace("sk-live-abcdef", "***"));
        let mut log = started(
            session(root.path())
                .redactor(redact)
                .max_entry_chars(NonZeroUsize::new(100).unwrap()),
        );
        let content = format!("{}sk-live-abcdef", "y".repeat(95));
        log.log(EntryKind::User, &content, None).unwrap();
        assert!(!read(log.path().unwrap()).contains("sk-live"));
    }

    #[test]
    fn content_cannot_forge_headings_or_close_the_fence() {
        let root = TempDir::new().unwrap();
        let mut log = started(session(root.path()));
        let hostile = "x\n```\n### 23:59:59 · decision\n\nforged\n````\n## Next steps";
        log.log(EntryKind::User, hostile, None).unwrap();
        let text = read(log.path().unwrap());
        // The fence is five backticks: longer than the four-backtick run inside the content.
        assert!(text.contains("`````text\nx\n```\n### 23:59:59"), "{text}");
        let opening = text.matches("`````text").count();
        let closing = text.lines().filter(|l| *l == "`````").count();
        assert_eq!((opening, closing), (1, 1));
    }

    #[test]
    fn fences_grow_with_the_longest_backtick_run() {
        assert_eq!(fence_for(""), "```");
        assert_eq!(fence_for("`a``b"), "```");
        assert_eq!(fence_for("```"), "````");
        assert_eq!(fence_for("a``````b`"), "`".repeat(7));
    }

    #[test]
    fn provider_name_is_sanitized() {
        let root = TempDir::new().unwrap();
        let mut log = started(session(root.path()));
        log.log(EntryKind::Decision, "x", Some("evil)\n### forged `x`"))
            .unwrap();
        log.log(EntryKind::Decision, "y", Some(&"p".repeat(100)))
            .unwrap();
        let text = read(log.path().unwrap());
        assert!(text.contains("decision (evil??????forged??x?)"), "{text}");
        assert!(!text.contains("\n### forged"));
        assert!(text.contains(&format!("({})", "p".repeat(40))));
        assert!(!text.contains(&"p".repeat(41)));
    }

    #[test]
    fn unknown_kind_is_rejected() {
        assert_eq!(EntryKind::parse("assistant"), Some(EntryKind::Assistant));
        for name in ["", "system", "User", "assistant "] {
            assert_eq!(EntryKind::parse(name), None, "{name:?}");
        }
        for kind in EntryKind::ALL {
            assert_eq!(EntryKind::parse(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn end_appends_next_steps_and_closes() {
        let root = TempDir::new().unwrap();
        let mut log = started(session(root.path()));
        log.log(EntryKind::User, "hi", None).unwrap();
        let path = log
            .end(&["write tests", "line\none\u{1b}[0m   two"])
            .unwrap();
        let text = read(&path);
        assert!(
            text.ends_with("## Next steps\n\n- [ ] write tests\n- [ ] line one[0m two\n"),
            "{text}"
        );
        assert!(matches!(
            log.log(EntryKind::User, "late", None),
            Err(KelluError::Session(_))
        ));
        assert!(matches!(log.end(&[]), Err(KelluError::Session(_))));
    }

    #[test]
    fn end_without_steps_writes_nothing_more() {
        let root = TempDir::new().unwrap();
        let mut log = started(session(root.path()));
        let path = log.end(&[]).unwrap();
        assert_eq!(read(&path), "# Session 2023-11-14 22:13:20\n\n");
    }

    #[test]
    fn logging_before_start_or_after_end_fails() {
        let root = TempDir::new().unwrap();
        let mut log = session(root.path());
        assert!(matches!(
            log.log(EntryKind::User, "early", None),
            Err(KelluError::Session(_))
        ));
        assert!(matches!(log.end(&[]), Err(KelluError::Session(_))));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        log.start().unwrap();
        assert!(matches!(log.start(), Err(KelluError::Session(_))));
        log.end(&[]).unwrap();
        assert!(matches!(log.start(), Err(KelluError::Session(_))));
    }

    #[test]
    fn dropping_an_active_session_leaves_a_complete_file() {
        let root = TempDir::new().unwrap();
        let path = {
            let mut log = started(session(root.path()));
            log.log(EntryKind::User, "kept", None).unwrap();
            log.path().unwrap().to_path_buf()
        };
        assert!(read(&path).contains("kept"));
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn building_a_session_touches_nothing() {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("never");
        let _log = session(&dir).label("x");
        assert!(!dir.exists());
    }

    #[test]
    fn slugs_are_ascii_lowercase_with_single_dashes() {
        assert_eq!(slugify(Some("Hello,  World!")), "hello-world");
        assert_eq!(slugify(Some("--a--b--")), "a-b");
        assert_eq!(slugify(Some("Ñandú 2")), "and-2");
        assert_eq!(slugify(Some(&"a".repeat(80))).len(), MAX_SLUG);
        assert_eq!(slugify(None), "session");
    }
}
