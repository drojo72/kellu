//! `kellu` binary: command-line entry point. Logic lives in [`cli`] so it can be tested.

mod cli;
mod doctor;
mod frontends_cmd;
mod input;
mod keys;
mod providers_cmd;
mod tools_cmd;

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
