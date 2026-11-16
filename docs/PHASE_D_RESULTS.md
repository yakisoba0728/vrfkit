# Phase D engineering and performance qualification

## Qualification decision

**FUNCTIONALLY_READY_PERFORMANCE_INCONCLUSIVE** is the scoped engineering
verdict. Controlled measurements, source/output verification and the bounded
independent QA passed. Neither the C-only comparison nor the one optimization
experiment establishes end-to-end performance superiority. The experiment is
withdrawn; **zero Phase D runtime optimizations are integrated**.

This documentation candidate precedes its final task merge and integration/PR
CI. The [fixed final qualification receipt](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6918909298)
records completion of those required gates, the exact final source/checkout
and artifact identities, and terminal worker counts. An incomplete receipt
must not be treated as final acceptance. This avoids embedding a commit's own
future hash or relabeling an earlier CI checkout as the final one.

This is not a main-merge, Draft-removal, release or registry-publication
approval. Historical Phase A-C receipts remain historical; none is counted as
a new Phase D run.

## Scope and frozen identities

| Item | Exact identity |
| --- | --- |
| Starting integration and normal release candidate measured | ff8596b546736ea4300e013248609c7c3987e231 |
| Unchanged main, outside mutation scope | 9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9 |
| Output-compatible C-only baseline | 1b9bbf222f003e35c882e640c1b19e0bbcc88472 |
| Baseline executable SHA-256 | 2e6d3852f608f6c4ef8311f0495383bbb6351df80aafb8f979dea93e6174e82a |
| Candidate executable SHA-256 | 182bf7c98de925f374b26bccf279507fe89f91c22ef2de9ff5f151cfe3b77f38 |
| Shared measured Cargo.lock SHA-256 | 599fd3fd5e24d8930fe899a2773131c84433416194b096a682e7450add4016d4 |
| 0.6.0 metadata Cargo.lock SHA-256 | ef4c199b359baeca53be4c5d634a288ab4c7e00e1590c8d0948b186e5c5c316a |
| Initial metadata task PR | [#74](https://github.com/yakisoba0728/vrfkit/pull/74) |
| Durable primary/profile evidence checkpoint | [813329b7434411de92d1a70ac3d6cbf247ff678f](https://github.com/yakisoba0728/vrfkit/tree/813329b7434411de92d1a70ac3d6cbf247ff678f/docs/evidence/phase_d/benchmark) |

The baseline already includes the exact FocusProjectiles route and two required
counters. The comparison measures the remaining C-only-to-accepted difference.
It does not measure all Phase A/B improvements, the cost of FocusProjectiles,
the current public 0.5.0 release versus 0.6.0, or one isolated feature.

Both measured executables are fresh default release builds, not the old
test-profile Phase C QA executable. Initially absent separate target directories,
Rust/Cargo 1.86.0, an explicit x86_64-unknown-linux-gnu target, locked dependencies
and the existing optimization-3/fat-LTO/one-codegen-unit/debug-1 profile were used.
The freeze helper checked newly compiled, non-test compiler artifacts for all
ten production crates and copied/hash-checked immutable executables. Independent
QA reconciled 193 baseline and 202 candidate tracked source files plus 11
compiler artifacts per build, including the CLI library/binary distinction.

The later 0.6.0 metadata changes exactly ten production package versions and
the workspace version. Dependency resolutions, release profile and 353
runtime/build/workflow/tool files match the measured candidate. The final
receipt must recheck that correspondence after all documentation changes.
The measured source and binary identities above are retained, not relabeled
as a future integration execution.

## Environment and protocol

The local executor exposed an AMD EPYC 9V74 and nine available logical CPUs,
Debian 13.6, Linux 6.18.57, GCC 14.2.0 and GNU ld 2.44. It reported
10,451,427,328 bytes of memory; the enforced cgroup CPU/memory quota was not
available. Inputs and outputs used the same overlay filesystem. The initial
119 MiB of free disk was recovered to about 6.7 GiB by removing four verified,
reproducible old vrfkit Cargo target caches. Replay originals, sources, frozen
binaries and prior evidence were preserved.

These are warm filesystem-cache, same-session measurements. There was no
cold-cache reset, fsync timing, CPU affinity/power-state tuning or exclusive-host
guarantee. Other local project workers held heavy work during the declared
measurement windows. No user-PC or GitHub-hosted-runner performance is claimed.

The [predeclared protocol](evidence/phase_d/benchmark/protocol.json) selected
12.05, 12.09 and 13.00, each separately main-only and checkpoint-inclusive.
The [input origins](evidence/phase_d/input-origins.json) give fixed public source
revisions and verified hashes. Twelve untimed exports first qualified all six
cases. Every timing series then used one warmup pair and six measured pairs,
alternating AB and BA, with the same-executable A/A control preceding A/B.

The complete primary matrix is **12 valid series, 168 exports: 24 warmups and
144 measured executions**, plus the 12 untimed preflight exports.
There were zero failed exports, no dropped outliers and no parameter changes.
Every execution passed unchanged-input/binary/harness provenance, exact Parquet
hashes, full nonvolatile manifest, mandatory summary and quality-counter gates.
The [preflight receipt](evidence/phase_d/benchmark/preflight.json) and all
individual comparison/control JSON files retain the full evidence.

### Primary measured results

B is the C-only baseline; C is the accepted Phase C candidate. Values in the
resource columns are medians of each metric's individual measured samples.
The paired change is the median of the six candidate/baseline ratios, so it
need not equal the ratio of the two independently calculated medians.
CPU utilization uses one occupied logical CPU = 100%.

| Build | Mode | Wall s B / C | CPU s B / C | CPU % B / C | Peak RSS MiB B / C | MiB/s B / C | Parquet bytes | Paired wall change | A/A wall range |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 12.05 | main | 1.2135 / 1.3180 | 2.5080 / 2.6989 | 202.4 / 205.5 | 74.15 / 74.49 | 43.00 / 39.63 | 31,820,307 | +9.55% | 0.8952 - 1.2842 |
| 12.05 | checkpoints | 1.3439 / 1.5063 | 3.3509 / 3.8871 | 250.9 / 254.8 | 108.47 / 108.94 | 38.83 / 34.69 | 33,843,013 | +3.09% | 0.8833 - 1.1777 |
| 12.09 | main | 1.8039 / 1.8348 | 3.8467 / 3.8799 | 212.9 / 210.8 | 97.35 / 96.47 | 32.40 / 31.73 | 41,345,617 | +1.12% | 0.7718 - 1.0086 |
| 12.09 | checkpoints | 1.6698 / 1.7126 | 4.2840 / 4.3603 | 258.0 / 249.3 | 130.71 / 131.33 | 34.85 / 33.98 | 43,984,122 | +3.21% | 0.8952 - 1.1189 |
| 13.00 | main | 0.0273 / 0.0262 | 0.0250 / 0.0240 | 94.4 / 91.7 | 14.88 / 14.88 | 15.09 / 15.74 | 211,858 | -6.48% | 0.8647 - 1.1680 |
| 13.00 | checkpoints | 0.0472 / 0.0420 | 0.0535 / 0.0484 | 111.5 / 113.1 | 17.53 / 17.65 | 8.76 / 9.81 | 325,675 | -4.78% | 0.9566 - 1.6411 |

| Build | Mode | Replay/s B | Replay/s C |
| --- | --- | ---: | ---: |
| 12.05 | main | 0.8242 | 0.7596 |
| 12.05 | checkpoints | 0.7443 | 0.6649 |
| 12.09 | main | 0.5567 | 0.5453 |
| 12.09 | checkpoints | 0.5989 | 0.5839 |
| 13.00 | main | 36.6318 | 38.2038 |
| 13.00 | checkpoints | 21.2612 | 23.8205 |

**Every primary condition is INCONCLUSIVE for performance superiority.**
The paired differences overlap same-executable variation and change direction
within each series. Observed slower results are retained: the 12.05 main paired
median is +9.55% wall time. That observation is not hidden or converted into
a neutrality claim; the controls do not establish a stable regression or gain.
Separately executed main/checkpoint windows must not be compared to conclude
that doing more checkpoint work is faster.

The [full summary](evidence/phase_d/benchmark/primary-summary.json) retains
CPU/RSS paired changes and all ranges; individual reports retain every pair,
min/max, standard deviation and median absolute deviation. Independent QA
recomputed all 168 records, AB/BA ordering, statistics, provenance and output
fingerprints. All 18 condition-by-metric control comparisons supported the
inconclusive classification. No universal speedup or 2x result is supported.

## Fresh stage diagnosis

A separate diagnostic-only source at
532421879c8dc9c99286e6d606eff5f5572f9d3a produced executable SHA-256
5dd4bc3d8e6f26e4783fc7deb38a21c9151a39f36db00696f7431337e3e6de88.
Three normal controls and three instrumented 12.09 checkpoint exports all
passed the same strict output and provenance gates. The
[patch](evidence/phase_d/benchmark/phase-d-stage-profile.patch),
[raw report](evidence/phase_d/benchmark/profile.json) and
[derived stages](evidence/phase_d/benchmark/profile-summary.json) are preserved.

| Diagnostic participant/path | Median |
| --- | ---: |
| Main parser scheduled CPU | 1.781 s |
| Main fields writer scheduled CPU | 0.960 s |
| Movement writer scheduled CPU | 0.664 s |
| Checkpoint pass scheduled CPU | 0.505 s |
| Checkpoint fields writer scheduled CPU | 0.255 s |
| Prefetch scheduled CPU | 0.342 s |
| ReplayData decompression wall | 0.323 s |
| Checkpoint decompression wall | 0.0242 s |
| Cached ReplayData payload reads wall | 0.0436 s |
| Main fields Arrow construction / Parquet encoding | 194.174 / 693.390 ms |
| Movement Arrow construction / Parquet encoding | 32.015 / 606.079 ms |
| Checkpoint fields Arrow construction / Parquet encoding | 57.451 / 173.251 ms |
| Main / checkpoint raw_bits column construction | 35.601 / 7.710 ms |

Instrumentation changes execution and these wall intervals overlap across
threads; they must not be summed into end-to-end wall time. The scheduled-CPU
source was explicitly probed as available. Dedicated fields/movement/checkpoint
field threads support direct per-column attribution; shared inline-table
column labels only name the context that flushed their thread-local counters.
Payload-read timing is cached read_exact work, not physical storage latency.
Parquet encoding includes its output I/O; isolated disk-write time and per-core
utilization are not measured.

## One bounded optimization experiment

The measured raw_bits column accounted for about 43.31 ms across the two field
writers. The observed main/checkpoint payload averages were about 10.56/9.44
bytes per row. A single experiment reserves 16 bytes per row, capped at the
existing 8,192-row batch bound, instead of starting Arrow's nullable byte
storage at 1,024 bytes. It preserves the same nullable builder, all payload
lengths, row/page/group policy, raw values and schemas.

- Actual local build source: df93e66f48d196ed3a1e4fada5da1ec97e61617e.
- [Remote experiment](https://github.com/yakisoba0728/vrfkit/commit/1848dc9910662d0247b3f1cebaedd939b01fc33b):
  1848dc9910662d0247b3f1cebaedd939b01fc33b, with the identical complete tree
  8bd1c22aaec09b6be89939996c6bc5c5fe11a29d.
- Frozen experiment executable SHA-256:
  e3a93d678ff9d262eed181c70f930b8d8275b24d514e7e5c9c5c06677c3d9eb3.
- All twelve workspace packages were invalidated before reusing a dependency
  target. Fresh runtime artifacts and immutable binaries were verified.
- Author checks: 26 passing tests, zero failed/ignored.
- Independent tests cover both main and checkpoint dictionary writers, null
  versus empty, inline/spilled/large payloads and batch/group boundaries.
  Both compiled semantic mutants were detected; exact restoration passed.
  Four positive test executions and four expected negative failures are
  separate from the production suite count.

A single existing glibc libmemusage diagnostic pair found 932,129 -> 930,558
realloc calls (-1,571), but total requested heap bytes increased
3,094,844,704 -> 3,122,046,678 and its peak increased
122,694,522 -> 126,158,622 bytes. These are instrumented allocator observations,
not process RSS, normal release timings or an efficiency acceptance. All output
hashes still matched. The initial report parser's regex missed glibc's separator;
it was corrected against the already successful output without rerunning that
baseline or deleting an unfavorable sample.

### Normal-release experiment result: withdrawn

A new predeclared comparison used four large-input/mode cases, each with a
contemporary same-executable control followed by a balanced A/B series,
one warmup pair and six measured pairs. The tiny 13.00 input remained a
functional preflight gate because its earlier millisecond timings were noisy.
All six untimed input/mode cases passed (12 exports), then all eight timed
series passed (112 exports: 16 warmups and 96 measured executions).
No failed export, sample deletion, confirmation series or second candidate
occurred. No allocator interposition was active in these normal-release runs.

| Build | Mode | Wall s base / exp | CPU s base / exp | CPU % base / exp | Peak RSS MiB base / exp | MiB/s base / exp | Paired wall | Paired CPU | Paired RSS | A/A wall range |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 12.05 | main | 1.2596 / 1.1740 | 2.6067 / 2.4057 | 207.7 / 202.1 | 74.72 / 74.62 | 41.49 / 44.47 | -5.88% | -9.00% | +0.78% | 0.9725-1.1553 |
| 12.05 | checkpoints | 1.2816 / 1.2366 | 3.1250 / 3.0572 | 240.7 / 256.5 | 108.81 / 108.98 | 40.74 / 42.20 | -4.55% | +0.31% | +0.73% | 0.8575-1.0550 |
| 12.09 | main | 1.6071 / 1.6104 | 3.2296 / 3.2456 | 204.5 / 200.2 | 93.87 / 95.35 | 36.21 / 36.13 | +0.69% | -1.53% | +1.18% | 0.9004-1.1565 |
| 12.09 | checkpoints | 1.6349 / 1.6863 | 4.1406 / 4.1221 | 253.8 / 249.5 | 129.51 / 131.29 | 35.59 / 34.51 | -0.87% | -1.46% | +1.58% | 0.9330-1.1660 |

Every wall-time condition changes direction between pairs and overlaps
contemporary A/A variation. All six 12.09 checkpoint RSS pairs increased,
by +0.67% to +3.11%; the paired median was +1.58%. These observed adverse
memory signals and unconfirmed time benefit do not justify adoption.
The experiment remains on its unmerged branch and is excluded from the
production source. **Disposition: WITHDRAWN_DO_NOT_INTEGRATE; accepted runtime
performance changes: 0.** No additional repeat was warranted.

The [full experiment summary](evidence/phase_d/optimization-measurement/summary.json),
[decision](evidence/phase_d/optimization-measurement/decision.json),
[author disposition](evidence/phase_d/optimization/disposition.json) and
[allocator diagnostic](evidence/phase_d/optimization/allocator.json)
retain all measurements, controls, build identities and limitations.

## Fresh independent functional QA

The retained candidate outputs from the primary preflight were independently
rehash-checked and footer-read: **six outputs across three public inputs,
57 Parquet files and 46 checkpoints**. Independent typed/raw decoding checked
**43,443 values**, with zero width failures or mismatches. Raw RPC loss was zero
in both scopes. Main tables are byte-identical with checkpoints enabled or
disabled.

The current-output FocusProjectiles oracle reconstructs 102 main and 18
checkpoint parents from original raw payloads, verifies exactly 53 ordered
main children and three empty main trailers, and retains all parent bytes.
All nine native and both Focus counters are required; 22 missing-key negative
controls were detected. These are new executions/reads, not the historical
Phase C ten-input/99-checkpoint/61,002-value receipt.

The rejected candidate's retained outputs were also independently checked:
the same 57 files, 46 checkpoints and 43,443 typed values passed. This adds no
unique replay coverage and does not make the rejected source the release
candidate. Independent positive writer tests and two compiled mutants qualify
its preservation property without asserting a performance benefit.

The final hosted log/source/package checks are recorded in the fixed final
receipt. Required coverage includes Rust 1.86/MSRV and stable, Windows/Linux,
all features and 27 feature cases, strict formatting/Clippy/Rustdoc, Python
3.12/3.13, Parquet/PyArrow and the extracted Windows CLI. The unchanged runtime
suite contains 871 Linux / 872 Windows Rust tests and 1,181 Python tests; actual
final job logs must confirm execution, not merely the inventory. Hosted public
fixtures are 12.10, 12.11 and 13.00. With the local corpus, the fresh public-input
union is five distinct inputs/builds, not six; 13.00 overlaps. The final receipt
separates local and hosted tests and records the actual final source artifacts.

## Private corpus and C# scope

The private archive was read-only. The
[aggregate metadata receipt](evidence/phase_d/private-corpus-summary.json)
reconciles all **1,011 catalog rows/assets, 24 builds and 50,938,107,071 bytes**.
All catalog/Release sizes, SHA-256 metadata and build labels agree; zero
missing, extra, duplicate or mismatched entries were found. Catalog content
SHA-256 is df31440dfb181eec4d9371005257e4528f6bfea0a7cef938fb4bf8dc22d674b9.

Classification is **METADATA_ONLY / PRIVATE_BINARY_UNAVAILABLE**: zero downloaded
private assets, zero payload hashes computed, zero private replay/checkpoint
executions. The connector returns metadata and lacks a Release-binary download
method; the local CLI has no authenticated session. One unauthenticated
octet-stream probe returned HTTP 404, which is not treated as proof of archive
authorization denial. No broad retry or full-corpus download was attempted.
Catalog/master remained unchanged, and no private filenames, rows, logs or
URLs enter the public receipts.

The [cross-parser review](evidence/phase_d/cross-parser-summary.json) rechecked
upstream fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8 and latest published
v0.1.0-alpha.2. .NET SDK 10.0.301 is runnable (10.0.404 also installed).
Default complete exports are **NOT_COMPARABLE**: C# skips container Events and
checkpoints, omits undecoded groups/empty shells, emits one latest movement
sample per update, and writes enriched NDJSON rather than the Rust typed/raw
Parquet product. No new C# replay run or speed ratio is claimed.

## Release candidate and remaining evidence gates

The workspace and ten production lockfile entries use unpublished 0.6.0.
The two development tools remain 0.0.0 and unpublished. See the
[release checklist](PHASE_D_RELEASE_CHECKLIST.md),
[migration](PHASE_C_MIGRATION.md) and [changelog](../CHANGELOG.md).
Windows CLI packaging is the distribution target. Rust registry dependencies
and publication policy remain unprepared; Python tools have no distribution
manifest. Neither registry publication is performed.

The following remain explicitly blocked by evidence: six raw replicated
movement classes and rotator/quantization choices, InputEventData,
Ability/GAS FastArray and StopEffectType, section/scalar/OwnerActor/healing
ambiguities, unsupported FocusProjectiles builds, RPC tails, China-region
mappings and complete account/kill/round semantics. No speculative decoder
was added to complete a checklist.

## Execution accounting and stop boundary

Six unique specialist workers were created: A benchmark/profiling, B optimization,
C private corpus, D independent QA, E release/DX and F cross-parser/runtime
contract review. The maximum observed simultaneous worker count was three,
four including the coordinator. One capacity-refused spawn created no worker;
it is not counted as a created or failed agent. No worker crash or task
resumption was recorded. The terminal receipt supplies final completion counts.

A executed 310 full CLI exports: 280 timed-series executions (240 measured,
40 warmups), 24 untimed preflight exports and six control/diagnostic profiling
exports. B executed two additional allocator-instrumented exports. Thus the
local engineering work executed **312 full CLI exports**, with zero export
failures. Repeated runs, synthetic writer tests and repeated reads of retained
outputs do not increase unique corpus coverage. Private replay executions are
zero. No matched C# export experiment was performed.

Two local evidence-harness issues were corrected: a PyArrow dictionary/string
cast in the Focus oracle and the glibc separator parser described above. The
failed harness attempts remain documented; they were not product failures or
omitted unfavorable timing samples. All actual timed observations are retained.

Task PR #74 is the only core Phase D PR. It carries the 0.6.0 metadata,
current-contract corrections and complete evidence. The rejected runtime
experiment remains unmerged. The final integration commit, task/push/PR CI,
source/Windows-package hashes and no-change checks are in the fixed receipt.
Main remains at the starting SHA; PR #58 must remain Open/Draft/Unmerged with
auto-merge disabled. No tag, public Release, registry upload or private-source
mutation is part of this work. Stop after final qualification; a subsequent
main merge or publication requires the user's separate instruction.
