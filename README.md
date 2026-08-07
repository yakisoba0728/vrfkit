# vrfkit

A Rust toolkit that parses VALORANT replay files (`.vrf`, Unreal Engine network
replay format) and exports them to Parquet. A workspace of 10 crates plus a
Python `tools/` validation suite. `#![forbid(unsafe_code)]` is in every crate;
there is no `unsafe` block anywhere in the workspace. The only native FFI the
parser depends on is Oodle decompression, and that lives entirely in the
external `oozextract` crate. Edition 2024, MSRV 1.86, MIT.

![CI](https://github.com/yakisoba0728/vrfkit/actions/workflows/ci.yml/badge.svg)
![license](https://img.shields.io/badge/license-MIT-blue.svg)
![rust](https://img.shields.io/badge/rust-1.86%2B-orange.svg)
![edition](https://img.shields.io/badge/edition-2024-orange.svg)
![builds](https://img.shields.io/badge/builds-12.10--13.02-green.svg)
![unsafe](https://img.shields.io/badge/unsafe-none-success.svg)

Derived from [ValorantReplayParser](https://github.com/michel-giehl/ValorantReplayParser)
by Michel Giehl; see [`NOTICE.md`](NOTICE.md). Not affiliated with, endorsed
by, or approved by Riot Games.

**Current state:** `cargo test --workspace` **496 passing**, `tools/tests`
**410 passing** -- see [Status](#status) for the rest.

- Run it: [`docs/USAGE.md`](docs/USAGE.md)
- What's extractable: [`docs/DATA.md`](docs/DATA.md)
- Build it, test it, open a PR: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- Working conventions (for an AI agent): [`CLAUDE.md`](CLAUDE.md)

## Why this exists

Most replay parsers export only the fields whose type they know. vrfkit's
design premise is the opposite: **export every value the replay carries.** This
is possible because Unreal's property stream is self-describing -- each field
carries a handle and a bit-length *before* its value, so field boundaries are
walkable without knowing the type, and the handle-to-name map ships inside the
replay itself (`NetFieldExportGroup`). The names come for free; only the types
are unknown. So vrfkit always emits the raw bits and layers typed values on top
as an additive overlay. Nothing is dropped because its format is not yet
understood.

## Supported VALORANT builds

| Build | Branch | Status | Verified by |
|---|---|---|---|
| **13.02** | `release-13.02` | ✅ Supported | Preserved replay + 27-file stress test |
| **13.01** | `release-13.01` | ✅ Supported | 215-replay full corpus |
| **13.00** | `release-13.00` | ✅ Supported | Preserved fixture + golden vectors |
| **12.11** | `release-12.11` | ✅ Supported | Preserved fixture + golden vectors |
| **12.10** | `release-12.10` | ✅ Supported | Preserved fixture + golden vectors |

All branches are `++Ares-Core+release-<build>`. Adding a build is one
`SeededTransform` impl (two constants + three word functions); see
[Adding a new build](#supported-builds-and-the-cost-of-a-new-build).

## Highlights

- **Lossless by construction** — every field's raw bits are always exported,
  even when the type is unknown or decoding fails. Typed values are an
  *additive* overlay on top, and each row carries the replay's own
  `compatible_checksum`, so an untyped field can be told apart from an
  undescribed one without guessing.
- **Self-describing stream** — field names come from the replay itself
  (`NetFieldExportGroup`); no hardcoded agent or map names in the parser.
- **Six Parquet tables + manifest** — `fields`, `movement`, `actors`,
  `net_guids`, `events`, `checkpoint_fields`, ready for polars / pandas /
  DuckDB.
- **Spike state** — plant site A/B (`PlantedAtSite` + position), defuser
  (`CurrentDefuser`), timer, and the canonical detonation signal.
- **Combat & abilities** — per-player economy, magazine and reserve ammo,
  equipped weapon over time, cooldowns, and absolute health, armour and overheal
  from the damage log — the value after each change, not a running subtraction.
- **Every ability cast** — one record per cast with the caster's account UUID,
  slot, round, time and world location, plus the statistics it produced and the
  players each one landed on: `EnemiesSuppressed`, `EnemiesSlowed`,
  `EnemiesVulnerabled`, `EnemiesBlinded` and 27 more.
- **Status effects per player** — nearsight, slow, detain, suppress and the rest
  arrive as start/stop pairs on the *affected* player's actor, so each one is an
  interval with a victim. Slows are independently legible from movement speed,
  which sits on a lattice off 675 cm/s and halves exactly inside a slow.
- **Persistent effects** — smoke / wall / molly / slow / trap position and
  lifetime from actor lifecycles; one command via
  `tools/extract_active_effects.py`.
- **Spike custody** — who carried the spike and when, resolved to the account
  UUID, including Gekko's Wingman as a proxy carrier and the planter at each
  `spikePlanted`; one command via `tools/extract_spike_carrier.py`.
- **Account identity & typed events** — `manifest.players` (account UUID →
  actor → character), event `word0`/`word1` (killer/killed NetGUID, round
  index), ping/latency.
- **Cross-validated** against the C# reference parser — movement is
  near-bit-identical, CombatReport identical, and the server-written Event
  chunk independently confirms the kill count.
- **Reproducible** — Parquet output is byte-for-byte identical run to run.
- **No `unsafe`** — `#![forbid(unsafe_code)]` in every crate; the only FFI is
  Oodle, isolated in an external crate.
- **496 tests** plus a layered validation suite (framing / bytes / decode
  errors / semantics).

## Table of contents

- [Supported VALORANT builds](#supported-valorant-builds)
- [Highlights](#highlights)
- [Extractable data](docs/DATA.md) — the full inventory of what each table carries
- [Quick start](#quick-start)
- [Output](#output)
- [Status](#status)
- [Performance](#performance)
- [Comparison with the C# reference parser](#comparison-with-the-c-reference-parser)
- [The Event chunk -- the server's own timeline](#the-event-chunk----the-servers-own-timeline)
- [Whole-corpus robustness](#whole-corpus-robustness)
- [Type overlay](#type-overlay)
- [Supported builds and the cost of a new build](#supported-builds-and-the-cost-of-a-new-build)
- [Design](#design)
- [Validation suite](#validation-suite)
- [Generated files](#generated-files)
- [License](#license)

## Quick start

```bash
cargo build --release -p vrfkit --features export

vrfkit inspect  <file.vrf>                          # header / branch / chunk summary
vrfkit validate <file.vrf> [--diagnostics]          # grammar oracle, writes nothing
vrfkit export   <file.vrf> --out <dir> [--checkpoints]
```

`inspect` prints replay info, header, branch, and a chunk summary; it does no
parsing and returns immediately. `validate` walks every content block through
the RepLayout grammar and reports a pass rate; it writes no files. `export`
writes the Parquet tables and manifest described under [Output](#output).

`export` is a default feature. Drop it with `--no-default-features` and
`arrow`/`parquet`/`zstd` never enter the dependency tree:

```bash
cargo tree -p vrfkit --no-default-features | grep -E "arrow|parquet|zstd"
# (no output)
```

A binary built without `export` **refuses the subcommand rather than failing
silently** -- a subcommand that printed nothing and exited 0 would be
indistinguishable from one that wrote the files.

On `02d4d478` (48,215,213 bytes, build 13.01), `export` takes ~0.79 s and
produces seven files:

| File | Rows | Bytes |
|---|---|---|
| `fields.parquet` | 1,256,947 | 15,585,129 |
| `movement.parquet` | 1,839,607 | 31,835,557 |
| `actors.parquet` | 3,827 | 87,281 |
| `net_guids.parquet` | 16,167 | 153,606 |
| `events.parquet` | 195 | 11,136 |
| `checkpoint_fields.parquet` | 78,850 | 202,960 |
| `manifest.json` |  | ~660,030 |

`checkpoint_fields.parquet` requires `--checkpoints`; with or without it, **the
other five tables are byte-for-byte identical.**

Two things about `movement.parquet` worth knowing up front: `timestamp` is the
128.0 Hz server tick and **resets each round** -- use `time_ms` for a global
timeline; and posture detail lives in `bCrouchHeld`, not in `movement_state`.

> Column schemas, the `tools/` scripts, the full validation suite, and
> per-crate usage live in [`docs/USAGE.md`](docs/USAGE.md).
> This document is about *why it is built this way*.

## Output

Six Parquet tables plus `manifest.json`. String columns are dictionary-encoded
with ZSTD.

### `fields.parquet` -- replicated properties and RPC parameters

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
| `bit_count` | u32 | Payload size in bits |
| `raw_bits` | bytes? | Raw payload |
| `value_i64` / `value_f64` / `value_bool` / `value_str` | | Only when the type is known |

**`raw_bits` is always present, even when the type is unknown or decoding
fails.** At most one `value_*` column is filled. If a field's format is worked
out later, rows already exported never need re-parsing.

On top of `raw_bits`, the overlay types per-player economy
(`MoneyManagementComponent.{Money,StartOfRoundMoney,TotalMoneyGranted}` as
Int32) and the nested `CombatReport` arrays, among others.

Arrays are flattened, so names look like
`Rounds[3].Reports[1].Interactions[0].DamageDealt` and can be filtered with
`LIKE 'Rounds[%].Reports[%].DamageDealt'`.

One exception: a ClassNetCache block whose group cannot be identified cannot be
walked as an inner stream, so it is emitted only as a preservation row
(`field_name` = `__vrfkit_unresolved_class_net_cache_payload__`,
`handle` = `u32::MAX`, full payload in `raw_bits`). Re-interpreting it as
fields requires re-exporting from the original `.vrf`.

### `movement.parquet` -- character position time series

14 columns: `time_ms`, `packet_id`, `character_net_guid`, `pos_x/y/z`, `yaw`,
`pitch`, `vel_x/y/z`, `timestamp`, `movement_state`, `move_type`.

- `timestamp` is a **128.0 Hz global server tick** and **resets at each round
  boundary.** Use it for in-round alignment; use `time_ms` for a global
  timeline.
- `movement_state` and `move_type` are constant (0, 1) across the entire 13.01
  corpus. Constant in one build is not constant in general, so they are
  exported verbatim.
- **Posture detail is `bCrouchHeld`, not `movement_state`.** It already ships
  as a separate field in `fields.parquet`.

### `actors.parquet` -- actor spawn/despawn

`event` (`open` / `close`), `class_path`, `archetype_path`,
`spawn_x/y/z`, `spawn_pitch/yaw/roll`. This is where weapon and ability
instance classes are found.

### `net_guids.parquet` -- GUID to path, and containment

`net_guid`, `path`, `outer_net_guid`. `outer_net_guid` is the containment
chain -- use it to walk from a firing effect's `FiringState` subobject back up
to the weapon actor.

### `events.parquet` -- the timeline the server wrote itself

`id`, `group`, `metadata`, `time1`, `time2`, `payload_size`, `raw_payload`,
`word0`, `word1`.

`group` is `characterDeath`, `characterUltimateUsed`, `roundStarted`,
`spikePlanted`, `spikeDefused`, `spikeExploded`, `switchTeams`, and so on. The
payload is `[u32 tag][N x u32 words][FString][f32 seconds]`, and `N` is fixed
per group (CharacterDeath = 2, CharacterUltimateUsed / RoundStart /
SwitchTeams = 1, SpikePlanted / Defused / Exploded = 0 -- derived as the
residual-zero count across the corpus), so the first two words are exported as
`word0`/`word1`. For `characterDeath`, `(word0, word1)` is `(killer, killed)`
NetGUID; for `roundStarted`, `word0` is the round number. The original bytes
remain in `raw_payload`, so re-interpretation is possible.

### `checkpoint_fields.parquet` -- snapshot

Same schema as `fields.parquet`. A Checkpoint is a full-state snapshot at one
instant and **is not redundant**: against the last ReplayData value at the same
timestamp in the exported parquet, about 1.4-1.6% of keys disagree (of which
~0.4% differ in value, the rest in bit-width) and about 0.4% of keys are
absent from ReplayData entirely. Results are identical for 13.01 and 13.02.
The earlier 6-11% figures were raw live-wire measurements; export's byte-width
normalization collapses them to ~1.4% (see `docs/archive/PROJECT_STATUS.md`
section 22-I; byte-level format in
[`docs/archive/CHECKPOINT_SPEC.md`](docs/archive/CHECKPOINT_SPEC.md)).

### `manifest.json`

The full ReplayInfo plus the header, statistics, and **every export group the
replay declares** (`net_field_export_groups`; 475 for `02d4d478`). The
handle-to-name mapping lives here.

The `players` array gives each `BombPlayerState` actor's `(actor_net_guid,
subject, character_net_guid)`. `subject` is the account UUID and
`character_net_guid` is the `SpawnedCharacter`, which exactly matches
`movement.parquet`'s `character_net_guid`. This bridges the wire actors to
stable account identity, so actor-level tables (movement, fields, actors) can
be joined on it -- and it disambiguates the case where two players pick the
same agent, where `playerLoadouts`'s `characterId` alone cannot tell them
apart. In `02d4d478`, 10/10 players join to movement.

`game_specific_data` carries the `playerLoadouts` JSON (per-subject
`characterId`, skins, sprays). `timestamp_ticks` is a UE `FDateTime`
(100-nanosecond ticks since 0001-01-01), **not** a Windows FILETIME -- reading
it as one gives the year 3626.

## Status

Work in progress. Currently verified: `cargo test --workspace` **496 passing**,
`clippy -D warnings` **0**, `cargo fmt` clean, `check_ascii` on 116 files. The
Python suite in `tools/tests` has 410 tests.

Re-measure per-crate counts with `cargo test -p <crate>`. Counts are omitted
from the table below on purpose -- they go stale, and re-measuring is one line.

| Layer | Crate | Feature flags |
|---|---|---|
| Bit reader / UE wire format | `vrf-bitio` | `alloc` (default; drop it for `no_std`) |
| Payload transform (5 builds) | `vrf-transform` | none (the `ALL_VERSIONS` type encodes the count) |
| Container (info/header/chunk/event/checkpoint, Oodle) | `vrf-container` | `oodle` `event` `checkpoint` |
| DemoFrame traversal | `vrf-frame` | none (sections are byte ranges for cursor alignment) |
| Replay dynamic schema + GUID cache + checkpoint tables | `vrf-schema` | `checkpoint` |
| Replication (packet/bunch/content block/field) | `vrf-net` | `diagnostics` |
| Field decoder + nested arrays + type overlay + effects | `vrf-decode` | `array` `effect` `overlay` `structs` |
| Movement decoder | `vrf-movement` | none (single protocol) |
| Parquet export | `vrf-export` | `parquet` + per-table |
| Unified CLI | `vrfkit` | `export` (default) |

Take only the layer you need:

```
cargo tree -p vrfkit --no-default-features | grep -E "arrow|parquet|zstd"
# (no output)
```

ZSTD is deliberately *not* feature-gated out -- every writer picks it, so
disabling it would produce files this crate could not explain.

## Performance

On `02d4d478` (48,215,213 bytes):

| | Before | Now |
|---|---|---|
| `export` | 1.64 s / 201 MB | **0.85 s / 109 MB** |
| `validate` | 1.42 s / 65 MB | **0.693 s / 65 MB** |

Figures are wall-clock / peak memory. Output is **byte-for-byte identical**
before and after. Detail and the optimizations measured and then rejected are
in `docs/archive/PROJECT_STATUS.md` section 25.

> These times fluctuate by +/-10% run to run on the same commit and machine:
> on 2026-08-04 export was 0.79 s and validate 0.65 s; on 2026-08-05 they were
> 0.85 s and 0.693 s. This is machine state, not code -- confirmed by A/B-ing
> before/after binaries at section 36-F.

Every chunk kind in the file is read -- ReplayData, Event, and Checkpoint.
There are no unopened regions.

## Comparison with the C# reference parser

The same replay (`02d4d478`) was diffed against the output of the existing C#
parser.

**Structure -- exact match.**

| | C# | vrfkit |
|---|---|---|
| Packets / bunches | 530,401 | 530,401 |
| Actor open / close | 2,028 / 1,799 | 2,028 / 1,799 |
| Export-group path set | 475 | 475 (intersection 475, both differences 0) |

**Movement -- effectively bit-identical.** Over a 50,000-row join (99.98%
matched), the maximum position error is 0.0005 (float rounding); yaw, pitch,
and velocity error is exactly 0. Row counts are 1,837,220 (C#) versus 1,839,607
(ours) -- the gap is the C# limitation of "emit only the last move of each
update"; we additionally recover 2,387 intermediate moves.

**CombatReport nested array -- every metric-input value matches.** This
structure is the sole source of K/D/A, ADR, HS%, multi-kills, and wallbangs,
so it was diffed as a multiset of values (`tools/compare_combat_report.py`).

```
..Interactions[].AssistType                               364    364  IDENTICAL
..Interactions[].DamageDealt                              553    553  IDENTICAL
..Interactions[].DamageReceived                           553    553  IDENTICAL
..Interactions[].HitsDealt                                553    553  IDENTICAL
..Interactions[].HitsReceived                             553    553  IDENTICAL
..Interactions[].DidKill                                  414    414  IDENTICAL
..Interactions[].DealtInteractions[].Regions[].Hits       390    390  IDENTICAL
..Interactions[].DealtInteractions[].Regions[].Damage     390    390  IDENTICAL
..Interactions[].ReceivedInteractions[].Regions[].Hits    390    390  IDENTICAL
..Interactions[].ReceivedInteractions[].Regions[].Damage  390    390  IDENTICAL
```

**Extraction volume -- we export more.** `(group, field)` pairs break down as
1,450 vrfkit-only / 302 both / 71 C#-only, and 49 of the 71 C#-only are naming
differences (C# uses `CrouchHeld`; we use the wire name `bCrouchHeld`). RPCs
are 342,735 versus 230,893 -- 48% more -- because the C# parser drops RPCs
without a descriptor.

**RPC parameters -- values match, and 13 kills the C# parser missed are
recovered.** The ~330,000 RPCs had parameter payloads that were entirely raw;
they were decoded using the 84 parameter-schema groups (`<Class>:<Function>`
paths) the replay itself declares. Diffed with `tools/compare_rpc_params.py`:

```
MulticastNotifyDamage_Point.DamageDealt          580  580  MATCH
MulticastNotifyDamage_Point.DamageTaken          580  580  MATCH
MulticastNotifyDamage_Point.RegionalDamage       580  580  MATCH
MulticastNotifyDamage_Point.bDamageKilledTarget  580  580  MATCH
MulticastEndRound.NewRoundNumber                  17   17  MATCH
MulticastNotifyKilledEnemy.KillerCharacter       119  132  ours +13
```

The last line is the interesting one. The C# parser sees 119 `KillerCharacter`
events across 9 characters; we see 132 across 10. The difference is exactly
the 13 kills by character 576; every other character matches in count.

`MulticastNotifyKilledEnemy` is hosted on the killer's character actor, and in
this replay one player's character never replicates that RPC. The existing
pipeline papered over the 13 missing kills by recovering them later as
CombatReport credit, so they vanished from the timeline. In vrfkit the timeline
itself is complete.

## The Event chunk -- the server's own timeline

The claim above was, for a long time, witnessed only by our own parser. It no
longer is.

The `.vrf` Event chunk is the event list the server labeled and wrote itself,
stored elsewhere in the file under a different encoding, and **the existing C#
parser does not even open this chunk** (`ReplayChunkDispatcher.cs:152` --
`"Skipping event chunk"`). We now read it.

```
characterDeath 132 | characterUltimateUsed 34 | roundStarted 18
spikePlanted 9 | spikeDefused 1 | switchTeams 1          (02d4d478, 195 events)
```

The 132 `characterDeath` events exactly match the 132 `MulticastNotifyKilledEnemy`
events we extracted from RPCs -- and the C# parser's 119 plus character 576's
13. The two payload words are the killer/killed NetGUIDs, and **132/132 match
in order** (0/132 matched reversed). "We are right and the C# parser missed
them" is no longer our claim; it is the result of diffing against the server's
own record.

Scope, to be precise: the killer/killed pair diff is for **one replay**, while
the chunk-framing check covers **all 215 files, 43,397 chunks in total**,
every one consumed with zero residual bytes.

The payload layout is `[u32 group tag][N x u32 words][FString
"EReplayEventGroup::<Name>"][f32 seconds]`, and `N` differs per group. The `N`
for all seven groups is derived from the corpus as the residual-zero count
(CharacterDeath = 2, CharacterUltimateUsed / RoundStart / SwitchTeams = 1,
SpikePlanted / Defused / Exploded = 0), so the first two words are exported as
`word0`/`word1` (killer/killed NetGUID for CharacterDeath; round number for
RoundStart). The original is left intact in `raw_payload`.

## Whole-corpus robustness

All 215 `.vrf` files were run through the oracle (`tools/validate_corpus.py`).

```
succeeded: 215/215        failed: 0
branches : 215  ++Ares-Core+release-13.01
pass rate: min 97.487378%  median 99.323434%  max 99.682485%
totals   : 136,545,822 content blocks / 98,884,839 fields / 75,571,092 RPCs
           malformed framing 0        unattributed 1,972,019,383 bits
```

`malformed framing 0` means the container, bunch, and content-block framing
does not disagree a single time across the whole corpus. The pass rate is not
100% because the gap is **attribution, not framing.** Blocks are cut exactly,
but some cannot be assigned to a `_ClassNetCache` group, so the handle width is
unknown and they cannot be expanded into records.

This number was once reported as 100%. That was not more accurate -- it was
**wrong.** The code of the day silently dropped blocks whose group it could not
find and incremented no counter, and the oracle reported a perfect score over
the data it had just discarded. Exposing that path surfaced 14,459 blocks /
18,831,872 bits in one replay and 2,276,559,577 bits corpus-wide. Later, class
-cache groups were recovered from actor-instance names, reclaiming about 300
million bits; the figure above is what remains.

The boundary of what remains is sharp: **97.283437% of all unconsumed failure
bits are `AbilitiesAndBuffsComponent`**, and the replay does not declare a
cache group for that class at all, so no lookup reaches it.
`MeleeAttackState1`-`4` and `_Alt` are already resolved by the schema-based
resolver through the shared `MeleeAttackStateComponent_ClassNetCache`, and
their failure blocks and bits are zero across the corpus. Unresolved
ClassNetCache blocks cannot be walked as an inner stream, however, so they emit
no Parquet rows and no `raw_bits`; re-interpreting them requires keeping the
original `.vrf`.

### One bug that took a while

For a while, every replay lost exactly one block and 695 bits. Four hypotheses
were tried and failed, and it was finally caught by **exhaustive search** --
re-framing an 831-bit payload from every start offset and scoring each by
"does it land exactly on the payload end?" Offset 108 passed, and ten blocks
with sequential even GUIDs (64, 6, 8, 10 ... 22) fit cleanly. We had started at
109, so it was a **1-bit under-consumption.**

Bit-level instrumentation pinned the location:

| Sub-read | Bits | Position |
|---|---|---|
| actor GUID `IntPacked(2)` | 8 | 0..8 |
| archetype `IntPacked(9)` | 8 | 8..16 |
| level `IntPacked(3)` | 8 | 16..24 |
| location (18-bit components) | 63 | 24..87 |
| rotation (flag, no pitch, yaw, no roll) | 20 | 87..107 |
| scale, absent | 1 | 107..108 |
| **velocity, absent** | **1** | **108..109** |

`PlayerController` has `bReplicateMovement = false`, so the server never
serializes velocity -- the field is not on the wire at all, not "present but
empty." On the first bunch `bHasPackageMapExports = false`, so the path is not
registered yet and the actor cannot be identified by archetype; the dynamic
GUID is a non-zero even number, so 2 is the minimum, and the first dynamic
actor a replay opens is always the replay controller.

The fix took malformed 215 -> 0 and newly decoded 2,150 blocks (10 per
replay). Residual under-consumed bits dropped to 3,671, and those last four
cases were later explained as the handle-minimum-width problem and went to 0.

## Type overlay

For any field whose inner stream can be walked, the raw bits are always
exported; when the type is known, the `value_*` columns are filled **as an
overlay.** If the type is unknown, or decoding fails, the row's `raw_bits`
remains. The only exception is the unresolved ClassNetCache block above: it
cannot be expanded into fields, so it emits one preservation row (`handle` =
`u32::MAX`, full payload in `raw_bits`) and a loud failure with skipped bits.

The overlay table is extracted mechanically from the C# descriptors
(`tools/extract_descriptors.py`) -- 198 groups, 1,255 entries, 84 handles.
Nothing is transcribed by hand, for the same reason S-boxes and golden vectors
are not: it is the kind of constant where a typo is invisible in review.

Four names resolve without a table entry: `Owner`, `Instigator`, `AttachParent`
and `Controller` are `AActor` / `USceneComponent` object references Unreal
replicates on every actor, always as a NetGUID. The descriptors declare them
only for the classes they happen to cover, which left the same four names typed
on 129 group/field pairs and untyped on 203 more. Since the type is fixed by the
engine rather than by the class, they resolve by name after the table misses --
a claim about Unreal, not a guess about any one Blueprint, and it holds for
groups no replay has spawned yet. It types 6,048 further rows with decode errors
still at zero across the 215-replay corpus.

`02d4d478` at the current HEAD:

```
Decoded OK:   716,633      Decode errors:      0
Raw/Skip:      74,657      Not in table: 195,697
No field name:  1,996      Typed:          72.5%
Effect blobs:  53,908
```

**Effect decoding is additive and does not move these buckets.** The overlay
buckets are settled before the effect pass, so rows that gained a value from an
effect are still counted under `Not in table`; merging them into `Decoded OK`
would double-count and move the baseline for unrelated reasons. `Effect blobs`
is reported separately -- without it, 53,908 rows gain a value yet the summary
prints identically. (The bucket counts themselves do move as overlay entries
are added; the figures above are post-economy-typing.)

The real coverage figure is the fraction of all 1,256,947 rows with a filled
`value_*`. Before effect linkage 68.8% were untyped; it is now **36.8%.**

**These numbers change often; re-measure before quoting** -- four of the six
were left stale at one point:

```
cargo build --release
.\target\release\vrfkit.exe export <replay.vrf> --out out\probe
```

`Typed` is the ratio printed in the summary: rows the overlay decoded
successfully (`Decoded OK`) over rows it examined (`Rows offered`). The
denominator includes every RPC parameter, so it reads low -- most of `Not in
table` is RPC parameters without a C# descriptor, plus the groups the replay
declares (475) that are not in the table. (Rows with a filled `value_*` also
include additive decoders like effects and structs, so real value coverage is
wider than this ratio -- see the 36.8% untyped figure above.) A row whose type
is unknown still ships with `raw_bits`, so it is **uninterpreted, not lost.**

`fields.parquet` also carries the replay's own `compatible_checksum` per row,
which turns that leftover into something searchable. Unreal hashes a property's
type into it, so it identifies the property across builds; bucketing untyped
rows on it separates three situations that otherwise look identical -- a type
the overlay knows and failed to apply, a described property nothing has typed,
and a value addressed inside a payload that declares no handle at all. Over 20
replays that splits 10,062,142 untyped rows 0.5% / 48.6% / 51.0%, and the first
bucket is supposed to be empty. The recipe is in
[`docs/USAGE.md`](docs/USAGE.md#fieldsparquet).

**Zero decode errors** holds across all 215 replays, checked corpus-wide by
`tools/check_decode_errors_corpus.py`. It exists because `vrfkit validate`
does not print overlay counters, so `validate_corpus.py` alone cannot see a
wrong type. Reaching zero found three places where the wire disagreed with the
C# declarations; they are recorded with evidence in
`tools/apply_type_corrections.py` (94 corrections, verified with `--check`).

| Symptom | Actual | Evidence |
|---|---|---|
| Time-related `Float` field consumes more than 32 bits | Wire is `Double` (64-bit) | Every error is "32 bits consumed, 32 bits residual" |
| `215`/`216` `Int32` field arrives in 3 bits | Variable-width actor bookkeeping | The C# weapon descriptor comment states "width varies per build" |
| SmokeScreen projectile `ReplicatedMovement` EOF | Rotation is `ByteComponents` | Four other projectiles in the same codebase explicitly use `ByteComponents` |

Byte-width handling was also corrected. A byte property inside an array stores
only its significant bits, so a fixed 8-bit read fails -- the C# parser also
reads only `archive.BitsRemaining`. Before this fix, all 364 rows of
`AssistType` (5 bits) were left without a value.

## Supported builds and the cost of a new build

The payload transform changes per game build, but far more is **constant**
across releases 12.10 through 13.02: the PRNG and its multipliers, the seed-mix
skeleton, the 64 -> 32 -> 8 -> tail staging, the tail-XOR handling, and even
the S-box table itself. What actually changes per build:

| | seed addend | offset | sign | S-box |
|---|---|---|---|---|
| release-12.10 | `0x12fd0ee5` | `0x1b` | - | unused |
| release-12.11 | `0x409d36a3` | `0x23` | **+** | unused |
| release-13.00 | `0x2949b6ef` | `0x11` | - | used |
| release-13.01 | `0xe62fcd5c` | `0x24` | - | unused |
| release-13.02 | `0x9e81a37c` | `0x04` | - | used |

In all five builds the **tail-XOR byte equals the low byte of the seed
addend.** It is a derived value, not an independent constant, and the
relationship is pinned by a test in `versions/mod.rs` -- if a future build breaks
the pattern, the test fails instead of the final byte silently corrupting.

So adding a build is one `SeededTransform` impl: two constants and three word
functions (`word64` / `word32` / `byte`); everything else is shared.

**13.02 was confirmed by live measurement.** The 215-file corpus is all 13.01,
so the 13.02 path was golden-vector-only until it was run against both a
preserved replay and the live demo directory:

```
1.vrf     (62 MB)  774,299 blocks  568,557 fields  408,591 RPCs  pass 98.919512%
                   malformed framing 0 / transform failed 0 / decode errors 0
```

`1.vrf` is the preserved copy in `%LOCALAPPDATA%\vrfkit\baseline-corpora`, so
this is reproducible. It is the same shape as 13.01, and the residual is the
same attribution problem. The C# parser the existing pipeline uses rejects
this build outright.

This section once also quoted `f1110ea5` (59 MB). It was removed not because
the new figure is more accurate but because **that file disappeared before it
was preserved**, as did four others quoted higher up -- `%LOCALAPPDATA%\VALORANT\Saved\Demos`
is game-owned and rotated. Beyond the one preserved copy, all 27 of the 13.02
replays in the demo directory at time of writing (12-94 MB, 1.67 GB total)
also validated with malformed 0 / transform failed 0 / pass rate
98.08-99.54% (median 99.14%). That directory rotates, so the reproducible
evidence is the preserved copy and the golden vectors.

The 768-byte S-box is shared across builds, which makes it usable as a
**signature for locating the transform function in a binary.**

## Design

### 1. Losslessness is structurally impossible to violate

Unreal's property stream is **self-describing.** Each field carries a handle
and a bit-length before its value:

```
[1-bit checksum] repeat {
    handle       = IntPacked   -> 0 ends the list
    payload_bits = IntPacked
    (payload_bits of value)
}
```

Field boundaries can be walked exactly without knowing the type. The
`handle -> name` map is the dynamic schema the replay itself delivers
(`NetFieldExportGroup`). **Names are free; only types are unknown.**

So decoding is split into two layers:

- **Base path** -- every field whose inner stream can be walked is emitted as
  `{group, handle, name, bit_count, raw_bits}`. No branch is skipped on the
  grounds that the type is unknown.
- **Overlay** -- if a type is registered for `(group, handle)`, the decoded
  value is emitted alongside it.

If a field's format is discovered later, rows already exported need no
re-parsing. The unresolved-ClassNetCache caveat still applies: those blocks
emit only a preservation row (full payload), so re-interpreting them at field
resolution requires re-exporting from the original `.vrf`.

### 2. Minimal cost per build update

See [Supported builds](#supported-builds-and-the-cost-of-a-new-build). Across
five builds the only per-build variables are two constants (seed addend,
offset) and a sign, plus whether the S-box stage is enabled; the PRNG,
staging, tail-XOR, and S-box table are shared. A new build is one
`SeededTransform` impl.

### 3. A clean parallelization point

The content-block **header and declared bit-length are plaintext**; the
transform only touches the payload that follows. So framing (sequential,
unavoidable because of the replication state machine) and block decode (fully
independent) can be separated. The transform is determined solely by
`(bits, seed)`, so it parallelizes per block.

### 4. Output is Parquet

Columnar storage collapses the repeated path and name strings via dictionary
encoding, zstd compresses it well, and it reads directly in `pyarrow` /
`polars` / `pandas` / `duckdb`. NDJSON is reader-bound: on a 2.8-million-row
movement stream, JSON parsing was measured at 84% of processing time.

## Validation suite

Full detail is in [`docs/USAGE.md`](docs/USAGE.md) section 6. The checks are
layered, and the layers catch different things:

- **Framing** (`validate_corpus.py`, all 215 files) -- content-block framing.
- **Bytes** (`check_export_baseline.py`, per-file row and byte counts) --
  regression in any of the 25 export counters.
- **Decode** (`check_decode_errors_corpus.py`, all 215 files) -- overlay type
  errors and struct-blob failures.
- **Semantics** (`check_metrics_baseline.py`, five builds) -- round count,
  score, K/D/A invariants that need no baseline.

Two of the headline metrics are **not** "100% / high is good" and reading them
that way is a trap:

- The **pass rate** is not 100% because of the *attribution gap* -- blocks are
  framed correctly but cannot always be assigned to a ClassNetCache group
  (97.28% of the residual is `AbilitiesAndBuffsComponent`, which the replay
  never declares). `Malformed framing` and `Transform failed` are the lines
  that must be zero; the pass rate is expected to sit below 100%.
- The **~72% `Typed`** ratio reads low because of the *RPC-parameter
  denominator* -- most of `Not in table` is RPC parameters with no C#
  descriptor. A low ratio is uninterpreted, not lost: those rows still carry
  `raw_bits`, and additive decoders (effects, structs, the economy typing)
  fill `value_*` without moving the bucket.

## Generated files

Four files in the tree are generated and must never be edited by hand:

| Generated file | Generator | Notes |
|---|---|---|
| `crates/vrf-decode/src/table.rs` | `tools/extract_descriptors.py` then `tools/apply_type_corrections.py` | The overlay table (1,255 entries, 198 groups, 84 handles) and handle table |
| `crates/vrf-transform/src/sbox.rs` | `tools/extract_sboxes.py` | 768-byte S-box, shared across builds |
| `crates/vrf-transform/tests/data/golden_vectors.rs` | `tools/extract_golden.py` | Per-build golden test vectors |
| `tools/equippable_table.py` | `tools/extract_equippables.py` | Weapon class path to display name |

The S-box and golden-vector generators require an upstream checkout:

```bash
python tools/extract_sboxes.py <path>/ValorantSeededTransformHelpers.cs \
    crates/vrf-transform/src/sbox.rs
python tools/extract_golden.py <path>/ValorantSeededTransformTests.cs \
    crates/vrf-transform/tests/data/golden_vectors.rs
```

Both embed an integrity check -- the S-box must be a permutation of 0..255 and
the golden-vector hex length must match the bit count, or generation is
refused.

**Order matters** for the overlay table:
`extract_descriptors.py` -> `apply_type_corrections.py` -> `cargo fmt`. The
corrections script works on both the just-generated single-line form and the
rustfmt form, but some patterns stop matching after `cargo fmt`, so the script
does not trust its own apply count -- it **re-verifies the final state after
applying** and fails if it disagrees:

```bash
python tools/apply_type_corrections.py           # apply, then verify
python tools/apply_type_corrections.py --check   # verify only
```

## License

MIT. Derivation and original authorship are in [`NOTICE.md`](NOTICE.md).

This is an independent, community-developed tool. It is not affiliated with,
endorsed by, sponsored by, or approved by Riot Games. VALORANT, Riot Games,
and all related trademarks are the property of Riot Games, Inc.
