mod commands;
mod config;
mod repl;

use std::process::ExitCode;

pub(crate) fn main() -> ExitCode {
    match commands::cli() {
        Ok(status) => ExitCode::from(status),
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}
