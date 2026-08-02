# Codex task brief #3 — preserve the bits of blocks we cannot read

## Who you are in this

You are a **delegate**, not a handoff target. A Claude session remains the main
worker on this repository and will review, verify and merge what you produce.
Work on your own branch and worktree, commit there, report back. Do not push to
`master`, do not merge anything yourself.

Use subagents where the work fans out.

This is the production implementation of the Task C you measured. Your own
measurement is committed at `PROJECT_STATUS.md` section 7-C under "PRESERVATION
COST". Read it first — it is your prior work and it already settles most of the
design.

---

## The change in one paragraph

`crates/vrf-net/src/field.rs`, `parse_class_net_cache`: when `function_count`
is 0 the export group could not be resolved, the handle width is unknown, and
the record stream cannot be walked. It returns
`Err(NetError::UnresolvedFunctionCount)` **before reading a single bit**. The
caller only calls `on_stream_failure`, which pushes a diagnostic string capped
at 32 lines. No `on_field` or `on_rpc` fires, so **no Parquet row is written and
the payload is gone**. Make the block's payload survive as one row in
`fields.parquet`.

## Why it is worth doing

Three reasons, in the order that matters:

1. **An archived `fields.parquet` cannot be reinterpreted.** When the missing
   descriptor eventually lands, there is nothing to re-run it against — the
   source `.vrf` (60-70 MB each) has to be kept forever. Preservation is what
   makes the Parquet the archive.
2. **It blocks investigation.** A 7-H audit this week tried to brute-force
   ClassNetCache payloads to identify a class by elimination and **could not
   run the test at all**, because the bits reach no file. That is a research
   capability the current behaviour removes.
3. **What is in there.** `AbilitiesAndBuffsComponent` is 17,264,706 of the
   17,507,210 lost bits on 02d4d478. It carries ability and buff/debuff
   *state* — charge counts over time, who was blinded or slowed and for how
   long, ult gauge between casts. The difference between "this ability was
   used" (we have it) and "this ability affected these players for this long"
   (we do not).

---

## Design — most of this is already decided, do not relitigate it

**One row per block, never a fabricated per-field split.** The whole reason the
block failed is that it cannot be split. Inventing a per-field structure you
could not read is the exact failure this repository has hit three times in one
week: 13-A (a fabricated `{0,0,0}` where the truth was "unknown"), 13-E (a
`null` payload that meant two different things), 13-I (a level instance name
fabricated into a class path). Your own measurement harness already did this
correctly — keep that shape.

**The row must be unambiguously distinguishable from an ordinary undecoded
field row.** This is the part your measurement did not have to solve, because
nothing downstream consumed it. Now something will.

`field_name: None` is **not** a sufficient discriminator: 33,529 rows on
02d4d478 already have it, from unmapped handles. Choose an explicit marker,
state why you chose it, and make it something a consumer can filter on in one
predicate. Whatever you pick, a test must assert that no pre-existing row can
match it — measure that against the real corpus, do not reason about it.

`FieldRecord` (`crates/vrf-export/src/field_writer.rs:34`) is the shape you are
filling. Adding a column is allowed if that is genuinely the cleanest
discriminator, but it costs a schema change on every row, so justify it against
the alternatives rather than reaching for it first.

**`skipped_bits` keeps its current meaning: "not parsed".** Preserving a
payload is not parsing it. Your Task C note already says this; hold to it.

**The 32-line `MAX_STREAM_FAILURE_RECORDS` diagnostic cap is a different
mechanism.** Leave it alone. It exists so a wrong transform does not print a
million lines; it is not a data path.

**The adapter must exclude these rows.** `tools/to_valplay_bundle.py` turns
Parquet into the valplay bundle. A block-payload row is not a field and must
not become one. Excluding it is not optional — and neither is proving the
exclusion works, because a silently-included row would corrupt an event
payload in a way no counter reports.

---

## What must not move

This is the strongest handle you have on correctness, so lean on it.

Preserving a payload changes **nothing about parsing**. So:

- `validate_corpus.py`'s five totals must be **identical**: blocks 136,545,822,
  fields 98,883,979, rpcs 75,571,092, malformed 0, skipped 1,972,080,670.
- Every overlay counter must be identical: decoded 369,395, raw/skip 73,984,
  not-in-table 512,071, no-field-name 33,529, rows offered 988,979,
  decode errors 0.
- `movement.parquet`, `actors.parquet`, `net_guids.parquet` must be
  **byte-identical**.
- `validate_metrics_corpus.py` must stay at 16/21, and — stronger — the
  `out/xval_summary.json` matrix must be identical **cell by cell** against a
  pre-change run. All 231 cells. Diff the file; the 16/21 total cannot
  distinguish "unchanged" from "one section flipped each way".

Only `fields.parquet` rows and bytes change. If anything else moves, you have
changed parsing, and that is a bug in the change, not a new baseline.

## Starting numbers, measured at HEAD `d494177`

    fields.parquet rows  : 1,240,444
    fields.parquet bytes : 13,334,132

