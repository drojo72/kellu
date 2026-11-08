//! What a tool is and the context it runs in.

use std::fmt;
use std::path::Path;
use std::time::Instant;

use kellu_common::KelluError;
use kellu_common::text::Redactor;
use kellu_sandbox::{Limits, Sandbox};
use kellu_security::{Grant, Risk, Scope};
use serde_json::Value;

use crate::host::HostTools;
use crate::proposal::Proposal;

/// The static description of a tool. There is no way to build one without a [`Risk`].
#[derive(Debug, Clone)]
pub struct ToolSpec {
    name: &'static str,
    description: &'static str,
    risk: Risk,
    parameters: Value,
}

impl ToolSpec {
    /// A tool called `name` (lowercase letters, digits and `_`) with its risk, a description for
    /// the model and a JSON Schema object for its arguments.
    #[must_use]
    pub fn new(
        name: &'static str,
        risk: Risk,
        description: &'static str,
        parameters: Value,
    ) -> Self {
        Self {
            name,
            description,
            risk,
            parameters,
        }
    }

    /// The tool name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// What the tool does, for the model.
    #[must_use]
    pub fn description(&self) -> &'static str {
        self.description
    }

    /// The most the tool can do to the user's machine or accounts.
    #[must_use]
    pub fn risk(&self) -> Risk {
        self.risk
    }

    /// The JSON Schema of the arguments.
    #[must_use]
    pub fn parameters(&self) -> &Value {
        &self.parameters
    }

    /// Whether the spec is well formed (checked when it is registered).
    pub(crate) fn is_valid(&self) -> bool {
        let name_ok = !self.name.is_empty()
            && self.name.len() <= 48
            && self
                .name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        let schema_ok = self.parameters.get("type").and_then(Value::as_str) == Some("object");
        name_ok && schema_ok && !self.description.is_empty()
    }
}

/// Why a tool call did not produce a result. The model sees the text; it never contains the
/// values the call carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    /// The arguments are wrong; the model may try again.
    Invalid(String),
    /// Policy refused it (denied path, expired grant, missing permission).
    Refused(String),
    /// It was allowed but did not work.
    Failed(String),
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(m) => write!(f, "invalid arguments: {m}"),
            Self::Refused(m) => write!(f, "refused: {m}"),
            Self::Failed(m) => write!(f, "failed: {m}"),
        }
    }
}

impl std::error::Error for ToolError {}

impl From<KelluError> for ToolError {
    fn from(error: KelluError) -> Self {
        match error {
            KelluError::Security(message) => Self::Refused(message),
            other => Self::Failed(other.to_string()),
        }
    }
}

/// What a tool returns: text for the model and, optionally, a proposed change.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// Text for the model. The registry redacts it, strips control characters and bounds it.
    pub text: String,
    /// A change the user may approve. Nothing is applied by returning it.
    pub proposal: Option<Proposal>,
}

impl ToolOutput {
    /// A text-only result.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            proposal: None,
        }
    }

    /// A result that carries a proposal.
    #[must_use]
    pub fn with_proposal(text: impl Into<String>, proposal: Proposal) -> Self {
        Self {
            text: text.into(),
            proposal: Some(proposal),
        }
    }
}

/// Everything a tool may use. It borrows; a tool keeps nothing between calls.
pub struct Context<'a> {
    /// The project, opened once.
    pub scope: &'a Scope,
    /// What this session may do, until it expires.
    pub grant: &'a Grant,
    /// The current monotonic time (checked against the grant).
    pub now: Instant,
    /// Removes secrets from what goes back to the model.
    pub redactor: &'a dyn Redactor,
    /// The sandbox, when this machine has a working one.
    pub sandbox: Option<&'a Sandbox>,
    /// Private directory under which staging copies are made.
    pub staging_base: &'a Path,
    /// Resource limits for sandboxed commands.
    pub limits: Limits,
    /// The host programs git and `gh`, when found.
    pub host: &'a HostTools,
    /// Where the request came from (`terminal`, `lsp`), for the audit log.
    pub origin: &'a str,
}

/// A tool the model may call.
///
/// `run` is only called after the registry checked the requirement for [`ToolSpec::risk`].
pub trait Tool: Send + Sync {
    /// The static description, including the mandatory risk.
    fn spec(&self) -> &ToolSpec;

    /// What this call would do, in one or two lines, for the confirmation prompt and the audit
    /// log. It must show the exact command or paths, not a paraphrase.
    ///
    /// # Errors
    /// The arguments are invalid.
    fn summary(&self, args: &Value) -> Result<String, ToolError>;

    /// Run the call.
    ///
    /// # Errors
    /// The arguments are invalid, policy refuses it, or it fails.
    fn run(&self, ctx: &Context<'_>, args: &Value) -> Result<ToolOutput, ToolError>;
}
