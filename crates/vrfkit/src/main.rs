//! vrfkit -- CLI for VALORANT replay (.vrf) inspection, validation, and export.
//! The subcommands are listed once, in `cli.rs`'s `USAGE`.
//!
//! `export` is the only optional feature (on by default). Without it the
//! binary still inspects, validates and runs diag -- the last two drive the
//! whole decode pipeline -- and nothing links arrow, parquet or zstd.

#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match vrfkit::run_cli(&args) {
        // Not always SUCCESS: `validate` returns its own code (`oracle::Verdict`).
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
