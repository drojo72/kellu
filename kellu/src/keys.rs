//! `kellu keys`: manage API keys in the OS keyring. Values are never printed.

use std::io::Write;

use kellu_security::Vault;

use crate::cli::{EXIT_ERROR, EXIT_OK, Services, say};

const PROMPT: &str = "Secret (input hidden, not echoed): ";

pub fn list(vault: &Vault, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    match vault.list() {
        Ok(keys) if keys.is_empty() => {
            say(out, "(no keys stored; add one with: kellu keys set <name>)");
            EXIT_OK
        }
        Ok(keys) => {
            for key in keys {
                let legacy = if key.legacy {
                    "  (earlier prototype)"
                } else {
                    ""
                };
                say(
                    out,
                    &format!("{:<9} {}{legacy}", key.tier.as_str(), key.name),
                );
            }
            EXIT_OK
        }
        Err(error) => fail(err, &error),
    }
}

pub fn set(
    vault: &Vault,
    name: &str,
    services: &Services,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    // Validate the name before asking for the secret, so a typo costs nothing.
    if let Err(error) = kellu_security::tier::resolve_entry(name) {
        return fail(err, &error);
    }
    let secret = match (services.read_secret)(PROMPT, err) {
        Ok(secret) => secret,
        Err(error) => return fail(err, &error),
    };
    match vault.set(name, &secret) {
        Ok(stored) => {
            let tier = kellu_security::tier::tier_of(&stored);
            say(out, &format!("stored {stored} ({})", tier.as_str()));
            EXIT_OK
        }
        Err(error) => fail(err, &error),
    }
}

pub fn remove(vault: &Vault, name: &str, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    match vault.remove(name) {
        Ok(entry) => {
            say(out, &format!("removed {entry}"));
            EXIT_OK
        }
        Err(error) => fail(err, &error),
    }
}

pub fn cleanup_legacy(vault: &Vault, apply: bool, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let report = match vault.cleanup_legacy(apply) {
        Ok(report) => report,
        Err(error) => return fail(err, &error),
    };
    if report.is_empty() {
        say(out, "no entries from the earlier prototype");
        return EXIT_OK;
    }
    for entry in &report {
        let state = match (entry.removed, entry.migrated, apply) {
            (true, _, _) => "removed",
            (false, true, false) => "has a kellu twin; would be removed with --apply",
            (false, true, true) => "has a kellu twin; removal failed",
            (false, false, _) => "kept: no kellu entry yet (run: kellu keys set)",
        };
        say(out, &format!("{}: {state}", entry.name));
    }
    EXIT_OK
}

fn fail(err: &mut dyn Write, error: &kellu_common::KelluError) -> u8 {
    say(err, &format!("error: {error}"));
    EXIT_ERROR
}
