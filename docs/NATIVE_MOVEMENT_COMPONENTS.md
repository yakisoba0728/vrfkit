# Native movement component envelopes

`ComponentDataStream` is a signed 32-bit byte count, exactly that many payload
bytes, and one unused-bit-count byte. The payload's logical bit length is
`8 * byteCount - unusedBits`. The count must be nonnegative, unused bits must be
between zero and seven, and an empty byte array requires zero unused bits.
The complete outer field must be consumed. No allocation is based on the count:
the decoder borrows a window only after checking its length against the RPC.

For a nonempty payload, the supported logical layout is:

1. Movement magic `0x52`, moves under wrapping three-bit markers, and an explicit
   zero marker. A short remainder does not substitute for this terminator.
2. A 16-bit absolute input end position, measured from the payload's first bit.
3. Input header `0xA3` and its body, ending exactly at that declared position.
4. Optional discrepancy header `0xA4` and its body, ending at the logical payload
   boundary. No discrepancy header is required when the input ends there.

The names *input* and *discrepancy* follow the reference source. Only their
header-only forms are currently decoded. A nonempty body or an unknown header
remains unresolved: it increments the component and movement error counters,
and the sink preserves the **entire original RPC payload**. Successfully
decoded movement prefixes and their provenance remain available. A failed
component does not prevent a following framed component or update from decoding.
Unused high bits are outside the logical window; their values need not be zero.

## Evidence and scope

The pinned reference is
[`ComponentDataStream.cs` at `fbf24b62`](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/Replay.Valorant/Movement/ComponentDataStream.cs).
Its layout was checked against six public replay fixtures, through the actual
transform and RPC dispatch path. An observation pass cloned each component
reader, checked the native envelope and logical boundaries, and compared its
move count with the unchanged decoder. It did not alter the production cursor.

| Public build | Component streams | Moves | Optional A4 present | A4 absent |
|---|---:|---:|---:|---:|
| 12.05 | 1,831,572 | 1,844,387 | 1,815,504 | 16,068 |
| 12.08 | 1,746,674 | 1,757,418 | 1,742,627 | 4,047 |
| 12.09 | 2,616,931 | 2,634,029 | 2,189,286 | 427,645 |
| 12.10 | 8,853 | 8,907 | 8,853 | 0 |
| 12.11 | 2,949 | 3,001 | 2,949 | 0 |
| 13.00 | 4,946 | 4,991 | 4,946 | 0 |
| Total | 6,211,925 | 6,252,733 | 5,764,165 | 447,760 |

Every component had an exact native envelope, a zero movement terminator and a
header-only A3 input component. Every present A4 component was header-only.
There were zero native structural failures and zero per-component move-count
differences. Original input SHA-256 values remained unchanged. The aggregate
receipt, input hashes, source hashes and per-build report hashes are in
[`native-movement-envelope-observation.json`](evidence/native-movement-envelope-observation.json).

This observation covers the main stream of these six files; it skipped
checkpoints and did not remeasure the private corpus. It reused Rust's movement
value decoder, so it establishes structural and count evidence, **not an
independent numeric-value comparison**. The receipt does not claim production
before/after Parquet parity; that remains a separate integration validation.
The reference implementation's limits of 16 moves and 8,192 payload bytes are
policy limits, not established wire bounds, and are not imposed here. The
20-move synthetic regression remains supported.

Ten independently generated synthetic cases were also evaluated by the old
Rust decoder and the pinned reference implementation. Both accepted the two
valid nonempty forms (A3 alone and A3 with A4). The old Rust decoder incorrectly
reported zero errors for seven cases refused by the reference: negative byte
count, unused-bit count eight, missing zero terminator, invalid input end,
unknown A3 magic, unknown A4 magic, and an extra outer byte. An empty native
array was accepted by the reference and is now accepted by Rust; this form was
not observed in the six replay fixtures. No synthetic bytes came from a replay.

## Why the earlier interpretation appeared to work

For the observed byte counts below 65,536, a `u16` read takes the low half of
the native count. Its zero high half was mistaken for a `movementBitCount`.
The movement start therefore happened to align, but the apparent envelope ended
16 bits before the real byte array ended. The supposed 24-bit outer trailer
combined the final 16 payload bits with the unused-bit-count byte.

The old `remaining <= 31` shortcut then stopped before reading the actual
terminator and A3/A4 headers. The native logical remainder after the last move
was 27 bits with A3 alone or 35 bits with A3 and A4. Accepting these fields as
opaque tails could report success for malformed or unrepresented data, which
prevented the sink's raw-RPC fallback from running.

