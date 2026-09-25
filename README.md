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
![builds](https://img.shields.io/badge/builds-11.06--13.06-green.svg)
![unsafe](https://img.shields.io/badge/unsafe-none-success.svg)

Derived from [ValorantReplayParser](https://github.com/michel-giehl/ValorantReplayParser)
by Michel Giehl; see [`NOTICE.md`](NOTICE.md). Not affiliated with, endorsed
by, or approved by Riot Games.

**Verified state (2026-09-25):** Rust has **714 passing** tests; Python has
**891 passing** tests. All 24 supported builds received the same verification
on **986 unique replays**; all **986** meet every strict criterion after fixing
the two ActiveBlinds decoding errors found by the first audit. See [build verification](docs/BUILD_VERIFICATION.md)
for the measured scope, common checks and remaining limits.

- Run it: [`docs/USAGE.md`](docs/USAGE.md)
- What's extractable: [`docs/DATA.md`](docs/DATA.md)
- Current corpus status and remaining work: [`docs/CURRENT_STATUS.md`](docs/CURRENT_STATUS.md)
- Latest build verification: [`docs/BUILD_VERIFICATION.md`](docs/BUILD_VERIFICATION.md)
- Historical field inventory: [`docs/TARGETING_AND_HEAL_VALUES.md`](docs/TARGETING_AND_HEAL_VALUES.md)
- Upstream parity and 13.06 validation: [`docs/UPSTREAM_PARITY.md`](docs/UPSTREAM_PARITY.md)
- Character-death and KillData state: [`docs/KILL_LEDGER.md`](docs/KILL_LEDGER.md)
- Damage, healing, decay and reset observations: [`docs/SECTION_OBSERVATIONS.md`](docs/SECTION_OBSERVATIONS.md)
- Observed section timelines and explicit continuity gaps: [`docs/SECTION_TIMELINE.md`](docs/SECTION_TIMELINE.md)
- Packet-ordered section comparisons: [`docs/SECTION_PACKET_TIMELINE.md`](docs/SECTION_PACKET_TIMELINE.md)
- Numeric FastArray observations and remaining item semantics: [`docs/GAS_AND_PATCHVOLUME_INVESTIGATION.md`](docs/GAS_AND_PATCHVOLUME_INVESTIGATION.md)
- Build it, test it, open a PR: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- Working conventions (for an AI agent): [`CLAUDE.md`](CLAUDE.md)

## Why this exists

Most replay parsers export only the fields whose type they know. vrfkit's
design premise is the opposite: **export every value the replay carries.** This
is possible because Unreal's property stream is self-describing -- each field
carries a handle and a bit-length *before* its value, so field boundaries are
walkable without knowing the type, and the handle-to-name map ships inside the
replay itself (`NetFieldExportGroup`). Where that map is available, names can
be resolved independently of types. Unknown properties retain raw payloads;
typed values are an additive overlay. This is a preservation strategy, not a
claim that every stream is currently understood: framing failures and missing
schema attribution are measured separately, and successful movement decodes
can be represented by their rows instead of a duplicate raw RPC.

## Supported VALORANT builds

| Build | Branch | Clean/checked | Verified by |
|---|---|---:|---|
| **13.06** | `release-13.06` | 6/6 | Validation + checkpoints + typed/raw |
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

Measured 2026-09-25 on all **986 unique available replays**, across all 24
supported branches. Every row uses the [same acceptance rule](docs/BUILD_VERIFICATION.md).
**Clean/checked** includes the strict array and array-leaf error counters:
**986/986** are clean after the ActiveBlinds empty-delta and null-reference
fixes. All 986 pass ReplayData validation, checkpoint-enabled export and
the independent comparisons on observed evidence fields. The full report
records counts and limits; this is not a claim that every field is understood.

All branches are `++Ares-Core+release-<build>`. Adding a build is one
`SeededTransform` impl; see [Adding a new build](#supported-builds-and-the-cost-of-a-new-build).

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
  replication keys, deleted/changed item IDs and raw property boundaries.
  All 2,882,152 measured inner windows close exactly; property names and
  gameplay meanings remain unverified. This output is separate from Parquet
  typed-value coverage.
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
- **Cross-validated** against the C# reference parser — movement is
  near-bit-identical, CombatReport identical, and the server-written Event
  chunk independently confirms the kill count.
- **Reproducible** — Parquet output is byte-for-byte identical run to run.
- **No `unsafe`** — `#![forbid(unsafe_code)]` in every crate; the only FFI is
  Oodle, isolated in an external crate.
- **714 Rust tests** plus a layered validation suite (framing / bytes / decode
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
cargo +1.86.0 build --release -p vrfkit --features export --locked

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
cargo +1.86.0 tree -p vrfkit --no-default-features --locked | grep -E "arrow|parquet|zstd"
# (no output)
```

A binary built without `export` **refuses the subcommand rather than failing
silently** -- a subcommand that printed nothing and exited 0 would be
indistinguishable from one that wrote the files.

On `02d4d478` (48,215,213 bytes, build 13.01), `export` produces thirteen
Parquet files plus a manifest when checkpoints are included:

| File | Rows | Bytes |
|---|---|---|
| `fields.parquet` | 1,296,660 | 16,455,178 |
| `movement.parquet` | 1,844,147 | 31,886,449 |
| `actors.parquet` | 3,827 | 87,281 |
| `net_guids.parquet` | 16,167 | 153,606 |
| `events.parquet` | 195 | 13,411 |
| `partials.parquet` | 0 | 2,505 |
| `checkpoint_fields.parquet` | 352,089 | 1,218,992 |
| `checkpoint_actors.parquet` | 3,014 | 27,118 |
| `checkpoint_net_guids.parquet` | 74,270 | 277,718 |
| `checkpoint_blocks.parquet` | 22,247 | 175,103 |
| `checkpoint_guid_entries.parquet` | 74,270 | 928,714 |
| `checkpoint_export_groups.parquet` | 8,307 | 27,041 |
| `checkpoint_export_fields.parquet` | 49,314 | 287,130 |
| `manifest.json` |  | ~660,030 |

`checkpoint_fields.parquet` requires `--checkpoints`. The partials row above
shows the default main-only export; both modes currently contain zero
rejected partial rows and occupy 2,505 bytes. The five original main tables remain
byte-for-byte identical across the checkpoint flag.

Two things about `movement.parquet` worth knowing up front: `timestamp` is the
128.0 Hz server tick and **resets each round** -- use `time_ms` for a global
timeline; and posture detail lives in `bCrouchHeld`, not in `movement_state`.

> Column schemas, the `tools/` scripts, the full validation suite, and
> per-crate usage live in [`docs/USAGE.md`](docs/USAGE.md).
> This document is about *why it is built this way*.

## Output

The main export writes six Parquet tables plus `manifest.json`.
`--checkpoints` adds seven checkpoint tables, for thirteen Parquet files in
total. String columns are dictionary-encoded with ZSTD.

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

`raw_bits` is nullable. Unnamed replicated properties and unresolved whole
RPC payloads retain their raw representation; successfully decoded movement
RPCs and synthesized child rows may instead be represented by their decoded
output. At most one `value_*` column is filled per row. Reinterpretation
without re-parsing is possible only where the required raw payload survives.

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
- `movement_state` and `move_type` are constant (0, 1) across all
  1,034,035,170 exported rows in the historical 2026-08-31 527-replay corpus (builds 13.01,
  13.02 and 13.04). A future build may break that invariant, so both bytes are
  exported verbatim.
- **Posture detail is `bCrouchHeld`, not `movement_state`.** It already ships
  as a separate field in `fields.parquet`.

### `actors.parquet` -- actor spawn/despawn

`event` (`open` / `close` / `dormant`), `class_path`, `archetype_path`,
`spawn_x/y/z`, `spawn_pitch/yaw/roll`. This is where weapon and ability
instance classes are found.

**Three values, not two, and only `close` is a despawn.**
`ChannelCloseReason::Dormancy` means the server stopped replicating an actor that
is still alive, so it is exported as `dormant`; every other close reason is the
actor going away. Both labels were written as `close` until the flag was plumbed
through to the row, and that cost real time: a persistent effect settling into
dormancy read as a despawn, so its lifetime ended early and the later wake-up
re-open read as a *second spawn of the same object*. No row and no timestamp was
ever lost -- only the label was wrong, and only for the closes that were never
despawns. Code that pairs spawns with despawns must therefore treat `dormant` as
neither: the channel keeps its archetype across a dormant close
(`crates/vrfkit/src/sink/stream.rs`), and the matching `open` is a wake-up, not a
new instance.

### `net_guids.parquet` -- GUID to path, and containment

`net_guid`, `path`, `outer_net_guid`. `outer_net_guid` is the containment
chain -- use it to walk from a firing effect's `FiringState` subobject back up
to the weapon actor.

### `events.parquet` -- the timeline the server wrote itself

`id`, `group`, `metadata`, `time1`, `time2`, `payload_size`, `raw_payload`,
`word0`, `word1`, `payload_tag`, `payload_name`, `payload_seconds`.

`group` is `characterDeath`, `characterUltimateUsed`, `roundStarted`,
`spikePlanted`, `spikeDefused`, `spikeExploded`, `switchTeams`, and so on. The
payload is `[u32 tag][N x u32 words][FString][f32 seconds]`, and `N` is fixed
per group (CharacterDeath = 2, CharacterUltimateUsed / RoundStart /
SwitchTeams = 1, SpikePlanted / Defused / Exploded = 0 -- derived as the
residual-zero count across the corpus). The structural overlay is populated
only when the arity, stable group tag, public `EReplayEventGroup` name and
payload time all agree; otherwise every overlay column is null. For
`characterDeath`, `(word0, word1)` is `(killer, killed)` NetGUID; for
`roundStarted`, `word0` is the round number. The original bytes always remain
in `raw_payload`, so a future layout change is preserved losslessly.

### `checkpoint_fields.parquet` -- snapshot

The field columns are preceded by `checkpoint_index` and `checkpoint_id`.
Separate `checkpoint_actors.parquet` and `checkpoint_net_guids.parquet` preserve
the snapshot's actor and GUID context with the same identity columns.
`checkpoint_blocks.parquet` links each content block to its field rows and
preserves class GUIDs and the path used to resolve the group name. Join
within that checkpoint: its packet, channel and GUID state is independent of
the main stream. A snapshot actor open is not a new timeline spawn.

`checkpoint_export_groups.parquet` and `checkpoint_export_fields.parquet`
preserve the checkpoint's schema declarations, sparse field slots, checksums,
and raw FName components. `checkpoint_guid_entries.parquet` preserves the
initial GUID entries, their order, path representation, and raw flags.
Name indices in that initial stream resolve against the zero-based table of
literal paths that precede them in the same checkpoint. The existing GUID
table remains the resolved cache after the frame walk. The manifest records
the mode and literal/index/resolved counts. See
[declaration columns and join rules](docs/USAGE.md#checkpoint-schema-declarations).
The [schema preservation report](docs/CHECKPOINT_SCHEMA_PRESERVATION.md)
separates the added registry evidence from gameplay interpretation, and the
[path-resolution report](docs/CHECKPOINT_PATH_RESOLUTION.md) gives the rule and
its validation boundary.

See [checkpoint output](docs/USAGE.md#checkpoint_fieldsparquet) and
[current semantic evidence](docs/SEMANTIC_CONTEXT_EXPANSION.md). Older comparisons
that joined checkpoint and main GUIDs by number do not establish actor identity.

### `partials.parquet` -- unresolved transport payloads

Rejected partial fragments and abandoned accumulators retain their exact raw
bits in this separate table. Rows record the source stream and checkpoint ID,
source packet and payload bit offset, channel and sequence, original header
flags, rejection position and reason. `current_fragment` and
`accumulated_payload` are distinct: an accumulator's source header describes
its first fragment, while its `bit_count` describes the assembled raw buffer.
These rows are preserved evidence, not successfully reconstructed RPCs.
With `--checkpoints`, both streams share this table and remain distinguishable
by `source` and `checkpoint_id`. CLI and manifest counters report each stream's
preserved rows and bits separately.

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

Work in progress. Currently verified: `cargo +1.86.0 test --workspace --locked`
**714 passing**; the full Python suite also has **891 passing** tests. The
full documentation check passes. The latest [common build audit](docs/BUILD_VERIFICATION.md)
records replay validation, checkpoint export, independent value checks and
the resolved array findings and remaining semantic limits for each supported build.

CI measures these test counts, runs Python checks on Windows and Ubuntu
with Python 3.12/3.13, and tests Rust stable with all features and core-only CLI
support. Three pinned public replays pass through the common checkpoint audit
on every run. See [CONTRIBUTING.md](CONTRIBUTING.md) for the full CI gates and
the separate private-corpus checks.

Re-measure per-crate counts with `cargo test -p <crate>`. Counts are omitted
from the table below on purpose -- they go stale, and re-measuring is one line.

| Layer | Crate | Feature flags |
|---|---|---|
| Bit reader / UE wire format | `vrf-bitio` | `alloc` (default; drop it for `no_std`) |
| Payload transform (24 builds) | `vrf-transform` | none (`ALL_VERSIONS` is a length-independent slice) |
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

CI also compiles every advertised core-only and singleton feature from
`--no-default-features`, checks all workspace targets/all features, builds the
standalone probe tool, and runs strict rustdoc. The executable matrix is in
[`CONTRIBUTING.md`](CONTRIBUTING.md#before-you-open-a-pr) as 27 `cargo check`
lines; `.github/workflows/ci.yml` expresses **the same 27 cases** as a PowerShell
array of `@("crate","feature")` pairs. Same set, same order, two notations -- so
they are not copies of one another. `tools/check_docs.py` (including `--fast`,
which CI runs) fails when the two lists differ in membership or order, and when
either count quoted here is stale. This paragraph used to say nothing checked
that they agree; by the time anyone looked, three cases were in a different
order and the count quoted here was two short. If you add a case, add it in
both. To see the two lists side by side:

```bash
python - <<'EOF'
import re
sh = re.findall(r'cargo \+1\.86\.0 check -p (\S+) --no-default-features'
                r'(?: --features (\S+))? --locked',
                open('CONTRIBUTING.md', encoding='utf-8').read())
block = re.search(r'\$matrix = @\((.*?)\n\s*\)',
                  open('.github/workflows/ci.yml', encoding='utf-8').read(), re.S).group(1)
ci = re.findall(r'@\("([^"]+)",\s*"([^"]*)"\)', block)
sh = [(c, f or '') for c, f in sh]
print(len(sh), 'in CONTRIBUTING,', len(ci), 'in ci.yml; identical:', sh == ci)
print('only in CONTRIBUTING:', sorted(set(sh) - set(ci)))
print('only in ci.yml:', sorted(set(ci) - set(sh)))
EOF
```

(Measured 2026-09-13 after reordering ci.yml: identical, including order.)

## Performance

Historical optimization measurement on `02d4d478` (48,215,213 bytes), August
2026. These timings predate the current decoding additions:

| | Before optimization | After optimization |
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

`inspect` inventories every chunk kind. `validate` walks ReplayData;
`export --checkpoints` additionally walks checkpoint replication and writes
its fields separately. A ReplayData verdict does not validate checkpoints.

## Comparison with the C# reference parser

The same replay (`02d4d478`) was diffed against the output of the existing C#
parser. The CombatReport and RPC-parameter comparisons below were re-measured
on 2026-09-14 against `CliReader export` built from two ValorantReplayParser
commits: upstream `b51d674`, and `8824794`, the descriptor commit vendored in
[`third_party/vrp/`](third_party/vrp/README.md). The structure, movement and
volume figures are from the earlier comparison.

**Structure -- exact match.**

| | C# | vrfkit |
|---|---|---|
| Packets / bunches | 530,401 | 530,401 |
| Actor open / close | 2,028 / 1,799 | 2,028 / 1,799 |
| Export-group path set | 475 | 475 (intersection 475, both differences 0) |

**Movement -- effectively bit-identical.** Over a 50,000-row join (99.98%
matched), the maximum position error is 0.0005 (float rounding); yaw, pitch,
and velocity error is exactly 0. In that earlier comparison, row counts were 1,837,220 (C#) versus 1,839,607
(ours) -- the gap is the C# limitation of "emit only the last move of each
update"; we additionally recover 2,387 intermediate moves.

**CombatReport nested array -- every metric-input value matches.** This
structure is the sole source of K/D/A, ADR, HS%, multi-kills, and wallbangs,
so it was diffed as a multiset of values (`tools/compare_combat_report.py`,
against `8824794`; upstream leaves `Rounds` as a raw payload, and `8824794`
binds upstream's own `CombatRoundReportsDecoder` to it):

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

**RPC parameters -- every C# value is also ours, and ours has 14 more
records.** The ~330,000 RPCs had parameter payloads that were entirely raw;
they were decoded using the 84 parameter-schema groups (`<Class>:<Function>`
paths) the replay itself declares. Diffed with `tools/compare_rpc_params.py`
(record counts; every difference is vrfkit-only, none C#-only):

```
                                              upstream  8824794  vrfkit
MulticastNotifyKilledEnemy.KillerCharacter         119      132     132
MulticastNotifyDamage_Point.DamageDealt            580      580     581
MulticastEndRound.NewRoundNumber                    17       17      17
```

The other `KilledEnemy` and `Damage_Point` parameters (`KilledCharacter`,
`MultikillLevel`; `DamageTaken`, `RegionalDamage`, `bDamageKilledTarget`) have
the same counts as the line above them.

The 13 kills are the ones by character 576, Gekko (`AggroBot_PC_C`).
`MulticastNotifyKilledEnemy` is hosted on the killer's character actor, and
upstream's Gekko descriptor spells the class path `Aggrobot` where the replay
says `AggroBot`, so upstream drops every RPC on that actor. `8824794` corrects
the path (`f67ea66`) and agrees with vrfkit, which takes names from the replay
and never needed the descriptor. This paragraph used to say the character never
replicates the RPC; it does. The existing pipeline papered over the 13 missing
kills by recovering them later as CombatReport credit, so they vanished from
the timeline. In vrfkit the timeline itself is complete.

The one extra damage record is a killing blow (29.45 dealt, 20 taken) on the
`DamageableComponent` of Gekko's E-ability projectile -- actor 27232, packet
391880, channel 194 -- whose actor closes six packets later. Neither C# build
emits any event for packet 391880, and why is not established
([follow-up](docs/FOLLOWUP.md#remaining-work)). vrfkit already produced it at
`d4731c8`, before the partial header correction, so that is not the cause.

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

The 2026-09-25 [common audit](docs/BUILD_VERIFICATION.md) checks 986 unique
replays across 24 builds. Every ReplayData validation and checkpoint export
succeeds. Independent typed/raw comparisons match 12,917,904
observed values. All 986 pass the strict quality gate. The two ActiveBlinds
fixes resolve the earlier 81-file findings and recover 522 additional typed
children; an independent before/after comparison verifies those values and
preserves every existing field row and raw payload.

The measurements below describe earlier corpus revisions.


The September 2026 follow-up re-exported and retained all 714 replays
(13.01: 215, 13.02: 204, 13.04: 108, 13.05: 187), with checkpoints.
ReplayData block validation passes on every file. Separate main/checkpoint
diagnostics reduce block loss from 240,679 / 53,582 to zero by recovering or
preserving post-RepLayout tails. Unknown payloads remain explicitly raw.
The subsequent [partial-header correction](docs/PARTIAL_HEADER_CORRECTION.md)
reassembles all 125,037 main and 835,967 checkpoint fragments into 293,720
complete bunches. The earlier missing-initial conclusion was a header-order
error; unknown inner payloads still remain explicitly preserved.

In that earlier tail-preservation run, main physical typed coverage increased
from 66.18% to **69.92%**; checkpoint
from 41.24% to **41.30%**. These percentages count non-null value columns, not
game facts understood. The time fields add 37,601,710 typed rows across the
two streams. Three analysis helpers expose physical coverage, the observed
32-ID ability-stat dictionary, and conservative match observations.
See [implementation and measurement notes](docs/FOLLOWUP.md) for exact
denominators, raw/decoded distinctions and before/after evidence.

The historical 2026-08-31 run covered 527 machine-local `.vrf` files with its
then-current preservation-aware oracle (`tools/validate_corpus.py`).

```
succeeded: 527/527        failed: 0
branches : 215  ++Ares-Core+release-13.01
           204  ++Ares-Core+release-13.02
           108  ++Ares-Core+release-13.04
pass rate: min 100.000000%  median 100.000000%  max 100.000000%
totals   : 344,569,357 content blocks / malformed framing 0
```

`malformed framing 0` is the content-block framing counter; it does not prove
that earlier transport stages retained every payload. A 100% block pass rate
means the measured blocks reached ordinary field/RPC rows or an explicit
preservation row. Partial reassembly rejections are outside that score.
It does **not** mean every property is typed: when a block cannot be assigned to
a `_ClassNetCache` group, its inner handles cannot be named, so vrfkit emits one
reserved row (`handle = u32::MAX`) with the complete decoded payload in
`raw_bits` and increments `RPC unresolved/raw`. Such a block is uninterpreted,
not lost, and therefore does not reduce the current oracle pass rate.

### Historical attribution-gap measurement

The earlier release-13.01-only 215-replay run, before unresolved whole-payload
rows were separated from genuine loss in the oracle score, reported:

```
succeeded: 215/215        failed: 0
branches : 215  ++Ares-Core+release-13.01
pass rate: min 97.487378%  median 99.323434%  max 99.682485%
totals   : 136,545,822 content blocks / 98,884,839 fields / 75,571,092 RPCs
           malformed framing 0        unattributed 1,972,019,383 bits
```

An even older implementation also printed 100%, but for the wrong reason: it
silently dropped blocks whose group it could not find and incremented no
counter. Exposing that path first produced the historical 97--99% figures;
preserving unresolved decoded payloads removed that attribution loss. It
did not address the separate field-stream tails until this follow-up.
The exact `185d452`/`a73ee3a` comparison now reproduces the accounting change
on two 13.01 inputs with unchanged block populations and emitted row counts.
That experiment is scoped in [FOLLOWUP.md](docs/FOLLOWUP.md); it does not
establish regression absence for all historical changes or for 13.05, which
both old revisions reject.

In that historical 13.01 measurement, **97.283437% of unattributed bits were
`AbilitiesAndBuffsComponent`**; the replay declared no cache group for that
class. `MeleeAttackState1`-`4` and `_Alt` were already resolved through the
shared `MeleeAttackStateComponent_ClassNetCache`. The attribution limitation
still exists, but the full unresolved payload now survives in `raw_bits` even
though it cannot yet be expanded into named field rows.

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

In that historical 215-replay release-13.01 corpus, the fix took malformed
215 -> 0 and newly decoded 2,150 blocks (10 per replay). Residual under-consumed
bits dropped to 3,671, and those last four cases were later explained as the
handle-minimum-width problem and went to 0.

## Type overlay

For any field whose inner stream can be walked, the raw bits are always
exported; when the type is known, the `value_*` columns are filled **as an
overlay.** If the type is unknown, or decoding fails, the row's `raw_bits`
remains. The only exception is the unresolved ClassNetCache block above: it
cannot be expanded into fields, so it emits one preservation row (`handle` =
`u32::MAX`, full payload in `raw_bits`) and an explicit unresolved/raw
diagnostic rather than pretending the properties were decoded.

The overlay table is extracted mechanically from the C# descriptors
(`tools/extract_descriptors.py`) -- 219 groups, 1,319 entries, 96 handles.
Those descriptors are vendored verbatim in
[`third_party/vrp/`](third_party/vrp/README.md),
and CI regenerates the table from them on every push.
Nothing is transcribed by hand, for the same reason S-boxes and golden vectors
are not: it is the kind of constant where a typo is invisible in review.

Four names resolve without a table entry: `Owner`, `Instigator`, `AttachParent`
and `Controller` are `AActor` / `USceneComponent` object references Unreal
replicates on every actor, always as a NetGUID. The descriptors declare them
only for the classes they happen to cover, which left the same four names typed
on 129 group/field pairs and untyped on 203 more. Since the type is fixed by the
engine rather than by the class, they resolve by name after the table misses --
a claim about Unreal, not a guess about any one Blueprint, and it holds for
groups no replay has spawned yet. In the historical 215-replay release-13.01
export sweep, it typed 6,048 further rows with decode errors still at zero.

`02d4d478` (`02d4d478-1dfb-4412-9a77-29ca29105a9d.vrf`), as recorded by the
committed export baseline `tools/baselines/export_02d4d478.json` after the
partial-header and shot-array corrections:

```
Decoded OK:   796,920      Decode errors:      0
Raw/Skip:      26,507      Not in table: 163,534
No field name:  2,034      Typed:          80.6%
Effect blobs:  61,617
```

The four buckets partition `Rows offered` exactly (796,920 + 26,507 + 163,534 +
2,034 = 988,995), and `Typed` is `Decoded OK / Rows offered`. The figures this
block held until 2026-08-30 partitioned the same 988,983 rows differently -- they
were an older snapshot, taken before overlay entries that moved rows out of `Not
in table`, and they contradicted the baseline this repo commits for the same
replay. `check_docs.py` now compares both the parquet row/byte table and these
counters against that baseline so the same drift cannot go unreported again.

**Effect decoding is additive and does not move these buckets.** The overlay
buckets are settled before the effect pass, so rows that gained a value from an
effect are still counted under `Not in table`; merging them into `Decoded OK`
would double-count and move the baseline for unrelated reasons. `Effect blobs`
is reported separately -- without it, 61,617 rows gain a value yet the summary
prints identically. (The bucket counts themselves do move as overlay entries
are added, which is exactly how the figures above went stale once; they are
whatever `tools/baselines/export_02d4d478.json` currently records.)

Physical value coverage is the fraction of `fields.parquet` rows with at
least one non-null `value_*` column. It cannot be computed by adding overlay,
effect-blob or struct counters: these count different units and may describe
parent/child expansions of the same input. The current reference
baseline has 914,117 typed rows out of 1,296,660 (70.50%), measured directly
from its columns.
Adding raw child windows changes this denominator even when every old typed
value survives; compare raw preservation and newly typed values separately.
That snapshot is not a fraction of all game information understood.

The historical [array and checkpoint expansion](docs/ARRAY_CONTEXT_EXPANSION.md)
predates the [structured-array expansion](docs/STRUCTURED_ARRAY_EXPANSION.md),
which added 75,275,443 raw child windows and 3,510,015 typed reward windows in
that completed export. Both 714-file corpus guards and the all-file comparison
passed at that phase revision.

The earlier [714-replay schema expansion](docs/SCHEMA_EXPANSION.md) independently
verified 6,805,323 additional typed values, with physical coverage of 69.98%
main and 52.90% checkpoint. Raw field columns and non-field exports are unchanged.

Measure the files you actually use, with checkpoint rows reported separately:

```bash
python tools/summarize_value_coverage.py <export-directory> > coverage.json
python tools/summarize_value_coverage.py <parent-of-export-directories> --jobs 4
```

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
include additive decoders like effects and structs, so that is a different
population from the overlay's input rows.) Unknown ordinary properties retain
raw bytes. The absence of a typed value does not by itself establish loss;
conversely, a high typing ratio does not establish complete block preservation.

`fields.parquet` also carries the replay's own `compatible_checksum` per row,
which turns that leftover into something searchable. Unreal hashes a property's
type into it, so it identifies the property across builds; bucketing untyped
rows on it separates three situations that otherwise look identical -- a type
the overlay knows and failed to apply, a described property nothing has typed,
and a value addressed inside a payload that declares no handle at all. Over 20
replays that splits 10,062,142 untyped rows 0.5% / 48.6% / 51.0%, and the first
bucket is supposed to be empty. The recipe is in
[`docs/USAGE.md`](docs/USAGE.md#fieldsparquet).

The historical 215-replay release-13.01 export sweep had **zero decode
errors**, checked by `tools/check_decode_errors_corpus.py`. The historical 108-file
release-13.04 export/checkpoint sweep likewise reports zero overlay, struct and
checkpoint decode failures. This separate check exists because `vrfkit
validate` does not print overlay counters, so `validate_corpus.py` alone cannot
see a wrong type. Reaching zero found three places where the wire disagreed
with the C# declarations; they are recorded with evidence in
`tools/apply_type_corrections.py` (187 corrections, verified with `--check`).

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
across supported releases 11.06 through 13.06: the PRNG and its multipliers, the seed-mix
skeleton, the 64 -> 32 -> 8 -> tail staging, the tail-XOR handling, and even
the S-box table itself. What actually changes per build:

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

In all twenty-four supported builds the **tail-XOR byte equals the low byte of the seed
addend.** It is a derived value, not an independent constant, and the
relationship is pinned by a test in `versions/mod.rs` -- if a future build breaks
the pattern, the test fails instead of the final byte silently corrupting.

So adding a build is one `SeededTransform` impl: two constants and three word
functions (`word64` / `word32` / `byte`); everything else is shared.

**13.02 and 13.04 are confirmed by live measurement, not only golden
vectors.** A 527-replay machine-local sweep on 2026-08-31 covered all three
then-available branches:

```
215  ++Ares-Core+release-13.01
204  ++Ares-Core+release-13.02
108  ++Ares-Core+release-13.04

527/527 succeeded, oracle pass rate 100.000000% on every replay
344,569,357 content blocks / malformed framing 0
```

The 13.04 subset was also exported with checkpoint decoding enabled. All 108
replays contained checkpoints and reported zero typed-overlay errors, zero
struct-blob failures and zero checkpoint failures over 110,152,399 offered
rows, 20,756 decoded struct blobs, 3,129,483 decoded checkpoint fields and
1,872 decoded checkpoint blobs. The
machine-local corpus can rotate; the reproducible transform oracle remains the
88 mechanically extracted upstream golden vectors (11 staging boundaries per
build, eight builds). The 13.06 implementation was also validated on six real
replays; [the upstream parity report](docs/UPSTREAM_PARITY.md) records the
before/after comparisons and the limits of that sample.

The sixteen recovered 11.06--12.09 builds add 1,264 native-machine-code
vectors and a full 48-sample main/checkpoint validation; see the
[build support validation report](docs/LEGACY_BUILD_SUPPORT.md).

The 768-byte S-box is shared across builds, which makes it usable as a
**signature for locating the transform function in a binary.**

## Design

### 1. Separate preservation, framing and typing

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
(`NetFieldExportGroup`) where the group and handle are declared. Missing group
attribution and unnamed handles are explicit cases, not guaranteed names.

So decoding is split into two layers:

- **Base path** -- every field whose inner stream can be walked is emitted as
  `{group, handle, name, bit_count, raw_bits}`. No branch is skipped on the
  grounds that the type is unknown.
- **Overlay** -- if a type is registered for `(group, handle)`, the decoded
  value is emitted alongside it.

If a field's format is discovered later, retained raw payloads can be
reinterpreted. Unresolved ClassNetCache blocks have a whole-payload row rather
than named parameters; interpreting them requires the corresponding schema
and framing. Payloads lost before export require parsing the original replay.

### 2. Minimal cost per build update

See [Supported builds](#supported-builds-and-the-cost-of-a-new-build). Across
the supported builds the only per-build variables are two constants (seed addend,
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
`polars` / `pandas` / `duckdb`. NDJSON is reader-bound: on a 1.8-million-row
movement stream, JSON parsing was measured at 84% of processing time.

## Validation suite

Full detail is in [`docs/USAGE.md`](docs/USAGE.md) section 6. The checks are
layered, and the layers catch different things:

- **Common build audit** (`verify_build_corpus.py`) -- identical validation,
  checkpoint export, quality-counter and independent typed/raw checks on
  every available replay; per-build clean/checked counts are in the support table.
- **Framing** (`validate_corpus.py`) -- content-block framing, loss accounting
  and unresolved-payload preservation.
- **Bytes** (`check_export_baseline.py`, per-file row and byte counts) --
  regression in any of the 28 export counters.
- **Decode** (`check_decode_errors_corpus.py`, scoped export corpora) -- overlay
  type errors and struct-blob failures; the recorded 13.04 scope is all 108 files
  with checkpoints enabled.
- **Semantics** (`check_metrics_baseline.py`, 7 builds) -- round count,
  score, K/D/A invariants that need no baseline.

Two of the headline metrics are **not** "100% / high is good" and reading them
that way is a trap:

- Unresolved ClassNetCache payloads are preserved whole as marked raw rows
  and therefore do not lower the oracle score. Field-stream failures
  lower the score when present. This is not a typing claim: those
  rows cannot yet be split into named properties. `Malformed framing`,
  `Transform failed`, and `RPC payload lost` must remain zero; a non-zero
  `RPC unresolved/raw` count describes preserved, uninterpreted data.
- The **~80.6% `Typed`** ratio reads low because of the *RPC-parameter
  denominator* -- most of `Not in table` is RPC parameters with no C#
  descriptor. A low ratio is uninterpreted, not lost: those rows still carry
  `raw_bits`, and additive decoders (effects, structs, the economy typing)
  fill `value_*` without moving the bucket.

## Generated files

The following files are generated and must never be edited by hand:

| Generated file | Generator | Notes |
|---|---|---|
| `crates/vrf-decode/src/table.rs` | `tools/extract_descriptors.py` then `tools/apply_type_corrections.py` | The overlay table (1,319 entries, 219 groups, 96 handles) and handle table, from the vendored descriptors in `third_party/vrp/` |
| `crates/vrf-decode/src/checksum_table.rs` | `tools/extract_checksum_types.py` | Replay-observed checksum-to-type propagation table; conflicting donors are omitted |
| `crates/vrf-decode/src/scoped_types.rs` | `tools/generate_scoped_types.py` | Exact group/name/checksum primitive types for ambiguous field names; no cross-group propagation |
| `crates/vrf-transform/src/sbox.rs` | `tools/extract_sboxes.py` | 768-byte S-box, shared across builds |
| `crates/vrf-transform/tests/data/golden_vectors.rs` | `tools/extract_golden.py` | Per-build golden test vectors |
| `crates/vrf-transform/tests/data/native_vectors.rs` | `tools/capture_native_transforms.py` | Expected bytes from pinned original executable readers |
| `tools/equippable_table.py` | `tools/extract_equippables.py` | Weapon class path to display name, from the vendored `ValorantEquippableResolver.cs` |

The overlay table's and the equippable table's input is in the tree, so anyone
can regenerate them; CI does, and fails if either result differs from the
committed file:

```bash
python tools/extract_descriptors.py third_party/vrp/Replay.Valorant \
    crates/vrf-decode/src/table.rs
python tools/apply_type_corrections.py
cargo +1.86.0 fmt -p vrf-decode
python tools/extract_equippables.py --check
```

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
