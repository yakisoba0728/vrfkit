# vrfkit

A Rust toolkit that parses VALORANT replay files (`.vrf`, Unreal Engine network
replay format), streams records through a Rust API and exports them to Parquet. A workspace of 10 crates (plus a
test-only `dev/vrf-testkit` and the `extract-component-classes` tool) and a
Python `tools/` validation suite. Every workspace crate forbids unsafe Rust;
that policy does not extend to third-party dependencies. Oodle-format
decompression uses the Rust `oozextract` implementation. Default Parquet
output uses native Zstd through `zstd-sys`. Edition 2024, MSRV 1.86, Apache-2.0.

![CI](https://github.com/yakisoba0728/vrfkit/actions/workflows/ci.yml/badge.svg)
![license](https://img.shields.io/badge/license-Apache--2.0-blue.svg)
![rust](https://img.shields.io/badge/rust-1.86%2B-orange.svg)
![edition](https://img.shields.io/badge/edition-2024-orange.svg)
![builds](https://img.shields.io/badge/builds-11.06--13.10-yellow.svg)
![workspace unsafe policy](https://img.shields.io/badge/workspace%20unsafe-forbidden-success.svg)

Derived from [ValorantReplayParser](https://github.com/michel-giehl/ValorantReplayParser)
(MIT), with the original reference at
[`2e4d3ad`](https://github.com/michel-giehl/ValorantReplayParser/commit/2e4d3aded2e892cda21e0a66c4b55c73b3b7a07f)
(2026-09-30). Subsequent upstream changes through
[`09a8e1e`](https://github.com/michel-giehl/ValorantReplayParser/commit/09a8e1e1d81c48840b8810f523f1595e539b7d55)
(2026-10-27) are inventoried in
[`docs/UPSTREAM_AUDIT_2026_10.md`](docs/UPSTREAM_AUDIT_2026_10.md), and through
[`400f729`](https://github.com/michel-giehl/ValorantReplayParser/commit/400f7295964a7f6486378eac6decda3cefe42acb)
(2026-11-07) in [`docs/UPSTREAM_AUDIT_2026_11.md`](docs/UPSTREAM_AUDIT_2026_11.md),
and through [`fbf24b6`](https://github.com/michel-giehl/ValorantReplayParser/commit/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8)
(2026-11-11) in [`docs/UPSTREAM_AUDIT_2026_11_16.md`](docs/UPSTREAM_AUDIT_2026_11_16.md);
only measured compatible behaviors are ported. See [`NOTICE.md`](NOTICE.md).
Not affiliated with, endorsed by, or approved by Riot Games.

**Test suites (2026-11-16):** **792 Rust tests (Windows)**,
**791 Rust tests (non-Windows)** and **1171 Python tests**.
The historical audit of the 24 builds through 13.06 used the same verification
on **1,018 unique replays**; all **1,018** meet every strict criterion. 13.08 was
checked the same way on 3 replays, and 2 meet every criterion; all 5
13.09 and all 28 13.10 replays meet every strict criterion. See
[build verification](docs/BUILD_VERIFICATION.md) for the measured scope, common
checks and remaining limits.

- Rust callbacks and ownership: [`docs/RUST_API.md`](docs/RUST_API.md)
- JSONL conversion and encodings: [`docs/JSONL.md`](docs/JSONL.md)
- Current engineering evidence and limitations: [`docs/OVERHAUL_RESULTS_2026_11_16.md`](docs/OVERHAUL_RESULTS_2026_11_16.md)
- Run it, every tool and every output column: [`docs/USAGE.md`](docs/USAGE.md)
- What's extractable, and whether it is typed: [`docs/DATA.md`](docs/DATA.md)
- Build it, test it, open a PR: [`CONTRIBUTING.md`](CONTRIBUTING.md) (agents: [`CLAUDE.md`](CLAUDE.md))

## Quick start

```bash
# From the repository root; install the pinned toolchain once if needed.
rustup toolchain install 1.86.0 --profile minimal
cargo +1.86.0 build --release -p vrfkit --locked

./target/release/vrfkit inspect match.vrf
./target/release/vrfkit validate match.vrf --diagnostics
./target/release/vrfkit export match.vrf --out match-export --checkpoints
```

On Windows PowerShell, use `.\target\release\vrfkit.exe` for the executable.
A source build does not automatically add the binary to `PATH`. The Rust API
and JSONL additions are available from source; see tagged releases for the
features of a particular packaged version.

`inspect` parses container metadata and lists chunks without decoding packets.
`validate` walks ReplayData content blocks and reports its structural verdict;
it writes no files and does not establish checkpoint or semantic correctness.
`export` writes the Parquet tables and manifest described under [Output](#output) into
`--out`, which must be new, empty or a previous export: anything else in it
is refused, never deleted.

> Column schemas, the `tools/` scripts, the full validation suite, and
> per-crate usage live in [`docs/USAGE.md`](docs/USAGE.md).
> This document is about *why it is built this way*.

### Rust applications

Use `ReplayReader` for main-stream records and native Event payloads without
writing Parquet. It borrows callback batches, preserves raw windows and returns
quality counters. `read_path` buffers the input file; checkpoint snapshots are
reported as skipped. The [Rust API guide](docs/RUST_API.md) covers ownership,
errors, source dependencies and quality checks. Run its aggregate-only example:

```bash
cargo +1.86.0 run --release -p vrfkit --no-default-features --example stream_replay -- match.vrf
```

## Why this exists

vrfkit preserves raw replicated-property windows alongside decoded values.
In framed RepLayout streams, a field carries a handle and bit length before
its value, so a known field boundary can be preserved without knowing its
type. The replay's `NetFieldExportGroup` declarations resolve handles to names
where that schema is available. Unknown properties retain raw payloads, and
unresolved RPCs can retain their whole payload; typed values are an additive
overlay.

This is a preservation strategy with explicit accounting. Framing failures,
missing schema attribution and semantic coverage remain separate measurements.
Successful movement decodes can be represented by their rows instead of a
duplicate raw RPC. See the [current validation evidence](docs/OVERHAUL_RESULTS_2026_11_16.md)
for the tested inputs, exact-output comparisons and remaining unknowns.

## Supported VALORANT builds

The table below retains the historical, revision-specific audits linked in
[build verification](docs/BUILD_VERIFICATION.md). This overhaul freshly checked
six public inputs from 12.05, 12.08, 12.09, 12.10, 12.11 and 13.00; it did not
rerun the private corpus. See the [current results](docs/OVERHAUL_RESULTS_2026_11_16.md)
for the tested source, unchanged output and remaining interpretation limits.

| 🎮 Build | 🌿 Branch | ✅ Clean/checked | 🔎 Verified by |
|---|---|---:|---|
| **13.10** | `release-13.10` | 28/28 | Validation + checkpoints + typed/raw |
| **13.09** | `release-13.09` | 5/5 | Validation + checkpoints + typed/raw |
| **13.08** | `release-13.08` | 2/3 | Validation + checkpoints + typed/raw |
| **13.06** | `release-13.06` | 38/38 | Validation + checkpoints + typed/raw |
| **13.05** | `release-13.05` | 401/401 | Validation + checkpoints + typed/raw |
| **13.04** | `release-13.04` | 108/108 | Validation + checkpoints + typed/raw |
| **13.02** | `release-13.02` | 205/205 | Validation + checkpoints + typed/raw |
| **13.01** | `release-13.01` | 215/215 | Validation + checkpoints + typed/raw |
| **13.00** | `release-13.00` | 1/1 | Validation + checkpoints + typed/raw |
| **12.11** | `release-12.11` | 1/1 | Validation + checkpoints + typed/raw |
| **12.10** | `release-12.10` | 1/1 | Validation + checkpoints + typed/raw |
| **12.09** | `release-12.09` | 3/3 | Validation + checkpoints + typed/raw |
| **12.08** | `release-12.08` | 3/3 | Validation + checkpoints + typed/raw |
| **12.07** | `release-12.07` | 3/3 | Validation + checkpoints + typed/raw |
| **12.06** | `release-12.06` | 3/3 | Validation + checkpoints + typed/raw |
| **12.05** | `release-12.05` | 3/3 | Validation + checkpoints + typed/raw |
| **12.04** | `release-12.04` | 3/3 | Validation + checkpoints + typed/raw |
| **12.03** | `release-12.03` | 3/3 | Validation + checkpoints + typed/raw |
| **12.02** | `release-12.02` | 3/3 | Validation + checkpoints + typed/raw |
| **12.01** | `release-12.01` | 3/3 | Validation + checkpoints + typed/raw |
| **12.00** | `release-12.00` | 3/3 | Validation + checkpoints + typed/raw |
| **11.11** | `release-11.11` | 3/3 | Validation + checkpoints + typed/raw |
| **11.10** | `release-11.10` | 3/3 | Validation + checkpoints + typed/raw |
| **11.09** | `release-11.09` | 3/3 | Validation + checkpoints + typed/raw |
| **11.08** | `release-11.08` | 3/3 | Validation + checkpoints + typed/raw |
| **11.07** | `release-11.07` | 3/3 | Validation + checkpoints + typed/raw |
| **11.06** | `release-11.06` | 3/3 | Validation + checkpoints + typed/raw |

Measured on all **1,018 unique available replays** across the 24
branches through 13.06, plus 3 13.08, 5 13.09 and 28 13.10 replays in separate runs, every row by the
[same acceptance rule](docs/BUILD_VERIFICATION.md):
ReplayData validation, checkpoint-enabled export, the independent comparisons on
observed evidence fields, and the strict array and array-leaf error counters.
✅ **1,018/1,018** through 13.06 are clean -- not a claim that every field is
understood. The 13.08 replay that is not clean fails the checkpoint GUID
cross-check on a name that differs only in case, and the audit stops there for
it ([details](docs/BUILD_VERIFICATION.md#1308)).
The 13.10 transform passes the block oracle, and the CombatReport and damage
RPC array layouts decode with zero main and checkpoint array leaf errors
([details](docs/BUILD_VERIFICATION.md#1310)).
Structured-array child rows are admitted per build and per route: all
measured routes on 13.01--13.06 and 13.09, all but the projectile path on 13.08, a measured subset on 11.06--13.00
([legacy route table](docs/LEGACY_BUILD_SUPPORT.md#measured-array-routes)).

All branches are `++Ares-Core+release-<build>`. Adding a build is one
`transforms!` line; see [Adding a new build](#supported-builds-and-the-cost-of-a-new-build).

## Highlights

- **Preservation with explicit accounting** — unknown property payloads and
  unresolved whole RPCs remain raw; stream failures remain visible. Typed
  values are an *additive* overlay, and the nullable `compatible_checksum`
  column helps distinguish untyped fields from fields lacking a descriptor.
- **Self-describing stream** — field names come from the replay itself
  (`NetFieldExportGroup`); no hardcoded agent or map names in the parser.
- **Main and checkpoint Parquet output** — six main tables are always written;
  `--checkpoints` adds seven checkpoint tables. All are ready for polars,
  pandas, or DuckDB.
- **Spike state** — plant site A/B (`PlantedAtSite` + position), defuser
  (`CurrentDefuser`), timer, and the canonical detonation signal.
- **Combat & abilities** — per-player economy, magazine and reserve ammo,
  equipped weapon over time, cooldowns, and absolute health, armour and overheal
  from the damage log — the value after each change, not a running subtraction.
- **Ability observations** — cast time/location and replicated ability
  statistics can be joined to actors and rounds. Repeated snapshots require
  deduplication; unresolved ownership and incomplete effect pairs remain gaps.
- **Numeric FastArray observations** — the standalone GAS extractor retains
  replication keys, deleted/changed item IDs and raw property boundaries; every
  window it reads in the 1,018-export audit closed exactly
  ([investigation](docs/GAS_AND_PATCHVOLUME_INVESTIGATION.md)). Property names
  and gameplay meanings remain unverified, and this output is separate from
  Parquet typed-value coverage.
- **Status-effect observations** — nearsight, slow, detain and suppress can
  arrive on affected actors. Matched start/stop records support intervals;
  unmatched records must not be assigned an invented duration.
- **Persistent effects** — smoke / wall / molly / slow / trap position and
  lifetime from actor lifecycles; one command via
  `tools/extract_active_effects.py`.
- **Spike custody** — who carried the spike and when, resolved to the account
  UUID, including Gekko's Wingman as a proxy carrier and the planter at each
  `spikePlanted`; one command via `tools/extract_spike_carrier.py`.
- **Account identity & typed events** — `manifest.players` (account UUID →
  actor → character), event `word0`/`word1` (killer/killed NetGUID, round
  index), ping/latency.
- **Cross-validated** -- the server-written Event chunk independently confirms
  the kill log: 132/132 in order on one 13.01 replay, 9,677/9,677 over 71
  replays on 13.02.
- **Reproducible** — Parquet output is byte-for-byte identical run to run.
- **Rust reader API** -- synchronous borrowed record batches and native Event
  payloads through the same main-stream decoder, with explicit quality counters;
  see the [library guide](docs/RUST_API.md).
- **Bounded schema declarations** -- configurable group and field-slot budgets
  reject excess declarations before allocating dense slots; see
  [schema resource limits](docs/SCHEMA_LIMITS.md) for the scope and migration.
- **Workspace unsafe-code policy** — `#![forbid(unsafe_code)]` in every crate.
  Dependencies have separate policies; default Parquet compression includes
  native Zstd. The core-only Reader excludes Arrow, Parquet and Zstd.
- **792 Rust tests (Windows)** / **791 Rust tests (non-Windows)** plus
  a layered validation suite (framing / bytes / decode
  errors / semantics).

## Table of contents

- [Supported VALORANT builds](#supported-valorant-builds)
- [Highlights](#highlights)
- [Extractable data](docs/DATA.md) — the full inventory of what each table carries
- [Quick start](#quick-start)
- [Output](#output)
- [Status](#status)
- [The Event chunk -- the server's own timeline](#the-event-chunk----the-servers-own-timeline)
- [Whole-corpus robustness](#whole-corpus-robustness)
- [Type overlay](#type-overlay)
- [Supported builds and the cost of a new build](#supported-builds-and-the-cost-of-a-new-build)
- [Design](#design)
- [Generated files](#generated-files)
- [License](#license)

## Output

The main export writes six Parquet tables plus `manifest.json`; `--checkpoints`
adds seven checkpoint tables. The tables, their columns, the join rules and
the traps (a per-round tick, `open` / `close` / `dormant`, an `FDateTime`
timestamp) are in [`docs/USAGE.md` section 3](docs/USAGE.md#3-output).

### JSONL for Python and other consumers

Convert any existing main or checkpoint Parquet table with selected columns:

```bash
python -m pip install -r requirements.txt
python tools/export_jsonl.py --export match-export --table fields --out fields.jsonl
```

The [JSONL format](docs/JSONL.md) includes a schema header, exact integer tokens,
hexadecimal binary values and explicit nulls. Conversion uses bounded row batches
and publishes the completed file atomically. It preserves selected values and
row order; it does not infer gameplay events or retain Parquet physical encoding.

## Status

Work in progress. Currently verified: `cargo +1.86.0 test --workspace --locked`
**791 passing (non-Windows)**; the Windows suite contains **792 tests (Windows)**.
The full Python suite has **1171 passing** tests. The documentation guard
measures the executing platform's suites. The historical [common build audit](docs/BUILD_VERIFICATION.md)
records replay validation, checkpoint export and independent value checks for
each supported build.

Tagged Windows releases provide a ZIP and SHA-256 checksum. The release
workflow runs the complete CI on the tagged commit, checks the packaged binary
against a public replay, and publishes the verified ZIP without rebuilding it.
The same packaging checks run on PRs. See [release maintenance](CONTRIBUTING.md#tagged-windows-releases)
for tag naming and validation before publishing.

What CI runs, three pinned public replays included, is in
[CONTRIBUTING.md](CONTRIBUTING.md#what-ci-runs); the private-corpus checks stay
local.

The crates, their layers and feature flags are in
[`docs/USAGE.md` section 4](docs/USAGE.md#4-using-it-as-a-library), and the
layered validation suite and what each layer misses in
[section 6](docs/USAGE.md#6-validation-suite). Timing
is `tools/bench_export.py`. For paired CPU, memory and exact-output comparisons,
see [resource benchmarks](docs/RESOURCE_BENCHMARKS.md). The optimizations kept
and rejected, with their measurements, are in
[`docs/PERFORMANCE_NOTES.md`](docs/PERFORMANCE_NOTES.md).

Same-input comparisons with the pinned C# reference are documented in
[reference comparison scope](docs/REFERENCE_COMPARISON.md): exact movement
identities, count units, unresolved matches and differences in exported work.
Neither parser is treated as ground truth, and no equal-work speed ratio is
claimed from their default exports.

## The Event chunk -- the server's own timeline

The `.vrf` Event chunk is the event list the server labeled and wrote itself,
stored elsewhere in the file under a different encoding. vrfkit reads it.

```
characterDeath 132 | characterUltimateUsed 34 | roundStarted 18
spikePlanted 9 | spikeDefused 1 | switchTeams 1          (02d4d478, 195 events)
```

The 132 `characterDeath` events exactly match the 132 `MulticastNotifyKilledEnemy`
events we extracted from RPCs. The two payload words are the killer/killed
NetGUIDs, and **132/132 match in order** (0/132 matched reversed).

Scope, to be precise: the killer/killed pair diff is for **one replay**. A
separate Event-only sweep covered **527 files and 109,126 chunks** across
releases 13.01, 13.02 and 13.04. Every outer chunk and known inner payload was
consumed exactly, with zero unknown groups and zero residual bytes.

The payload layout is `[u32 group tag][N x u32 words][FString
"EReplayEventGroup::<Name>"][f32 seconds]`, and `N` differs per group. The `N`
for all seven groups is derived from the corpus as the residual-zero count
(CharacterDeath = 2, CharacterUltimateUsed / RoundStart / SwitchTeams = 1,
SpikePlanted / Defused / Exploded = 0). The same sweep established one stable
tag per group (respectively 8, 11, 2, 3, 4, 5 and 6), public enum-name strings
of at most 40 bytes, and a maximum `payload_seconds * 1000 - time1` absolute
difference of 0.999878 ms. vrfkit requires all four checks within a 1.001 ms
time tolerance before exporting `word0`/`word1`, `payload_tag`, `payload_name`
and `payload_seconds`; on any mismatch those nullable columns stay empty. The
original is always left intact in `raw_payload`.

## Whole-corpus robustness

The [common audit](docs/BUILD_VERIFICATION.md) checks 1,018 unique replays
across 24 builds: every ReplayData validation and checkpoint export succeeds,
independent typed/raw comparisons match 13,387,751 observed values, and all
1,018 pass the strict quality gate.

A 100% block pass rate means every measured block reached an ordinary
field/RPC row or an explicit preservation row; partial reassembly rejections
are outside that score, and `malformed framing 0` does not prove that earlier
transport stages kept every payload. It is **not** a typing claim either: a
block that cannot be assigned a `_ClassNetCache` group has inner handles
nothing can name, so it becomes one reserved row (`handle = u32::MAX`, the
complete decoded payload in `raw_bits`) counted under `RPC unresolved/raw` --
uninterpreted, not lost.

## Type overlay

For any field whose inner stream can be walked, the raw bits are always
exported; when the type is known, the `value_*` columns are filled **as an
overlay.** If the type is unknown, or decoding fails, the row's `raw_bits`
remains. The only exception is a ClassNetCache block whose group cannot be
identified: it cannot be expanded into fields, so it emits one preservation
row (`handle` = `u32::MAX`, full payload in `raw_bits`) and an explicit
unresolved/raw diagnostic rather than pretending the properties were decoded.

The overlay table (`crates/vrf-decode/src/table.rs`) -- 224 groups, 1,124
entries, 96 handles -- is maintained in the repository, and
`tools/apply_type_corrections.py --check` keeps every measured type in it.
Four names resolve without a table entry: `Owner`, `Instigator`,
`AttachParent` and `Controller` are object references the engine replicates
on every actor, always as a NetGUID, so they resolve by name after the table
and the scoped types miss -- a claim about Unreal, not about one Blueprint.

`02d4d478` (`02d4d478-1dfb-4412-9a77-29ca29105a9d.vrf`), as recorded by the
committed export baseline `tools/baselines/export_02d4d478.json`:

```
Decoded OK:   822,251      Decode errors:      0
Raw/Skip:      24,747      Not in table: 140,748
No field name:  1,249      Typed:          83.1%
Effect blobs:  61,617
```

The four buckets partition `Rows offered` exactly (822,251 + 24,747 + 140,748 +
1,249 = 988,995), and `Typed` is `Decoded OK / Rows offered`. `check_docs.py`
compares these counters and USAGE's Parquet row/byte table with that baseline.

**Effect decoding is additive and does not move these buckets.** The overlay
buckets are settled before the effect pass, so rows that gained a value from an
effect are still counted under `Not in table`; merging them into `Decoded OK`
would double-count and move the baseline for unrelated reasons. `Effect blobs`
is reported separately -- without it, 61,617 rows gain a value yet the summary
prints identically.

Physical value coverage -- `fields.parquet` rows with a non-null `value_*` --
is a different population from the overlay's input rows: the reference
baseline has 939,474 typed rows out of 1,296,660 (72.45%), measured from its
columns, and it is not a fraction of all game information understood.
`Typed`, the overlay counter, the `compatible_checksum` buckets that separate
"nobody described this" from "we missed this", and
`tools/summarize_value_coverage.py` for your own exports are in
[`docs/USAGE.md`](docs/USAGE.md#fieldsparquet). Decode errors are checked
separately by `tools/check_decode_errors_corpus.py`, because `vrfkit validate`
prints no overlay counters.

## Supported builds and the cost of a new build

The payload transform changes per game build, but far more is **constant**
across registered releases 11.06 through 13.10: the PRNG and its multipliers, the seed-mix
skeleton, the 64 -> 32 -> 8 -> tail staging, the tail-XOR handling, and even
the S-box table itself. Each build supplies its own `word64` / `word32` /
`byte` operation sequence (order, rotations, complements, and whether the
S-box stage is used) plus these constants:

| | seed addend | offset | sign | S-box |
|---|---|---|---|---|
| release-11.06 | `0x3325e3bd` | `0x3d` | **+** | used |
| release-11.07 | `0x17b077d3` | `0x2d` | - | used |
| release-11.08 | `0xacf2cdff` | `0x01` | - | unused |
| release-11.09 | `0x12cf14e5` | `0x1b` | - | used |
| release-11.10 | `0x34e9d3ec` | `0x14` | - | used |
| release-11.11 | `0xc4445c41` | `0x3f` | - | used |
| release-12.00 | `0x70876679` | `0x07` | - | unused |
| release-12.01 | `0x13fdd831` | `0x31` | **+** | used |
| release-12.02 | `0x9830d09d` | `0x1d` | **+** | used |
| release-12.03 | `0x33d59dff` | `0x01` | - | used |
| release-12.04 | `0xa5684b42` | `0x3e` | - | used |
| release-12.05 | `0xc21d548c` | `0x0c` | **+** | used |
| release-12.06 | `0x8d686ca6` | `0x26` | **+** | unused |
| release-12.07 | `0x2d21d7c3` | `0x3d` | - | used |
| release-12.08 | `0xce2e33e5` | `0x1b` | - | used |
| release-12.09 | `0x7ff2feec` | `0x14` | - | unused |
| release-12.10 | `0x12fd0ee5` | `0x1b` | - | unused |
| release-12.11 | `0x409d36a3` | `0x23` | **+** | unused |
| release-13.00 | `0x2949b6ef` | `0x11` | - | used |
| release-13.01 | `0xe62fcd5c` | `0x24` | - | unused |
| release-13.02 | `0x9e81a37c` | `0x04` | - | used |
| release-13.04 | `0x076dc658` | `0x28` | - | unused |
| release-13.05 | `0x48c26613` | `0x13` | **+** | unused |
| release-13.06 | `0xe974593c` | `0x3c` | **+** | used |
| release-13.08 | `0x3c17a9fa` | `0x06` | - | used |
| release-13.09 | `0x7e8cb50f` | `0x0f` | **+** | used |
| release-13.10 | `0x2c249eb3` | `0x33` | **+** | unused |

In all twenty-seven registered builds the **tail-XOR byte equals the low byte of the seed
addend.** It is a derived value, not an independent constant: `SeededTransform`
in `crates/vrf-transform/src/lib.rs` defaults `TAIL_XOR` to `SEED_ADDEND as
u8`, and a build that broke the pattern would fail its 1- and 7-bit vectors,
which `vectors_cover_the_staging_boundaries` in
`crates/vrf-transform/tests/golden.rs` requires for every registered build.

So adding a build is one `transforms!` line in `crates/vrf-transform/src/lib.rs`:
its branch, `SEED_ADDEND`, signed `INIT_A_OFFSET` and the step list of its
word64 / word32 / byte stages; everything else is shared.

Every build in the table above has been tried on a live replay. A machine-local corpus can rotate; the reproducible oracle is
88 mechanically extracted golden vectors (11 staging boundaries per
build, eight builds) plus 1,422 native-machine-code vectors: 1,264 for the
sixteen recovered 11.06--12.09 builds, with a full 48-sample main/checkpoint
validation ([build support validation report](docs/LEGACY_BUILD_SUPPORT.md)),
and 79 each for 13.08 and 13.09. The 13.10 transform has eleven fixed boundary
vectors, five checked against its native reader, plus a full-replay block oracle;
the 13.10 array layout has a pinned replay payload regression. The [common audit](docs/BUILD_VERIFICATION.md) checks 38
13.06 replays.

The 768-byte S-box is shared across builds, which makes it usable as a
**signature for locating the transform function in a binary.**

## Design

### 1. Parallel across replays, sequential within one

The content-block header and declared bit-length are plaintext, and only the
payload transform is a pure function of `(bits, seed)`. Decode is not: it reads
the GUID cache and channel state earlier blocks mutate, and a stale group sets
the wrong handle width, so a replay decodes in order and parallelism is per
replay ([why](docs/PERFORMANCE_NOTES.md#decode-stays-sequential-within-a-replay)).

### 2. Output is Parquet

Columnar storage collapses the repeated path and name strings via dictionary
encoding, zstd compresses it well, and it reads directly in `pyarrow` /
`polars` / `pandas` / `duckdb`. NDJSON is reader-bound: on a 1.8-million-row
movement stream, JSON parsing was measured at 84% of processing time.

## Generated files

Three files are generated and must never be edited by hand; their targets,
generators and regeneration steps are in
[`CONTRIBUTING.md`](CONTRIBUTING.md#generated-files--never-hand-edit). The
S-box and golden vectors are extracted tables whose integrity
`extracted_tables_are_intact` (`crates/vrf-transform/tests/golden.rs`) checks.
The overlay table (`crates/vrf-decode/src/table.rs`) and
`tools/equippable_table.py` are maintained in the repository, not generated.

## License

Apache License 2.0; see [`LICENSE`](LICENSE). Releases up to and including
v0.2.0 were published under the MIT License. Third-party notices are in
[`NOTICE.md`](NOTICE.md).

This is an independent, community-developed tool. It is not affiliated with,
endorsed by, sponsored by, or approved by Riot Games. VALORANT, Riot Games,
and all related trademarks are the property of Riot Games, Inc.
