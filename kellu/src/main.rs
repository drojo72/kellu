//! `kellu` binary: command-line entry point. Logic lives in [`cli`] so it can be tested.

mod cli;
mod doctor;

use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let code = cli::run(
        std::env::args_os(),
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    );
    ExitCode::from(code)
}