The formerly synthetic direct/u16-sized alternatives had no replay evidence.
Their tests now explicitly assert refusal, preserving coverage of every former
test shape. They are not accepted as speculative fallback grammars.

## Counter and source API migration

`MovementRecord` and the Parquet schemas stay unchanged. Successfully decoded
movement prefixes keep their arrival order. Correctly rejecting an invalid
outer envelope can emit fewer moves than the earlier decoder; a later unknown
body retains its decoded prefix and the complete raw RPC. The change affects
`RpcDecodeResult`, export statistics, human-readable summary lines and manifest
quality keys. Consumers that construct or match these Rust types must update
their source.

The following eight `RpcDecodeResult` fields are removed:

```text
sized_section_tails             sized_section_tail_bits
open_section_tails              open_section_tail_bits
sized_section_trailers          sized_section_trailer_bits
envelope_trailer_streams        envelope_trailer_bits
```

The removed summary/manifest fields have the corresponding `movement_` prefix,
except `envelope_trailer_streams`, whose published key was
`movement_envelope_trailers`. Checkpoint summary keys add `cp_`; the checkpoint
manifest uses the same unprefixed keys inside `quality.checkpoints.sink`.
There is no numerical conversion from those fields to native component counts.

The replacement fields are listed below. Manifest keys and `ExportStats` fields
add `movement_` to each name; `RpcDecodeResult` uses the names shown.

| Field | Meaning |
|---|---|
| `component_streams` | Component decodes attempted with a known character GUID |
| `component_complete` | Attempts that consume the complete supported logical layout |
| `component_errors` | Failed component attempts, also included in `error_count` |
| `component_empty` | Complete native arrays with zero byte count and zero unused bits |
| `component_envelopes` | Valid outer length and unused-bit count, even if inner parsing fails |
| `component_payload_bytes` | Total payload bytes within validated envelopes |
| `component_unused_bits` | Total declared unused high bits in validated envelopes |
| `component_input_headers` | Recognized A3 headers, including headers before a body error |
| `component_discrepancy_headers` | Recognized optional A4 headers, including headers before a body error |

`component_streams = component_complete + component_errors`. Orphan components
without a character GUID and outer framing errors remain represented by the
existing broader `error_count`/`movement_rpc_errors`; they are not attributed
component attempts. A hard RPC framing error returns `Err` without a result
object, so only the broader error tally survives that return path.

`MovementError` adds `NegativeComponentByteCount`, `ComponentLengthMismatch`,
`InvalidUnusedBitCount`, `InvalidInputBoundary`, `UnknownComponentMagic` and
`UnparsedComponentBody`. Exhaustive matches must add these variants. The
existing `TruncatedComponentHeader` now refers to the 32-bit byte count.

The main and checkpoint summaries replace the tail/trailer lines with
`Movement components`, `Component envelopes` and `Component headers`. Python
guards share the new contract in `tools/summary_counters.py`; missing counters,
impossible partitions, impossible header counts and obsolete names fail.

Historical private export baselines and historical reports remain byte for
byte unchanged. `check_baseline_schemas.py` validates their archived key sets;
this is not a current-executable comparison. `check_export_baseline.py` refuses
an obsolete baseline with a nonzero exit **before** a missing replay can turn
the run into a skip. To create a current baseline, genuinely export its original
input and review the resulting changes before using `--update`; never fill in
new values from the old trailer numbers. `benchmark_resources.py` likewise
refuses an obsolete movement counter contract and does not normalize unequal
counter contracts into equivalent benchmark work.

Removing published manifest keys and summary lines is a CLI output breaking
change. Under the repository's release policy, the next release containing
this migration must increment the minor version (from 0.5.x to at least 0.6.0).
This source change does not publish a release or claim that the historical
private baselines have been rerun.

## Focused verification

The decoder regressions are generated from independent test bit writers and
exercise malformed count/length/unused-bit fields, movement termination,
input boundaries, unknown headers and bodies, recovery of subsequent updates,
empty native arrays, nonzero unused high bits, optional A4, both move variants,
and more than 16 moves. Sink tests assert full raw RPC bytes and bit lengths
alongside retained movement prefixes and provenance for unknown bodies.

```sh
cargo +1.86.0 test -p vrf-movement --locked
cargo +1.86.0 test -p vrfkit --no-default-features --locked
python -W error -m unittest discover -b -s tools/tests -p 'test_*.py'
python -W error tools/check_baseline_schemas.py
```

