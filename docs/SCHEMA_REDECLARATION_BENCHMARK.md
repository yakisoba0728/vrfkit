# Schema re-declaration benchmark, 2026-11-16

The optimization reduces work when a schema group is repeatedly declared with
the same canonical path and numeric index. **It does not establish faster
ordinary replay exports or lower peak RSS.** The six measured public fixtures
contained zero repeated declarations, including all 76 independent checkpoint
streams. Their complete-export timing results were noisy and changed direction
on confirmation; an identical-binary control showed substantial variation too.

The adopted scope is repeated-declaration API and wire workloads. No RPC hash
change is included. The evidence below keeps the initial slower results, the
confirmation, and the control. No aggregate replay speed ratio is reported.

## Change and correctness contract

The resource policy from [SCHEMA_LIMITS.md](SCHEMA_LIMITS.md) is checked before
either shortcut:

- `declare_export_group` retains an existing group's populated descriptors and
  grows its field vector only when its declared size increases. It avoids the
  temporary empty field vector for an unchanged canonical path and index.
- `add_export_group` still merges supplied populated descriptors. It avoids a
  full lookup-index rebuild only when both the canonical path and index are
  unchanged. A changed spelling, changed index or replacement retains the
  existing rebuild behavior.

Both paths preserve resource accounting and advance `schema_generation` once
per successful registration. A smaller declaration cannot shrink existing
slots. Identity conflicts and budget failures keep their existing errors.
This does not make all declarations allocation-free: incoming path strings,
new groups and vector growth still require storage.

The existing independent model tests now exercise both registration entry
points over every identity transition and 300 fixed-seed sequences of 40 calls
per entry point. The model scans stored identities rather than reusing the
cache's index algorithm, and checks every lookup against a full rebuild.
The resource regression also retains a populated descriptor beyond a smaller
re-declaration, through later growth and a rejected over-budget growth.

Rust 1.86.0 default, core-only and release schema tests passed, along with
Clippy, formatting and strict rustdoc. Default/release runs contain 61 unit,
15 integration and 1 documentation test; core-only runs contain 45 unit,
12 integration and 1 documentation test. The new coverage strengthens existing
tests and does not increase their count.

## What the public fixtures exercise

The [schema census](benchmarks/schema-redeclaration/schema-reuse-census.json)
walked each ReplayData frame and each independent checkpoint's schema tables
and frames. Successful registrations are `schema_generation`; retained groups
are `group_count`. These caches are never cleared during their stream. Equal
counts therefore mean that no registration merged or replaced an earlier
group. Packet payloads were skipped by this diagnostic; complete export
validation was performed separately.

| Build | Main registrations / retained groups | Checkpoints | Checkpoint registrations / retained groups | Re-declarations or identity changes |
| --- | ---: | ---: | ---: | ---: |
| 12.05 | 461 / 461 | 19 | 8,720 / 8,720 | 0 |
| 12.08 | 462 / 462 | 17 | 7,578 / 7,578 | 0 |
| 12.09 | 500 / 500 | 21 | 10,296 / 10,296 | 0 |
| 12.10 | 129 / 129 | 7 | 1,091 / 1,091 | 0 |
| 12.11 | 137 / 137 | 6 | 969 / 969 | 0 |
| 13.00 | 134 / 134 | 6 | 941 / 941 | 0 |

Neither shortcut executes on these fixtures. The census does not describe all
replay versions or the private corpus. It prevents attributing their ordinary
export times to a repeated-declaration workload they do not contain.

## Focused repeated-declaration measurement

The public [registration benchmark example](../crates/vrf-schema/examples/registration_bench.rs)
initializes 512 groups with 32 slots each, then performs 4,096 declarations of
the same identities. `add` supplies an empty preallocated group on every call;
`declare` uses the wire reader's admission-before-allocation entry point.
The timed Rust loop excludes initialization and final identity checks.
It does not time wire decoding itself. Live and checkpoint wire readers call
the same declaration entry point, so they receive the reduction in registration
work when identities repeat; no complete malformed-replay throughput result is
claimed.

Both versions use the identical example and Cargo lockfile, Rust 1.86.0 and
the workspace release profile: optimization level 3, fat LTO, one codegen unit,
and debug information level 1. One warmup pair precedes nine measured pairs;
the execution order alternates each pair. Each process is launched in a fresh
measurement worker. Other agents' builds and test suites were paused for the
measurement window; this remains a shared Linux environment.

