# vrfkit Audit Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the confirmed safety, correctness, observability, build and validation defects found by the whole-codebase audit.

**Architecture:** Preserve the existing crate pipeline while tightening every boundary: bounded inputs, chronological schema/packet processing, explicit loss results, complete aggregation and transactional publication. Python tools become containment-safe and fail closed. Rust and Python file ownership stays disjoint for parallel integration.

**Tech Stack:** Rust 1.86.0, Cargo workspace, Python 3.12/unittest, PyArrow/NumPy, PowerShell CI.

## Global Constraints

- Do not edit game replay files or generated Rust tables by hand.
- Do not change protocol behavior based only on an unconfirmed hypothesis.
- Write each regression test first and record its expected RED failure before production edits.
- Preserve `#![forbid(unsafe_code)]`, MSRV 1.86.0 and existing public output schemas unless a typed failure field is additive.
- Never convert unknown or malformed data into a plausible value; preserve raw data and surface a tally/error.
- The Rust agent owns `crates/`, root Cargo manifests and Rust examples. The Python agent owns `tools/`, `.github/`, README and `docs/` except this plan/spec.

---

### Task 1: Bound Rust public and wire inputs

**Files:**
- Modify: `crates/vrf-schema/src/reader.rs`, `export.rs`, `resolve.rs`
- Modify: `crates/vrf-bitio/src/lib.rs`
- Modify: `crates/vrf-decode/src/decode.rs`, `array.rs`, `cnc.rs`
- Modify: `crates/vrf-transform/src/lib.rs`, `crates/vrf-export/src/writer.rs`
- Test: the corresponding inline test modules

- [ ] Add failing tests for extreme live `num_fields`, oversized bit counts, truncated FString allocation preflight, undersized transform output and row-group size zero.
- [ ] Run each focused test and confirm the intended old behavior fails or panics.
- [ ] Add protocol-derived count limits/fallible reservation, explicit bit-length guards and typed errors; validate FString remaining bytes before allocation and row-group size before Parquet property construction.
- [ ] Re-run focused tests and all affected crate tests.

### Task 2: Preserve schema chronology and cache invariants

**Files:**
- Modify: `crates/vrf-frame/src/lib.rs`
- Modify: `crates/vrf-schema/src/cache.rs`, `checkpoint.rs`, `export.rs`
- Modify: production callers under `crates/vrfkit/src/driver/` and `oracle.rs`

- [ ] Add a failing integration test for `Export1, Packet1, Export2, Packet2` proving Packet1 sees only Export1.
- [ ] Add failing merge-matrix tests for same/new/crossed path-index pairs and checkpoint collisions.
- [ ] Change the frame API/caller flow to process packets with the cache state at their exact wire position.
- [ ] Reject or explicitly fail closed on crossed cache identities; refresh canonical path/index/leaf metadata for supported re-exports; make checkpoint collisions visible and unusable for decode.
- [ ] Re-run frame/schema/vrfkit tests.

### Task 3: Make every partial decode and loss observable and recoverable

**Files:**
- Modify: `crates/vrf-net/src/field.rs`, `pipeline/framing.rs`, channel/reassembly state
- Modify: `crates/vrf-decode/src/array.rs`, structs framing and overlay stats ordering
- Modify: `crates/vrf-movement/src/*.rs`
- Modify: `crates/vrfkit/src/sink/*.rs`, `driver/totals.rs`, `driver/summary.rs`, `manifest.rs`, `driver/checkpoints.rs`

- [ ] Add failing tests for abandoned field/RPC bits counting as failures, root array residual/OOB/exact-limit cases, malformed object arrays, short movement components and raw fallback after partial RPC/movement failure.
- [ ] Add a failing aggregation test that seeds every `OverlayStats` and `ArrayDecodeStats` field, including handle conflicts, truncations, residual bits and implicit terminations.
- [ ] Introduce explicit complete/partial/error results where needed; retain the whole raw parent/RPC payload whenever a child decoder is incomplete.
- [ ] Propagate all quality counters through replay and checkpoint totals, summaries and manifest; finalize checkpoint readers and include their network statistics.
- [ ] Bound channel/partial-reassembly lifetime and total bits with checked arithmetic.
- [ ] Re-run net/decode/movement/vrfkit tests.

### Task 4: Make validate and export honest and transactional

**Files:**
- Modify: `crates/vrfkit/src/oracle.rs`, `driver/mod.rs`, `driver/writers.rs`, CLI tests
- Modify: `crates/vrf-container/src/preamble.rs` and typed FString error adapters

