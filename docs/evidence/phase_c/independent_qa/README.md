# Phase C independent QA - completed correctness gates

**PASS for the bounded independent A/B/C/D correctness scope.** The final
public comparison completed with zero mismatches, all four compiled source
mutations were detected, and the restored source passed its final 12 contract
tests. `final-acceptance.json` records the aggregate verdict and hashes of the
supporting receipts and QA sources. Hosted CI and integration merge records
are maintained separately by the coordinator.

## Exact source and executable correspondence

The effective frozen build is commit
`8c2c6216fcbf1675b2ba050dd2470cb1466faf21`, tree
`10b32307fe786e8d686ae934dc716b0e6b4866ae`. `build.json` records 136 runtime
source hashes, compiler identity, actual Cargo artifact and dependency paths,
temporary probe sources, five executables, and four test executables. All
source hashes and all nine frozen artifacts were checked again after the
mutations and before this final receipt was written.

The compiler was Rust 1.86.0 on Linux. The functional verification used the
Cargo test profile at opt-level 2 with debug assertions, no debuginfo, and no
incremental compilation. It supports no performance claim.

The completed public harness ran from commit
`05b30c038d9a468c84217ed8c4074e010db45f23`, tree
`49fb9cc557707d0678d405ade0dd01c9ec723cb7`. The later changes were confined to
QA adapters and receipts; its 136 runtime hashes matched the frozen build.
The first complete fresh CLI audit and typed/raw scan were reused only after
checking the exact executable and parser sources. `public-acceptance.json`
retains their original runner commit and report hashes.

The six original public inputs use their accepted historical Phase B exports
as baselines. Four added public inputs were freshly validated and exported by
the frozen Phase B executable before comparison. That executable has SHA-256
`29fa495b19b7dbce2bfe7bbedc1dc9be2492a4c6a5aca2a9d1468d3d74c657d4`
and source commit `2fbead4f716f12f7c5d41ff067ff4ee63a55dc90`;
`extra-baseline.json` preserves the distinct old-code attribution. These ten
inputs cover seven build labels and contain no private replay inputs.

## Observed acceptance results

| Gate | Observed result | Receipt |
| --- | --- | --- |
| Fresh CLI validation/export and typed/raw scan | 10 public inputs; 99 checkpoints; 61,002 typed/raw values; zero failures or missing typed specifications | `public-corpus.json`, `typed-summary.json` |
| Reader checkpoint callback tables | All 70 Parquet files byte-identical to fresh CLI output, seven per input | `checkpoint-callbacks.json` |
| Main and checkpoint counters | Per input: exact 46 network and 54 sink numeric keys in each stream, 25 checkpoint scalars, and four first-error options checked as `None` | `checkpoint-callbacks.json` |
| Prior CLI output equivalence | 129 of 130 Parquet files byte-identical; the remaining file has the precisely predicted additive decoder change | `export-equivalence.json` |
| Narrow 12.05 decoder change | All 1,240,315 existing main field rows unchanged; exactly 53 predicted child rows and three recognized empty zero trailers | `export-equivalence.json` |
| Established gameplay raw oracle | 583 native observations and 386 phase observations, with exact raw values and stream order | `gameplay.json` |
| Packet/lifecycle movement identity | 6,310,086 movement rows and exact proof tuples; 39 raw SpawnedCharacter declarations; zero mismatches | `identity.json` |
| Existing C# movement reference | Fresh Rust output matches pinned historical latest-move references on 12.10, 12.11, and 13.00 | `movement-reference.json` |
| Focused synthetic contracts | 35 passed, zero failed or ignored; origins distinguished below | `build.json`, `checkpoint-native-positive.json` |
| Compiled source controls | Four compiled defects detected; restored 9 E plus 3 external tests pass; sources and frozen binaries unchanged | `mutations.json` |

The identity result comprises 5,669,526 `Resolved` observations and 640,560
`Unresolved(NoReference)` observations. The comparison also checks 6,269,101
movement component groups and 16,663 fragmented movement rows. It decodes raw
SpawnedCharacter values independently, constructs packet intervals from actor
lifecycle rows, and checks every movement value and proof tuple. It does not
use time proximity, account identifiers, or final GUID paths to infer earlier
identity. Null, ambiguity, retired lives, dormancy, concurrent changes, and
malformed evidence are additionally exercised by synthetic tests.

### Test origins and nonempty checkpoint coverage

| Origin | Passed |
| --- | ---: |
| E independently authored `contracts.rs` | 9 |
| External public API lifecycle regressions, independently rerun | 3 |
| A author `reader_checkpoints`, independently rerun | 11 |
| D author `reader_bounded`, independently rerun | 11 |
| A author nonempty native/movement checkpoint test, independently rerun | 1 |
| Total distinct focused tests | 35 |

The final row is
`movement_reader::checkpoint_movement_keeps_provenance_raw_rpc_and_all_native_counters`.
It positively exercises four checkpoint moves plus their provenance, raw RPC
preservation, and all nine native counters. It is an A-authored test, not an
additional independently authored E case. Its exact one-test invocation and
two filtered tests are recorded in `checkpoint-native-positive.json`.

The three external lifecycle tests remain byte-identical to commit
`57f992575989e03d94c33ad4c5a89772fcd08494`. Repeated executions during freeze
and restoration are not counted as new distinct tests.

### Meaningful source mutations