| Mode | Baseline loop median | Candidate loop median | Baseline loop sample SD | Candidate loop sample SD |
| --- | ---: | ---: | ---: | ---: |
| `add` | 218.976432 ms | 0.467511 ms | 12.323302 ms | 0.041863 ms |
| `declare` | 224.664727 ms | 0.242902 ms | 32.608030 ms | 0.039175 ms |

The whole-process measurements include initialization, checks and process
startup, so they have a different scope from the timed registration loop:

| Mode | Wall median, baseline / candidate | Total CPU median, baseline / candidate | Peak RSS median, baseline / candidate |
| --- | ---: | ---: | ---: |
| `add` | 0.222499 / 0.002871 s | 0.221933 / 0.002612 s | 15,597,568 / 15,593,472 bytes |
| `declare` | 0.229247 / 0.002269 s | 0.228413 / 0.001986 s | 15,593,472 / 15,593,472 bytes |

RSS is effectively unchanged at this measurement scale. Short candidate
processes include substantial startup overhead; lower CPU utilization alone
is not treated as an efficiency result. The evidence reports user and system
CPU separately, total CPU, utilization, RSS and OS I/O block counters. Allocation
counts, allocated bytes and separate I/O time were not measured.

All samples, paired ratios, medians, sample standard deviations, median
absolute deviations and source/executable/harness hashes are in
[schema-reuse-measurements.json](benchmarks/schema-redeclaration/schema-reuse-measurements.json).
These timings apply to the repeated-declaration loop and must not be used as
a replay-parser or competitor speed ratio.

## Complete exports: initial six-fixture series

Each public fixture was exported with checkpoints using one warmup pair and
five measured pairs. The comparison checks exact Parquet bytes, all nonvolatile
manifest values and required counters. Every run passed those checks; output
scope and quality requirements were identical.

The table shows wall medians and sample standard deviations. A positive paired
change means that the candidate took more time. Paired changes are medians of
the five per-pair ratios, not ratios of the two medians.

| Build and complete evidence | Baseline wall median (SD), s | Candidate wall median (SD), s | Paired wall change | Paired total CPU change |
| --- | ---: | ---: | ---: | ---: |
| [12.05](benchmarks/schema-redeclaration/schema-reuse-1205-checkpoints.json) | 1.152865 (0.050375) | 1.230069 (0.039844) | +8.96% | +5.39% |
| [12.08](benchmarks/schema-redeclaration/schema-reuse-1208-checkpoints.json) | 1.195183 (0.063057) | 1.224378 (0.076874) | +0.38% | -4.66% |
| [12.09](benchmarks/schema-redeclaration/schema-reuse-1209-checkpoints.json) | 1.566604 (0.126851) | 1.732380 (0.104389) | +3.09% | +9.04% |
| [12.10](benchmarks/schema-redeclaration/schema-reuse-1210-checkpoints.json) | 0.042400 (0.004786) | 0.044891 (0.053633) | +7.52% | +23.50% |
| [12.11](benchmarks/schema-redeclaration/schema-reuse-1211-checkpoints.json) | 0.043186 (0.002396) | 0.042198 (0.072390) | -2.31% | -5.33% |
| [13.00](benchmarks/schema-redeclaration/schema-reuse-1300-checkpoints.json) | 0.038050 (0.004393) | 0.035157 (0.000729) | -6.27% | -12.78% |

CPU time and memory are separate measurements:

| Build | Total CPU median, baseline / candidate, s | Peak RSS median, baseline / candidate, bytes |
| --- | ---: | ---: |
| 12.05 | 2.851475 / 3.046389 | 169,422,848 / 166,551,552 |
| 12.08 | 2.904398 / 2.839189 | 160,616,448 / 160,387,072 |
| 12.09 | 3.783737 / 4.280686 | 196,321,280 / 194,363,392 |
| 12.10 | 0.053033 / 0.062751 | 27,783,168 / 28,065,792 |
| 12.11 | 0.043406 / 0.041250 | 20,357,120 / 20,258,816 |
| 13.00 | 0.042370 / 0.036953 | 18,530,304 / 18,370,560 |

The linked reports retain dispersion for CPU, RSS and every other metric too.
Small fixtures had individual timing outliers over three times their paired
baseline; the sample standard deviations retain their effect.

## Narrow confirmation and identical-binary control

The initial 12.05 slowdown prompted one further series, followed by a control
using the **same baseline executable in both positions**. Each used two warmup
pairs and seven measured pairs in a coordinated quiet window. No further timing
was attempted after these results showed the environment's variability.

