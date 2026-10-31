//! Logging: a private rotating file sink behind the `log` facade, with redaction.
//!
//! Nothing here runs at import time. [`configure_logging`] is the only function that touches the
//! filesystem or the global logger, and calling it again replaces the previous sink instead of
//! duplicating output. Only records whose target starts with `kellu` are written, so dependencies
//! cannot fill the file.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use kellu_common::paths;
use kellu_common::text::{IdentityRedactor, Redactor, strip_control_chars};
use kellu_common::utc::UtcStamp;
use kellu_common::{KelluError, Result};
use log::{Level, LevelFilter, Log, Metadata, Record};

/// File name of the application log inside the state directory.
pub const LOG_FILENAME: &str = "kellu.log";

/// Only records whose target starts with this prefix are written.
const TARGET_PREFIX: &str = "kellu";

/// Verbosity of the application log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Everything, including debugging detail.
    Debug,
    /// Normal operation (the default).
    Info,
    /// Something unexpected that Kellu recovered from.
    Warning,
    /// Failures only.
    Error,
}

impl LogLevel {
    /// Names accepted in the configuration, sorted.
    pub const NAMES: [&'static str; 4] = ["DEBUG", "ERROR", "INFO", "WARNING"];

    /// Parse a level name, ignoring case.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_uppercase().as_str() {
            "DEBUG" => Some(Self::Debug),
            "INFO" => Some(Self::Info),
            "WARNING" => Some(Self::Warning),
            "ERROR" => Some(Self::Error),
            _ => None,
        }
    }

    /// The canonical upper-case name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warning => "WARNING",
            Self::Error => "ERROR",
        }
    }

    fn filter(self) -> LevelFilter {
        match self {
            Self::Debug => LevelFilter::Debug,
            Self::Info => LevelFilter::Info,
            Self::Warning => LevelFilter::Warn,
            Self::Error => LevelFilter::Error,
        }
    }
}

/// Log file settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogSettings {
    /// Minimum level that is written.
    pub level: LogLevel,
    /// Size at which the file is rotated.
    pub max_bytes: u64,
    /// Rotated files to keep (`kellu.log.1` is the newest). Zero discards the old content.
    pub backups: u32,
}

impl Default for LogSettings {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            max_bytes: 1_000_000,
            backups: 3,
        }
    }
}

fn level_name(level: Level) -> &'static str {
    match level {
        Level::Error => "ERROR",
        Level::Warn => "WARNING",
        Level::Info => "INFO",
        Level::Debug | Level::Trace => "DEBUG",
    }
}

/// Format one record: redact the final text, strip control characters, keep it on one line.
///
/// Redacting the final string also covers formatted arguments and error text. Newlines are
/// escaped so a message cannot forge additional log lines.
fn format_line(
    stamp: UtcStamp,
    level: Level,
    target: &str,
    message: &str,
    redact: &dyn Redactor,
) -> String {
    let raw = format!("{stamp} {} {target}: {message}", level_name(level));
    let cleaned = strip_control_chars(&redact.redact(&raw));
    let mut line = cleaned.replace('\n', "\\n").replace('\t', " ");
    line.push('\n');
    line
}

/// An append-only file that rotates by size and is always opened privately (0600, no symlinks).
#[derive(Debug)]
struct RotatingFile {
    path: PathBuf,
    max_bytes: u64,
    backups: u32,
    file: File,
    size: u64,
}

impl RotatingFile {
    fn open(path: PathBuf, max_bytes: u64, backups: u32) -> Result<Self> {
        let file = paths::open_private_append(&path, false)?;
        let size = file.metadata().map_err(|e| KelluError::io(&path, e))?.len();
        Ok(Self {
            path,
            max_bytes,
            backups,
            file,
            size,
        })
    }

    fn numbered(&self, n: u32) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(format!(".{n}"));
        PathBuf::from(name)
    }

    fn rotate(&mut self) -> Result<()> {
        let ignore_missing = |result: io::Result<()>| match result {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(KelluError::io(&self.path, e)),
            _ => Ok(()),
        };
        if self.backups == 0 {
            ignore_missing(fs::remove_file(&self.path))?;
        } else {
            for n in (1..self.backups).rev() {
                ignore_missing(fs::rename(self.numbered(n), self.numbered(n + 1)))?;
            }
            ignore_missing(fs::rename(&self.path, self.numbered(1)))?;
        }
        self.file = paths::open_private_append(&self.path, false)?;
        self.size = 0;
        Ok(())
    }

    fn write_line(&mut self, line: &str) -> Result<()> {
        let len = line.len() as u64;
        if self.size > 0 && self.size.saturating_add(len) > self.max_bytes {
            self.rotate()?;
        }
        self.file
            .write_all(line.as_bytes())
            .and_then(|()| self.file.flush())
            .map_err(|e| KelluError::io(&self.path, e))?;
        self.size = self.size.saturating_add(len);
        Ok(())
    }
}

