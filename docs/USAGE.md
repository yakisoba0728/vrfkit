# vrfkit usage

The CLI, output schemas, library use, `tools/` scripts, and validation suite.

Design rationale and the comparison against the existing parser are in
[`../README.md`](../README.md); work history and measurement records are in
[`archive/PROJECT_STATUS.md`](archive/PROJECT_STATUS.md). The byte-level format
of the checkpoint chunks is in [`../CHECKPOINT_SPEC.md`](../CHECKPOINT_SPEC.md),
and finished task specs are in [`archive/`](archive/README.md) -- all of these
are for the record, not things to run.

## Table of contents

1. [Build](#1-build)
2. [CLI](#2-cli) -- [`inspect`](#inspect) / [`validate`](#validate) / [`export`](#export)
3. [Output](#3-output) -- [`fields`](#fieldsparquet) / [`movement`](#movementparquet) / [`actors`](#actorsparquet) / [`net_guids`](#net_guidsparquet) / [`events`](#eventsparquet) / [`checkpoint_fields`](#checkpoint_fieldsparquet) / [`manifest.json`](#manifestjson)
4. [Using it as a library](#4-using-it-as-a-library)
5. [`tools/` reference](#5-tools-reference) -- [Generators](#generators) / [Validation](#validation) / [Downstream conversion](#downstream-conversion) / [Analysis helpers](#analysis-helpers)
6. [Validation suite](#6-validation-suite)
7. [Supported builds](#7-supported-builds)
8. [Known limits](#8-known-limits)

---

## 1. Build

```bash
cargo build --release -p vrfkit                          # inspect / validate / export
cargo build --release -p vrfkit --no-default-features    # inspect / validate only
```

`export` is a default feature. Drop it with `--no-default-features` and
`arrow`/`parquet`/`zstd` never enter the dependency tree at all.

```bash
cargo tree -p vrfkit --no-default-features | grep -E "arrow|parquet|zstd"   # no output
```

A binary built without `export` **refuses the subcommand rather than succeeding
silently**. A subcommand that wrote nothing and exited 0 would be
indistinguishable from one that wrote the files.

---

## 2. CLI

```
vrfkit inspect  <file.vrf>
vrfkit validate <file.vrf> [--diagnostics]
vrfkit export   <file.vrf> --out <dir> [--checkpoints]
```

### `inspect`

See what the file is -- ReplayInfo, header, branch, chunk summary. It does no
parsing, so it returns immediately. Use it to check up front whether the build
is supported and whether the file is encrypted.

```
=== Header ===
  Replay version:   5.3.2 (changelist 2152699011)
  Branch:           ++Ares-Core+release-13.02
  Platform:         LinuxServer
=== Chunks ===
  ReplayData:       23 chunks (55297993 bytes)
  Checkpoint:       22 chunks
  Event:           238 chunks
```

### `validate`

Grammar oracle. It walks every content block through the RepLayout grammar and
reports a pass rate. **It writes no files.**

```
  Total content blocks: 743110
  Malformed framing:  0          <- blocks whose framing slipped. Nonzero is serious
  Transform failed:   0          <- payload transform failed. Signals an unsupported build
  RPC stream failed:  7889       <- blocks whose group could not be identified, so the handle width is unknown
  NOT COVERED:          18 Checkpoint chunk(s) were NOT walked
  ORACLE PASS RATE:     98.938381%
  VERDICT: PASS - every content block framed (exit 0)
```

**Exit code**: `0` every content block framed, `1` malformed framing was found,
`2` there was nothing to validate (no ReplayData blocks). Those are three
different outcomes and are kept apart deliberately -- a file this command
cannot read must not be reported as a file that passed. The pass rate itself is
NOT gated; see below.

The oracle walks the **ReplayData stream only**. Checkpoint chunks carry their
own replication framing and are not covered by this verdict; the count above
says how many were skipped. Use `export --checkpoints` to decode them.

**A pass rate below 100% is normal.** This is an attribution problem, not a
framing one -- blocks are cut exactly, but some cannot be definitively assigned
to a `_ClassNetCache` group. That is the `RPC stream failed` attribution gap,
originating from `AbilitiesAndBuffsComponent`. The lines to watch are `Malformed
framing` and `Transform failed`, and **both must be zero.**

`--diagnostics` prints context for every failed block. By default it shows up to
32 lines and prints totals / shown / omitted counts in the header.

### `export`

Parquet export.

```bash
vrfkit export replay.vrf --out out/
vrfkit export replay.vrf --out out/ --checkpoints
```

`--checkpoints` reads the Checkpoint chunks as well and **additionally** writes
`checkpoint_fields.parquet`. It is off by default because it is a separate pass
that reads roughly 10% more of the file, and **with or without it, the other
five tables are byte-for-byte identical.**

#### Lines to actually watch in the summary

```
  Malformed pkts:   0        <- nonzero means framing is broken
  Struct blobs:     207 decoded / 0 failed
  Decode errors:    0        <- nonzero means an overlay type is wrong
```

`Struct blobs` is the output of the dedicated decoders for `RoundResults` /
`TeamEconomy` / `RoundInfos`. These decoders are **additive**, so failing
completely does not move a single other counter -- when build 13.02 shifted a
handle, the entire summary looked healthy while the match score simply
disappeared (archive/PROJECT_STATUS.md section 26). **`0 decoded` is an alarm
even if `failed` is 0.** On failure, the `Struct blob err:` line prints the
member and handle by name.

#### Reading the `Typed` ratio

```
  Typed:            72.5% (properties + RPC parameters)
```

The denominator is **every row offered** to the overlay, and thanks to RPC
parameter expansion it includes both replicated properties and RPC parameters.
The two populations have very different type coverage -- the descriptor set grew
up property-first -- so writing "against all fields" without naming the
denominator makes added parameters read like a regression. **Untyped != lost.**
Even when the type is unknown, `raw_bits` is always preserved (see
[`fields.parquet`](#fieldsparquet)).

---

## 3. Output

Measured on `02d4d478` (48,215,213 bytes):

| File | Rows | Bytes | Notes |
|---|---|---|---|
| `fields.parquet` | 1,256,947 | 14,878,464 | |
| `movement.parquet` | 1,839,607 | 31,835,557 | |
| `actors.parquet` | 3,827 | 87,281 | |
| `net_guids.parquet` | 16,167 | 153,606 | |
| `events.parquet` | 195 | 11,136 | |
| `checkpoint_fields.parquet` | 78,850 | 202,960 | requires `--checkpoints` |
| `manifest.json` | -- | ~660,030 | varies: it records `elapsed_ms` |

`export` takes 0.79 s (median of 5; re-measure with
[`bench_export.py`](#analysis-helpers)). String columns are dictionary-encoded
+ ZSTD.

### `fields.parquet`

Replicated properties and RPC parameters.

| Column | Type | Description |
|---|---|---|
| `time_ms` | u32 | Milliseconds since replay start |
| `packet_id` | u32 | Packet sequence number |
| `channel_index` | u32 | Actor channel |
| `actor_net_guid` | u32 | Actor NetGUID |
| `object_net_guid` | u32? | Subobject NetGUID |
| `group_path` | str | `NetFieldExportGroup` path; RPCs use `<Class>:<Function>` |
| `handle` | u32 | Field handle within the group |
| `field_name` | str? | Name the replay declares for that handle |
| `compatible_checksum` | u32? | The replay's own checksum for that handle -- see below |
| `bit_count` | u32 | Payload size in bits |
| `raw_bits` | bytes? | Raw payload |
| `value_i64` / `value_f64` / `value_bool` / `value_str` | | Only when the type is known |

**`compatible_checksum` is what separates "nobody described this" from "we
missed this".** Unreal hashes a property's type into it alongside its name, so
it identifies the property across builds -- the overlay already uses it as a
last-resort type lookup, and exporting it lets a reader run the same reasoning.
Bucket the untyped rows by it and three different situations come apart:

| bucket | meaning |
|---|---|
| checksum present, **in** `CHECKSUM_TYPES` | the type is known and was not applied -- a resolution bug |
| checksum present, **not** in the table | a real coverage gap: a described property nothing has typed |
| **no checksum** | addressed inside a payload (array leaves, struct blobs), so the replay declares none |

`None` means the third of those, not that the export failed to carry a value.
Over 20 replays on 13.02 the split is 0.5% / 48.6% / 51.0% of 10,062,142
untyped rows, and it barely moves replay to replay.

The middle bucket is a work list, not a bug list -- some of it is deliberate.
Its largest members over those 20 replays:

| rows | field |
|---|---|
| 1,227,330 | `ClientReplayReceiveInputEventProcessingCapture.InputEventData` |
| 908,046 | `ReplayLastTransformUpdateTimeStamp` (every agent class) |
| 344,426 | `ClientPlayOneShotEffectAtLocation.249` |
| 131,884 | `ServerMovementTime` |
| 120,853 | `AuthCurrentRandomSeed` |
| 114,299 | `TransitionContext` |
| 97,573 | `MulticastStopContinuousEffect.StopEffectType` |

`ServerMovementTime` is the reminder that this is not a bug list: `docs/DATA.md`
records it as untyped on purpose, its epoch being unknown. The bucket says a
property is described and unclaimed, not that it should be claimed.

Note what separates the second and third buckets among RPCs, since both hold
`ClassNetCache` rows: an RPC whose parameters were resolved gets a checksum per
parameter and lands in the second, while an RPC whose payload could not be
split into parameters is emitted whole with no declared handle and lands in the
third. `ClientPlayOneShotEffectAtLocation.249` sits in the second because it is
a parameter -- one whose name the replay gives as a bare handle number, and
whose sibling `248` this repo already types as a `VectorDouble`.

Without this column those three are one undifferentiated pile. Phoenix's smoke
wall sat in the middle bucket for the life of the project -- 2,791 rows of null
with decode errors at 0 -- and was found only because a sibling class happened
to share its RPC name.

**`raw_bits` is always present, even when the type is unknown or decoding
fails.** At most one `value_*` column is filled. If a field's format is worked
out later, rows already exported need no re-parsing.

One exception: a ClassNetCache block whose group cannot be identified cannot be
walked as an inner stream, so it is emitted only as a single **preservation
row** and cannot be expanded into fields.

| Column | Value |
|---|---|
| `field_name` | `__vrfkit_unresolved_class_net_cache_payload__` |
| `handle` | `u32::MAX` |
| `raw_bits` | Full payload |

These blocks are counted under `validate`'s `RPC stream failed`, and
re-interpreting them as fields requires re-exporting from the original `.vrf`.

Arrays are flattened, so names come out like
`Rounds[3].Reports[1].Interactions[0].DamageDealt`. Filter with
`LIKE 'Rounds[%].Reports[%].DamageDealt'`.

### `movement.parquet`

Character position time series. 14 columns, all NOT NULL. The coordinate system
follows Unreal Engine's (left-handed Z-up) -- positions in cm, yaw/pitch in
degrees **[0, 360)**, velocity in cm/s. The angles are the 16-bit UE rotator
scaled by 360/65536, so they never go negative; `pitch > 180` is a downward
look. (This said -180..180 for a while, which no row has ever matched.)

| Column | Type | Description |
|---|---|---|
| `time_ms` | u32 | Milliseconds since replay start |
| `packet_id` | u32 | Packet sequence number |
| `character_net_guid` | u32 | Character NetGUID |
| `pos_x` / `pos_y` / `pos_z` | f32 | Position (cm) |
| `yaw` / `pitch` | f32 | Rotation (degrees) |
| `vel_x` / `vel_y` / `vel_z` | f32 | Velocity (cm/s) |
| `timestamp` | u32 | Server tick |
| `movement_state` | u8 | Posture byte |
| `move_type` | u8 | 0=variant0 (no velocity) / 1=variant1 (velocity) |

**Three things to note:**

- `timestamp` is a **128.0 Hz global server tick** and **resets at each round
  boundary.** Use it for in-round alignment; do not use it as a global timeline
  -- that is `time_ms`.
- `movement_state` and `move_type` are constant (0, 1) across the entire 13.01
  corpus. Constant in one build is not constant in general, so they are exported
  verbatim.
- **Posture detail is `bCrouchHeld`, not `movement_state`.** It already ships as
  a separate field in `fields.parquet`.

`mode_flags` is intentionally omitted -- it is assigned from the same local as
`movement_state`, so there is no code path where the two differ, and it would
only add a byte-identical column on top of ~1.8M rows.

### `actors.parquet`

One row per channel open/close (`event`). `event` is `open` / `close` / `dormant`, not
spawn/close. This is where weapon and ability instance classes are found --
actors that produce no field rows at all (DefuserItem, HeavyArmorItem, etc.)
still show up here when they open a channel.

| Column | Type | Description |
|---|---|---|
| `time_ms` | u32 | Milliseconds since replay start |
| `packet_id` | u32 | Packet sequence number |
| `channel_index` | u32 | Actor channel |
| `actor_net_guid` | u32 | Actor NetGUID |
| `event` | str | `open` / `close` / `dormant` (dormancy is not destruction -- the actor stopped replicating but still exists) |
| `class_path` | str? | Actor class path |
| `archetype_path` | str? | Archetype path (absent for static actors) |
| `spawn_x` / `spawn_y` / `spawn_z` | f32? | Spawn position |
| `spawn_pitch` / `spawn_yaw` / `spawn_roll` | f32? | Spawn rotation |

Static actors and `close` rows carry no spatial data, so the spawn fields are
null.

### `net_guids.parquet`

GUID to path, and containment.

| Column | Type | Description |
|---|---|---|
| `net_guid` | u32 | Registered NetGUID |
| `path` | str | Object path |
| `outer_net_guid` | u32? | NetGUID of the containing object |

`outer_net_guid` is the containment chain -- use it to walk from a firing
effect's `FiringState` subobject back up to the weapon actor. `actors.parquet`
only covers GUIDs that opened a channel, so it misses subobjects; this table
fills that gap.

Why nullable: GUID 0 is the engine's "invalid" sentinel, so folding "no parent"
into 0 would make unknown-parent indistinguishable from explicitly-invalid.

### `events.parquet`

The timeline the server wrote itself. One row per Event chunk.

| Column | Type | Description |
|---|---|---|
| `id` | | Event identifier |
| `group` | str | `characterDeath`, `characterUltimateUsed`, `roundStarted`, `spikePlanted`, `spikeDefused`, `spikeExploded`, `switchTeams` ... |
| `metadata` | | Metadata |
| `time1` / `time2` | | Timestamp pair |
| `payload_size` | | Payload size |
| `raw_payload` | bytes | Raw payload |
| `word0` / `word1` | u32 | First two payload words |

The payload is structured as `[u32 tag][N x u32 words][FString][f32 seconds]`,
and `N` is fixed per group (CharacterDeath=2, CharacterUltimateUsed / RoundStart
/ SwitchTeams=1, SpikePlanted / Defused / Exploded=0 -- derived as the
residual-zero count across the corpus), so the first two words are exported as
`word0`/`word1`. For `characterDeath`, `(word0, word1)` is the `(killer,
killed)` NetGUID; for `roundStarted`, `word0` is the round number. The original
remains intact in `raw_payload`, so re-interpretation is possible.

### `checkpoint_fields.parquet`

Same schema as `fields.parquet`. A Checkpoint is a full-state snapshot at one
instant and **is not redundant** -- against the last ReplayData value at the same
timestamp in the exported parquet, about 1.4% disagree and about 0.4% are keys
absent from ReplayData entirely (identical for 13.01 and 13.02). The earlier
6-11% figures were raw live-wire measurements, and export's byte-width
normalization collapses them to ~1.4% (archive/PROJECT_STATUS.md 22-I).

### `manifest.json`

The full ReplayInfo plus the header, statistics, and **every export group the
replay declares** (`net_field_export_groups`; 475 for `02d4d478`). The
handle-to-name mapping lives here.

`game_specific_data` carries the `playerLoadouts` JSON -- per-subject-UUID
`characterId` (agent), skins, sprays.

The `players` array gives each `BombPlayerState` actor's triple.

| Field | Description |
|---|---|
| `actor_net_guid` | BombPlayerState actor NetGUID |
| `subject` | Account UUID |
| `character_net_guid` | `SpawnedCharacter` NetGUID |

`character_net_guid` exactly matches `movement.parquet`'s `character_net_guid`.
So actor-level tables like movement, fields, and actors can be joined on a
stable account identifier. **When two players pick the same agent**,
`playerLoadouts`'s `characterId` alone cannot tell them apart, but `subject`
can.

`timestamp_ticks` is a UE `FDateTime` (100-nanosecond ticks since 0001-01-01).
It is **not** a Windows FILETIME -- reading it as one gives the year 3626.

---

## 4. Using it as a library

Take only the layer you need. Every crate is `#![forbid(unsafe_code)]`, and
`vrf-bitio` is `no_std` + optional `alloc`.

| Layer | Crate | Feature flags |
|---|---|---|
| Bit reader / UE wire format | `vrf-bitio` | `alloc` (default; drop it for `no_std`) |
| Payload transform (5 builds) | `vrf-transform` | none |
| Container (info/header/chunk/event/checkpoint, Oodle) | `vrf-container` | `oodle` `event` `checkpoint` |
| DemoFrame traversal | `vrf-frame` | none |
| Dynamic schema + GUID cache + checkpoint tables | `vrf-schema` | `checkpoint` |
| Replication (packet/bunch/content block/field) | `vrf-net` | `diagnostics` |
| Field decoder + nested arrays + type overlay + effects | `vrf-decode` | `array` `effect` `overlay` `structs` |
| Movement decoder | `vrf-movement` | none |
| Parquet writer | `vrf-export` | `parquet` + per-table |
| Unified CLI | `vrfkit` | `export` (default) |

ZSTD is deliberately *not* feature-gated out -- every writer picks it, so
disabling it would produce files this crate could not explain.

---

## 5. `tools/` reference

### Generators

**Never hand-edit the output.**

| Script | Produces |
|---|---|
| `extract_descriptors.py` | `crates/vrf-decode/src/table.rs` (overlay table 1,255 + 84 handles) |
| `apply_type_corrections.py` | Applies verified corrections/additions to that file and recomputes the two-line generation header |
| `extract_checksum_types.py` | `crates/vrf-decode/src/checksum_table.rs` -- `compatible_checksum` -> `FieldType`, learned from the fields the overlay table already declares. Needs an export directory rather than the C# tree, since checksums come from the replay. Checksums whose donors disagree are dropped, which is the safety property. Repeat `--export` to widen the basis; the run **merges** into the committed table rather than replacing it, because a checksum this basis did not happen to see is still correct. `--check` asks whether the two agree *where they overlap* -- not whether they are byte-identical, which a content-addressed table cannot be across different sets of replays. |
| `extract_sboxes.py` | `crates/vrf-transform/src/sbox.rs` |
| `extract_golden.py` | `crates/vrf-transform/tests/data/golden_vectors.rs` |
| `extract_equippables.py` | `tools/equippable_table.py` |

**Order matters:** `extract_descriptors.py` -> `apply_type_corrections.py` ->
`cargo fmt`. The corrections script works on both the just-generated single-line
form and the rustfmt form, but some patterns stop matching after `cargo fmt`. So
the script does not trust its own apply count -- it **re-verifies the final
state after applying** and fails if it disagrees.

```bash
python tools/apply_type_corrections.py           # apply, then verify (94 corrections)
python tools/apply_type_corrections.py --check   # verify only
```

Those 94 corrections are the whole `EXPECTED` set the script re-verifies; `ADDITIONS` is the
subset of it that has no C# descriptor behind it at all.

The `ADDITIONS` pass inserts items the C# descriptor is **silent on**. There are
currently 70 of them, and every one is admitted on wire evidence written into the
comment above the list -- bit width, value range, distribution -- and nothing else.
The original three still show the bar: `BaseTeamState.LoadoutValue` /
`AverageLoadoutValue` (26-I, where the reference declares the type of the same
property and only moves the group) and `BombGameState.ChosenCeremonyForRound`
(section 32, wire evidence only). Broadening it without evidence voids the very
reason these additions are allowed -- read archive/PROJECT_STATUS.md 26-I and 32
first, and read the "Deliberately NOT added" note in the same comment, which
records the fields that failed the bar and why.

### Validation

| Script | What it watches |
|---|---|
| `validate_corpus.py` | Framing (full preserved corpus) |
| `validate_metrics_corpus.py` | Metrics pipeline passes |
| `check_corpus_baseline.py` | Per-build corpus baseline |
| `check_export_baseline.py` | Export counters + per-file rows/bytes |
| `check_decode_errors_corpus.py` | Overlay type errors + struct blob failures |
| `check_component_remaps.py` | Whether each Blueprint-component remap still matches. Needs only an export, so it works on a replay from a build that has no baseline -- which is the case a renamed component would otherwise slip through. |
| `check_metrics_baseline.py` | **Semantics** -- rounds, score, K/D/A |
| `compare_combat_report.py` | Metrics-input multiset |
| `compare_rpc_params.py` | RPC parameter comparison |
| `compare_with_csharp.py` | Diff against the C# parser |
| `check_effect_decoder.py` | Effect decoder (12 cases) |
| `check_ascii.py` | Rust source ASCII sweep (116 files) |
| `check_docs.py` | This document itself (below) |

`check_docs.py` checks this document -- that every `tools/` script is
mentioned, every crate is in the table, every link resolves, and every quoted
table size and test count is the live value. It even checks **table sizes
quoted in Rust doc comments and `Cargo.toml`** -- at section 36 it caught
`vrf-decode`'s crate docs, feature table, and `Cargo.toml` all still saying
1,185. In this repo, doc numbers have gone stale repeatedly (the test count
alone six times, the overlay table size twice as 1,185 -> 1,187 -> 1,188, and
four game-deleted replays lingered for weeks). A stale sentence compiles and
passes every test, so no other check catches it.

```bash
python tools/check_docs.py           # also runs the test suites to compare counts
python tools/check_docs.py --fast    # skip the count comparison
```

### Downstream conversion

| Script | What it does |
|---|---|
| `to_valplay_bundle.py` | Parquet -> NDJSON bundle (events/movement/manifest). The format valplay's `compute_metrics.py` consumes |
| `equippable_table.py` | **Generated file.** Weapon class path -> display name |

```bash
python tools/to_valplay_bundle.py <export_dir> -o <bundle_dir>
python "<valplay>/pipeline/metrics/compute_metrics.py" <bundle_dir> -o metrics.json
```

**This is the pipeline bottleneck.** For a single 48 MB replay:

| Stage | Time |
|---|---|
| `vrfkit export` | 0.85 s |
| `to_valplay_bundle.py` | **21.7 s** |
| `compute_metrics.py` | ~14 s |

Bundle conversion is ~25x the parse (figure after the 1.9x improvement in
section 35). If you process multiple replays, **parallelizing is the biggest
lever** -- each replay is fully independent, and the measurements above are
deliberately sequential for accuracy.

> **The time figures fluctuate by +/-10%.** On the same machine and commit,
> export was 0.79 s on 2026-08-04 and 0.85 s on 2026-08-05. At section 36-F the
> before/after binaries were A/B-ed across 7 pairs -- the medians were 0.870 vs
> 0.874, so the code is neutral and the difference is machine state. **Do not
> chase a regression because a number here reads slightly high.** Whether it is
> a regression can only be answered by an A/B.

### Analysis helpers

| Script | What it does |
|---|---|
| `analyze_coverage.py` | Coverage analysis |
| `find_skips.py` | Finds skipped bits |
| `bench_export.py` | Times a full `export` against `tools/baselines/bench.json`. A smoke detector, not a profiler -- wall clock is noisy, so the default tolerance is 25% and it answers "did something get twice as slow", nothing finer. Reports a run *faster* than the baseline too: that means the recorded number no longer describes the code. |
| `extract_active_effects.py` | Derives an `active_effects.parquet` view from an export -- one row per persistent ability instance (smoke/wall/molly/slow/trap/recon/orb) with class, spawn position, and open/close lifetime. A `dormant` event does NOT end an instance -- a settled smoke that stops replicating has not despawned -- so those instances stay open-ended and the summary counts them. The data already lives in `actors.parquet`; this filters and pairs it. |
| `extract_spike_carrier.py` | Derives a `spike_carrier.parquet` view -- one row per spike custody interval, resolved through to the manifest `subject`. Reads `BombEquippable_C.Owner` on the spike's own channel rather than the inventory side, so it covers carrying-in-the-backpack and not just in-hand, and it follows proxy carriers (Gekko's Wingman) back through `Instigator`. |
| `viewer_projection.py` | World-to-minimap projection and the park-slot filter, used by `build_replay_viewer` (a later task in this plan) |
| `viewer_data.py` | Slices a match into rounds from `events.roundStarted` and downsamples movement from 125 Hz to 20 Hz for playback. Downsampling touches the playback pass only -- every check still reads the full-rate stream, so a teleport between two rendered frames cannot go unseen. Used by `build_replay_viewer` (a later task in this plan) |

---

## 6. Validation suite

### Quick sweep -- after any change

```bash
cargo test                                        # 496 passing
cargo clippy --all-targets -- -D warnings         # 0
cargo fmt --check
python tools/check_ascii.py --check               # 116 files, ASCII only
python tools/check_effect_decoder.py --check      # 12 cases
python -m unittest discover -s tools/tests -p "test_*.py"   # 446 passing
python tools/check_docs.py --fast                 # do the docs still describe this repo
python tools/apply_type_corrections.py --check    # 94 corrections present
```

**The ASCII rule is correctness, not style.** The Windows console is cp949, so a
single non-ASCII character in a format string truncates output at that point.
Rust sources are ASCII down to the comments.

### Regression guards -- after non-trivial changes

```bash
cargo build --release -p vrfkit --features export

python tools/check_export_baseline.py --baseline tools/baselines/export_02d4d478.json
python tools/check_corpus_baseline.py --baseline tools/baselines/build_1302.json
python tools/validate_corpus.py ./target/release/vrfkit.exe <corpus>
python tools/check_decode_errors_corpus.py ./target/release/vrfkit.exe <corpus>
python tools/check_metrics_baseline.py
python tools/compare_combat_report.py
```

These read their inputs from `VRFKIT_CORPUS_DIR`, `VRFKIT_CSHARP_DIR` and
`VRFKIT_VALPLAY_DIR` -- see
[Environment](../CONTRIBUTING.md#environment) for what each one points at.
**With the variable unset they print `SKIP` and exit 0**, so read the output
rather than the exit code.

### What each check catches -- this is the point

| Check | Watches | Misses | Cost |
|---|---|---|---|
| `validate_corpus.py` | Framing (all 215) | Type errors, broken semantics | ~30 s |
| `check_export_baseline.py` | 25 export counters + per-file rows/bytes | Other builds | 1 s |
| `check_decode_errors_corpus.py` | Overlay type errors + struct blob failures (all 215) | Broken semantics | ~50 s |
| `check_metrics_baseline.py` | **Semantics** -- rounds, score, K/D/A (5 builds) | Errors in the metrics pipeline itself | ~46 s |
| `compare_combat_report.py` | Metrics-input multiset | Framing | seconds |

**The layers differ.** The first three of those four read framing counters or
diff bytes, and **a decoder that cannot produce a value moves neither.** When
13.02 shifted the `RoundResults` handle, these checks stayed entirely green
while the match score simply disappeared.

`check_metrics_baseline.py` watches that layer -- it runs
`export -> to_valplay_bundle -> compute_metrics` on the preserved replays per
build and asserts five invariants that need no baseline:

```
R1  objective.round_count > 0
R2  rounds.round_count == objective.round_count      (two independent sources)
R3  sum(team_score) == objective.round_count
R4  players > 0
R5  if kills > 0 then damage > 0
```

**Proven:** build the commit just before the fix (309cf05) and run this guard --
13.02 fails R1 and R2 while 13.01 passes.

### Baseline updates

Every baseline check takes `--update`. **When DRIFT appears, explain each line
before** you use it. The point is not that the numbers are sacred but that
silent change must be impossible.

---

## 7. Supported builds

| Build | How it is verified |
|---|---|
| 12.10, 12.11, 13.00 | One preserved fixture each + golden vectors |
| 13.01 | Full 215-replay corpus |
| 13.02 | 62 MB preserved replay + live measurement |

13.02 measurement (preserved copy `1.vrf`; the result is identical across all 27
demos in the directory at time of writing):

```
1.vrf     (62 MB)  774,299 blocks  568,557 fields  408,591 RPCs  pass 98.919512%
                   malformed 0 / transform failed 0 / decode errors 0
```

Adding a new build takes one `SeededTransform` impl -- two constants and three
word functions. See the README's
[Supported builds and the cost of a new build](../README.md#supported-builds-and-the-cost-of-a-new-build)
section.

**When pinning a replay as a baseline, do not point at
`%LOCALAPPDATA%\VALORANT\Saved\Demos`.** The game owns and rotates that
directory -- four pinned replays have disappeared wholesale. Preserved copies
live in `%LOCALAPPDATA%\vrfkit\baseline-corpora`.

---

## 8. Known limits

- **Untyped residual** -- the [`export`](#export) `Typed` is ~72% (denominator
  including RPC parameters). **Untyped != lost** (`raw_bits` preserved). Typing
  the rest needs the game binary or UE headers -- this is not a table-editing
  problem (archive/PROJECT_STATUS.md section 24).
- **`AbilitiesAndBuffsComponent`** -- the replay declares no ClassNetCache group
  for that class at all. Confirmed across all 4,024 checkpoints. This is the
  attribution gap that keeps the `validate` pass rate below 100%.
- **ACS** -- `PlayerScoreComponent` is not replicated. There is nothing to
  compute from.
- **`economy.per_round` (13.02)** -- team economy moved to `BaseTeamState` and
  the values decode, but valplay's `compute_economy` looks at the old path.
  valplay is out of scope to fix.
- **Non-Bomb game modes** -- five of the 215-corpus are Swiftplay, and **the
  parser side is done** (section 33): `GROUP_ALIASES` maps Swiftplay's
  GameState/PlayerState to the Bomb classes, so the fields all gain types. All
  that remains is that valplay's `compute_metrics.py` hardcodes class names in
  five places; the patch is in `docs/swiftplay-metrics.patch`. valplay is out of
  scope, so applying it is their call.
- **ADR is 0.1-0.2 higher than the tracker.** This is not a bug -- wire damage
  is fractional, while the Riot API reports integers. **Do not "fix" it by
  adding truncation** (archive/PROJECT_STATUS.md 27-B).
