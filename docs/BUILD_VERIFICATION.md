# Common build verification, 2026-09-25

All **986 unique available replays**, spanning **24 supported builds**, were
checked with parser commit `bf8ab9c94b602061bcd65c606f116aabb82a0c9a`.
Discovery found 1,048 paths across the version-organized archive, preserved
fixtures and Demos directory. SHA-256 deduplication removed 62 duplicate copies.
The archive contributes 982 unique files; preserved fixtures add the three
12.10/12.11/13.00 replays and one additional 13.02 replay. Demos adds no unique file.

All 986 pass ReplayData validation and checkpoint-enabled export. The oracle
passes **635,388,312/635,388,312** scored main blocks;
**23,222,700** checkpoint blocks were walked. Independent
Python comparisons match **12,917,904** observed values, with no
width failures or typed mismatches. All 24 builds contain positive main and
checkpoint decoding work. No run is counted as passing because it was skipped.

The stronger all-counter acceptance rule is **905/986 clean**. The remaining
**81 replays** have array or array-leaf errors listed below. The audit command
therefore exits **1**; these findings are not relabelled as a clean corpus.

The [machine-readable report](../tools/fixtures/build_verification.json)
contains build aggregates, input hashes and the complete finding list.

| Build | Checked | Strictly clean | Scored main blocks | Checkpoint blocks | Values compared |
|---|---:|---:|---:|---:|---:|
| 11.06 | 3 | 3 | 1,983,592 | 73,760 | 38,742 |
| 11.07 | 3 | 3 | 1,987,726 | 72,553 | 40,654 |
| 11.08 | 3 | 3 | 1,969,145 | 76,440 | 45,996 |
| 11.09 | 3 | 3 | 2,158,062 | 78,172 | 41,271 |
| 11.10 | 3 | 3 | 1,839,369 | 67,573 | 47,289 |
| 11.11 | 3 | 3 | 2,205,209 | 83,268 | 48,123 |
| 12.00 | 3 | 3 | 2,031,069 | 77,828 | 37,752 |
| 12.01 | 3 | 3 | 1,905,287 | 72,795 | 38,374 |
| 12.02 | 3 | 3 | 2,027,973 | 77,588 | 59,482 |
| 12.03 | 3 | 3 | 1,944,438 | 74,386 | 40,751 |
| 12.04 | 3 | 3 | 2,425,410 | 86,588 | 58,200 |
| 12.05 | 3 | 3 | 1,886,622 | 69,916 | 51,263 |
| 12.06 | 3 | 3 | 1,642,411 | 58,280 | 45,476 |
| 12.07 | 3 | 3 | 1,934,113 | 77,802 | 38,249 |
| 12.08 | 3 | 3 | 1,977,496 | 69,130 | 38,100 |
| 12.09 | 3 | 3 | 2,170,389 | 74,575 | 40,455 |
| 12.10 | 1 | 1 | 13,680 | 1,501 | 175 |
| 12.11 | 1 | 1 | 6,506 | 1,363 | 112 |
| 13.00 | 1 | 1 | 8,860 | 1,343 | 123 |
| 13.01 | 215 | 193 | 136,874,459 | 4,999,774 | 2,763,537 |
| 13.02 | 205 | 188 | 141,928,372 | 5,117,150 | 2,923,122 |
| 13.04 | 108 | 98 | 67,440,898 | 2,474,063 | 1,357,848 |
| 13.05 | 401 | 369 | 253,658,021 | 9,310,150 | 5,085,060 |
| 13.06 | 6 | 6 | 3,369,205 | 126,702 | 77,750 |

## Method

`tools/verify_build_corpus.py` recursively discovers each supplied corpus root,
deduplicates by SHA-256, and applies the following checks to every unique file:

1. `vrfkit validate`: a successful exit, an identified branch, positive scored
   block count, and an exact passed/scored match. The rounded percentage alone
   is insufficient.
2. `vrfkit export --checkpoints`: a successful exit and all thirteen Parquet
   tables. Required CLI counters must be present, overlay categories must
   reconcile, and exported row counts and checkpoint GUID-path counts must
   agree with their independent table/manifest measurements.
