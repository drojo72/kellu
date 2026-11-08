//! The registry: where the risk of every tool is enforced.
//!
//! A tool cannot exist without a [`Risk`], and [`Registry::call`] looks up what that risk
//! requires (`kellu_security::requirement`) **before** the tool runs:
//!
//! * `Automatic` runs at once (reads, and proposals, which change nothing).
//! * `Confirm` and `TypedConfirm` ask the [`Approver`] with the exact summary of the call, and
//!   record the decision in the audit log first; if the log cannot be written, nothing runs.
//! * `Denied` (a network tool without network permission) never runs and never asks.
//!
//! What a tool returns goes through the redactor, has its control characters removed and is
//! bounded before the model sees it. Errors are returned as text and never repeat arguments.

use std::collections::BTreeMap;

use kellu_common::text::{strip_control_chars, truncate};
use kellu_common::utc::UtcStamp;
use kellu_security::hash::sha256_hex;
use kellu_security::{AuditEvent, AuditLog, AuditRecord, Requirement, Risk, requirement};
use serde_json::Value;

use crate::fs_tools::{ListFiles, ProposeCreate, ProposeDelete, ProposeEdit, ReadFile, SearchText};
use crate::git::{GhPrCreate, GitCommit, GitDiff, GitLog, GitPush, GitStatus};
use crate::proposal::Proposal;
use crate::shell::RunCommand;
use crate::spec::{Context, Tool, ToolError, ToolSpec};

/// The most characters of tool output that go back to the model.
pub const MAX_RESULT_CHARS: usize = 60_000;

/// What the user said when asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirmation {
    /// No.
    Denied,
    /// Yes.
    Confirmed,
    /// Yes, and here is the text the user typed (needed for `TypedConfirm`).
    Typed(String),
}

/// What the user is asked to confirm.
#[derive(Debug, Clone)]
pub struct ConfirmRequest<'a> {
    /// The tool name.
    pub tool: &'a str,
    /// Its risk.
    pub risk: Risk,
    /// What is required: [`Requirement::Confirm`] or [`Requirement::TypedConfirm`]. For the typed
    /// kind the user must type the tool name.
    pub requirement: Requirement,
    /// The exact command or paths, one or two lines, without control characters.
    pub summary: &'a str,
}

/// Asks the user. The terminal and LSP frontends implement it.
pub trait Approver {
    /// Ask once. Anything but a matching answer is a refusal.
    fn confirm(&self, request: &ConfirmRequest<'_>) -> Confirmation;
}

/// The result of one tool call, ready for the model.
#[derive(Debug, Clone)]
pub struct CallResult {
    /// Text for the model: redacted, clean and bounded.
    pub text: String,
    /// A change for the user to approve, when the tool proposed one.
    pub proposal: Option<Proposal>,
    /// Whether the call failed or was refused.
    pub is_error: bool,
}

impl CallResult {
    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            proposal: None,
            is_error: true,
        }
    }
}

