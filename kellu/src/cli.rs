//! Command-line parsing and dispatch.
//!
//! Exit codes: 0 ok, 1 error, 2 usage (clap), 3 not implemented.

use std::ffi::OsString;
use std::io::Write;

use clap::{CommandFactory, Parser, Subcommand};
use kellu_common::app::{DISPLAY_NAME, NAME};

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
    /// Check the environment (keyring and sandbox tooling)
    Doctor,
    /// (planned for M1)
    Config,
    /// (planned for M3)
    Keys,
    /// (planned for M5)
    Tools,
    /// (planned for M6)
    Lsp,
}

impl Command {
    /// The milestone that will implement this command, for commands that are only planned.
    fn planned_for(&self) -> Option<&'static str> {
        match self {
            Self::Doctor => None,
            Self::Config => Some("M1"),
            Self::Keys => Some("M3"),
            Self::Tools => Some("M5"),
            Self::Lsp => Some("M6"),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Doctor => "doctor",
            Self::Config => "config",
            Self::Keys => "keys",
            Self::Tools => "tools",
            Self::Lsp => "lsp",
        }
    }
}

fn say(out: &mut dyn Write, text: &str) {
    // A closed pipe (`kellu doctor | head`) must not crash the process.
    let _ = writeln!(out, "{text}");
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
    match cli.command {
        None => {
            say(out, &Cli::command().render_help().to_string());
            say(err, "\nThe interactive assistant arrives in milestone M6.");
            EXIT_OK
        }
        Some(Command::Doctor) => doctor::run(&doctor::find_in_path, out),
        Some(command) => {
            let milestone = command.planned_for().unwrap_or("a later milestone");
            say(
                err,
                &format!(
                    "{NAME} {}: not implemented yet (planned for {milestone}).",
                    command.name()
                ),
            );
            EXIT_NOT_IMPLEMENTED
        }
    }
}