3. Main and checkpoint quality: no malformed packets, rejected partials,
   unfinished partials, framing/transform/field-stream loss, lost RPCs,
   resource-limit failures, typed-overlay failures, struct failures, movement
   failures, array errors, array truncations or array-leaf errors. Missing or
   invalid counters fail. Positive checkpoint work is checked per build.
4. Independent value comparison: Python decodes the observed fields specified
   by `public_fixture_type_evidence.json` directly from raw payload bits and
   compares them with the Rust-exported typed values. Each replay must contain
   observed evidence. Absent field identities are recorded separately; they
   are not treated as successful comparisons.

**Clean/checked** means the number of replays meeting all four conditions,
divided by every unique replay checked. A successful export alone is not a
strict clean result. Registered build support and a completely clean field
audit are separate measurements; nonzero findings remain visible below.

Unknown whole RPCs preserved as raw payloads are counted separately from RPC
loss. Skipped bits, RPC suffix bits, refused handle conflicts and untyped
properties remain explicit in the report. A clean audit does not imply full
semantic coverage, complete understanding of every field, or verification
against what the game displayed.

## Findings

| Build | Affected replays | Main array errors | Main leaf errors | Checkpoint leaf errors |
|---|---:|---:|---:|---:|
| 13.01 | 22/215 | 14 | 0 | 17 |
| 13.02 | 17/205 | 15 | 1 | 9 |
| 13.04 | 10/108 | 5 | 0 | 11 |
| 13.05 | 32/401 | 23 | 2 | 19 |

These are error-counter occurrences, while the affected-replay column counts
each input once. Some inputs contribute more than one error. The audit does
not establish the cause or field-level impact of each occurrence. It records
reproducible input hashes and keeps the private exports for investigation.
The earlier overlay-only summaries did not establish a clean result for all
of these additional counters. No parser behavior or acceptance threshold was
changed to suppress them.

Malformed/partial/framing/transform/field-stream/RPC-loss counters, typed-overlay
errors, struct failures, movement errors and evidence mismatches are zero.
Array parents and typed children remain subject to the documented preservation
rules; these aggregate counters do not prove every child value is understood.

Preserved unresolved RPC payloads total
**5,404,667 main / 1,601 checkpoint**.
Their associated skipped-bit counters are
**10,755,182,079 main / 367,783 checkpoint**.
The separately reported RPC suffix counters remain
**24,107,731 main / 0 checkpoint bits**.
These nonzero populations remain outside a claim of complete semantic decoding.

## Reproduce

Build the parser from the revision recorded in the report. Use fresh output
paths and supply all desired replay roots; filenames and folder names do not
determine the build, which is read from validation output and cross-checked
against the export manifest.

```powershell
cargo +1.86.0 build --release -p vrfkit --locked
python tools/verify_build_corpus.py --exe target/release/vrfkit.exe --corpus '<archive-root>' --corpus '<preserved-fixture-root>' --corpus '<Demos-root>' --work-dir '<new-private-work-dir>' --output '<new-report.json>' --jobs 4
```

The report pins the parser commit, Rust source digest, executable digest,
runner digest, evidence specification and input content hashes. Inputs are
hashed again after processing, and the executable is checked at completion.
Private logs, per-replay reports, manifests and exports remain in the work
directory. The published report contains no source paths or player identities.
The command writes findings even when its strict acceptance gate exits nonzero.

## Arithmetic evidence

The shared replay audit supplements the transform tests. The sixteen builds
11.06--12.09 have 79 independently captured native-machine-code cases each;
the eight other builds have eleven upstream golden cases each. These remain
different sources of arithmetic evidence. They are not used as substitutes
for any of the common replay checks.

## Earlier measurements

[Build recovery](LEGACY_BUILD_SUPPORT.md) records the 48 samples used while
adding 11.06--12.09. [Upstream parity](UPSTREAM_PARITY.md) records the earlier
13.06 and ability-decoding sample. The physical-field and gameplay-observation
measurements in [CURRENT_STATUS.md](CURRENT_STATUS.md) and related phase reports
retain their own dates and denominators. This audit does not recompute those
historical semantic inventories.