Your Task C note forecasts 1,246,809 rows. That row forecast still holds
(1,240,444 + 6,365). **Its byte forecast of 13,484,318 does not** — it was
computed against a 13,255,044-byte file, before the descriptor work in 13-J and
your own Task B both landed and merged. Re-measure the bytes; do not reconcile
the old forecast.

---

## Verification that actually discriminates

**Round-trip, not "it wrote something".** For a sample of preserved rows, read
the payload back out of Parquet and prove the bits are byte-identical to what
the parser saw — length, bit count, and the high padding bits of the final
byte. Your measurement harness audited 14,755 rows this way; do the same
against the production path, since it is a different code path.

**Prove the discriminator is exclusive.** Run the predicate a consumer would
use against the full 215-replay corpus and show that zero pre-existing rows
match it. A discriminator that collides with real data is worse than none.

**Prove the adapter exclusion works by breaking it.** Remove the exclusion,
show the bundle changes (and how), restore it, show the bundle is byte-identical
to the pre-change bundle. "The adapter ignores them" is a claim; a demonstrated
diff is evidence.

**Cost, re-measured on at least three replays.** Rows added, ZSTD bytes added,
export wall-clock. Your prior timing method — one warmup, 10 interleaved OFF/ON
pairs, bootstrap CI — was sound; reuse it.

---

## Environment constraints (all of these bite on this machine)

- Windows 11, PowerShell 5.1 primary. **PowerShell here-strings and heredocs
  break here.** Write a script file for multi-line content, or use a Bash tool.
- `python -c` with a Windows path raises
  `SyntaxError: truncated \UXXXXXXXX escape` (`C:\Users` contains `\U`). Use
  raw strings or a `.py` file.
- `pytest` is **not installed** and must not be added. Verification convention
  is a self-checking script with a `--check` mode, plus `tools/tests/*.py` run
  directly via stdlib `unittest`.
- **ASCII only in Rust code and comments.** `tools/check_ascii.py --check`
  enforces it (61 tracked files). The console is cp949 and truncates output at
  the first non-ASCII byte.
- `#![forbid(unsafe_code)]`. No skip path, no silent success. TDD.
- `valplay` is **read only**. Run its scripts by absolute path.
  `compute_metrics.py` writes `metrics.json` into whatever directory you give
  it, so always pass a directory under vrfkit's `out/`.
- Generated files only via their generators. If you touch `table.rs` at all:
  regenerate from `ValorantReplayParser` on `local/vrfkit-descriptors`
  (currently `f0dd7e7`), then `apply_type_corrections.py`, then `cargo fmt`.
  `main` there **must stay at `2d2e05e`** — it is the commit the reference
  bundles were built from. You should not need to touch `table.rs` for this
  task at all.

---

## Full sweep before reporting

```powershell
cargo test --workspace              # 246 (243 regular + 3 doctests)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
python tools\check_ascii.py --check
Get-ChildItem tools\tests\*.py | ForEach-Object { python $_.FullName }

cargo build --release
python tools\validate_corpus.py .\target\release\vrfkit.exe `
  "C:\Users\yakihyuk0728\Documents\GitHub\valplay\data\raw\vrf"
python tools\check_decode_errors_corpus.py .\target\release\vrfkit.exe `
  "C:\Users\yakihyuk0728\Documents\GitHub\valplay\data\raw\vrf" --jobs 12
python tools\check_export_baseline.py --baseline tools\baselines\export_02d4d478.json
python tools\check_corpus_baseline.py --baseline tools\baselines\build_1210.json
python tools\check_corpus_baseline.py --baseline tools\baselines\build_1211.json
python tools\check_corpus_baseline.py --baseline tools\baselines\build_1300.json
python tools\check_corpus_baseline.py --baseline tools\baselines\build_1302.json
python tools\compare_combat_report.py
python tools\validate_metrics_corpus.py --jobs 3
```

`check_export_baseline.py` will drift on `fields.parquet` rows and bytes and
**must not drift on anything else**. Explain every line before `--update`.

---

## What "done" means here — stricter than usual

This repository has caught **twelve-plus** documented claims that turned out to
be false, several of them in the last week, several of them its own. Reviews are
calibrated to that.

1. **Every figure comes with the command that produced it.** A number without a
   command is treated as unverified.
2. **State which fields any comparison read.** 13-I shipped two wrong fields
   through two rounds of "spawns match" because the comparison read only
   `location`.
3. **A claim about what the data contains is not established by the code that
   produces it.** 13-A rested on "there are zero genuine (0,0,0) spawns", never
   checked against the reference. There were 66.
4. **Never trust a guard you have not seen fail.** Break it, show the failure
   output, restore, show it passes.
5. **Report what you did not do.** Scope dropped, a case you could not verify, a
   measurement you skipped. Under-reporting is a worse failure here than
   incomplete work.

## Report back with

- Your branch and HEAD, and your worktree path.
- The discriminator you chose, why, and the corpus measurement proving no
  pre-existing row matches it.
- Round-trip evidence on the production path.
- The adapter-exclusion break/restore demonstration.
- Cost table: rows, bytes, timing, on three or more replays.
- The full sweep, before and after, with every baseline drift line explained.
- Anything you find that contradicts `PROJECT_STATUS.md`. It is wrong about
  figures more often than it is right, and correcting it is part of the job.
