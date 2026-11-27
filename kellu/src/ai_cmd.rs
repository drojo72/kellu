//! `kellu ai status|on|off`: flip the AI switch without editing the configuration and without
//! restarting the editor. The decision itself lives in `kellu_core::switch`; this file only creates
//! or removes the flag file it looks at.

use std::io::{ErrorKind, Write};

use kellu_common::paths::{ensure_private_dir, ensure_private_file};
use kellu_common::text::strip_control_chars;
use kellu_core::switch::AiSwitch;

use crate::cli::{EXIT_ERROR, EXIT_OK, Services, say};

/// `kellu ai status`: whether anything can go to a model right now, and why not.
pub fn status(services: &Services, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let config = match (services.load)() {
        Ok(config) => config,
        Err(error) => {
            say(err, &format!("error: {error}"));
            return EXIT_ERROR;
        }
    };
    let flag = (services.ai_flag)().ok();
    let switch = match AiSwitch::from_config(config.ai(), flag) {
        Ok(switch) => switch,
        Err(error) => {
            say(err, &format!("error: {error}"));
            return EXIT_ERROR;
        }
    };
    match switch.check(None, None) {
        Ok(()) => say(out, "AI: on"),
        Err(error) => say(out, &format!("AI: off\n  {error}")),
    }
    let ai = config.ai();
    if !ai.disabled_languages.is_empty() {
        let list = strip_control_chars(&ai.disabled_languages.join(", "));
        say(out, &format!("off for languages: {list}"));
    }
    if !ai.disabled_globs.is_empty() {
        let list = strip_control_chars(&ai.disabled_globs.join(", "));
        say(out, &format!("off for files: {list}"));
    }
    say(
        out,
        "a project can also switch it off for itself in its Kate settings",
    );
    EXIT_OK
}

/// `kellu ai off`: nothing is sent to any model until `kellu ai on`. Needs no configuration, so
/// it works even when the configuration file is broken.
pub fn off(services: &Services, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let result = (services.ai_flag)().and_then(|flag| {
        if let Some(dir) = flag.parent() {
            ensure_private_dir(dir)?;
        }
        ensure_private_file(&flag)
    });
    match result {
        Ok(()) => {
            say(
                out,
                "AI is off: nothing is sent to any model until `kellu ai on` (an editor that is \
                 already running obeys at once)",
            );
            EXIT_OK
        }
        Err(error) => {
            say(err, &format!("error: {error}"));
            EXIT_ERROR
        }
    }
}

/// `kellu ai on`: remove the flag file. The configuration can still keep the AI off.
pub fn on(services: &Services, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let flag = match (services.ai_flag)() {
        Ok(flag) => flag,
        Err(error) => {
            say(err, &format!("error: {error}"));
            return EXIT_ERROR;
        }
    };
    match std::fs::remove_file(&flag) {
        Ok(()) => say(out, "AI is on again"),
        Err(error) if error.kind() == ErrorKind::NotFound => say(out, "AI was already on"),
        Err(error) => {
            say(
                err,
                &format!("error: cannot remove {}: {error}", flag.display()),
            );
            return EXIT_ERROR;
        }
    }
    // Best effort: the configuration may be unreadable, which must not block turning the AI on.
    if let Ok(config) = (services.load)()
        && !config.ai().enabled
    {
        say(
            out,
            "note: [ai] enabled = false in the configuration still keeps it off",
        );
    }
    EXIT_OK
}
