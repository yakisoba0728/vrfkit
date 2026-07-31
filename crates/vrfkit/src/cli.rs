//! Argument parsing — hand-rolled, no external dependencies.
//!
//! Three subcommands:
//!   inspect <file>
//!   validate <file>
//!   export <file> --out <dir>

use crate::driver;
use crate::error::CliError;
use crate::inspect;
use crate::oracle;

const USAGE: &str = "\
vrfkit — VALORANT replay (.vrf) toolkit

USAGE:
    vrfkit inspect  <file.vrf>
    vrfkit validate <file.vrf> [--diagnostics]
    vrfkit export   <file.vrf> --out <dir>

SUBCOMMANDS:
    inspect   Print replay info, header, branch, and chunk summary
    validate  Run the RepLayout grammar oracle on all content blocks
              --diagnostics  Print full context for every malformed/skipped event
    export    Write fields.parquet + movement.parquet + manifest.json
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
        "export" => {
            let file = args
                .get(2)
                .ok_or_else(|| CliError::Usage("export requires <file.vrf>".to_string()))?;
            let mut out_dir: Option<&str> = None;
            let mut i = 3;
            while i < args.len() {
                if args[i] == "--out" {
                    i += 1;
                    out_dir = Some(args.get(i).map(|s| s.as_str()).ok_or_else(|| {
                        CliError::Usage("--out requires a directory path".to_string())
                    })?);
                } else {
                    return Err(CliError::Usage(format!("unknown option: {}", args[i])));
                }
                i += 1;
            }
            let out_dir = out_dir
                .ok_or_else(|| CliError::Usage("export requires --out <dir>".to_string()))?;
            driver::run(file, out_dir)
        }
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(CliError::Usage(format!(
            "unknown subcommand: {other}\n{USAGE}"
        ))),
    }
}
