//! `run_command`: a command in the sandbox, on a staging copy of the project.
//!
//! The real project is never mounted. With `write_back` the copy is writable (formatters,
//! generators) and what the command changed comes back as a **proposal** that goes through the
//! same approval as any other change; anything unusual (a symlink, a file that is too large, a
//! denied path) is reported and never proposed. The command never gets the network, even when
//! the session allows it for something else.

use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration;

use kellu_common::text::strip_control_chars;
use kellu_sandbox::{Change, Policy, Profile, Staging, StagingLimits};
use kellu_security::Risk;
use kellu_security::hash::sha256_hex;
use rustix::fs::OFlags;
use serde_json::{Value, json};

use crate::args::Args;
use crate::host::{clean, describe};
use crate::proposal::{Item, MAX_FILE_BYTES, MAX_ITEMS, Proposal};
use crate::spec::{Context, Tool, ToolError, ToolOutput, ToolSpec};

const MAX_ARGV: usize = 64;
const MAX_SHOWN_OUTPUT_CHARS: usize = 20_000;

/// `run_command`.
#[derive(Debug)]
pub struct RunCommand(ToolSpec);

impl RunCommand {
    /// The tool.
    #[must_use]
    pub fn new() -> Self {
        Self(ToolSpec::new(
            "run_command",
            Risk::Read,
            "Run a program (no shell) in a sandbox on a copy of the project, without network. \
             Give the program and its arguments as a list. With write_back=true the copy is \
             writable and its changes come back as a proposal for the user to approve.",
            json!({"type":"object","properties":{
                "argv":{"type":"array","items":{"type":"string"},"description":"Program first, then arguments"},
                "write_back":{"type":"boolean","description":"Allow writes to the copy and propose them (default false)"},
                "timeout_s":{"type":"integer","description":"Wall-clock limit in seconds, never above the configured one"}
            },"required":["argv"]}),
        ))
    }
}

impl Default for RunCommand {
    fn default() -> Self {
        Self::new()
    }
}

fn line(argv: &[&str]) -> String {
    strip_control_chars(&argv.join(" ")).replace('\n', " ")
}

impl Tool for RunCommand {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    fn summary(&self, args: &Value) -> Result<String, ToolError> {
        let a = Args::new(args, &["argv", "write_back", "timeout_s"])?;
        Ok(format!(
            "run in the sandbox: {}",
            line(&a.str_list("argv", MAX_ARGV)?)
        ))
    }

    fn run(&self, ctx: &Context<'_>, args: &Value) -> Result<ToolOutput, ToolError> {
        let a = Args::new(args, &["argv", "write_back", "timeout_s"])?;
        let argv = a.str_list("argv", MAX_ARGV)?;
        let write_back = a.bool_or("write_back", false)?;
        let sandbox = ctx.sandbox.ok_or_else(|| {
            ToolError::Refused(
                "there is no working sandbox on this machine, so commands do not run (see `kellu doctor`)".into(),
            )
        })?;
        let mut limits = ctx.limits;
        if let Some(seconds) = a.opt_u64("timeout_s")? {
            limits.wall_clock = limits.wall_clock.min(Duration::from_secs(seconds.max(1)));
        }
        let profile = if write_back {
            Profile::WorkspaceWrite
        } else {
            Profile::ReadOnly
        };
        let offline = ctx.grant.without_network();
        let policy =
            Policy::from_grant(&offline, profile, limits, ctx.now)?.with_env(std::env::vars_os());
        let staging = Staging::create(
            ctx.staging_base,
            ctx.scope,
            &policy,
            StagingLimits::default(),
            ctx.now,
        )?;
        let command: Vec<OsString> = argv.iter().map(OsString::from).collect();
        let output = sandbox.run(&policy, &staging, &command, ctx.now)?;

        let mut text = format!("$ {}\n{}\n", line(&argv), describe(output.outcome));
        for (label, bytes) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
            if !bytes.is_empty() {
                text.push_str(&format!(
                    "--- {label} ---\n{}\n",
                    clean(bytes, MAX_SHOWN_OUTPUT_CHARS)
                ));
            }
        }
        if !staging.skipped().is_empty() {
            text.push_str(&format!(
                "({} granted file(s) were not copied into the sandbox: links, special files or too large)\n",
                staging.skipped().len()
            ));
        }
        if !write_back {
            return Ok(ToolOutput::text(text));
        }
        let changes = staging.changes()?;
        let (proposal, notes) = proposal_from(ctx, &staging, &changes);
        if changes.is_empty() {
            text.push_str("The command changed nothing.\n");
        }
        for note in &notes {
            text.push_str(&format!("note: {note}\n"));
        }
        match proposal {
            Some(proposal) => {
                text.push_str(&format!(
                    "Proposal {} records {} change(s). Nothing was changed in the project: the user decides.\n",
                    proposal.id(),
                    proposal.items().len()
                ));
                Ok(ToolOutput::with_proposal(text, proposal))
            }
            None => Ok(ToolOutput::text(text)),
        }
    }
}

