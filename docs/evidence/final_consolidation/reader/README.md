# Reader, gameplay and CLI audit

Baseline: `8581642283bd5a571967129d73897db8ee5408ba`.
Role D preserved the baseline plus regression tests in `final/reader-cli` and
implemented the fixes in `final/reader-fixes`. The coordinator owns integration,
public-replay comparison, final hosted CI and publication state.

## Scope inspected

| Area | Inspected contracts |
| --- | --- |
| `reader.rs`, `reader_input.rs`, `pass.rs` | Borrowed callback lifetimes, original consumer-error priority, bounded nonseekable input and EOF probe, independent file cursors, prefetch ordering, main packet state and EOF partial flush |
| `checkpoints.rs`, `driver/checkpoints.rs` | Fresh checkpoint schema/replication state, separate counters, declaration callbacks, checkpoint identity and row offsets, callback aborts |
| `gameplay.rs`, `gameplay/identity.rs` and their tests | Raw/typed validation, explicit null/unresolved/ambiguous states, strictly ordered packet observations, historical lifetime queries, dormancy, replacement, provenance-aligned movement and concurrent changes |
| `cli.rs`, `driver/mod.rs`, `driver/publish.rs`, `driver/writers.rs` | Argument and feature refusal, source protection, staged directory publication and rollback, foreign-entry protection, bounded writer queues, writer error/panic joins |
| `sink/mod.rs`, `sink/stream.rs`, `sink/rpc.rs`, `sink/paths.rs` | Packet buffer reuse, raw fallback, source stamps, unresolved RPC preservation, exact cache generations and identity scope |
| `diagnose.rs`, existing CLI integration tests | Source/output alias checks, JSON publication, partial failure, platform behavior and preserved prior output |

This is a targeted source and contract audit, not a claim that all wire
semantics are understood. No speculative gameplay or RPC types were added.
Reader API and parsing/output schemas remain unchanged.

## Findings

### D1: export can replace its own input (P1)

The CLI parses input by content, so a replay can be named `fields.parquet`,
`manifest.json`, or any other recognized export filename. If it is inside
`--out`, the existing foreign-entry check accepts it. Successful directory
publication then replaces and deletes that input as prior output.

The regression uses generated, minimal replay bytes; original replay assets
are not modified. Production admission must reject a canonical input inside
the canonical output directory after the existing foreign-entry check and
before creating staging. This preserves the established foreign-file error
priority. Canonicalization covers input/output symlink aliases; a safe input
outside the output remains permitted even when an existing output entry is
a hard link to it. Path components, not textual prefixes, determine containment.

### D2: Windows diagnostic publication removes prior output too early (P2)