A zero error count alone is not evidence that decoded values are correct.

## Post-fix public validation

The release CLI built with Rust 1.86.0 from combined source
`2e3526fb867c58400046b5d294aa58d01496e11a` was frozen before validation:

- Executable SHA-256:
  `e6b855ed2ea154260a30bca8606eefa15454e1b76cecca09a0e886e07d096a69`.
- Parser source fingerprint:
  `0fc5b7c26fece8aab301d8f3da56b5831255833c2483dd92722cbd4e0ad312f4`.
- [Build and per-file source receipt](evidence/native-movement-build.json)
  binds the measured local commit to the published parser contents. Later
  documentation and comparator integration changes do not change those files.
- [Aggregate validation receipt](evidence/native-movement-public-validation.json):
  `99b455920c778d55e78d9253fe8cd9ebfc06567f940e906d5124769f37c1bdd0`.

All six fixed public inputs (12.05, 12.08, 12.09, 12.10, 12.11 and 13.00)
passed the common validation and checkpoint-enabled export audit. The
[complete report](evidence/native-movement-public-audit.json) records 76
checkpoints, 1,958,155 scored main blocks and 60,169 independent typed/raw value
comparisons, with zero mismatches or width failures. The outputs contain
4,207,888 main field rows, 1,142,904 checkpoint field rows and 6,252,733 movement
rows. All 22,561 unresolved main RPC payloads remain preserved; RPC loss and
movement error counters are zero.

All **78 Parquet files are byte-identical** to the prior accepted combined
export. The [exact parity receipt](evidence/native-movement-parquet-parity.json)
includes every file hash and every allowed manifest difference. Its recursive
comparison permits only the eight old counter removals and nine new counter
additions in the two sink scopes, plus a finite, nonnegative top-level
`elapsed_ms` change. No other manifest difference occurred. The new main
component counters also agree with the separate wire-observation counts; all
non-movement common-audit counts are unchanged.

The first discovery attempt covered only the three `.vrf` fixtures because the
corpus scanner does not discover `.replay` names. That limited report is
retained in the evidence chain. The complete rerun used read-only `.vrf`
symlinks resolving to all six original inputs; their filenames and bytes were
not modified. Source, executable, harness and input hashes remained stable.

The [independent code review](evidence/native-movement-independent-code-review.json)
includes the generated end-to-end Reader raw-preservation regression and a
mutation that removes the A3 body-completion guard and reproduces the failure.
The [separate public-output review](evidence/native-movement-independent-public-review.json)
independently checks all remaining manifest values and types, every unchanged
non-movement counter, the 78 table hash/size records and the source/harness chain.
It reviews the byte-comparison harness and retained evidence without claiming
a second read of every large Parquet file.
The numerical full-match reference comparison documented in
[MOVEMENT_COMPARISON.md](MOVEMENT_COMPARISON.md) was executed before this native
correction. These fresh post-fix exports establish exact Parquet parity with
the accepted output, rather than claiming that the reference parser was rerun.
No private replay, performance improvement or whole-format reconstruction
claim follows from these results.

## Phase A recovery and new verification

The interrupted local candidate `97e6c59` was recovered onto verified GitHub
integration `d44bcd2`, then committed locally as `603aa714`. The
[recovery receipt](evidence/phase-a-native-recovery.json) records a fresh
Rust 1.86 failure of the generated A3-body preservation regression before the
fix and its successful execution afterward. The focused core/movement suites
passed 171 tests, the complete Python suite passed 1,178 tests, and core Clippy,
rustfmt, baseline-schema validation and the fast documentation guard passed.
These checks do not substitute for independent Phase A QA or hosted Windows CI.

A [new six-public execution](evidence/phase-a-native-public-validation.json)
again validates all 76 checkpoints, 1,958,155 scored main blocks and 60,169
independent typed/raw comparisons. It freshly compares all 78 Parquet files
byte for byte with the previously accepted output and finds no unexplained
manifest differences. All 22,561 unresolved main RPCs remain preserved, with
zero RPC loss or movement errors. The real fixtures contain header-only A3/A4
components; unsupported bodies are tested using generated inputs.

This new execution reuses the frozen `2e3526f` executable after comparing all
151 recorded parser-source files to the recovered tree. It is not a new binary
build. The pre-native baseline exports and original receipts are also reused
without rewriting their commit attribution. The receipt names both histories,
records the new execution's time and source fingerprint, and publishes each
new before/after table hash. Performance, private-corpus validation and a new
full-match reference-parser execution are outside this correctness receipt.