/// Read a changed file from the copy without following a link, and only if it is what
/// [`Staging::changes`] hashed.
fn read_staged(staging: &Staging, path: &Path, expected: &str) -> Result<Vec<u8>, String> {
    let full = staging.path().join(path);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(
            i32::try_from((OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC).bits())
                .unwrap_or(0),
        )
        .open(&full)
        .map_err(|_| "a changed file could not be read safely".to_owned())?;
    let meta = file
        .metadata()
        .map_err(|_| "a changed file could not be read".to_owned())?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return Err("a changed file is not a plain file or is too large".into());
    }
    let mut data = Vec::new();
    File::take(file, MAX_FILE_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|_| "a changed file could not be read".to_owned())?;
    if sha256_hex(&data) == expected {
        Ok(data)
    } else {
        Err("a changed file differs from what was measured".into())
    }
}

fn proposal_from(
    ctx: &Context<'_>,
    staging: &Staging,
    changes: &[Change],
) -> (Option<Proposal>, Vec<String>) {
    let mut items = Vec::new();
    let mut notes = Vec::new();
    let why = "result of a command run in the sandbox";
    for change in changes {
        let name = strip_control_chars(&change.path().display().to_string()).replace('\n', " ");
        if items.len() >= MAX_ITEMS {
            notes.push(format!(
                "{name}: not proposed, a proposal holds at most {MAX_ITEMS} items"
            ));
            continue;
        }
        let made = match change {
            Change::Unsafe { what, .. } => {
                notes.push(format!("{name}: {what}; reported only, never proposed"));
                continue;
            }
            Change::Modified {
                path,
                base_hash,
                new_hash,
            } => read_staged(staging, path, new_hash)
                .map_err(ToolError::Failed)
                .and_then(|bytes| {
                    String::from_utf8(bytes)
                        .map_err(|_| ToolError::Invalid("not UTF-8 text".into()))
                })
                .and_then(|text| Item::modify_to(ctx.scope, ctx.grant, ctx.now, path, &text, why))
                .and_then(|item| {
                    if item.base_hash() == Some(base_hash.as_str()) {
                        Ok(item)
                    } else {
                        Err(ToolError::Refused(
                            "the project changed while the command ran".into(),
                        ))
                    }
                }),
            Change::Created { path, new_hash } => read_staged(staging, path, new_hash)
                .map_err(ToolError::Failed)
                .and_then(|bytes| {
                    String::from_utf8(bytes)
                        .map_err(|_| ToolError::Invalid("not UTF-8 text".into()))
                })
                .and_then(|text| Item::create(ctx.scope, path, &text, why)),
            Change::Deleted { path, base_hash } => {
                Item::delete(ctx.scope, ctx.grant, ctx.now, path, why).and_then(|item| {
                    if item.base_hash() == Some(base_hash.as_str()) {
                        Ok(item)
                    } else {
                        Err(ToolError::Refused(
                            "the project changed while the command ran".into(),
                        ))
                    }
                })
            }
        };
        match made {
            Ok(item) => items.push(item),
            Err(error) => notes.push(format!("{name}: not proposed ({error})")),
        }
    }
    let proposal = if items.is_empty() {
        None
    } else {
        Proposal::new(ctx.scope, items).ok()
    };
    (proposal, notes)
}