Baseline `write_json_file` explicitly removed the old `--json` destination on
Windows before its final rename. If that rename failed, the previous report
was lost.
The Rust 1.86 Windows implementation already uses `MOVEFILE_REPLACE_EXISTING`:
[pinned standard-library source](https://github.com/rust-lang/rust/blob/1.86.0/library/std/src/sys/pal/windows/fs.rs).
Publication should call rename directly and retain the old destination if
the staged source cannot be renamed. A missing-stage fault injection is the
negative control; local Linux execution of the old remove-then-rename
sequence is not a native Windows baseline run.

## Verification

The Linux regression at test-only commit
`6442eeed6a61ad3fcb11582e1726302eefc34e20` failed against unchanged baseline
runtime: the CLI returned success and replaced the generated replay with a
1,894-byte Parquet file. The [negative receipt](baseline_export_input_failure.json)
and [captured failure](baseline_export_input_failure.log) retain exact hashes
and exit codes. One generated replay shape was executed twice; the second run
captured the complete test log without rebuilding. These repetitions add no
unique replay coverage.

The candidate code is commit `8dd465c0f773c45676c2a039ba9301c0435d8bb0` on top of
the test-only commit. Its full `vrfkit` package run passed:

```sh
CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR="$WORKSPACE/target-reader" \
  cargo +1.86.0 test -p vrfkit --all-features --locked
cargo +1.86.0 fmt --check
git diff --check
```

The package run passed **259 tests: 171 library tests, 87 integration tests,
and 1 documentation test**, with zero failures and zero ignored tests. The
binary unit-test target contains zero tests and is not counted as coverage.
See [the command receipt](candidate_package_tests.json) and
[complete package log](candidate_package_tests.log). The log records the
parsing crates and `vrfkit` compiling at the candidate worktree; the `vrfkit`
dep-info also records that worktree's `CARGO_MANIFEST_DIR`. Unchanged dependency
artifacts may be reused. The only source differences from the baseline are the
four files named in the receipt; other workspace crates and dependency versions
are unchanged in this role's candidate. The diagnostic control additionally
cleans the `vrfkit` package before its rebuild sequence.

| Executed integration target | Passed | Contracts exercised |
| --- | ---: | --- |
| `adapter_contract` | 11 | Rust/Python constant agreement, event payload shapes and mutation-sensitive contract checks |
| `checkpoint_partials`, `checkpoint_unread` | 3 | Checkpoint partial rows and explicit counting of bytes the codec did not read |
| `cli_input` | 6 | Source containment, permitted external hardlink, sparse input, main/checkpoint output equality and failure rollback |
| `export_destination` | 3 | Foreign files, ordinary replacement and complete published output |
| `frame_skips` | 3 | Skipped-frame diagnostics and export/validate agreement |
| `gameplay`, `gameplay_lifecycle` | 15 | Borrowed raw provenance, strict overlays, lifetime retirement and same-packet ambiguity |
| `movement_reader` | 3 | Movement callbacks and source provenance |
| `reader_api` | 15 | Raw row order, original callback errors, schema limit causes and independent reads |
| `reader_bounded` | 11 | Exact-limit EOF probe, excess-byte refusal, interruptions, I/O errors and callback/schema priority |
| `reader_checkpoints` | 11 | Fresh checkpoint state, separate partials, callback order and abort/late-failure behavior |
| `reader_input` | 6 | File/slice parity, truncated/skipped chunk bounds, sparse input and original error categories |

Existing library tests additionally exercise directory publication fault
injection and rollback, concurrent foreign-entry appearance, leftover handling,
writer failure propagation, identity history and raw/typed row validation.
The new report-publication unit test removes the staged file at the helper
boundary and verifies that the prior report remains byte-identical.

### Audit conclusions and limits

The bounded nonseekable API buffers encoded input before parsing. Its limit
covers retained encoded bytes and requested vector capacity; it does not claim
a bound on parser state, decompression, allocator overhead or consumer storage.
The file API seeks across irrelevant payloads and owns independent cursors;
tests compare its callbacks and failure order with slice input. Callbacks
remain provisional until the read succeeds, and an original consumer error
outranks later errors encountered while unwinding the current pass.

Main and checkpoint passes have independent schema/replication state. Identity
queries use packet boundaries, preserve null/unresolved/ambiguous outcomes,
retire completed lifetimes, and avoid backfilling earlier observations. Movement
projection requires aligned source provenance and keeps changes during a
fragment interval unresolved. No new defect in these contracts was established
by this scoped audit and package execution; this is not a claim that all
possible replay wire data or external consumer behavior has been verified.

The containment check is performed before staging and resolves existing path
aliases. It is deliberately conservative for an output-directory symlink whose
target contains the input. It does not claim to lock paths against external
filesystem mutation during an export. A hard link within output to an input
outside output is safe to replace and remains permitted.

### Diagnostic negative control and restoration

After cleaning the `vrfkit` package artifacts, the
[saved mutation](diagnostic_remove_before_rename.patch) inserted the baseline
Windows remove-before-rename sequence into the publication helper on Linux.
The compiled new regression failed with exit **101**, because the previous
report was absent (`left: None`). This detects the loss at the actual helper
boundary rather than mirroring the rename implementation.

The source was then restored byte-for-byte and recompiled. The same regression
passed with exit **0**. Its restored unit-test executable has exactly the same
SHA-256 as before mutation. The normal CLI was rebuilt separately in the same
`test` profile used by the original package run and also has exactly the same
SHA-256 as before mutation. `cargo fmt --check`, `git diff --check`, and the
supported normal-CLI `--help` smoke command all passed. The source has no
remaining tracked diff. Full commands, timestamps, exit codes, source and
executable hashes, and log digests are in
[the mutation/restoration receipt](diagnostic_mutation_restoration.json).
Logs normalize workspace paths, trailing whitespace and final blank lines;
receipts retain hashes before and after whitespace normalization. The saved
patch removes a trailing empty context line for a clean diff; the receipt also
retains the exact executed patch text and hash. Its inserted source is unchanged.

An auxiliary default `dev` CLI build also passed, but its different profile is
not used for the byte-for-byte comparison. An author smoke command mistakenly
used unsupported `--version` and returned exit 1; its log is retained in the
receipt and the supported `--help` command was then checked successfully. This
was a harness invocation error, not a skipped product acceptance gate.

Native Windows pre-fix execution is **NOT_EXECUTED** by this author. The local
negative control reproduces the old Windows sequence on Linux; it must not be
presented as a Windows execution.
Whole-workspace acceptance, actual Windows runtime/package execution and
public/private corpus coverage are coordinator-owned gates. This role used
zero public or private replay files; generated test inputs are not real-corpus
coverage. Independent Role E's real-replay reproduction and final integrated
CLI checks are separately attributable evidence.
