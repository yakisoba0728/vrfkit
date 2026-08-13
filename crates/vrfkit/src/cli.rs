//! Argument parsing -- hand-rolled, no external dependencies.
//!
//! Three subcommands:
//!   inspect `<file>`
//!   validate `<file>`
//!   export `<file>` --out `<dir>`      (feature `export`)

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
    validate  Run the RepLayout grammar oracle on every ReplayData content
              block. Exits 0 when all of them framed, 1 when any did not,
              and 2 when the file carried no content blocks to check.
              --diagnostics  Print full context for every malformed/skipped event
    export    Write five Parquet tables (fields, movement, actors,
              net_guids, events) + manifest.json
              --checkpoints  Also parse Checkpoint chunks into
                             checkpoint_fields.parquet. Off by default: the
                             snapshots are ~10% of the file and a separate
                             read, and the five other tables are unaffected
                             either way.
";

/// Dispatch one command line and report the process exit code it earns.
///
/// `Ok(0)` for every subcommand that has nothing to conclude. `validate` is the
/// exception: it is an oracle, so it returns its own code and `Ok` no longer
/// means "clean". See [`oracle::Verdict`].
pub fn run(args: &[String]) -> Result<u8, CliError> {
    // args[0] = binary name
    if args.len() < 2 {
        return Err(CliError::Usage(USAGE.to_string()));
    }

    match args[1].as_str() {
        "inspect" => {
            let file = args
                .get(2)
                .ok_or_else(|| CliError::Usage("inspect requires <file.vrf>".to_string()))?;
            if args.len() != 3 {
                return Err(CliError::Usage(
                    "inspect accepts exactly one <file.vrf> argument".to_string(),
                ));
            }
            inspect::run(file).map(|()| 0)
        }
        "validate" => {
            let file = args
                .get(2)
                .ok_or_else(|| CliError::Usage("validate requires <file.vrf>".to_string()))?;
            let mut diagnostics = false;
            for arg in args.iter().skip(3) {
                match arg.as_str() {
                    "--diagnostics" if !diagnostics => diagnostics = true,
                    "--diagnostics" => {
                        return Err(CliError::Usage(
                            "duplicate option: --diagnostics".to_string(),
                        ));
                    }
                    other => {
                        return Err(CliError::Usage(format!(
                            "unknown validate option or surplus argument: {other}"
                        )));
                    }
                }
            }
            oracle::run(file, diagnostics).map(oracle::Verdict::exit_code)
        }
        "export" => export(args).map(|()| 0),
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(0)
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
            if out_dir.is_some() {
                return Err(CliError::Usage("duplicate option: --out".to_string()));
            }
            i += 1;
            out_dir =
                Some(args.get(i).map(String::as_str).ok_or_else(|| {
                    CliError::Usage("--out requires a directory path".to_string())
                })?);
        } else if args[i] == "--checkpoints" {
            if with_checkpoints {
                return Err(CliError::Usage(
                    "duplicate option: --checkpoints".to_string(),
                ));
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn inspect_rejects_surplus_arguments_before_opening_the_file() {
        let err = run(&owned(&["vrfkit", "inspect", "missing.vrf", "extra"]))
            .expect_err("inspect must not ignore a surplus positional argument");
        assert!(matches!(err, CliError::Usage(_)), "got {err:?}");
    }

    #[test]
    fn validate_rejects_unknown_options_before_opening_the_file() {
        let err = run(&owned(&["vrfkit", "validate", "missing.vrf", "--unknown"]))
            .expect_err("validate must not ignore an unknown option");
        assert!(matches!(err, CliError::Usage(_)), "got {err:?}");
    }

    #[test]
    fn validate_rejects_surplus_positional_arguments() {
        let err = run(&owned(&["vrfkit", "validate", "missing.vrf", "other.vrf"]))
            .expect_err("validate must not ignore another input path");
        assert!(matches!(err, CliError::Usage(_)), "got {err:?}");
    }

    #[cfg(feature = "export")]
    #[test]
    fn export_rejects_duplicate_out_before_opening_the_file() {
        let err = run(&owned(&[
            "vrfkit",
            "export",
            "missing.vrf",
            "--out",
            "first",
            "--out",
            "second",
        ]))
        .expect_err("export must reject a duplicate --out");
        assert!(matches!(err, CliError::Usage(_)), "got {err:?}");
    }

    #[cfg(feature = "export")]
    #[test]
    fn export_rejects_duplicate_checkpoints_before_opening_the_file() {
        let err = run(&owned(&[
            "vrfkit",
            "export",
            "missing.vrf",
            "--out",
            "out",
            "--checkpoints",
            "--checkpoints",
        ]))
        .expect_err("export must reject a duplicate --checkpoints");
        assert!(matches!(err, CliError::Usage(_)), "got {err:?}");
    }
}
