//! What the binary says when asked for help or its version, when a path
//! cannot be used, and when a report section has nothing in it.

mod common;

use std::fs;
use std::process::Command;

use common::*;

fn replay() -> Vec<u8> {
    let frames = frame(1.0, &[], 0, &[]);
    let mut data = replay_info(&Info::default());
    data.extend(chunk(0, &header_payload()));
    data.extend(chunk(1, &replay_data(&frames, frames.len())));
    data
}

fn subcommands() -> Vec<&'static str> {
    let mut all = vec!["inspect", "validate", "diag"];
    if cfg!(feature = "export") {
        all.push("export");
    }
    all
}

#[test]
fn help_and_version_exit_zero_on_stdout() {
    let mut asks: Vec<Vec<&str>> = vec![vec!["--help"], vec!["-h"]];
    for sub in subcommands() {
        asks.push(vec![sub, "--help"]);
        asks.push(vec![sub, "-h"]);
        asks.push(vec![sub, "missing.vrf", "--help"]);
    }
    for args in asks {
        let run = vrfkit(&args);
        assert_eq!(run.code, Some(0), "{args:?}: {}", run.stderr);
        assert!(run.stdout.contains("USAGE:"), "{args:?}: {}", run.stdout);
        assert!(run.stderr.is_empty(), "{args:?}: {}", run.stderr);
    }
    let version = format!("vrfkit {}", env!("CARGO_PKG_VERSION"));
    let mut asks: Vec<Vec<&str>> = vec![vec!["--version"], vec!["-V"]];
    for sub in subcommands() {
        for flag in ["--version", "-V"] {
            asks.push(vec![sub, flag]);
            asks.push(vec![sub, "missing.vrf", flag]);
        }
    }
    for args in asks {
        let run = vrfkit(&args);
        assert_eq!(run.code, Some(0), "{args:?}: {}", run.stderr);
        assert_eq!(run.stdout.trim_end(), version, "{args:?}");
        assert!(run.stderr.is_empty(), "{args:?}: {}", run.stderr);
    }
}

/// A valued option's value is a value even when it reads `-h` or `-V`.
#[test]
fn a_valued_options_value_is_not_a_help_or_version_request() {
    for flag in ["-h", "--help", "-V", "--version"] {
        let run = vrfkit(&["diag", "missing.vrf", "--json", flag]);
        assert_eq!(run.code, Some(1), "{flag}: {}", run.stdout);
        assert!(run.stderr.contains("missing.vrf"), "{flag}: {}", run.stderr);
    }
}

#[test]
fn input_errors_name_the_path() {
    let dir = scratch("cli-messages", "input");
    let missing = dir.join("missing.vrf");
    let out = dir.join("out");
    for sub in subcommands() {
        let mut args = vec![sub, path_arg(&dir)];
        if sub == "export" {
            args.extend(["--out", path_arg(&out)]);
        }
        let run = vrfkit(&args);
        assert_eq!(run.code, Some(1), "{args:?}");
        let expected = format!("error: I/O error: {}: is a directory", dir.display());
        assert!(run.stderr.contains(&expected), "{args:?}: {}", run.stderr);

        args[1] = path_arg(&missing);
        let run = vrfkit(&args);
        assert_eq!(run.code, Some(1), "{args:?}");
        let expected = format!("error: I/O error: {}: ", missing.display());
        assert!(run.stderr.contains(&expected), "{args:?}: {}", run.stderr);
    }
    fs::remove_dir_all(dir).unwrap();
}

/// An output under an existing file cannot be created; the message says where.
#[test]
fn output_errors_name_the_path() {
    let dir = scratch("cli-messages", "output");
    let input = dir.join("replay.vrf");
    fs::write(&input, replay()).unwrap();
    let blocker = dir.join("not-a-directory");
    fs::write(&blocker, b"").unwrap();

    let json = blocker.join("report.json");
    let run = vrfkit(&["diag", path_arg(&input), "--json", path_arg(&json)]);
    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert!(
        run.stderr
            .contains(&format!("I/O error: {}", blocker.display())),
        "{}",
        run.stderr
    );

    if cfg!(feature = "export") {
        let out = blocker.join("out");
        let run = vrfkit(&["export", path_arg(&input), "--out", path_arg(&out)]);
        assert_eq!(run.code, Some(1), "{}", run.stderr);
        // Which step fails differs by platform (listing the destination on
        // Linux, creating the staging directory on Windows), so the wording
        // before the path differs; the path under the blocker is in both.
        let error = run
            .stderr
            .lines()
            .find(|line| line.starts_with("error: I/O error: "))
            .unwrap_or_else(|| panic!("no I/O error line: {}", run.stderr));
        assert!(
            error.contains(&blocker.display().to_string()),
            "{}",
            run.stderr
        );
    }
    fs::remove_dir_all(dir).unwrap();
}

/// `.` and `..` are refused with the rule, not as a "filesystem root".
#[cfg(feature = "export")]
#[test]
fn an_export_destination_without_a_name_states_the_rule() {
    let dir = scratch("cli-messages", "dot");
    let input = dir.join("replay.vrf");
    fs::write(&input, replay()).unwrap();
    let parent = dir.join("..");
    for (cwd, out) in [(&dir, "."), (&dir, path_arg(&parent))] {
        let output = Command::new(env!("CARGO_BIN_EXE_vrfkit"))
            .args(["export", path_arg(&input), "--out", out])
            .current_dir(cwd)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{out}: {stderr}");
        assert!(
            stderr.contains("does not end in a directory name"),
            "{out}: {stderr}"
        );
        assert!(!stderr.contains("not a filesystem root"), "{out}: {stderr}");
    }
    fs::remove_dir_all(dir).unwrap();
}

/// Both report sections print their header at zero, with and without the flag.
#[test]
fn validate_prints_empty_sections_with_a_zero_count() {
    let dir = scratch("cli-messages", "zero");
    let input = dir.join("replay.vrf");
    fs::write(&input, replay()).unwrap();
    for extra in [None, Some("--diagnostics")] {
        let mut args = vec!["validate", path_arg(&input)];
        args.extend(extra);
        let run = vrfkit(&args);
        assert_eq!(run.code, Some(2), "{args:?}: {}", run.stderr);
        for header in [
            "=== Stream failures (0 shown) ===",
            "=== Diagnostic Events (0 total) ===",
        ] {
            assert!(run.stdout.contains(header), "{args:?}: {}", run.stdout);
        }
        assert!(!run.stdout.contains("use --diagnostics"), "{}", run.stdout);
    }
    fs::remove_dir_all(dir).unwrap();
}
