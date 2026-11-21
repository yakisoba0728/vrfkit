//! vrfkit -- CLI for VALORANT replay (.vrf) inspection, validation, and export.
//! The subcommands are listed once, in `cli.rs`'s `USAGE`.
//!
//! `export` is the only optional feature (on by default). Without it the
//! binary still inspects, validates and runs diag -- the last two drive the
//! whole decode pipeline -- and nothing links arrow, parquet or zstd.

#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    quiet_caught_codec_panics();
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

/// vrf-container turns a panic inside the Oodle codec into an error that
/// carries the panic message, but the default hook would first print a
/// `panicked at` report that reads like a crash. The library leaves the
/// process-global hook alone; this binary owns it, so it drops the report for
/// panics located in `oozextract` and defers every other panic to the default.
fn quiet_caught_codec_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let in_codec = info
            .location()
            .is_some_and(|at| at.file().contains("oozextract"));
        if !in_codec {
            default(info);
        }
    }));
}
