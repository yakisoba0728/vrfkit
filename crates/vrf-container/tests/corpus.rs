//! Integration test: parse all .vrf files in the corpus directory.
//!
//! The corpus lives outside the repo and is machine-local, so this test
//! returns early when it is absent. That early return used to be invisible:
//! the test reported as PASSING on any machine without the corpus, which
//! meant its whole body was an untaken branch everywhere but one
//! workstation, and `cargo test`'s green count silently included it.
//!
//! Two things make it honest now. The path is read from `VRFKIT_CORPUS_DIR`
//! rather than hardcoded to one user's home directory, and setting
//! `VRFKIT_REQUIRE_CORPUS=1` turns the skip into a failure -- so a machine
//! that is SUPPOSED to have the corpus can say so and be held to it. The
//! skip message names both, so anyone reading the output knows the coverage
//! was not taken and how to take it.
//!
//! # What counts as a failure
//!
//! The test used to assert only that every preamble parsed, which left three
//! ways to pass over broken data: an Oodle failure was tallied and then
//! ignored, a malformed chunk header ended the `while let Ok(..)` walk exactly
//! as a clean end-of-stream would, and an existing but EMPTY corpus directory
//! satisfied `0 == 0`. All three are now assertions, and all three are about
//! signals this test was already computing over the corpus.
//!
//! [`FileReport::notes`] is the other half: measurements printed but not
//! asserted, because nothing has yet measured them across the corpus and a
//! test that fails on an unmeasured signal is guessing, not checking.

use std::path::{Path, PathBuf};

use vrf_container::{
    ChunkIterator, ChunkType, decompress_replay_data_with_trailing, parse_preamble,
};

/// Fallback used when `VRFKIT_CORPUS_DIR` is unset. Empty so that on a machine
/// without the corpus `is_dir()` is false and the test skips honestly.
const DEFAULT_VRF_DIR: &str = "";

