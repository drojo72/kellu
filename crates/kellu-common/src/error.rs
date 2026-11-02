//! Error types. Messages never contain secrets.

use std::io;
use std::path::{Path, PathBuf};

/// Result alias used across Kellu crates.
pub type Result<T> = std::result::Result<T, KelluError>;

/// Base error for everything Kellu does.
#[derive(Debug, thiserror::Error)]
pub enum KelluError {
    /// Invalid or unreadable configuration.
    #[error("{0}")]
    Config(String),
    /// A security invariant would be violated (unsafe path, wrong ownership, denied access).
    #[error("{0}")]
    Security(String),
    /// A session log was used incorrectly (not started, already ended, bad entry kind).
    #[error("{0}")]
    Session(String),
    /// A model provider call failed.
    #[error(transparent)]
    Provider(#[from] ProviderError),
    /// Every provider in the chain for a task failed or was skipped.
    #[error("all providers failed for task '{task}': {}", summarize(failures))]
    AllProvidersFailed {
        /// The task type that was routed.
        task: String,
        /// One entry per provider tried or skipped, in chain order.
        failures: Vec<ProviderFailure>,
    },
    /// An I/O operation failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// File or directory involved.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
}

/// Why one provider in a chain did not produce an answer. Messages never contain secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderFailure {
    /// Provider name from the configuration.
    pub provider: String,
    /// HTTP status, or 0 when there is none.
    pub status: u16,
    /// Scrubbed reason.
    pub message: String,
    /// Whether another attempt might have succeeded.
    pub retryable: bool,
}

fn summarize(failures: &[ProviderFailure]) -> String {
    if failures.is_empty() {
        return "empty chain".to_owned();
    }
    failures
        .iter()
        .map(|failure| format!("{}: {}", failure.provider, failure.message))
        .collect::<Vec<_>>()
        .join("; ")
}

impl KelluError {
    /// Build a [`KelluError::Config`].
    pub fn config(message: impl Into<String>) -> Self {
        Self::Config(message.into())
    }

    /// Build a [`KelluError::Security`].
    pub fn security(message: impl Into<String>) -> Self {
        Self::Security(message.into())
    }

    /// Build a [`KelluError::Session`].
    pub fn session(message: impl Into<String>) -> Self {
        Self::Session(message.into())
    }

    /// Build a [`KelluError::Io`] for `path`.
    pub fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    /// The I/O error kind, when this is an I/O error (for example `AlreadyExists`).
    #[must_use]
    pub fn io_kind(&self) -> Option<io::ErrorKind> {
        match self {
            Self::Io { source, .. } => Some(source.kind()),
            _ => None,
        }
    }
}

/// A model provider call failed.
///
/// `retryable` is a hint for the router: transient failures (timeouts, 429, 5xx) are retryable,
/// configuration problems (missing key, bad request) are not.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("[{provider}] {reason}{}", status_suffix(*.status))]
pub struct ProviderError {
    /// Provider name from the configuration.
    pub provider: String,
    /// Scrubbed human-readable reason.
    pub reason: String,
    /// HTTP status, or 0 when there is none.
    pub status: u16,
    /// Whether another attempt or another provider may succeed.
    pub retryable: bool,
}

fn status_suffix(status: u16) -> String {
    if status == 0 {
        String::new()
    } else {
        format!(" (HTTP {status})")
    }
}

impl ProviderError {
    /// A retryable error without an HTTP status.
    pub fn new(provider: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            reason: reason.into(),
            status: 0,
            retryable: true,
        }
    }

    /// Attach an HTTP status.
    #[must_use]
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    /// Mark the error as not worth retrying.
    #[must_use]
    pub fn not_retryable(mut self) -> Self {
        self.retryable = false;
        self
    }

    /// Set the retryable flag explicitly.
    #[must_use]
    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_error_formats_like_the_prototype() {
        let plain = ProviderError::new("deepseek", "request timed out");
        assert_eq!(plain.to_string(), "[deepseek] request timed out");
        assert!(plain.retryable);
        let with_status = ProviderError::new("deepseek", "insufficient balance").with_status(402);
        assert_eq!(
            with_status.to_string(),
            "[deepseek] insufficient balance (HTTP 402)"
        );
    }

    #[test]
    fn provider_error_keeps_fields_for_the_router() {
        let err = ProviderError::new("google", "bad key")
            .with_status(400)
            .not_retryable();
        assert_eq!(
            (
                err.provider.as_str(),
                err.reason.as_str(),
                err.status,
                err.retryable
            ),
            ("google", "bad key", 400, false)
        );
    }

    #[test]
    fn provider_error_converts_into_the_base_error() {
        let err: KelluError = ProviderError::new("x", "boom").into();
        assert!(matches!(err, KelluError::Provider(_)));
        assert_eq!(err.to_string(), "[x] boom");
    }

    #[test]
    fn all_providers_failed_lists_every_failure_in_order() {
        let failure = |provider: &str, message: &str| ProviderFailure {
            provider: provider.to_owned(),
            status: 0,
            message: message.to_owned(),
            retryable: true,
        };
        let error = KelluError::AllProvidersFailed {
            task: "code".to_owned(),
            failures: vec![failure("a", "boom"), failure("b", "bust")],
        };
        assert_eq!(
            error.to_string(),
            "all providers failed for task 'code': a: boom; b: bust"
        );
        let empty = KelluError::AllProvidersFailed {
            task: "x".to_owned(),
            failures: Vec::new(),
        };
        assert!(empty.to_string().ends_with("empty chain"));
    }

    #[test]
    fn io_error_reports_path_and_kind() {
        let err = KelluError::io(
            Path::new("/tmp/x"),
            io::Error::from(io::ErrorKind::AlreadyExists),
        );
        assert_eq!(err.io_kind(), Some(io::ErrorKind::AlreadyExists));
        assert!(err.to_string().starts_with("/tmp/x: "));
        assert_eq!(KelluError::config("x").io_kind(), None);
    }
}
