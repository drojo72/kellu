//! `kellu providers test`: one tiny request per provider, to check endpoint, model and key.

use std::io::Write;
use std::sync::Arc;
use std::time::Instant;

use kellu_common::text::{Redactor, strip_control_chars};
use kellu_core::config::Config;
use kellu_providers::base::KeyLookup;
use kellu_providers::types::{ChatRequest, Message};
use kellu_security::{PatternRedactor, Vault};

use crate::cli::{EXIT_ERROR, EXIT_OK, Services, say};

/// A key lookup backed by the vault. Only `ai_*` entries can ever be asked for (see
/// [`Vault::provider_key`]); any failure reads as "no key", which the provider reports.
pub fn key_lookup(vault: &Vault) -> KeyLookup {
    let vault = vault.clone();
    Arc::new(move |name: &str| vault.provider_key(name).ok().flatten())
}

pub fn test(
    config: &Config,
    only: Option<&str>,
    vault: &Vault,
    services: &Services,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let names: Vec<&str> = match only {
        Some(name) if config.providers().contains_key(name) => vec![name],
        Some(name) => {
            say(
                err,
                &format!(
                    "error: no provider named '{}' in the configuration",
                    strip_control_chars(name)
                ),
            );
            return EXIT_ERROR;
        }
        None => config.providers().keys().map(String::as_str).collect(),
    };
    let lookup = key_lookup(vault);
    let redactor = PatternRedactor::new();
    let Some(request) = ChatRequest::new(vec![Message::user("Reply with the single word: ok")])
        .map(|request| request.with_max_tokens(8))
    else {
        return EXIT_ERROR;
    };
    let mut failed = 0_u32;
    for name in names {
        let Some(provider_config) = config.providers().get(name) else {
            continue;
        };
        let provider = (services.build_probe)(provider_config, &lookup);
        // Progress goes to stderr first, so a slow provider does not look like a freeze.
        say(
            err,
            &format!(
                "testing {name} (up to {:.0}s)...",
                provider_config.timeout_s
            ),
        );
        let started = Instant::now();
        match provider.chat(&request) {
            Ok(_) => say(
                out,
                &format!(
                    "[  ok] {name}: {} ({:.1}s)",
                    provider_config.model,
                    started.elapsed().as_secs_f64()
                ),
            ),
            Err(error) => {
                failed += 1;
                let message = strip_control_chars(&redactor.redact(&error.to_string()));
                say(out, &format!("[FAIL] {name}: {message}"));
            }
        }
    }
    if failed == 0 { EXIT_OK } else { EXIT_ERROR }
}
