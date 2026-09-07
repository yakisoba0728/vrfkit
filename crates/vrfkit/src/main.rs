//! vrfkit -- CLI for VALORANT replay (.vrf) inspection, validation, and export.
//!
//! Subcommands:
//!   inspect <file.vrf>           -- print replay info, header, and chunk summary
//!   validate <file.vrf>          -- run the transform-validation oracle
//!   `diag <file.vrf> [--json <path>] [--include-payloads]` -- failure aggregate
//!   export `<file.vrf>` --out `<dir>` -- emit five Parquet tables + manifest.json
//!
//! `export` is behind the `export` feature (on by default). With it off the
//! binary still inspects, validates and runs diag -- all three drive the whole
//! decode pipeline -- and nothing links arrow, parquet or zstd.

#![forbid(unsafe_code)]

mod cli;
mod diagnose;
#[cfg(feature = "export")]
mod driver;
mod error;
mod inspect;
#[cfg(feature = "export")]
mod manifest;
mod oracle;
mod sink;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match cli::run(&args) {
        // Not always SUCCESS: `validate` is an oracle and returns its own code.
        // See `oracle::Verdict`.
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
