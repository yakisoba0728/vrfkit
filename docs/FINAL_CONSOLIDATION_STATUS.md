# Final consolidation and independent QA

## Acceptance and review surface

The six corrections below are implemented, covered by author regressions, and
independently reviewed and functionally checked within the stated scopes.
Local source preparation and all regular/independent checks are complete;
hosted acceptance is recorded in the
[fixed final receipt](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6932599335).
That receipt is authoritative for the final branch SHA/tree, PR synthetic
checkout, all required CI jobs and actual Windows package identity. This
source document is frozen before those runs to avoid changing the tested
commit merely to record its own hash.

[Draft PR #58](https://github.com/yakisoba0728/vrfkit/pull/58) remains the sole
review surface. No main merge, Draft removal, automatic merge, tag, GitHub
Release or registry publication is authorized by this work. The source stays
an unpublished **0.6.0** candidate. The [migration](FINAL_CONSOLIDATION_MIGRATION.md)
and [evidence index](evidence/final_consolidation/README.md) are the review entry
points; the original PR body is preserved byte-for-byte in the evidence index.

## Source identities

| Item | Identity / scope |
| --- | --- |
| Main at recovery | `9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9`; final unchanged-state check belongs to the receipt |
| Recovered integration baseline | `8581642283bd5a571967129d73897db8ee5408ba` |
| Baseline tree | `03fb832d96714343e85c30ea9468993c1af63d97` |
| Baseline synthetic merge | `7360230343071cc5f357c49b0fe765602f11f75e`; same complete tree |
| First published audit checkpoint | `6e54158f0747ce55334f7138e7d5ce7ee8068bb1` |
| Published correction/public-QA checkpoint | `003854fc87892fd90ae12a549ea53f817a751da9`, tree `e64effd9282d4f42f308d9889fd25718725f9712` |
| Clean isolated runtime build/test source | `1d2d8f6fe68f3ad1f196bca5adac24b0d3fbee6c`, tree `833dae7add06f21589ffdae9680d5df12a5f89fe` |
| Final integration and PR checkouts | Resolved after the final push and recorded in the fixed receipt, with complete-tree equality and parent checks |
| Cargo.lock SHA-256 | `ef4c199b359baeca53be4c5d634a288ab4c7e00e1590c8d0948b186e5c5c316a`, unchanged |
| Publication baseline | Ten tags through `v0.5.0`; latest public release `v0.5.0` |

The isolated runtime commit is a local review/build identity, not a claim that
it is separately fetchable from GitHub. Its 355 runtime, development-tool,
fixture, workflow and Cargo file fingerprints equal the published correction
checkpoint and must equal final integration. Later changes contain only
review documents and evidence. All ten production package versions,
dependencies, workflows and generated wire mappings remain unchanged.

## Corrected findings

| Severity | Defect and correction | Reproduction / independent check |
| --- | --- | --- |
| **P1** | Export could replace/delete its own replay if the input had an export-owned name inside `--out`. Canonical containment is checked after foreign-entry refusal and before staging. | Author baseline returned success and overwrote the generated input. Independent public-input checks reproduced five actual input losses plus one conservative alias-refusal failure; all six refusals and safe sibling/hardlink cases pass after correction. |
| **P2** | Windows diagnostic publication deleted the old report before rename. Direct rename now retains the old report if publication fails. | A compiled Linux simulation of the old Windows sequence fails at the real publication helper; restored source, unit executable and normal CLI hashes match. Native Windows post-fix execution is a hosted gate; no Windows pre-fix run is claimed. |
| **P2** | A malformed repeated shooter GUID kept a stale identity for following components. The identity is cleared until another valid GUID arrives. | Author failure/pass plus independent ordinary/partial RPC, five malformed widths, main and two snapshots; valid prefix/recovery and complete raw parents survive. Compiled mutations fail and exact restored test executable hashes match. |
| **P2** | Short checkpoint path indices could amplify stored and detached path strings far beyond encoded input size. Every resolved declaration is charged against a fixed 256 MiB per-archive budget before indexed cloning, callbacks or insertion. | Seven author regressions plus independent public-API exact-boundary/excess probes. Baseline accepts 268,500,992 resolved bytes from 90,019 encoded bytes; the candidate refuses the excess while preserving the admitted prefix. |
| **P2** | Direct partial-buffer calls could panic on short slices or count nonexistent discarded bits. Early `PayloadOverrun` refusal returns the prior assembly and counts supplied/prior bits only. | Two author regressions and three independent public-API cases establish refusal, neighboring-channel preservation and restart; network package debug/release suites pass. |
| **P2** | Packet bit lengths narrowed to signed 32-bit arithmetic and overflowed at 268,435,456 bytes. Checked `u64` arithmetic preserves sentinel positions and rejects unrepresentable sizes. | The unchanged arithmetic was extracted and failed an allocation-free boundary test before correction; the fixed helper and all 119 network package tests pass in debug and release. Normal frame packets already have a small packet limit. |

The high-value refactoring is confined to these boundaries: checkpoint path
admission before ownership/copying, an explicit required input path on the
private export transaction, a diagnostic publication helper, and checked
packet length arithmetic. Existing shared Reader/CLI decoding, checkpoint
state isolation, writer backpressure and gameplay identity architecture are
retained. No speculative type is added. Neither broad module replacement nor
unmeasured parser/writer tuning is included.

## Fresh independent execution

| Gate | Actual local result |
| --- | --- |
| Distinct public inputs | **5**: builds **12.05, 12.09, 12.10, 12.11, 13.00**; pinned origins and actual SHA-256 checks |
| Baseline and candidate main/checkpoint processing | Both five-input runs pass; **59** checkpoint chunks and **1,380,345** independently checked blocks per run |
| Typed/raw evidence | **43,844** comparisons; all **16** specifications observed across the inputs; no mismatch/width failure |
| Complete export preservation | **65** Parquet files byte-identical, **5** stable manifests equal and **765** CLI counter values equal |
| Main-only versus checkpoint-enabled | Five additional main exports; all **30** main table bytes equal |
| Raw RPC and Focus | Main/checkpoint raw RPC loss zero; 12.05 main **102** parents/**53** children/**3** empty trailers, checkpoint **18** parents/**0** children |
| Required counters | Nine native plus two Focus counters per scope; all **22** missing-counter controls rejected |
| JSONL | **39** tables/**138,727** rows/**298,133** binary bytes compared as exact logical values and schemas |
| CLI | Five `inspect` checks; **25/25** independent containment, legal-output, malformed-input rollback and diagnostic replacement cases |
| Reader/core contracts | **43/43** on exact runtime: 28 maintained tests, nine retained independent contracts, six newly authored independent tests |
| GameplayIdentity | **4,495,315** movement rows and proof tuples match an independent interval oracle; **3,925,225** resolved and **570,090** explicitly unresolved (`NoReference`), no mismatch |
| Mutation controls | Compiled stale-GUID and old diagnostic-publication algorithms fail; exact source and executable restoration verified |

The identity comparison includes **4,465,251** component rows,
**11,703** fragmented rows and **23** raw `SpawnedCharacter` declarations.
It is evidence-qualified body association, not account identification or kill
credit. Synthetic fixtures, repeated CLI commands, the 28 overlapping tests
and CI executions do not increase the distinct real-input count. No raw
identity rows or private replay data are published.

Across 196,727 checkpoint GUID declarations, the largest observed resolved
path total is **252,908 bytes** per checkpoint and the longest path is 130
UTF-8 bytes. This establishes public-input margin for the new budget, not a
process-RSS bound or private-corpus compatibility guarantee.

## Builds, regular suites and hosted gates

The [build evidence](evidence/final_consolidation/builds/README.md) records
source fingerprints and complete commands. The frozen Linux default-release
candidate SHA-256 is
`c3a3010083b81332e03d90b888d38dfe069dfb659f3c866637cfed0051d6d417`
(38,832,064 bytes). All 12 workspace release packages were cleaned before
building; all 11 local Cargo artifacts report `fresh: false` and the correct
source paths. Rust 1.86, fat LTO, one codegen unit and the existing release
profile are unchanged. The baseline's first concurrent build ended in SIGKILL
and remains a failed attempt; its one serialized retry succeeded without
changing those flags.

The fresh Python 3.12 local suite passes all **1,181** tests with warnings as
errors. The integrated Rust workspace suite passes **885** tests with zero failures
or ignored tests, and rustfmt passes. The 14 added regular regressions make
the suite totals 885 on non-Windows and 886 on Windows;
platform execution is evidenced separately and independent probes are not
added to those totals.

Final hosted acceptance requires the complete integration push workflow
(**11 jobs**) and PR workflow (**10 jobs**), with no failed, cancelled or
skipped required gate. The matrix covers Rust 1.86 on Windows/Linux, stable on
Windows, core-only/single-feature checks, Oodle/non-Oodle paths, rustfmt,
Clippy with warnings denied, strict Rustdoc, Python 3.12/3.13 on both platforms,
Rust/PyArrow interoperability, public replay validation and optimized Windows
packaging. The final receipt names each actual job checkout rather than
assuming the run's head SHA is the PR checkout.

The [artifact verifier](evidence/final_consolidation/release-final/README.md)
checks downloaded GitHub digests, ZIP members, checksum sidecar, build-info,
AMD64 PE32+ executable hash, legal files, source archive and the hosted public
replay report. Windows CRLF source fingerprints are reconciled explicitly
with Git/Linux LF bytes. Actual Windows execution comes from hosted logs;
local PE inspection is not Windows execution. No tag or release is created by
the CI-only archive name.

## Performance decision

**No new runtime optimization or quantitative performance claim is adopted.**
Role C independently recomputed 20 retained Phase D series: 280 historical
records (40 warmups, 240 measured) and 2,640 statistic cells, with four rejected
metadata corruptions. These are fresh reviews of old measurements, not new
replay timings. The focused harness's 20 tests pass.

All six primary and four withdrawn-experiment wall comparisons remain
**INCONCLUSIVE**. The adverse primary 12.05 main paired median (+9.5463%),
withdrawn candidate's 12.09 checkpoint RSS increase in all six pairs, and
allocator total/peak increases remain documented. The old reservation
experiment stays withdrawn. No claim of ordinary speed, throughput, RSS or
performance nonregression follows from the correctness/resource changes.
Fresh controlled performance execution is **NOT_EXECUTED** because no isolated
control window and new supported optimization mechanism were established.
Reference default exports still perform different work and remain
**NOT_COMPARABLE**; no new C# replay benchmark is claimed.

## Scope limits and blocked evidence

Private archive metadata reports **24 releases, 1,011 assets and
50,938,107,071 bytes**. Actual private binary downloads, hash checks and replay
executions are all **NOT_EXECUTED (0)**: the available authenticated connector
has no private Release asset-byte transport, and the local Git client has no
private credential. This is a capability boundary, not evidence that the
repository permission itself was denied. No credential extraction or payload
publication was attempted.

Unresolved `ReplicatedMovement`/quantization, `InputEventData`, GAS/FastArray,
`StopEffectType`, healing/OwnerActor ambiguities, unsupported Focus builds,
RPC tails and China-region mappings remain **blocked_by_evidence**. No guessed
wire semantics were introduced. The original preservation strategy is not a
lossless replay archive; documented unknowns and provisional callbacks remain.

The checkpoint budget is per archive and counts declaration path bytes;
reader/writer queues and encoded-input limits do not establish a universal
process-memory cap. File-backed input is not an immutable snapshot against
same-length concurrent rewrites. Publication tests cover in-process errors,
not hostile filesystem races or crash durability. No newly discovered P0/P1
is left open by the independent functional review; untested inputs remain
outside its evidence scope.

## Orchestration and recovery

Six unique specialists were actually created: A core correctness, B
architecture, C performance/memory, D Reader/gameplay/CLI, E independent QA and
F CI/release. Maximum observed concurrency was **three specialists plus the
coordinator**. Authoring and review were separated; E did not author the product
fixes, and C reviewed the coordinator's network changes. The final receipt
records completed/failed/unexecuted role counts after F's hosted gate review.
C also independently reconciled the final build/QA receipt hashes, mutation
linkage, 885/1,181 actual test counts and all 355 frozen source fingerprints.

1. Read the fixed receipt and resolve PR #58's current head.
2. Fetch `integration/vrfkit-overhaul`, compare the complete tree and the frozen
   runtime/tool/workflow fingerprints, then inspect any local worktree.
3. If a final gate is still pending, resume that gate on the exact current
   checkout. Do not credit intermediate runs or restart completed phases.
4. Preserve any new failure, correct only the affected scope, rerun its
   independent regression, then repeat final exact-head CI if source changes.
5. Leave PR #58 Draft/open/unmerged and main/tags/releases/archive unchanged.
   Final status must be `READY_FOR_MAINTAINER_REVIEW` or `BLOCKED_WITH_EVIDENCE`
   in the fixed receipt; no next large phase starts automatically.