/// The file sink. Use [`configure_logging`] to install it as the process logger.
pub struct FileLogger {
    level: LogLevel,
    redact: Arc<dyn Redactor>,
    sink: Mutex<RotatingFile>,
}

impl FileLogger {
    /// Create the state directory (0700) and open `kellu.log` inside it (0600).
    ///
    /// # Errors
    ///
    /// [`KelluError::Security`] when the directory or the file is a symlink, not the right kind
    /// of file, or owned by another user; [`KelluError::Io`] for anything else.
    pub fn open(
        state_dir: &Path,
        settings: LogSettings,
        redact: Arc<dyn Redactor>,
    ) -> Result<Self> {
        paths::ensure_private_dir(state_dir)?;
        let sink = RotatingFile::open(
            state_dir.join(LOG_FILENAME),
            settings.max_bytes,
            settings.backups,
        )?;
        Ok(Self {
            level: settings.level,
            redact,
            sink: Mutex::new(sink),
        })
    }
}

impl std::fmt::Debug for FileLogger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileLogger")
            .field("level", &self.level)
            .finish_non_exhaustive()
    }
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target().starts_with(TARGET_PREFIX) && metadata.level() <= self.level.filter()
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format_line(
            UtcStamp::now(),
            record.level(),
            record.target(),
            &record.args().to_string(),
            self.redact.as_ref(),
        );
        let mut sink = self.sink.lock().unwrap_or_else(PoisonError::into_inner);
        // A failing log write must never crash or block the assistant.
        let _ = sink.write_line(&line);
    }

    fn flush(&self) {}
}

/// Forwards to the currently configured sink, so reconfiguring replaces it.
struct Forwarder {
    inner: RwLock<Option<Arc<FileLogger>>>,
}

