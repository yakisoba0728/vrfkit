# Gameplay projection evidence

`receipt.json` records a fresh execution of the optional Rust gameplay layer
on six SHA-256-checked public replays: 12.05, 12.08, 12.09, 12.10, 12.11 and
13.00. It preserves the local source commit/tree, runtime source hashes,
explicitly built probe hash and per-input aggregate results. Its source is the
feature checkpoint before this evidence-only commit; the functional files
have not changed since that checkpoint.

The explicit example build exited zero before its executable was frozen.
Its dependency file listed 95 workspace source entries, all under the feature
worktree. The 94 retained files matched the feature source exactly;
`workspace-source-hashes.json` records them. The remaining entry is the
temporary probe archived as `probe.rs`. This checks executable attribution
rather than assuming a shared target directory contains the latest binary.
The locked dependency versions did not change.

The comparison found **862 exact observations, zero mismatches**:

| Observation | Count |
| --- | ---: |
| Native round start | 76 |
| Native character death | 424 |
| Native spike plant | 37 |
| Native spike defuse | 18 |
| Native spike explosion | 5 |
| Derived phase value | 302 |

The Rust probe freshly traversed the actual replays through
`ReplayReader::read_path`. The Python oracle reread accepted Phase A Parquet
exports from source `16345d94c2a35e50aeb1c1a98012e19cbd38b448`, tree
`ed7a5a7afe39231a9624d5deebb1eed2e2977acd`. These are historical exports,
not a new CLI export. All 134 existing recorded runtime files other than
`lib.rs` matched the feature source; `lib.rs` only exports the new optional
module. The oracle calls the existing independently implemented
`extract_kill_ledger.validate_event_payload`, checks original native bytes and
the exact float32 timestamp bits, and compares all projected words and times.
For phases it independently checks schema identity, the raw eight-bit window,
the sole typed value, and the existing Python round extractor's supported
phase codes. Both multiplicity and order within each source stream must match.

The probe's detailed TSV includes actor GUIDs and is deliberately not committed.
The receipt contains aggregates and hashes only. Unknown native groups are
counted, not silently interpreted. The Reader counted **76 skipped
checkpoints**; this is not checkpoint gameplay validation.

## Reproduce

Use a checkout whose functional hashes match `receipt.json`, Rust 1.86 and
the repository's Python requirements. The temporary example below is a
verification probe, not a published CLI command:

```bash
cp docs/evidence/phase_b/gameplay/probe.rs crates/vrfkit/examples/gameplay_probe.rs
CARGO_BUILD_JOBS=1 cargo +1.86.0 build --release -p vrfkit \
  --no-default-features --locked --example gameplay_probe
rm crates/vrfkit/examples/gameplay_probe.rs
python docs/evidence/phase_b/gameplay/verify_gameplay.py \
  --probe target/release/examples/gameplay_probe \
  --inputs /path/to/inputs.json \
  --exports /path/to/accepted-public-audit \
  --source . --out /path/to/new-local-gameplay-check
```

`inputs.json` maps the six input SHA-256 digests in the receipt to local replay
paths. The accepted export root has one `<digest>/export` directory per
input, containing `events.parquet` and `fields.parquet`. Use the recorded
accepted exports to repeat the historical comparison, or explicitly record
a newly generated oracle's commit and binary instead of relabeling it.
The script asserts each replay's digest before running the probe. Do not
publish its detailed local TSV output.

Author-side checks at the recorded feature checkpoint:

```bash
cargo +1.86.0 test --release -p vrfkit --no-default-features --locked --test gameplay
cargo +1.86.0 fmt --all --check
python -W error tools/check_ascii.py --check
```

The focused suite passed **12 tests**. It covers all 256 phase byte values,
both native string encodings, truncation at every byte offset, tag/name/arity
and time mutations, missing/conflicting overlays, exact raw-value matching,
unknown/null references, Reader cancellation, unchanged raw delivery and
explicit skipped-checkpoint scope. `example-12_10.txt` records a separate
execution of the aggregate-only public `gameplay_events` example.

These runs were correctness checks while another branch built. No speed,
CPU or memory claim is made. Independent review, full workspace checks and
the final integration CI are recorded by the Phase B coordinator separately.
