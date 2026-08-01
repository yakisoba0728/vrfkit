//! Integration test: parse all .vrf files in the corpus directory.
//!
//! The corpus lives outside the repo and is machine-local, so this test
//! returns early when it is absent. That early return used to be invisible:
//! the test reported as PASSING on any machine without the corpus, which
//! meant its whole body was an untaken branch everywhere but one
//! workstation, and `cargo test`'s green count silently included it.
//!
//! Two things make it honest now. The path is read from `VRFKIT_CORPUS`
//! rather than hardcoded to one user's home directory, and setting
//! `VRFKIT_REQUIRE_CORPUS=1` turns the skip into a failure -- so a machine
//! that is SUPPOSED to have the corpus can say so and be held to it. The
//! skip message names both, so anyone reading the output knows the coverage
//! was not taken and how to take it.

use std::path::{Path, PathBuf};

use vrf_container::{ChunkIterator, ChunkType, decompress_replay_data, parse_preamble};

/// Fallback used when `VRFKIT_CORPUS` is unset. Convenience for the machine
/// this project is developed on, not a requirement.
const DEFAULT_VRF_DIR: &str = r"C:\Users\yakihyuk0728\Documents\GitHub\valplay\data\raw\vrf";

fn corpus_dir() -> PathBuf {
    std::env::var_os("VRFKIT_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_VRF_DIR))
}

#[test]
fn parse_all_vrf_files() {
    let corpus = corpus_dir();
    if !corpus.is_dir() {
        let message = format!(
            "corpus directory not found at {}; set VRFKIT_CORPUS to point at one",
            corpus.display()
        );
        assert!(
            std::env::var_os("VRFKIT_REQUIRE_CORPUS").is_none(),
            "VRFKIT_REQUIRE_CORPUS is set but {message}"
        );
        eprintln!("SKIP (body not executed): {message}");
        return;
    }

    let dir: &Path = &corpus;
    let mut total = 0u32;
    let mut info_ok = 0u32;
    let mut branches: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut oodle_ok = 0u32;
    let mut oodle_fail = 0u32;
    let mut failures: Vec<(String, String)> = Vec::new();

    for entry in std::fs::read_dir(dir).expect("read corpus dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("vrf") {
            continue;
        }
        total += 1;
        let filename = path.file_name().unwrap().to_string_lossy().to_string();

        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                failures.push((filename, format!("read error: {e}")));
                continue;
            }
        };

        // (a) Parse preamble (info + header)
        let preamble = match parse_preamble(&data) {
            Ok(p) => p,
            Err(e) => {
                failures.push((filename, format!("preamble: {e}")));
                continue;
            }
        };
        info_ok += 1;

        // (b) Branch string
        let branch = &preamble.header.replay_version.branch;
        *branches.entry(branch.clone()).or_insert(0) += 1;

        // (c) Oodle decompression -- try first ReplayData chunk
        // A file with no ReplayData chunk at all is unusual but not a failure:
        // the loop just ends without recording an Oodle result either way.
        let mut iter = ChunkIterator::new(&data, preamble.remaining_offset);
        while let Ok(Some(chunk)) = iter.next_chunk() {
            if chunk.chunk_type == ChunkType::ReplayData {
                let payload =
                    &data[chunk.data_offset..chunk.data_offset + chunk.size_in_bytes as usize];
                match decompress_replay_data(
                    payload,
                    preamble.info.compressed,
                    preamble.info.encrypted,
                ) {
                    Ok(_) => {
                        oodle_ok += 1;
                    }
                    Err(e) => {
                        oodle_fail += 1;
                        failures.push((filename.clone(), format!("oodle: {e}")));
                    }
                }
                break;
            }
        }
    }

    // Print summary
    eprintln!("=== VRF Corpus Test Results ===");
    eprintln!("Total .vrf files: {total}");
    eprintln!("Info+Header parse OK: {info_ok}/{total}");
    eprintln!("Branch distribution:");
    let mut branch_list: Vec<_> = branches.iter().collect();
    branch_list.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    for (branch, count) in &branch_list {
        eprintln!("  {branch}: {count}");
    }
    eprintln!("Oodle decompress OK: {oodle_ok}");
    eprintln!("Oodle decompress FAIL: {oodle_fail}");
    if !failures.is_empty() {
        eprintln!("Failures ({}):", failures.len());
        for (file, err) in &failures {
            eprintln!("  {file}: {err}");
        }
    }
    eprintln!("===============================");

    // The test passes if all preambles parsed successfully
    assert_eq!(
        info_ok, total,
        "Not all files parsed successfully: {info_ok}/{total}"
    );
}