impl Forwarder {
    fn current(&self) -> Option<Arc<FileLogger>> {
        self.inner
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Log for Forwarder {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.current().is_some_and(|sink| sink.enabled(metadata))
    }

    fn log(&self, record: &Record<'_>) {
        if let Some(sink) = self.current() {
            sink.log(record);
        }
    }

    fn flush(&self) {}
}

static FORWARDER: Forwarder = Forwarder {
    inner: RwLock::new(None),
};

/// Install a private rotating file sink as the process logger.
///
/// Idempotent: calling it again replaces the sink instead of adding a second one. If another
/// logger was installed first by someone else, Kellu's records are not delivered to this sink.
///
/// # Errors
///
/// Same as [`FileLogger::open`].
pub fn configure_logging(
    state_dir: &Path,
    settings: LogSettings,
    redact: Arc<dyn Redactor>,
) -> Result<()> {
    let logger = Arc::new(FileLogger::open(state_dir, settings, redact)?);
    *FORWARDER
        .inner
        .write()
        .unwrap_or_else(PoisonError::into_inner) = Some(logger);
    // Fails when a logger is already set (ours, from an earlier call): that is fine.
    let _ = log::set_logger(&FORWARDER);
    log::set_max_level(settings.level.filter());
    Ok(())
}

/// A redactor for callers that have none yet.
#[must_use]
pub fn no_redaction() -> Arc<dyn Redactor> {
    Arc::new(IdentityRedactor)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{MetadataExt, symlink};

    use tempfile::TempDir;

    use super::*;

    fn logger(dir: &Path, settings: LogSettings, redact: Arc<dyn Redactor>) -> FileLogger {
        FileLogger::open(dir, settings, redact).unwrap()
    }

    fn emit(logger: &FileLogger, level: Level, target: &str, message: &str) {
        logger.log(
            &Record::builder()
                .args(format_args!("{message}"))
                .level(level)
                .target(target)
                .build(),
        );
    }

    fn contents(dir: &Path) -> String {
        fs::read_to_string(dir.join(LOG_FILENAME)).unwrap()
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().mode() & 0o7777
    }

    #[test]
    fn level_names_parse_ignoring_case() {
        assert_eq!(LogLevel::parse("debug"), Some(LogLevel::Debug));
        assert_eq!(LogLevel::parse("Warning"), Some(LogLevel::Warning));
        for name in LogLevel::NAMES {
            assert_eq!(LogLevel::parse(name).map(LogLevel::as_str), Some(name));
        }
    }

    #[test]
    fn unknown_levels_are_rejected() {
        for name in ["", "loud", "TRACE", "WARN", "INFO "] {
            assert_eq!(LogLevel::parse(name), None, "{name:?}");
        }
    }

    #[test]
    fn creates_private_directory_and_file() {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("state");
        logger(&dir, LogSettings::default(), no_redaction());
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join(LOG_FILENAME)), 0o600);
    }

    #[test]
    fn writes_one_formatted_line_per_record() {
        let root = TempDir::new().unwrap();
        let sink = logger(root.path(), LogSettings::default(), no_redaction());
        emit(&sink, Level::Warn, "kellu_core::router", "falling back");
        let text = contents(root.path());
        assert!(
            text.ends_with(" WARNING kellu_core::router: falling back\n"),
            "{text}"
        );
        assert_eq!(text.lines().count(), 1);
    }

    #[test]
    fn records_below_the_level_are_dropped() {
        let root = TempDir::new().unwrap();
        let settings = LogSettings {
            level: LogLevel::Warning,
            ..LogSettings::default()
        };
        let sink = logger(root.path(), settings, no_redaction());
        emit(&sink, Level::Info, "kellu", "quiet");
        emit(&sink, Level::Error, "kellu", "loud");
        let text = contents(root.path());
        assert!(!text.contains("quiet") && text.contains("loud"), "{text}");
    }

    #[test]
    fn records_from_other_crates_are_not_written() {
        let root = TempDir::new().unwrap();
        let sink = logger(root.path(), LogSettings::default(), no_redaction());
        emit(&sink, Level::Error, "hyper::client", "noise");
        assert_eq!(contents(root.path()), "");
    }

    #[test]
    fn redaction_applies_to_messages_and_error_text() {
        let root = TempDir::new().unwrap();
        let redact: Arc<dyn Redactor> = Arc::new(|text: &str| text.replace("hunter2", "***"));
        let sink = logger(root.path(), LogSettings::default(), redact);
        emit(&sink, Level::Info, "kellu", "login with hunter2");
        let error = KelluError::config("bad value hunter2");
        sink.log(
            &Record::builder()
                .args(format_args!("failed: {error}"))
                .level(Level::Error)
                .target("kellu")
                .build(),
        );
        let text = contents(root.path());
        assert!(!text.contains("hunter2"), "{text}");
        assert!(text.contains("login with ***") && text.contains("failed: bad value ***"));
    }

    #[test]
    fn control_characters_are_stripped() {
        let root = TempDir::new().unwrap();
        let sink = logger(root.path(), LogSettings::default(), no_redaction());
        emit(
            &sink,
            Level::Info,
            "kellu",
            "\u{1b}[31mred\u{1b}[0m\r\u{202e}x\0",
        );
        let text = contents(root.path());
        assert!(text.ends_with("kellu: [31mred[0mx\n"), "{text:?}");
    }

    #[test]
    fn a_message_cannot_forge_another_log_line() {
        let root = TempDir::new().unwrap();
        let sink = logger(root.path(), LogSettings::default(), no_redaction());
        emit(
            &sink,
            Level::Info,
            "kellu",
            "one\n2026-01-01 00:00:00 ERROR kellu: forged",
        );
        let text = contents(root.path());
        assert_eq!(text.lines().count(), 1, "{text}");
    }

    #[test]
    fn rotation_keeps_the_newest_backups_and_stays_private() {
        let root = TempDir::new().unwrap();
        let settings = LogSettings {
            max_bytes: 200,
            backups: 2,
            ..LogSettings::default()
        };
        let sink = logger(root.path(), settings, no_redaction());
        for i in 0..40 {
            emit(&sink, Level::Info, "kellu", &format!("record number {i}"));
        }
        let base = root.path().join(LOG_FILENAME);
        let first = root.path().join("kellu.log.1");
        let second = root.path().join("kellu.log.2");
        assert!(first.exists() && second.exists());
        assert!(!root.path().join("kellu.log.3").exists());
        for path in [&base, &first, &second] {
            assert_eq!(mode(path), 0o600, "{}", path.display());
            assert!(fs::metadata(path).unwrap().len() <= 200);
        }
        assert!(
            fs::read_to_string(&base)
                .unwrap()
                .contains("record number 39")
        );
    }

    #[test]
    fn zero_backups_discards_old_content_and_stays_bounded() {
        let root = TempDir::new().unwrap();
        let settings = LogSettings {
            max_bytes: 150,
            backups: 0,
            ..LogSettings::default()
        };
        let sink = logger(root.path(), settings, no_redaction());
        for i in 0..30 {
            emit(&sink, Level::Info, "kellu", &format!("record number {i}"));
        }
        assert!(
            fs::metadata(root.path().join(LOG_FILENAME)).unwrap().len() <= 150,
            "the file must stay bounded"
        );
        assert!(!root.path().join("kellu.log.1").exists());
    }

    #[test]
    fn a_symlinked_log_file_is_refused_and_its_target_untouched() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        fs::write(&target, "precious").unwrap();
        let state = root.path().join("state");
        fs::create_dir(&state).unwrap();
        symlink(&target, state.join(LOG_FILENAME)).unwrap();
        let result = FileLogger::open(&state, LogSettings::default(), no_redaction());
        assert!(matches!(result, Err(KelluError::Security(_))));
        assert_eq!(fs::read_to_string(&target).unwrap(), "precious");
    }

    #[test]
    fn a_symlinked_state_directory_is_refused() {
        let root = TempDir::new().unwrap();
        let real = root.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = root.path().join("link");
        symlink(&real, &link).unwrap();
        let result = FileLogger::open(&link, LogSettings::default(), no_redaction());
        assert!(matches!(result, Err(KelluError::Security(_))));
    }
}