- [ ] Add failing tests proving validation rejects ReplayData trailing bytes and unfinished partial bunches.
- [ ] Add fault-injection tests proving a failed export preserves a prior complete destination and joins/cancels writer threads.
- [ ] Finalize the replication reader before verdict, include every payload failure, and use the trailing-aware decompression API.
- [ ] Write to a unique sibling staging directory, finish all writers, then atomically publish; clean staging on every failure.
- [ ] Reject or surface recognized chunks before Header and preserve typed string errors.
- [ ] Make inspect/validate reject unknown/surplus arguments and make summary Unicode truncation boundary-safe.
- [ ] Re-run CLI/container/vrfkit tests and one real replay.

### Task 5: Repair Rust build, documentation and package contracts

**Files:**
- Modify: `crates/vrf-decode/src/lib.rs` and feature layout
- Modify: `tools/probe_offset/Cargo.toml` or root `Cargo.toml`
- Modify: Rust doc comments across affected crates
- Modify: crate Cargo manifests/package inclusions and the broken container example

- [ ] Add the documented no-default/single-feature commands to a local matrix and capture their current failures.
- [ ] Decouple generated overlay tables from core-only builds, make the standalone tool a valid workspace/excluded workspace, and fix the example to use library parsing and library limits.
- [ ] Repair all strict-rustdoc links/HTML and keep public error variants additive across features.
- [ ] Ensure publishable packages include LICENSE/NOTICE, or explicitly mark internal crates non-publishable.
- [ ] Run the full feature matrix, strict rustdoc and package listing.

### Task 6: Make Python tools fail closed

**Files:**
- Modify: `tools/validate_metrics_corpus.py`, `check_corpus_baseline.py`, `check_decode_errors_corpus.py`
- Modify: `tools/extract_descriptors.py`, `extract_golden.py`, `extract_checksum_types.py`, `apply_type_corrections.py`
- Modify: `tools/to_valplay_bundle.py`, `compare_with_csharp.py`, `find_skips.py`
- Test: corresponding files under `tools/tests/`

- [ ] Add failing containment tests for absolute/parent `--only` paths and input/output aliases.
- [ ] Add failing tests for empty descriptor sources, non-55 golden vectors, duplicate corpus basenames, missing counters, ambiguous checksum donors, duplicate movement samples and unknown correction flags/non-atomic failure.
- [ ] Validate identifiers and resolved descendants before deletion; use temporary output plus replace for all mutating tools.
- [ ] Key corpus entries by relative path, require every parsed counter, enforce generator source/cardinality invariants, fail closed on ambiguous checksum donors, and compare movement as consumable multisets.
- [ ] Correct `find_skips` parsing and controlled error handling/timeouts.
- [ ] Run all Python tests with warnings as errors.

### Task 7: Strengthen baselines, CI and documentation

**Files:**
- Modify: `tools/check_export_baseline.py`, `tools/check_docs.py`, committed baseline JSON as measured
- Modify: `.github/workflows/ci.yml`, `README.md`, `docs/USAGE.md`, `CONTRIBUTING.md`, PR template

- [ ] Add failing tests showing equal-size different Parquet bytes do not satisfy byte identity and committed baseline schemas are validated.
- [ ] Store/compare SHA-256 content hashes and require explicit non-skip corpus status in CI-facing modes.
- [ ] Add the feature matrix, strict rustdoc, generated checksum guard and automated Python interop to CI.
- [ ] Refresh measured export/checkpoint/correction counts and generated-file inventories from live baselines; align the PR checklist and quick start with executable commands.
- [ ] Run docs, generator, baseline-schema and full test guards.

### Task 8: Integrated verification

- [ ] Run `cargo +1.86.0 fmt --check`.
- [ ] Run `cargo +1.86.0 clippy --workspace --all-targets --all-features --locked -- -D warnings`.
- [ ] Run `cargo +1.86.0 test --workspace --locked` and every advertised no-default/single-feature check.
- [ ] Run strict workspace rustdoc with `RUSTDOCFLAGS=-D warnings`.
- [ ] Run `python -W error -m unittest discover -s tools/tests -v` and all documented guards.
- [ ] Run validate/export/adapter on a real replay and corpus checks on all configured local corpora.
- [ ] Confirm `git diff --check` and summarize any deliberately deferred protocol hypotheses separately.

