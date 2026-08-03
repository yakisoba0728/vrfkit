//! Argument parsing -- hand-rolled, no external dependencies.
//!
//! Three subcommands:
//!   inspect <file>
//!   validate <file>
//!   export <file> --out <dir>      (feature `export`)

use crate::error::CliError;
use crate::inspect;
use crate::oracle;

const USAGE: &str = "\
vrfkit -- VALORANT replay (.vrf) toolkit

USAGE:
    vrfkit inspect  <file.vrf>
    vrfkit validate <file.vrf> [--diagnostics]
    vrfkit export   <file.vrf> --out <dir> [--checkpoints]

SUBCOMMANDS:
    inspect   Print replay info, header, branch, and chunk summary
    validate  Run the RepLayout grammar oracle on all content blocks
              --diagnostics  Print full context for every malformed/skipped event
    export    Write fields.parquet + movement.parquet + manifest.json
              --checkpoints  Also parse Checkpoint chunks into
                             checkpoint_fields.parquet. Off by default: the
                             snapshots are ~10% of the file and a separate
                             read, and the four other tables are unaffected
                             either way.
";

pub fn run(args: &[String]) -> Result<(), CliError> {
    // args[0] = binary name
    if args.len() < 2 {
        return Err(CliError::Usage(USAGE.to_string()));
    }

    match args[1].as_str() {
        "inspect" => {
            let file = args
                .get(2)
                .ok_or_else(|| CliError::Usage("inspect requires <file.vrf>".to_string()))?;
            inspect::run(file)
        }
        "validate" => {
            let file = args
                .get(2)
                .ok_or_else(|| CliError::Usage("validate requires <file.vrf>".to_string()))?;
            let diagnostics = args.iter().skip(3).any(|a| a == "--diagnostics");
            oracle::run(file, diagnostics)
        }
        "export" => export(args),
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(CliError::Usage(format!(
            "unknown subcommand: {other}\n{USAGE}"
        ))),
    }
}

#[cfg(feature = "export")]
fn export(args: &[String]) -> Result<(), CliError> {
    let file = args
        .get(2)
        .ok_or_else(|| CliError::Usage("export requires <file.vrf>".to_string()))?;
    let mut out_dir: Option<&str> = None;
    let mut with_checkpoints = false;
    let mut i = 3;
    while i < args.len() {
        if args[i] == "--out" {
            i += 1;
            out_dir =
                Some(args.get(i).map(String::as_str).ok_or_else(|| {
                    CliError::Usage("--out requires a directory path".to_string())
                })?);
        } else if args[i] == "--checkpoints" {
            with_checkpoints = true;
        } else {
            return Err(CliError::Usage(format!("unknown option: {}", args[i])));
        }
        i += 1;
    }
    let out_dir =
        out_dir.ok_or_else(|| CliError::Usage("export requires --out <dir>".to_string()))?;
    crate::driver::run(file, out_dir, with_checkpoints)
}

/// Refusal, not silence. A build without the `export` feature has no Parquet
/// writers at all, and a subcommand that printed nothing and exited 0 would be
/// indistinguishable from one that wrote the files.
#[cfg(not(feature = "export"))]
fn export(_args: &[String]) -> Result<(), CliError> {
    Err(CliError::Usage(
        "export is not available: this binary was built without the `export` feature".to_string(),
    ))
}