Each defect was compiled successfully before its detector ran. Compilation
failures were not counted as mutation detections.

| Deliberate defect | Detector and observed failure |
| --- | --- |
| Omit the checkpoint Summary callback | Independent consumer-error/phase contract: one failed test, exit 101 |
| Admit same-packet SpawnedCharacter declarations | Independent strict packet boundary contract: one failed test, exit 101 |
| Skip the bounded reader's excess-byte EOF probe | Independent exact-cap/EOF contract: one failed test, exit 101 |
| Suppress the exact FocusProjectiles route | Mutant export exits successfully; independent predicted child-row stream rejects all 53 missing children |

The clean executables were frozen before these changes. Each source file was
restored in a `finally` block and checked against the original hashes. The
restored code compiled successfully and passed all nine E contracts plus the
three external lifecycle regressions. The final source and artifact checks
also passed.

## Findings corrected before the freeze

Independent source review identified that reopening an already active GUID
on the same channel could retain a stale life. The gameplay implementation
now records conflicting active opens on either the same or another channel;
dormancy remains a separate supported case.

The preserved external lifecycle regressions and independent review also
exposed retired declarations being counted as competing current claims.
The author reproduced two failures, then corrected the candidate filtering.
A declaration strictly before a current life's previous end is retired before
ambiguity is counted. Equality remains uncertain because cross-vector
same-packet order is unavailable. The fixed source is the one frozen and
independently compared here. The author follow-up receipt is preserved in
`../gameplay/lifecycle_followup.json`.

QA harness failures are separately retained in `rejected-attempts.json`:
non-exhaustive synthetic record construction, relative dependency paths,
overly strict diagnostic-list assumptions, counter aliases/retention, and an
incorrect phase-field selector. These rejected attempts are distinct from
production findings and from the four intentional source mutations. No
interrupted or rejected run is credited with completed acceptance.

## Decoder evidence and remaining scope boundaries

`focus-array-historical.json` is an independent reread of accepted historical
12.05 output. It establishes the exact Clay class, parent handle 45/checksum
957945936, and member handle 46/checksum 831657485. It covers 102 main parents
and 18 checkpoint parents, complete bit consumption, three positive framing
shapes, and ten malformed framing controls. Its actor-open corroboration is
an aggregate class check; it does not establish time-scoped identity.

`decoder_delta.py` independently predicts every new child row from the old raw
parent bytes and verifies children-before-parent order, exact member raw bits,
packed GUID value, and copied context. It checks the two new route counters
and three exact +53 counter increments before normalization. Apart from
source-file and elapsed-time metadata, all other stable manifest fields and
summary counters remain equal. This excludes no unexplained data difference.

`decoder-candidates.json` records bounded presence/absence, exact groups and
checksums, raw widths, and typed presence on the fresh ten-input exports.
The accepted decoder route remains restricted to the measured 12.05 Clay
schema. Presence in sibling classes or other builds does not expand it, and
other candidate families retain their evidence-blocked status.

The callback adapter uses the production export schemas. Its four explicit
counter aliases are `rpcs_emitted -> sink_rpcs_emitted`,
`actor_opens -> sink_actor_opens`, `actor_closes -> sink_actor_closes`, and
`content_blocks -> sink_content_blocks`. Diagnostic retention is derived from
the actual vectors; checkpoint aggregation uses `NetStats::absorb` to retain
the public cap/drop semantics. Exact old-CLI manifest and counter equivalence
provides a separate check of production aggregation. Informational diagnostics
are recorded even when the first-error options and loss counters are clear.

The movement reference was not regenerated. It covers only latest-move
values emitted by the historical reference, after f32 quantization, for the
three pinned inputs. The full new identity comparison covers every main
movement row in all ten inputs. Native event timestamp joins remain outside
the identity API contract.

`read_bounded` caps encoded input and probes one additional byte. It buffers
the complete allowed input before parsing; the cap does not bound decoded,
parser, or consumer memory. The synthetic tests verify short reads,
Interrupted retries, exact and exceeded caps, original error identity,
callback ordering, and option forwarding.

## Reproduction and retained evidence

`freeze_build.py` builds from a clean checkout with a unique target and output
directory. `public_qa.py --focus-route` consumes those frozen executables plus
the explicit original corpus, extra corpus, historical exports, and fresh
old-code baseline paths. `--reuse-audit` permits reuse only after executable
and parser-source checks. `movement_reference.py` runs the existing bounded
reference gate. `mutations.py` requires a complete clean public receipt
before changing any source and verifies restoration afterward.

The extra-baseline command must use pre-Focus-route tool sources because the
new CLI summary contains counters the frozen old executable cannot print.
Its receipt identifies the actual runner commit and script hash.

Raw probe streams, full manifests, detailed logs, replay inputs, and Parquet
exports stay local. This directory publishes reviewed aggregate receipts,
hashes, and synthetic sources. It contains no benchmark commands or claims
about throughput, timing, CPU, or memory improvement.

The initial metadata-only recovery branch remains a historical interruption
record. Reconnection recovered the original four QA source files, whose exact
bytes were preserved in `9f2b2de02026b4d971ce52ad918f5abbacb1ebd2`. The restored
Focus checker subsequently completed, including the previously interrupted
actor-open corroboration. `partial-acceptance.json` is the earlier A/D/C
checkpoint; `final-acceptance.json` supersedes its pending-gate status.
