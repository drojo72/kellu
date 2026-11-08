//! `kellu tools`: the tools the assistant may use and what each one needs from you.
//!
//! Listing builds the registry only; it runs no tool, opens no project and writes nothing.

use std::io::Write;

use kellu_common::paths::state_dir;
use kellu_common::text::strip_control_chars;
use kellu_security::{AuditLog, Requirement, Risk, requirement};
use kellu_tools::default_registry;

use crate::cli::{EXIT_ERROR, EXIT_OK, say};

/// What the user is asked, in words, for a tool of this risk.
fn asks(risk: Risk) -> &'static str {
    match requirement(risk, true) {
        Requirement::Automatic => "no",
        Requirement::Confirm if risk == Risk::Network => "yes, and only with network permission",
        Requirement::Confirm => "yes",
        Requirement::TypedConfirm => "yes, you type its name",
        Requirement::Denied => "never",
    }
}

fn first_sentence(text: &str) -> String {
    let cut = text.find(". ").map_or(text.len(), |i| i + 1);
    strip_control_chars(&text[..cut]).replace('\n', " ")
}

/// Print the table.
pub fn list(out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let audit = AuditLog::new(state_dir().unwrap_or_default().join("audit.log"));
    let registry = match default_registry(audit) {
        Ok(registry) => registry,
        Err(error) => {
            say(err, &format!("error: {error}"));
            return EXIT_ERROR;
        }
    };
    let mut specs: Vec<_> = registry.specs().collect();
    specs.sort_by_key(|spec| (spec.risk(), spec.name()));
    let width = specs.iter().map(|s| s.name().len()).max().unwrap_or(0);
    say(
        out,
        &format!(
            "{:<width$}  {:<11}  {:<38}  what it does",
            "tool", "risk", "asks you first"
        ),
    );
    for spec in specs {
        say(
            out,
            &format!(
                "{:<width$}  {:<11}  {:<38}  {}",
                spec.name(),
                spec.risk().as_str(),
                asks(spec.risk()),
                first_sentence(spec.description())
            ),
        );
    }
    say(out, "");
    say(
        out,
        "Changes to files are never written by a tool: they come back as a proposal that you approve, \
         per file or per hunk, and each deletion needs its path typed.",
    );
    EXIT_OK
}
