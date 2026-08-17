//! Cross-language pins for the constants `tools/to_valplay_bundle.py` shares
//! with this workspace.
//!
//! # Why this file exists
//!
//! The Python adapter is the only consumer of the export that has to agree
//! with the Rust side on *values*, not on symbols. Two of them decide how
//! every exported row is classified:
//!
//! * `UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME` -- the reserved
//!   `field_name` that marks a whole preserved ClassNetCache block. The
//!   adapter drops those rows before grouping; a value drift makes it keep
//!   them, and a preservation blob is then published as if it were a decoded
//!   field.
//! * `CLASS_NET_CACHE_SUFFIX` -- the group-path suffix that separates an RPC
//!   from a replicated property. A value drift reclassifies every RPC in the
//!   bundle as a replicated property, which produces a complete-looking
//!   document with no kills, no damage and no abilities in it.
//!
//! Neither failure raises anything. `crates/vrf-export/tests/roundtrip.rs`
//! uses the Rust constant by *symbol*, so changing what it points at leaves
//! the whole Rust suite green. This file reads the Python source and compares
//! the literals, so the drift fails here instead of downstream.
//!
//! # Why it parses rather than greps
//!
//! Both values also appear in the adapter's module docstring. A substring
//! search would be satisfied by the prose alone and would keep passing after
//! the code below it changed -- a check that cannot fail. So the assignment
//! statement is located by name and only the string literals inside that
//! statement are compared.

use std::fs;
use std::path::PathBuf;

/// The adapter source, read from the workspace this test was compiled in.
///
/// A missing file is a failure, never a skip: the adapter is the published
/// path from an export to valplay, and "the contract could not be checked"
/// must not read the same as "the contract holds".
fn adapter_source() -> String {
    let path = adapter_path();
    fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "cannot read the valplay adapter at {}: {err}",
            path.display()
        )
    })
}

fn adapter_path() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/vrfkit; the workspace root is two up. It is
    // fixed at compile time and differs per worktree, so this resolves inside
    // the checkout under test rather than against a working directory that
    // `cargo test` does not promise.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tools")
        .join("to_valplay_bundle.py")
}

/// Every string literal in the top-level `name = ...` assignment, joined.
///
/// Joined rather than taken singly because Python concatenates adjacent
/// literals, so a value split across lines inside parentheses is one value.
/// Returns `None` when no such assignment exists -- which is itself a failure
/// at the call site, not a pass.
fn python_constant(source: &str, name: &str) -> Option<String> {
    let start = source.lines().position(|line| {
        line.starts_with(name) && line[name.len()..].trim_start().starts_with('=')
    })?;

    // Take the statement: the first line, plus continuation lines while the
    // parentheses opened so far have not been closed.
    let mut statement = String::new();
    let mut depth: i32 = 0;
    for line in source.lines().skip(start) {
        statement.push_str(line);
        statement.push('\n');
        for c in line.chars() {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ => {}
            }
        }
        if depth <= 0 {
            break;
        }
    }

    // Concatenate the contents of every quoted run in the statement. The
    // adapter's constants carry no escapes, so a literal scanner that does not
    // model backslashes is sufficient -- and a value that grew one would stop
    // matching here, which is the safe direction to fail in.
    let mut value = String::new();
    let mut quote: Option<char> = None;
    for c in statement.chars() {
        match quote {
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                }
            }
            Some(open) => {
                if c == open {
                    quote = None;
                } else {
                    value.push(c);
                }
            }
        }
    }
    Some(value)
}

#[test]
fn adapter_pins_the_unresolved_class_net_cache_field_name() {
    let source = adapter_source();
    let python = python_constant(&source, "UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME")
        .expect("the adapter must assign UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME");
    assert_eq!(
        python,
        vrf_export::UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME,
        "the adapter's reserved field name no longer matches vrf-export's; \
         preserved ClassNetCache blocks would be published as decoded fields"
    );
}

#[test]
fn adapter_pins_the_class_net_cache_suffix() {
    let source = adapter_source();
    let python = python_constant(&source, "CLASS_NET_CACHE_SUFFIX")
        .expect("the adapter must assign CLASS_NET_CACHE_SUFFIX");
    assert_eq!(
        python,
        vrf_schema::CLASS_NET_CACHE_SUFFIX,
        "the adapter's RPC discriminator no longer matches vrf-schema's; \
         every RPC in the bundle would be classified as a replicated property"
    );
}

/// The parser itself has to be able to fail, and to see through prose.
///
/// Without this, a `python_constant` that always returned the Rust value --
/// or that matched the first quoted text anywhere in the file -- would keep
/// both tests above green forever.
#[test]
fn the_constant_scanner_reads_the_assignment_and_not_the_prose() {
    let source = concat!(
        "\"\"\"A docstring mentioning WIDGET = \"decoy\" in prose.\"\"\"\n",
        "\n",
        "OTHER = \"not this one\"\n",
        "WIDGET = (\n",
        "    \"real\"\n",
        "    \"_value\"\n",
        ")\n",
        "TRAILING = \"after\"\n",
    );
    assert_eq!(
        python_constant(source, "WIDGET").as_deref(),
        Some("real_value")
    );
    assert_eq!(
        python_constant(source, "OTHER").as_deref(),
        Some("not this one")
    );
    assert_eq!(
        python_constant(source, "ABSENT"),
        None,
        "a missing assignment must be reported, not silently treated as empty"
    );
}