| Series | Paired median wall change | Individual wall-pair range | Paired median total CPU change |
| --- | ---: | ---: | ---: |
| [12.05 confirmation](benchmarks/schema-redeclaration/schema-reuse-1205-confirm.json) | -9.78% | -38.98% to +20.95% | -13.96% |
| [Identical-binary control](benchmarks/schema-redeclaration/schema-reuse-1205-control.json) | +7.86% | -11.06% to +17.37% | +7.25% |

The confirmation reverses the initial slowdown, while the identical binary
also produces a sizable apparent difference. These series establish neither
an ordinary-replay throughput benefit nor a reproducible ordinary-replay
regression. They do not establish a peak-RSS benefit either. A controlled runner
would be needed to resolve smaller end-to-end changes.

All 108 complete export processes across the initial, confirmation and control
series passed the same byte/counter equivalence checks. They cover six unique
fixtures, not 108 unique replays. Each export contains the same 13 Parquet
tables; source replay files were opened for reading only.

## Reproduction and provenance

Start with the [source reconstruction guide](benchmarks/schema-redeclaration/README.md).
It reconstructs all three measured source snapshots from public commit
`67c0a2d94fbcbeeefa56761f1bc13d9f6c829fb0` and retained text patches, checking
their expected Git tree hashes. **No historical local commit object is
required.** The patches, exact measurement scripts and helper modules are
stored with SHA-256 values in
[reconstruction.json](benchmarks/schema-redeclaration/reconstruction.json).

From a fresh checkout containing this report, the portable preparation command
creates new detached worktrees and verifies their complete source trees:

```sh
python3 docs/benchmarks/schema-redeclaration/reconstruct_sources.py \
  --repo . --out ../vrfkit-schema-reproduction
```

It does not build or run timing. Exact archived orchestration scripts retain
their original workspace prefix; only the `ROOT` assignment is relocated in
new copies. The preparation receipt records their new hashes, while the
archived originals continue to match the historical reports. The linked guide
provides the complete build and measurement commands for those worktrees.

The JSON evidence includes the actual source, executable, Cargo lockfile and
measurement-harness hashes. The focused baseline was compiled from local
source snapshot `470372e325c6197699b6e507dd14d3f8ceb03753`; the candidate was
`dc31cd57b2564a812b84868e21fef55469419730`. These identify the local measurement
snapshots; publishing and integration commits can differ. The recorded file
hashes identify the source contents independently of commit metadata.

The full-export baseline executable was already built from `90aa7539963973c2b4c7505bfc8c7aa8ee4b5fcb`.
Its only difference from `470372e` is a Rust documentation comment; its runtime
source is equivalent. Its executable hash is recorded in each full-export
report. No benchmark report, schema census or command below contains player
identifiers or source replay filenames.

Build the identical registration example in both source checkouts, with the
same lockfile and toolchain; the earlier checkout needs the example copied
from the candidate:

```sh
cargo +1.86.0 build --release --locked -p vrf-schema --example registration_bench
target/release/examples/registration_bench add
target/release/examples/registration_bench declare
```

Measure the executable directly, excluding `cargo` startup. Use one warmup pair
and nine measured pairs, alternating baseline/candidate order. The example's
`seconds` field measures only the registration loop. Fresh workers from
`tools/benchmark_resources.py` recorded whole-process resources separately.

For complete exports, use the resource comparison tool from an integration
checkout containing that tool; replace the executable and input placeholders:

```sh
python tools/benchmark_resources.py \
  --baseline-exe BASELINE_VRFKIT --candidate-exe CANDIDATE_VRFKIT \
  --replay PUBLIC_FIXTURE --checkpoints \
  --warmups 1 --repeats 5 --work-dir NEW_WORK_DIR --output NEW_REPORT.json
```

For the narrow confirmation, use `--warmups 2 --repeats 7`. For the control,
pass the same baseline executable to both executable options. Preserve all
reports; do not select only the faster pairs.

The exact [schema census source](benchmarks/schema-redeclaration/schema_declaration_counts.rs)
is retained with its source and executable hashes. In a throwaway checkout,
copy it to `crates/vrfkit/examples/schema_declaration_counts.rs`, then run:

```sh
cargo +1.86.0 build --release --locked -p vrfkit --no-default-features \
  --example schema_declaration_counts
target/release/examples/schema_declaration_counts PUBLIC_FIXTURE
```

This diagnostic prints aggregate schema counts only. It is not a replacement
for packet decoding, semantic verification or the complete-export checks.