fn corpus_dir() -> PathBuf {
    std::env::var_os("VRFKIT_CORPUS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_VRF_DIR))
}

/// What one file contributed to the tally.
struct FileReport {
    /// Branch string from the header, when the preamble parsed.
    branch: Option<String>,
    /// A ReplayData chunk was found and decompressed.
    oodle_ok: bool,
    /// Every problem found in this file. Empty means clean, and the test
    /// asserts on it.
    problems: Vec<String>,
    /// Measurements that are NOT assertions.
    ///
    /// The two residual counts below are new and have never been measured over
    /// the corpus, so failing on them would be this test guessing. They are
    /// printed instead: the first corpus run says whether they are ever
    /// non-zero, and that evidence is what would justify promoting them.
    notes: Vec<String>,
}

/// Parse one replay as far as the container layer goes, collecting problems
/// rather than stopping at the first.
fn scan_file(data: &[u8]) -> FileReport {
    let mut problems = Vec::new();
    let mut notes = Vec::new();

    let preamble = match parse_preamble(data) {
        Ok(p) => p,
        Err(e) => {
            problems.push(format!("preamble: {e}"));
            return FileReport {
                branch: None,
                oodle_ok: false,
                problems,
                notes,
            };
        }
    };

    let branch = Some(preamble.header.replay_version.branch.clone());
    if preamble.header.trailing_bytes != 0 {
        notes.push(format!(
            "header: {} bytes past the parsed layout",
            preamble.header.trailing_bytes
        ));
    }

    let mut oodle_ok = false;
    let mut iter = ChunkIterator::new(data, preamble.remaining_offset);
    loop {
        // A chunk-header error means a malformed file, NOT the end of the
        // stream. `while let Ok(Some(chunk))` could not tell those apart, so a
        // truncated header or a negative size ended the walk exactly like a
        // clean end-of-buffer and the file passed.
        let chunk = match iter.next_chunk() {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(e) => {
                problems.push(format!("chunk: {e}"));
                break;
            }
        };

        if chunk.chunk_type != ChunkType::ReplayData {
            continue;
        }

        // First ReplayData chunk only: this test is a container smoke test, and
        // the whole-stream pass belongs to the driver.
        let payload = &data[chunk.data_offset..chunk.data_offset + chunk.size_in_bytes as usize];
        match decompress_replay_data_with_trailing(
            payload,
            preamble.info.compressed,
            preamble.info.encrypted,
        ) {
            Ok((_, trailing)) => {
                oodle_ok = true;
                if trailing != 0 {
                    notes.push(format!(
                        "replay data: {trailing} bytes past the declared archive"
                    ));
                }
            }
            Err(e) => problems.push(format!("oodle: {e}")),
        }
        break;
    }

    FileReport {
        branch,
        oodle_ok,
        problems,
        notes,
    }
}

#[test]
fn parse_all_vrf_files() {
    let corpus = corpus_dir();
    if !corpus.is_dir() {
        let message = format!(
            "corpus directory not found at {}; set VRFKIT_CORPUS_DIR to point at one",
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
    let mut clean = 0u32;
    let mut branches: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut oodle_ok = 0u32;
    let mut failures: Vec<(String, String)> = Vec::new();
    let mut notes: Vec<(String, String)> = Vec::new();

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

        let report = scan_file(&data);
        if let Some(branch) = report.branch {
            *branches.entry(branch).or_insert(0) += 1;
        }
        if report.oodle_ok {
            oodle_ok += 1;
        }
        if report.problems.is_empty() {
            clean += 1;
        }
        for problem in report.problems {
            failures.push((filename.clone(), problem));
        }
        for note in report.notes {
            notes.push((filename.clone(), note));
        }
    }

    // Print summary
    eprintln!("=== VRF Corpus Test Results ===");
    eprintln!("Total .vrf files: {total}");
    eprintln!("Clean: {clean}/{total}");
    eprintln!("Branch distribution:");
    let mut branch_list: Vec<_> = branches.iter().collect();
    branch_list.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    for (branch, count) in &branch_list {
        eprintln!("  {branch}: {count}");
    }
    eprintln!("Oodle decompress OK: {oodle_ok}");
    // Measured, not asserted. Both counts are new; if either is ever non-zero
    // that is the evidence needed to decide whether it should be a failure.
    eprintln!("Unaccounted trailing bytes: {} file(s)", notes.len());
    for (file, note) in &notes {
        eprintln!("  {file}: {note}");
    }
    if !failures.is_empty() {
        eprintln!("Failures ({}):", failures.len());
        for (file, err) in &failures {
            eprintln!("  {file}: {err}");
        }
    }
    eprintln!("===============================");

    // An existing but empty directory used to satisfy `0 == 0` and report a
    // pass over nothing at all.
    assert!(
        total > 0,
        "corpus directory {} contains no .vrf files; an empty corpus is not a pass",
        dir.display()
    );
    assert!(
        failures.is_empty(),
        "{} problem(s) across {total} files: {failures:#?}",
        failures.len()
    );
}

/// A minimal but structurally valid replay, built from the same field order
/// `info.rs` and `header.rs` read. Only enough to reach the chunk walk.
mod fixture {
    fn add_u16(buf: &mut Vec<u8>, v: u16) {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    fn add_u32(buf: &mut Vec<u8>, v: u32) {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    fn add_i32(buf: &mut Vec<u8>, v: i32) {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    fn add_i64(buf: &mut Vec<u8>, v: i64) {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    fn add_f32(buf: &mut Vec<u8>, v: f32) {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    fn add_fstring(buf: &mut Vec<u8>, s: &str) {
        add_i32(buf, (s.len() + 1) as i32);
        buf.extend_from_slice(s.as_bytes());
        buf.push(0);
    }

    fn replay_info() -> Vec<u8> {
        let mut buf = Vec::new();
        add_u32(&mut buf, 0x43F4_EFDD); // file magic
        add_u32(&mut buf, 7); // file version
        add_i32(&mut buf, 1); // one custom version
        for word in [0x95A4_F03E_u32, 0x7E0B_49E4, 0xBA43_D356, 0x94FF_87D9] {
            add_u32(&mut buf, word);
        }
        add_i32(&mut buf, 7); // LocalFileReplay version
        add_i32(&mut buf, 60_000); // length in ms
        add_u32(&mut buf, 19); // network version
        add_u32(&mut buf, 1234); // changelist
        add_fstring(&mut buf, "Match");
        add_u32(&mut buf, 0); // is live
        add_i64(&mut buf, 42); // timestamp
        add_u32(&mut buf, 0); // compressed
        add_u32(&mut buf, 0); // encrypted
        add_i32(&mut buf, 0); // encryption key length
        buf
    }

    fn header_payload() -> Vec<u8> {
        let mut buf = Vec::new();
        add_u32(&mut buf, 0x2CF5_A13D); // network magic
        add_u32(&mut buf, 19); // network version
        add_i32(&mut buf, 0); // custom version count
        add_u32(&mut buf, 0x1122_3344); // network checksum
        add_u32(&mut buf, 32); // engine net proto version
        add_u32(&mut buf, 0x5566_7788); // game net proto version
        for word in [0x0011_2233_u32, 0x4455_6677, 0x8899_AABB, 0xCCDD_EEFF] {
            add_u32(&mut buf, word);
        }
        add_u16(&mut buf, 12); // major
        add_u16(&mut buf, 10); // minor
        add_u16(&mut buf, 1); // patch
        add_u32(&mut buf, 123_456); // changelist
        add_fstring(&mut buf, "++Ares-Core+release-12.10");
        buf.extend_from_slice(&[3, 0, 0, 0, 49, 56, 0]); // valorant skip: 3 bytes
        add_u32(&mut buf, 1001); // UE4 version
        add_u32(&mut buf, 1002); // UE5 version
        add_u32(&mut buf, 1003); // package version license
        add_i32(&mut buf, 1); // one level name
        add_fstring(&mut buf, "Ascent");
        add_u32(&mut buf, 42); // level time
        add_u32(&mut buf, 0b1010); // flags
        add_i32(&mut buf, 0); // game-specific data count
        add_f32(&mut buf, 15.0);
        add_f32(&mut buf, 30.0);
        add_f32(&mut buf, 33.3);
        add_f32(&mut buf, 250.0);
        add_fstring(&mut buf, "Windows");
        buf.push(7); // build config
        buf.push(3); // build target type
        buf
    }

    /// Replay info followed by a single Header chunk, and nothing else.
    pub fn minimal_replay() -> Vec<u8> {
        let mut data = replay_info();
        let payload = header_payload();
        add_u32(&mut data, 0); // chunk type: Header
        add_i32(&mut data, payload.len() as i32);
        data.extend_from_slice(&payload);
        data
    }
}

/// The fixture itself must be clean, or the malformed case below proves
/// nothing.
#[test]
fn the_fixture_replay_scans_without_problems() {
    let report = scan_file(&fixture::minimal_replay());
    assert!(
        report.problems.is_empty(),
        "fixture should be clean, got {:?}",
        report.problems
    );
    assert_eq!(report.branch.as_deref(), Some("++Ares-Core+release-12.10"));
}

/// Four stray bytes after the last chunk are too few for a chunk header. That
/// is a malformed file, and the walk used to treat it as the normal end of the
/// stream: `while let Ok(Some(chunk))` exits on `Err` and on `Ok(None)`
/// alike, so the error was discarded and the file counted as a pass.
#[test]
fn a_malformed_chunk_header_is_reported_not_read_as_a_clean_end() {
    let mut data = fixture::minimal_replay();
    data.extend_from_slice(&[0u8; 4]);

    let report = scan_file(&data);
    assert!(
        report.problems.iter().any(|p| p.starts_with("chunk:")),
        "a truncated chunk header must be reported, got {:?}",
        report.problems
    );
}

/// A negative chunk size is the other shape of the same swallow: the iterator
/// rejects it, and that rejection must reach the tally.
#[test]
fn a_negative_chunk_size_is_reported_not_read_as_a_clean_end() {
    let mut data = fixture::minimal_replay();
    data.extend_from_slice(&1u32.to_le_bytes()); // chunk type: ReplayData
    data.extend_from_slice(&(-1i32).to_le_bytes()); // negative size

    let report = scan_file(&data);
    assert!(
        report.problems.iter().any(|p| p.starts_with("chunk:")),
        "a negative chunk size must be reported, got {:?}",
        report.problems
    );
}