/// The set of tools a session offers.
pub struct Registry {
    tools: BTreeMap<&'static str, Box<dyn Tool>>,
    audit: AuditLog,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Registry {
    /// An empty registry that records to `audit`.
    #[must_use]
    pub fn new(audit: AuditLog) -> Self {
        Self {
            tools: BTreeMap::new(),
            audit,
        }
    }

    /// Add a tool.
    ///
    /// # Errors
    /// The name is malformed or taken, or the schema is not an object.
    pub fn register(&mut self, tool: Box<dyn Tool>) -> Result<(), ToolError> {
        let spec = tool.spec();
        if !spec.is_valid() {
            return Err(ToolError::Invalid(format!(
                "tool {}: malformed spec",
                spec.name()
            )));
        }
        if self.tools.contains_key(spec.name()) {
            return Err(ToolError::Invalid(format!(
                "tool {} is already registered",
                spec.name()
            )));
        }
        self.tools.insert(spec.name(), tool);
        Ok(())
    }

    /// The specs, by name, for offering to the model.
    pub fn specs(&self) -> impl Iterator<Item = &ToolSpec> {
        self.tools.values().map(|tool| tool.spec())
    }

    /// The audit log this registry records to.
    #[must_use]
    pub fn audit(&self) -> &AuditLog {
        &self.audit
    }

    fn record(
        &self,
        ctx: &Context<'_>,
        id: &str,
        event: AuditEvent,
        risk: Risk,
        validation: &str,
        summary: &str,
    ) -> bool {
        self.audit
            .append(
                &AuditRecord {
                    proposal: id.to_owned(),
                    event,
                    path: "-".to_owned(),
                    permission: risk,
                    origin: ctx.origin.to_owned(),
                    validation: validation.to_owned(),
                    hash: Some(sha256_hex(summary.as_bytes())),
                },
                UtcStamp::now(),
            )
            .is_ok()
    }

    /// Run the call `name` with `args`.
    ///
    /// It never panics and never returns the arguments in an error. A call that needs
    /// confirmation but cannot be recorded does not run.
    pub fn call(
        &self,
        name: &str,
        args: &Value,
        ctx: &Context<'_>,
        approver: &dyn Approver,
    ) -> CallResult {
        let Some(tool) = self.tools.get(name) else {
            return CallResult::error("unknown tool");
        };
        let spec = tool.spec();
        if ctx.grant.is_expired(ctx.now) {
            return CallResult::error("refused: the grant has expired");
        }
        let need = requirement(spec.risk(), ctx.grant.allows_network(ctx.now));
        let id = format!(
            "call-{}",
            &sha256_hex(format!("{name}|{:?}|{}", ctx.now, std::process::id()).as_bytes())[..12]
        );
        let mut audited = false;
        let mut summary = String::new();
        match need {
            Requirement::Automatic => {}
            Requirement::Denied => {
                return CallResult::error(format!(
                    "refused: {} needs network permission, which this session does not have",
                    spec.name()
                ));
            }
            Requirement::Confirm | Requirement::TypedConfirm => {
                summary = match tool.summary(args) {
                    Ok(text) => strip_control_chars(&text).replace('\n', " "),
                    Err(error) => return CallResult::error(error.to_string()),
                };
                let answer = approver.confirm(&ConfirmRequest {
                    tool: spec.name(),
                    risk: spec.risk(),
                    requirement: need,
                    summary: &summary,
                });
                let granted = match (&answer, need) {
                    (Confirmation::Confirmed | Confirmation::Typed(_), Requirement::Confirm) => {
                        true
                    }
                    (Confirmation::Typed(text), Requirement::TypedConfirm) => text == spec.name(),
                    _ => false,
                };
                let event = if granted {
                    AuditEvent::Approved
                } else {
                    AuditEvent::Rejected
                };
                if !self.record(
                    ctx,
                    &id,
                    event,
                    spec.risk(),
                    if granted { "ok" } else { "declined" },
                    &summary,
                ) {
                    return CallResult::error(
                        "failed: the audit log cannot be written; nothing ran",
                    );
                }
                if !granted {
                    return CallResult::error("refused: the user did not confirm");
                }
                audited = true;
            }
        }
        match tool.run(ctx, args) {
            Ok(output) => {
                if let Some(proposal) = &output.proposal {
                    for item in proposal.items() {
                        let record = AuditRecord {
                            proposal: proposal.id().to_owned(),
                            event: AuditEvent::Proposed,
                            path: item.path().display().to_string(),
                            permission: item.risk(),
                            origin: ctx.origin.to_owned(),
                            validation: "ok".to_owned(),
                            hash: item.base_hash().map(str::to_owned),
                        };
                        if self.audit.append(&record, UtcStamp::now()).is_err() {
                            return CallResult::error(
                                "failed: the audit log cannot be written; the proposal was dropped",
                            );
                        }
                    }
                }
                if audited {
                    self.record(ctx, &id, AuditEvent::Applied, spec.risk(), "ok", &summary);
                }
                CallResult {
                    text: bound(ctx, &output.text),
                    proposal: output.proposal,
                    is_error: false,
                }
            }
            Err(error) => {
                if audited {
                    self.record(
                        ctx,
                        &id,
                        AuditEvent::Failed,
                        spec.risk(),
                        "failed",
                        &summary,
                    );
                }
                CallResult::error(bound(ctx, &error.to_string()))
            }
        }
    }
}

fn bound(ctx: &Context<'_>, text: &str) -> String {
    let clean = strip_control_chars(&ctx.redactor.redact(text));
    match std::num::NonZeroUsize::new(MAX_RESULT_CHARS) {
        Some(limit) => truncate(&clean, limit),
        None => clean,
    }
}

/// Every tool Kellu ships, registered. A new tool is added here, with its risk in its spec, and
/// in the table of `tests/registry.rs`, which pins each risk.
///
/// # Errors
/// Never in practice; a duplicate name would be a programming error caught by the tests.
pub fn default_registry(audit: AuditLog) -> Result<Registry, ToolError> {
    let mut registry = Registry::new(audit);
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(ListFiles::new()),
        Box::new(ReadFile::new()),
        Box::new(SearchText::new()),
        Box::new(ProposeEdit::new()),
        Box::new(ProposeCreate::new()),
        Box::new(ProposeDelete::new()),
        Box::new(RunCommand::new()),
        Box::new(GitStatus::new()),
        Box::new(GitDiff::new()),
        Box::new(GitLog::new()),
        Box::new(GitCommit::new()),
        Box::new(GitPush::new()),
        Box::new(GhPrCreate::new()),
    ];
    for tool in tools {
        registry.register(tool)?;
    }
    Ok(registry)
}
