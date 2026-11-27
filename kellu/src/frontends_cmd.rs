//! `kellu chat` and `kellu lsp`: build the assistant from the configuration and hand it to a
//! frontend. Nothing here decides policy; see `kellu_core::assist` and `kellu_frontends`.

use std::collections::BTreeMap;
use std::io::Write;
use std::num::NonZeroUsize;
use std::sync::Arc;

use kellu_common::paths::state_dir;
use kellu_core::assist::Assistant;
use kellu_core::config::Config;
use kellu_core::log::{configure_logging, default_redactor};
use kellu_core::router::{Router, monotonic_clock};
use kellu_core::session::SessionLog;
use kellu_core::switch::AiSwitch;
use kellu_frontends::lsp::{Features, Server};
use kellu_frontends::repl::Repl;
use kellu_providers::KeyLookup;
use kellu_security::PatternRedactor;

use crate::cli::{EXIT_ERROR, Services, say};
use crate::providers_cmd::key_lookup;

/// The assistant over every configured provider. Keys are looked up per request, not here.
///
/// Without a usable keyring the assistant still starts: providers that need a key then fail with a
/// clear "no key" error, and ones that need none (a local server) keep working.
fn assistant(config: &Config, services: &Services, err: &mut dyn Write) -> Assistant {
    let lookup: KeyLookup = match (services.vault)() {
        Ok(vault) => key_lookup(&vault),
        Err(error) => {
            say(
                err,
                &format!(
                    "note: keyring unavailable ({error}); providers that need a key will fail"
                ),
            );
            Arc::new(|_: &str| None)
        }
    };
    let providers = config
        .providers()
        .iter()
        .map(|(name, provider)| {
            (
                name.clone(),
                (services.build_assistant_provider)(provider, &lookup),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let router = Router::new(config.clone(), providers, monotonic_clock());
    // The flag file of `kellu ai off` is looked at on every request. If the switch cannot be built
    // it fails closed: nothing is sent.
    let switch =
        AiSwitch::from_config(config.ai(), (services.ai_flag)().ok()).unwrap_or_else(|error| {
            say(err, &format!("error: {error}; the AI stays off"));
            AiSwitch::closed()
        });
    Assistant::new(router, Arc::new(PatternRedactor::new())).with_switch(switch)
}

fn load(services: &Services, err: &mut dyn Write) -> Option<Config> {
    match (services.load)() {
        Ok(config) => Some(config),
        Err(error) => {
            say(err, &format!("error: {error}"));
            None
        }
    }
}

/// `kellu chat`.
pub fn chat(services: &Services, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let Some(config) = load(services, err) else {
        return EXIT_ERROR;
    };
    let assistant = assistant(&config, services, err);
    let session = if config.sessions().enabled {
        state_dir().ok().map(|dir| {
            let limit =
                NonZeroUsize::new(config.sessions().max_entry_chars).unwrap_or(NonZeroUsize::MIN);
            SessionLog::new(dir.join("sessions"))
                .label("chat")
                .max_entry_chars(limit)
        })
    } else {
        None
    };
    let mut input = (services.stdin)();
    Repl::new(&assistant, (services.theme)(), session).run(&mut *input, out, err)
}

/// `kellu lsp`: the protocol owns `out`; everything else goes to `err` or the private log.
pub fn lsp(services: &Services, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let Some(config) = load(services, err) else {
        return EXIT_ERROR;
    };
    // Warnings from the server go to the private log, never to the protocol stream.
    if let Ok(dir) = state_dir() {
        let _ = configure_logging(&dir, *config.logging(), default_redactor());
    }
    let home = match (services.home)() {
        Ok(home) => home,
        Err(error) => {
            say(err, &format!("error: {error}"));
            return EXIT_ERROR;
        }
    };
    let assistant = assistant(&config, services, err);
    // With the AI off in the configuration the editor is not even offered the features.
    let features = Features {
        hover: config.lsp().hover && config.ai().enabled,
        completion: config.lsp().completion && config.ai().enabled,
    };
    if !config.ai().enabled {
        say(
            err,
            "note: the AI is off ([ai] enabled = false): hover and completion are not offered",
        );
    } else if !features.hover && !features.completion {
        say(
            err,
            "note: hover and completion are off; turn them on under [lsp] in the configuration \
             (they send code around the cursor to a model provider)",
        );
    }
    let mut input = (services.stdin)();
    let code = Server::new(&assistant, features, home).serve(&mut *input, out);
    let usage = assistant.usage();
    if !usage.is_empty() {
        for line in usage.lines() {
            say(err, &format!("usage: {line}"));
        }
    }
    code
}
