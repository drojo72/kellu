//! Command-line parsing and dispatch.
//!
//! Exit codes: 0 ok, 1 error, 2 usage (clap), 3 not implemented.

use std::ffi::OsString;
use std::io::Write;

use clap::{CommandFactory, Parser, Subcommand};
use kellu_common::app::{DISPLAY_NAME, NAME};
use kellu_core::config::{self, Config};

use crate::doctor;

/// Success.
pub const EXIT_OK: u8 = 0;
/// A runtime error or a failed check.
pub const EXIT_ERROR: u8 = 1;
/// The command exists in the design but is not built yet (2 is reserved for usage errors).
pub const EXIT_NOT_IMPLEMENTED: u8 = 3;

#[derive(Debug, Parser)]
#[command(
    name = NAME,
    version,
    about = format!("{DISPLAY_NAME}: a secure, modular AI assistant for the Kate editor.")
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check the environment (config, keyring and sandbox tooling)
    Doctor,
    /// Inspect the configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// (planned for M3)
    Keys,
    /// (planned for M5)
    Tools,
    /// (planned for M6)
    Lsp,
}

#[derive(Debug, Clone, Copy, Subcommand)]
enum ConfigAction {
    /// Print the user configuration file location
    Path,
    /// Validate the effective configuration
    Check,
    /// Show providers and routes (never secrets)
    Show,
}

impl Command {
    /// The milestone that will implement a command that is only planned.
    fn planned(&self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Keys => Some(("keys", "M3")),
            Self::Tools => Some(("tools", "M5")),
            Self::Lsp => Some(("lsp", "M6")),
            Self::Doctor | Self::Config { .. } => None,
        }
    }
}

fn say(out: &mut dyn Write, text: &str) {
    // A closed pipe (`kellu doctor | head`) must not crash the process.
    let _ = writeln!(out, "{text}");
}

fn load() -> kellu_common::Result<Config> {
    config::load_config(None)
}

fn config_path(out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    match config::user_config_path() {
        Ok(path) => {
            say(out, &path.display().to_string());
            if !path.exists() {
                say(err, "(file does not exist; packaged defaults are in use)");
            }
            EXIT_OK
        }
        Err(error) => {
            say(err, &format!("error: {error}"));
            EXIT_ERROR
        }
    }
}

fn config_check(out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    match load() {
        Ok(config) => {
            let source = config.source().map_or_else(
                || "packaged defaults only".to_owned(),
                |p| p.display().to_string(),
            );
            say(
                out,
                &format!(
                    "ok: {} providers, {} routes ({source})",
                    config.providers().len(),
                    config.routing().len()
                ),
            );
            EXIT_OK
        }
        Err(error) => {
            say(err, &format!("error: {error}"));
            EXIT_ERROR
        }
    }
}

fn config_show(out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let config = match load() {
        Ok(config) => config,
        Err(error) => {
            say(err, &format!("error: {error}"));
            return EXIT_ERROR;
        }
    };
    say(out, "providers:");
    for (name, provider) in config.providers() {
        let key = match &provider.keyring_name {
            Some(keyring) => format!("keyring: ai_{keyring}_api_key"),
            None => "no key".to_owned(),
        };
        say(
            out,
            &format!(
                "  {name:<13} {:<14} {}  {}  ({key})",
                provider.kind.as_str(),
                provider.model,
                provider.base_url
            ),
        );
    }
    say(out, "routes:");
    for (task, route) in config.routing() {
        let chain: Vec<_> = route.chain().collect();
        say(out, &format!("  {task:<13} {}", chain.join(" -> ")));
    }
    say(
        out,
        &format!(
            "router: total time budget {}s",
            config.router().total_timeout_s
        ),
    );
    EXIT_OK
}

/// Run the CLI and return the process exit code.
pub fn run<I, T>(args: I, out: &mut dyn Write, err: &mut dyn Write) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let text = error.render().to_string();
            if error.use_stderr() {
                let _ = write!(err, "{text}");
            } else {
                let _ = write!(out, "{text}");
            }
            return u8::try_from(error.exit_code()).unwrap_or(EXIT_ERROR);
        }
    };
    let Some(command) = cli.command else {
        say(out, &Cli::command().render_help().to_string());
        say(err, "\nThe interactive assistant arrives in milestone M6.");
        return EXIT_OK;
    };
    match command {
        Command::Doctor => doctor::run(&doctor::find_in_path, &load, out),
        Command::Config { action } => match action {
            ConfigAction::Path => config_path(out, err),
            ConfigAction::Check => config_check(out, err),
            ConfigAction::Show => config_show(out, err),
        },
        planned => {
            let (name, milestone) = planned
                .planned()
                .unwrap_or(("command", "a later milestone"));
            say(
                err,
                &format!("{NAME} {name}: not implemented yet (planned for {milestone})."),
            );
            EXIT_NOT_IMPLEMENTED
        }
    }
}
