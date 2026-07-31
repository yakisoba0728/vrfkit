//! vrfkit — CLI for VALORANT replay (.vrf) inspection, validation, and export.
//!
//! Subcommands:
//!   inspect <file.vrf>           — print replay info, header, and chunk summary
//!   validate <file.vrf>          — run the transform-validation oracle
//!   export <file.vrf> --out <dir> — emit fields.parquet + movement.parquet + manifest.json

#![forbid(unsafe_code)]

mod cli;
mod driver;
mod error;
mod inspect;
mod manifest;
mod oracle;
mod sink;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match cli::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
