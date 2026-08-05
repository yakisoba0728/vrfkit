//! vrfkit -- CLI for VALORANT replay (.vrf) inspection, validation, and export.
//!
//! Subcommands:
//!   inspect <file.vrf>           -- print replay info, header, and chunk summary
//!   validate <file.vrf>          -- run the transform-validation oracle
//!   export <file.vrf> --out <dir> -- emit five Parquet tables + manifest.json
//!
//! `export` is behind the `export` feature (on by default). With it off the
//! binary still inspects and validates -- both drive the whole decode pipeline
//! -- and nothing links arrow, parquet or zstd.

#![forbid(unsafe_code)]

mod cli;
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
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
