# vrfkit usage

The CLI, output schemas, library use, `tools/` scripts, and validation suite.

Design rationale and the comparison against the existing parser are in
[`../README.md`](../README.md); work history and measurement records are in
[`archive/PROJECT_STATUS.md`](archive/PROJECT_STATUS.md). The byte-level format
of the checkpoint chunks is in
[`archive/CHECKPOINT_SPEC.md`](archive/CHECKPOINT_SPEC.md), and finished task
specs are in [`archive/`](archive/README.md) -- all of these are for the
record, not things to run.

## Table of contents

1. [Build](#1-build)
2. [CLI](#2-cli) -- [`inspect`](#inspect) / [`validate`](#validate) / [`diag`](#diag) / [`export`](#export)
3. [Output](#3-output) -- [`fields`](#fieldsparquet) / [`movement`](#movementparquet) / [`actors`](#actorsparquet) / [`net_guids`](#net_guidsparquet) / [`events`](#eventsparquet) / [`checkpoint_fields`](#checkpoint_fieldsparquet) / [`manifest.json`](#manifestjson)
4. [Using it as a library](#4-using-it-as-a-library)
5. [`tools/` reference](#5-tools-reference) -- [Generators](#generators) / [Validation](#validation) / [Downstream conversion](#downstream-conversion) / [Analysis helpers](#analysis-helpers)
6. [Validation suite](#6-validation-suite)
7. [Supported builds](#7-supported-builds)
8. [Known limits](#8-known-limits)

---

## 1. Build

```bash
cargo +1.86.0 build --release -p vrfkit --locked                       # inspect / validate / export
cargo +1.86.0 build --release -p vrfkit --no-default-features --locked # inspect / validate only
```

`export` is a default feature. Drop it with `--no-default-features` and
`arrow`/`parquet`/`zstd` never enter the dependency tree at all.

```bash
cargo +1.86.0 tree -p vrfkit --no-default-features --locked | grep -E "arrow|parquet|zstd"   # no output
```

A binary built without `export` **refuses the subcommand rather than succeeding
silently**. A subcommand that wrote nothing and exited 0 would be
indistinguishable from one that wrote the files.

---

## 2. CLI

```
vrfkit inspect  <file.vrf> [--redact-identifiers]
vrfkit validate <file.vrf> [--diagnostics]
vrfkit diag     <file.vrf> [--json <path>] [--include-payloads]
vrfkit export   <file.vrf> --out <dir> [--checkpoints]
```

### `inspect`

See what the file is -- ReplayInfo, header, branch, chunk summary. It parses
container metadata without decoding replication payloads. Use the branch to
check the supported-build list and the info flags to check container encryption.

Builds 11.06 through 12.09 also have verified payload transforms. All 48
available samples pass validation and checkpoint-enabled export; see
[build support findings](LEGACY_BUILD_SUPPORT.md).

ReplayInfo includes a free-form friendly name. When command output will be
shared or archived, pass `--redact-identifiers`; the command prints
`Friendly name: [redacted]` while leaving structural header and chunk fields
available for diagnosis.

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

Grammar oracle. It walks ReplayData payloads that reach content-block framing
and reports a block pass rate. **It writes no files.**

```
  Total content blocks: 608020
  Malformed framing:  0          <- blocks whose framing slipped. Nonzero is serious
  Transform failed:   0          <- payload transform failed. Signals an unsupported build
  RPC payload lost:   0          <- unresolved payloads that were not preserved. Must stay zero
  RPC unresolved/raw: 6471       <- full decoded payload preserved, but inner handles cannot be named
  NOT COVERED:          18 Checkpoint chunk(s) were NOT walked
  Field stream failed: 0
  ORACLE PASS RATE:     100.000000% (608011 / 608011 blocks passed)
  VERDICT: PASS - ReplayData block validation passed (exit 0)
```

**Exit code**: `0` ReplayData block validation passed, `1`
validation found a counted failure, `2` there was nothing to validate (no ReplayData
blocks). Those are three different outcomes and are kept apart deliberately --
a file this command cannot read must not be reported as a file that passed.

The oracle walks the **ReplayData stream only**. Checkpoint chunks carry their
own replication framing and are not covered by this verdict; the count above
says how many were skipped. Use `export --checkpoints` to decode them.
Partial reassembly rejections discard payloads before block framing and are
also reported under `NOT COVERED`. They are excluded from the block score and
exit verdict; a pass does not establish end-to-end preservation. Other counted
transport failures, including unfinished partials and resource limits, fail
the verdict.

The example is the preserved `02d4d478` replay after the September 2026
tail-preservation change. All 714 ReplayData block runs in that historical
sweep passed, and the separate checkpoint diagnostic reported no lost framed
blocks. The later [header-order correction](PARTIAL_HEADER_CORRECTION.md)
reassembled all 125,037 main and 835,967 checkpoint partial fragments with
zero partial errors. Unknown inner payloads still
exist: a pass means each measured block was decoded or explicitly preserved, not that
all values have known types or meanings. `RPC unresolved/raw` includes whole
unparsed tails as well as unresolved standalone RPC blocks. See
[the measured before/after results](FOLLOWUP.md).

`--diagnostics` prints context for every failed block. By default it shows up to
32 lines and prints totals / shown / omitted counts in the header.

Export also writes `partials.parquet` for rejected partial fragments and
abandoned partial accumulators. `source` separates main and checkpoint rows;
the checkpoint ID disambiguates their independently numbered packet streams.
Source packet/bit offset and header flags identify the original wire input.
The payload kind distinguishes a single current fragment from an accumulated
buffer; the latter retains its first fragment's source header and a separate
aggregate bit count. These payloads remain unresolved and do not enter the
block validation numerator. Main/checkpoint row and bit totals are available
in both the export summary and manifest quality object.

### `diag`

Walk ReplayData and checkpoint streams without writing Parquet:

```bash
vrfkit diag match.vrf --json failures.json
vrfkit diag match.vrf --json failure-samples.json --include-payloads
```

JSON schema version 3 separates main/checkpoint counters and aggregates by
stream kind, cause, resolved group, function count, handle and consumed bits.
Totals include every failure. Distinct cells are bounded; an explicit overflow
bucket accounts for additional keys. Check overflow before treating the listed
groups as a complete distribution. Whole RPC payloads preserved by the parser
are counted separately from lost streams.

Pre-framing partial diagnostics have a separate attempted-bunch denominator
(`partial_bunches`) and accepted-fragment counter (`partial_fragments`). Cause
counts distinguish missing initial fragments, overlapping initials, mismatched
continuations, alignment refusals, channel closure and resource limits.
Unclassified and overclassified residuals expose incomplete or duplicate
attribution. These are error events: one attempted fragment can trigger more
than one cause, so their sum is not a rejected-fragment percentage.

Payload samples are disabled by default. `--include-payloads` adds bounded
decoded byte samples; prefixes are marked as truncated. The file path and group
identifiers remain in either output. The diagnostic command's successful exit
means it completed its walk, not that the replay was lossless. Use `validate`
for the ReplayData verdict. Ordinary exports do not enable this aggregation.

### `export`

Parquet export.

```bash
vrfkit export replay.vrf --out out/
vrfkit export replay.vrf --out out/ --checkpoints
```

`--checkpoints` reads the Checkpoint chunks as well and **additionally** writes
`checkpoint_fields.parquet`, `checkpoint_actors.parquet`,
`checkpoint_net_guids.parquet`, `checkpoint_blocks.parquet`,
`checkpoint_guid_entries.parquet`, `checkpoint_export_groups.parquet`, and
`checkpoint_export_fields.parquet`.
It is off by default because it is a separate pass
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
  Typed:            80.6% (properties + RPC parameters)
```

(That figure is `02d4d478`'s, from `tools/baselines/export_02d4d478.json`:
`overlay_decoded_ok / overlay_rows_offered` = 796,920 / 988,995. It moves as
overlay entries are added -- re-measure before quoting it.)

The denominator is **every row offered** to the overlay, and thanks to RPC
parameter expansion it includes both replicated properties and RPC parameters.
The two populations have very different type coverage -- the descriptor set grew
up property-first -- so writing "against all fields" without naming the
denominator makes added parameters read like a regression. **Untyped != lost.**
For unknown properties, `raw_bits` preserves the input (see
[`fields.parquet`](#fieldsparquet)).

---

## 3. Output

Measured on `02d4d478` (48,215,213 bytes):

| File | Rows | Bytes | Notes |
|---|---|---|---|
| `fields.parquet` | 1,296,660 | 16,455,178 | |
| `movement.parquet` | 1,844,147 | 31,886,449 | |
| `actors.parquet` | 3,827 | 87,281 | |
| `net_guids.parquet` | 16,167 | 153,606 | |
| `events.parquet` | 195 | 13,411 | |
| `partials.parquet` | 0 | 2,505 | main-only; with checkpoints: 0 rows, 2,505 bytes |
| `checkpoint_fields.parquet` | 352,089 | 1,218,992 | requires `--checkpoints` |
| `checkpoint_actors.parquet` | 3,014 | 27,118 | requires `--checkpoints` |
| `checkpoint_net_guids.parquet` | 74,270 | 277,718 | requires `--checkpoints` |
| `checkpoint_blocks.parquet` | 22,247 | 175,103 | requires `--checkpoints` |
| `checkpoint_guid_entries.parquet` | 74,270 | 928,714 | requires `--checkpoints` |
| `checkpoint_export_groups.parquet` | 8,307 | 27,041 | requires `--checkpoints` |
| `checkpoint_export_fields.parquet` | 49,314 | 287,130 | requires `--checkpoints` |
| `manifest.json` | -- | ~660,030 | varies: it records `elapsed_ms` |

Use [`bench_export.py`](#analysis-helpers) to measure runtime on your machine.
String columns are dictionary-encoded + ZSTD.

### `fields.parquet`

Replicated properties and RPC parameters.

| Column | Type | Description |
|---|---|---|
| `time_ms` | u32 | Milliseconds since replay start |
| `packet_id` | u32 | Packet sequence number |
| `channel_index` | u32 | Actor channel |
| `actor_net_guid` | u32 | Actor NetGUID |
| `object_net_guid` | u32? | Subobject NetGUID |
| `group_path` | str | Export group path; expanded RPC parameters retain the enclosing `_ClassNetCache` group |
| `handle` | u32 | Property handle, or enclosing function handle for expanded RPC parameters/children |
| `field_name` | str? | Name the replay declares for that handle |
| `compatible_checksum` | u32? | The replay's own checksum for that handle -- see below |
| `bit_count` | u32 | Payload size in bits |
| `raw_bits` | bytes? | Raw payload |
| `value_i64` / `value_f64` / `value_bool` / `value_str` | | Only when the type is known |

Qualified map cursor/click vectors use `(x,y,z)` in `value_str`, and HealCauser
references use `value_i64`. Multi-click vectors appear as additive indexed
children immediately before their raw parent. Their inner declaration handle
differs from the exported enclosing function handle. See
[TARGETING_AND_HEAL_VALUES.md](TARGETING_AND_HEAL_VALUES.md) for exact routes,
counts and interpretation limits.

**`compatible_checksum` is what separates "nobody described this" from "we
missed this".** Unreal hashes a property's type into it alongside its name, so
it identifies the property across builds -- the overlay already uses it as a
last-resort type lookup, and exporting it lets a reader run the same reasoning.
Bucket the untyped rows by it and three different situations come apart:

| bucket | meaning |
|---|---|
| checksum present, **in** `CHECKSUM_TYPES` | the type is known and was not applied -- a resolution bug |
| checksum present, **not** in the table | a real coverage gap: a described property nothing has typed |
| **no checksum** | this row carries no checksum; a nested member may still have a separate replay declaration |

`None` means the third of those, not that the export failed to carry a value.
For example, targeting array children have null checksums in their rows even
though their member declaration is checked before decoding.
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

This is a historical raw inventory, not the current typing state.
`ServerMovementTime` and `ReplayLastTransformUpdateTimeStamp` are now Float
after the 714-file wire audit; see [time-field limits](DATA.md). A raw bucket
alone is not evidence that a field should be assigned a type.

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

`raw_bits` is nullable. Unnamed replicated properties and unresolved whole
RPC payloads retain their raw representation; successfully decoded movement
RPCs and synthesized child rows may instead be represented by their decoded
output. At most one `value_*` column is filled per row. Reinterpretation
without re-parsing is possible only where the required raw payload survives.

One exception: a ClassNetCache block whose group cannot be identified cannot be
walked as an inner stream, so it is emitted only as a single **preservation
row** and cannot be expanded into fields.

| Column | Value |
|---|---|
| `field_name` | `__vrfkit_unresolved_class_net_cache_payload__` |
| `handle` | `u32::MAX` |
| `raw_bits` | Full payload |

These preserved blocks are counted under `validate`'s `RPC unresolved/raw`
line, and excluded from `RPC payload lost`. Their retained
raw bytes can be investigated directly; naming their inner fields also needs
the correct class/function schema and replication context.

A content block can contain a RepLayout prefix followed by a ClassNetCache
tail. Prefix properties keep their original rows. Two reserved names identify
additional raw rows from that tail:

| `field_name` | Meaning |
|---|---|
| `__vrfkit_chained_cnc_h1__` | Exact handle-1 RPC body, recognized only with the direct pre-remap `AbilitiesAndBuffsComponent` identity and the measured frame/flag checks. This is structural recovery; no GAS word semantics are assigned. |
| `__vrfkit_unparsed_rep_layout_tail__` | Entire remaining bit window when its inner framing is unknown or rejected. It remains an unresolved RPC failure, counted as preserved when the complete raw copy succeeds. |

Both have exact `bit_count`/`raw_bits`, null `compatible_checksum` and null
typed values. Treat these names as export metadata rather than native game
properties. `RepLayout tails` and `Checkpoint tails` print the decoded/preserved
split; manifest quality uses `rep_layout_cnc_tails_decoded` and
`rep_layout_cnc_tails_preserved` in each stream. A malformed property length
does not become a valid tail merely because bytes remain.

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
- `movement_state` and `move_type` are constant (0, 1) across all
  1,034,035,170 exported rows in the historical 2026-08-31 527-replay corpus (builds 13.01,
  13.02 and 13.04). A future build may break that invariant, so both bytes are
  exported verbatim.
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
| `word0` / `word1` | u32? | First two payload words |
| `payload_tag` | u32? | Stable group tag, only for an exact known layout |
| `payload_name` | str? | Fixed public `EReplayEventGroup` enum name, only for an exact known layout |
| `payload_seconds` | f32? | Payload time in seconds, only when it agrees with `time1` |

The payload is structured as `[u32 tag][N x u32 words][FString][f32 seconds]`,
and `N` is fixed per group (CharacterDeath=2, CharacterUltimateUsed / RoundStart
/ SwitchTeams=1, SpikePlanted / Defused / Exploded=0 -- derived as the
residual-zero count across the corpus). A 527-replay Event-only sweep spanning
13.01, 13.02 and 13.04 consumed all 109,126 payloads exactly, found one stable
tag per group, public enum names no longer than 40 bytes, and a maximum 0.999878
ms absolute difference between `payload_seconds` and `time1`. The nullable
overlay is populated atomically only when arity, tag, name and a 1.001 ms time
tolerance all match. For `characterDeath`, `(word0, word1)` is the `(killer,
killed)` NetGUID; for `roundStarted`, `word0` is the round number. On any future
layout mismatch the overlay stays null and the original remains intact in
`raw_payload`.

### `checkpoint_fields.parquet`

The existing `fields.parquet` columns, preceded by non-null `checkpoint_index`
(UInt32, zero-based checkpoint chunk order) and `checkpoint_id` (Utf8, original
wire ID). Each checkpoint has independent packet, channel and NetGUID state.
Use the checkpoint identity when joining its fields; matching a main-stream
GUID by number alone does not establish that it identifies the same actor.

`checkpoint_actors.parquet` and `checkpoint_net_guids.parquet` carry the same
two identity columns followed by the columns of their main-stream counterparts.
Actor opens are snapshot observations, not new spawns on the main timeline.
The GUID table records the cache after that checkpoint's frame walk. Wire IDs
may repeat; the chunk index keeps those snapshots distinct within one replay.
Initial name-index path entries resolve through the zero-based table of literal
paths that appeared earlier in the same checkpoint. The cache is reset for each
checkpoint; do not carry paths across repeated checkpoint IDs.

### `checkpoint_blocks.parquet`

One row per checkpoint content block, including deleted blocks and blocks
that emit no fields. `block_index` starts at zero in each checkpoint.
`field_row_start` is a zero-based physical row offset in the entire
`checkpoint_fields.parquet`; `field_row_count` includes raw parents and
additive children. A zero count is a real empty interval.

The table preserves actor/object/class GUIDs, header flags, the resolved group,
the resolver branch used, and the GUID paths available at that moment.
`resolution_memo_hit` marks a cached resolution; its source still describes
the original selected branch. A null `class_net_guid` means the header did not
carry a class GUID; zero means the field was read with the invalid GUID value.
These are lookup observations, not proof that an unresolved numeric group is
the enclosing actor's class.

Historical snapshot-versus-main percentages predate the partial-header fix and
do not validate cross-stream identity. See [current context and semantic
evidence](SEMANTIC_CONTEXT_EXPANSION.md).

### Checkpoint schema declarations

These three tables preserve the declarations before each checkpoint's frame.
All carry `checkpoint_index` and the original `checkpoint_id`. Join with the
replay identity and checkpoint index; the wire ID alone may repeat.

| Table | Preserved columns after checkpoint identity |
|---|---|
| `checkpoint_guid_entries.parquet` | `ordinal`, `net_guid`, `outer_net_guid`, `path_is_string`, `literal_path`, `name_index`, `flags` |
| `checkpoint_export_groups.parquet` | `ordinal`, `path_name_index`, `group_path`, `declared_slots` |
| `checkpoint_export_fields.parquet` | `group_ordinal`, `path_name_index`, `slot`, `handle`, `compatible_checksum`, `rendered_name`, `exported_flag`, `fname_kind`, `fname_base`, `fname_index`, `fname_number` |

GUID entry order is the initial wire order, including entries later overwritten
in the cache. `checkpoint_net_guids.parquet` continues to describe the cache
after the frame. A literal numeric string and a name index remain distinct:
`path_is_string` selects exactly one of `literal_path` and `name_index`.
`flags` retains the raw byte; its bit meanings and the name-index lookup scope
are separate questions. The lookup scope is established: `name_index = n`
selects literal entry `n` among earlier literal GUID entries in the same
checkpoint. Indexed entries do not append to that literal table. The public
reader uses `CheckpointPathMode::LiteralPathTable` by default;
`CheckpointPathMode::LegacyDecimal` retains the earlier decimal rendering for
callers that explicitly request it.

Group ordinals retain declaration order, including groups with no populated
fields. `declared_slots` and the populated field slots preserve sparse holes.
Join fields to groups using checkpoint identity and `group_ordinal`.
`exported_flag` retains the nonzero wire byte. For an FName, `fname_kind = 0`
carries `fname_base` and `fname_number`; any nonzero kind carries `fname_index`.
The exact kind byte is retained. This polarity differs from `path_is_string`.
`rendered_name` is the existing parser's string rendering, alongside the
components needed to distinguish forms that render alike.

These are schema and registry observations, not additional gameplay values or
proof of a numeric group's class. The parser's declaration counts and the
writer's row counts are independently checked against the three Parquet files.
See [Checkpoint path resolution](CHECKPOINT_PATH_RESOLUTION.md) for the exact
algorithm, counters, corpus validation, and remaining provenance limit.

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

#### `quality` -- the completeness accounting

Every loss and fallback counter for the run, including the checkpoint pass when
`--checkpoints` is used. One key is the verdict and the rest are the evidence:

| Field | Meaning |
|---|---|
| `content_blocks_lost` | Content blocks whose payload never reached the tables. **Non-zero means the exported tables are missing replicated state.** |
| `event_payloads_decoded` | Event payloads whose exact known arity, tag, public enum name and time relation populated the structural overlay. |
| `event_payload_unknown_groups` | Event groups outside that measured vocabulary; their raw payload remains preserved. |
| `event_layout_mismatches` | Known groups that failed any structural guard; all nullable overlay columns remain empty. |

It is `malformed_content_blocks + transform_failures + field_stream_failures +
max(0, rpc_stream_failures - unresolved_rpc_payloads_preserved)`, computed by
`NetStats::lost_content_blocks` and shared with `validate`'s summary so the two
cannot drift.

Read this together with the oracle pass rate. The current `02d4d478` run has
zero lost blocks and 6,471 preserved unresolved RPC payloads. Its previous
325 field-stream failures were post-terminator tails: 218 now have verified
RPC framing and 107 are preserved whole. `skipped_bits` still includes
preserved, uninterpreted streams, so it is not a count of lost payload bits.

---

## 4. Using it as a library

Take only the layer you need. Every crate is `#![forbid(unsafe_code)]`, and
`vrf-bitio` is `no_std` + optional `alloc`.

| Layer | Crate | Feature flags |
|---|---|---|
| Bit reader / UE wire format | `vrf-bitio` | `alloc` (default; drop it for `no_std`) |
| Payload transform (24 builds) | `vrf-transform` | none |
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

CI compiles every core-only and singleton feature listed in this table, plus
workspace all-features/all-targets, the standalone probe tool, and strict
rustdoc. The exact copy-paste matrix is in
[`CONTRIBUTING.md`](../CONTRIBUTING.md#before-you-open-a-pr).

---

## 5. `tools/` reference

`pip install -r requirements.txt` first (pyarrow, numpy) -- everything below
needs it.

### Generators

**Never hand-edit the output.**

| Script | Produces |
|---|---|
| `extract_descriptors.py` | `crates/vrf-decode/src/table.rs` (overlay table 1,319 + 96 handles) from the vendored C# descriptors in `third_party/vrp/Replay.Valorant` |
| `apply_type_corrections.py` | Applies verified corrections/additions to that file and recomputes the two-line generation header |
| `extract_checksum_types.py` | `crates/vrf-decode/src/checksum_table.rs` -- `compatible_checksum` -> `FieldType`, learned from the fields the overlay table already declares. Needs an export directory rather than the C# tree, since checksums come from the replay. Checksums whose donors disagree are dropped, which is the safety property. Repeat `--export` to widen the basis; the run **merges** into the committed table rather than replacing it, because a checksum this basis did not happen to see is still correct. `--check` asks whether the two agree *where they overlap* -- not whether they are byte-identical, which a content-addressed table cannot be across different sets of replays. |
| `extract_sboxes.py` | `crates/vrf-transform/src/sbox.rs` |
| `extract_golden.py` | `crates/vrf-transform/tests/data/golden_vectors.rs` |
| `extract_equippables.py` | `tools/equippable_table.py` from the vendored `third_party/vrp/Replay.Valorant/Combat/ValorantEquippableResolver.cs`; `--check` runs in CI |

**Order matters:** `extract_descriptors.py` -> `apply_type_corrections.py` ->
`cargo fmt`. The corrections script works on both the just-generated single-line
form and the rustfmt form, but some patterns stop matching after `cargo fmt`. So
the script does not trust its own apply count -- it **re-verifies the final
state after applying** and fails if it disagrees.

```bash
python tools/extract_descriptors.py third_party/vrp/Replay.Valorant \
    crates/vrf-decode/src/table.rs
python tools/apply_type_corrections.py           # apply, then verify (187 corrections)
cargo +1.86.0 fmt -p vrf-decode

python tools/apply_type_corrections.py --check   # verify only
```

CI runs the extract, apply and fmt lines on every push and fails if
`table.rs` then differs from the committed file.

Those 187 corrections are the whole live expectation set the script re-verifies; `ADDITIONS` is the
subset absent from the vendored C# descriptor input (`third_party/vrp`).

The `ADDITIONS` pass inserts items the pinned C# input is **silent on**. There are
currently 125 of them, and every one is admitted on wire evidence written into the
comment above the list -- bit width, value range, distribution -- and nothing else.
The original three still show the bar: `BaseTeamState.LoadoutValue` /
`AverageLoadoutValue` (26-I, where the reference declares the type of the same
property and only moves the group) and `BombGameState.ChosenCeremonyForRound`
(section 32, wire evidence only). Broadening it without evidence voids the very
reason these additions are allowed -- read archive/PROJECT_STATUS.md 26-I and 32
first, and read the "Deliberately NOT added" note in the same comment, which
records the fields that failed the bar and why.

Both counts above are measured, not maintained by hand. `check_docs.py` reads the
187 against `expectation_count(table.rs)`, so a stale one is caught -- but
**nothing checks the `ADDITIONS` figure**, which is why it sat at 70 while the
list held 73. Re-measure it by importing the module rather than counting the
source by eye (`tools/` has to be on the path; the module imports `atomic_io`
from beside itself):

```bash
python -c "import importlib.util,sys; sys.path.insert(0,'tools'); \
spec=importlib.util.spec_from_file_location('atc','tools/apply_type_corrections.py'); \
m=importlib.util.module_from_spec(spec); spec.loader.exec_module(m); print(len(m.ADDITIONS))"
```

### Validation

| Script | What it watches |
|---|---|
| `validate_corpus.py` | Framing (preserved corpus, top level; `--recursive` for subdirectories) |
| `validate_metrics_corpus.py` | Metrics pipeline passes |
| `check_corpus_baseline.py` | Per-build corpus baseline |
| `check_export_baseline.py` | Export counters + per-file rows/bytes/SHA-256 content identity |
| `check_baseline_schemas.py` | All committed baseline schemas, measured SHA-256 hashes, and cross-file replay/counter/table identities. |
| `check_decode_errors_corpus.py` | Overlay type errors + struct blob failures (top level; `--recursive` for subdirectories, `--checkpoints` to also decode Checkpoint chunks) |
| `corpus_scan.py` | Not a check -- the `.vrf` discovery `validate_corpus.py` and `check_decode_errors_corpus.py` share, so the two can no longer glob a directory two different ways and disagree about what "the corpus" is without saying so. Non-recursive by default; read its docstring for why. |
| `check_component_remaps.py` | Whether each Blueprint-component remap still matches. Needs only an export, so it works on a replay from a build that has no baseline -- which is the case a renamed component would otherwise slip through. |
| `check_metrics_baseline.py` | **Semantics** -- rounds, score, K/D/A |
| `compare_combat_report.py` | Metrics-input multiset |
| `compare_rpc_params.py` | RPC parameter comparison |
| `compare_with_csharp.py` | Diff against the C# parser |
| `check_effect_decoder.py` | Effect decoder (12 cases) |
| `check_ascii.py` | Rust source ASCII sweep (147 files) |
| `check_docs.py` | This document itself (below) |
| `atomic_io.py` | Internal containment, recursive-removal and atomic-replacement helpers shared by mutating tools |

Both `validate_corpus.py` and `check_decode_errors_corpus.py` print a
`corpus scope:` line before doing any work, stating how many `.vrf` files were
found, whether the scan was recursive, and how many more sit in subdirectories
and were excluded -- `0 excluded` prints too, not just a nonzero count, so the
line always answers "what did this run actually scan" without having to
compare it against the other tool's output. Pass `--recursive` on either one
to also walk subdirectories; pass it on both if you need them to agree on a
wider corpus than the top level.

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

### Unresolved payload and observation audits

`summarize_unresolved_fields.py` inventories rows with all four value columns
null. It groups by main/checkpoint table, replay build, exact group, name and
checksum, and reports physical occurrences, declared and preserved bit sums,
missing nonempty payloads, zero-bit markers and affected exports.
Raw parents can contain already decoded children; recurrence is not a count of
distinct game facts. Per-export SQLite shards keep aggregation bounded and
`--jobs` controls parallel scanning.

```bash
python tools/summarize_unresolved_fields.py out/exports --output-dir out/raw-audit --jobs 12
python tools/audit_match_observations.py --exports out/exports --out out/ammo-audit.json --jobs 12
python tools/generate_scoped_types.py --check
```

`audit_match_observations.py` compares magazine decreases with an explicit
weapon-scoped continuous-effect RPC through the component's outer NetGUID.
It reports unmatched and ambiguous evidence; it does not classify the RPC as
a shot. Conflicting same-packet ammo values break the transition chain, and
conflicting object mappings cannot support a match. Sampling, when requested,
is evenly spaced by export name, not stratified by game build.

`generate_scoped_types.py` regenerates `scoped_types.rs` from the reviewed
`tools/fixtures/scoped_type_evidence.json`. These primitive types require the
exact group, field name and compatible checksum. They never propagate to an
unobserved class alias or globally by checksum. The ordinary
`validate_type_evidence.py` specification also accepts an optional `checksum`
to independently verify this narrower scope.

### Downstream conversion tools

| Script | What it does |
|---|---|
| `to_valplay_bundle.py` | Parquet -> NDJSON bundle (events/movement/manifest). The format valplay's `compute_metrics.py` consumes |
| `equippable_table.py` | **Generated file.** Weapon class path -> display name |

#### What the bundle manifest carries

The bundle's `manifest.json` forwards the export's `quality` object and its
`net_field_export_groups` table **verbatim**, and adds an `adapter` object of
its own measurements -- `events_written`, `events_time_ms_regressions`,
`field_rows_read`, the movement/net_guid/Event row counts read back against the
ones the export declared, and the conversion `losses` tally.
`server_timeline_rows_read` and `server_timeline_events_written` make the Event
path independently recountable. `bundle_schema_version`
names the shape; valplay's resume marker records it and rebuilds when it moves.

`players` is deliberately **not** forwarded: valplay derives the same table
from the same `BombPlayerState` rows and its version keeps a *set* of character
GUIDs, which is what attributes a resurrected player's kills. Forwarding the
single-character copy would add account UUIDs to a second file and offer a
lossy alternative to the richer table.

An export with no `quality` object is forwarded as `"quality": null`, never as
zeroes: a consumer must be able to tell "nothing was lost" from "nobody
counted".

`events.parquet` crosses as additive `server_timeline_event` rows through a
deliberately narrow allowlist: server `event_group`, `time_ms`, `time2_ms`, and
only `word0`/`word1` for groups whose fixed payload arity Rust already
validated. The neutral `payload_tag`, `payload_name` and `payload_seconds`
fields cross only as a complete tuple after the adapter independently checks
the exact public tag/name allowlist, finiteness and the same 1.001 ms time
tolerance. Older exports and any mismatching row simply omit the tuple. Replay
event id, free-form metadata, payload size and raw payload never enter the
NDJSON bundle. Actor lifecycle rows likewise retain the
already-decoded channel and spawn rotation; absent rotation stays null rather
than becoming a fabricated zero rotation.

The ordering contract is the adapter's: events are written in `(packet_id,
time_ms)` order with a stable sort, so ties keep wire order (actors, then
properties, then RPCs, then server timeline). Event chunks carry no packet id,
so the adapter derives an ordering-only key from the greatest packet observed
at or before their timestamp; that key is never published as a source field.
`time_ms` is **not** guaranteed monotonic -- it comes
from an unvalidated `read_f32` per demo frame and is 0 for a non-finite one --
so a regression in written order is counted in
`adapter.events_time_ms_regressions` rather than repaired by sorting away from
packet order. `crates/vrfkit/tests/adapter_contract.rs` pins the two constants
the adapter shares with the Rust side; `tools/tests/test_to_valplay_bundle.py`
pins the manifest shape and the ordering.

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

`extract_player_effects.py --export <export-directory> --out player-effects.json`
extracts `BlindManagerComponent.ActiveBlinds` updates and
`EffectManagerComponent` continuous-effect start/stop observations, including
the nearsight effect containers. Each record carries `target_identity` and
the original typed members. Only manifest `character_net_guid`, populated from
`SpawnedCharacter`, admits `player_body`; a controlled camera or drone does not
become a player through possession or ownership. Missing/conflicting identities
remain explicit. Non-player observations stay in the output, preserving flash
source evidence even when no player was affected.

Totals count observations, not unique hits. Array re-replication is retained;
there is no inferred cast attribution, explosion timing or start/stop interval
join. Only the main stream is read. Repeated RPC parameter names split adjacent
same-packet invocations; the export has no explicit invocation ID, so these
groups are not proof of unique effects. Untyped members and ambiguous identity
counts are printed even when zero. See [UPSTREAM_REVEALS.md](UPSTREAM_REVEALS.md).

`compare_descriptor_sources.py --baseline <checkout-or-repo::ref>
--candidate <checkout-or-repo::ref> --downstream-table <table.rs> --output audit.json`
compares C# descriptor inputs without fetching or changing their checkouts.
It reports source-file, parsed type and handle changes, plus downstream entries
that wholesale regeneration would remove or overwrite. Git commits, input
digests and the extractor digest identify the compared sources. Changes are
review candidates; unsupported C# syntax can appear only in the source-file
diff, so an empty parsed diff does not prove an unchanged schema. The extractor
understands inherited movement quantization, class-scoped constant paths and the
reviewed ClassNetCache factories. Unsupported forms of these declarations fail
explicitly. The schema-v3 report lists version-selected custom decoders that
remain `Raw` separately.

`validate_type_evidence.py <export-or-parent> <specifications.json>` independently
reads raw payloads against explicit primitive type proposals. Each specification
names an exact exported group and field, and the decoder requires full payload
consumption. This checks structure and observed numeric ranges, not gameplay
meaning. Use it before adding overlay types and when comparing their emitted
values after export (`--compare-typed`). The shipped `tools/fixtures/type_evidence.json`
covers the 38 additions; `tools/fixtures/type_evidence_aliases.json` separately
covers their existing Swiftplay class-alias propagation. Both were checked on
all corresponding observed rows in the 714-replay corpus. A specimen must not
be promoted to gameplay semantics just because this primitive check passes.

`validate_ability_array_evidence.py <export-directory> [...] --compare-typed
--require-routes` checks the measured `ActiveBlinds` and
`MulticastSetPath.NetworkedProjectilePath` routes. It independently reads each
parent's raw bits, requires explicit terminators and exact consumption, and
compares child paths, context, raw bits and typed values. `--require-routes`
rejects an aggregate with no sample of either route; an individual replay may
legitimately contain neither. The tool reads main `fields.parquet`; checkpoint
behavior is separately covered by the export comparison and corpus guards.
See [UPSTREAM_PARITY.md](UPSTREAM_PARITY.md) for the measured sample scope.

`summarize_value_coverage.py <export-or-parent> [--jobs 4]` reads the physical
`value_i64/f64/bool/str` columns and emits JSON to stdout. It counts each row
once if any typed column is non-null, including zero, false and empty strings.
Main and checkpoint denominators stay separate; missing checkpoint tables are
reported by the number of exports containing them. Malformed inputs produce
`complete: false` and a nonzero exit, rather than an apparently complete total.
This measures value presence, not semantic understanding or block preservation.

`--semantic-evidence <catalog.json>` additionally reports rows selected by an
opt-in, reviewed evidence catalog. It is a bounded audit count, never a
semantic-coverage percentage: only `reviewed` claims with exact criteria are
counted; `unknown` and `unsupported` claims remain explicit and uncounted. A
catalog has `schema_version: 1`, a versioned `sources` list, and `claims`.
Every source needs an `id`, `version`, and non-empty `scope` (record build,
replay count, and commands there when known). Every claim names its source,
table, evidence status, and non-empty exact-match `criteria`; reviewed claims
also require a semantic label, review date, evidence note, exact `group_path`
and `field_name`, and enforceable `applicability`. Applicability must name
explicit export-directory IDs and/or replay builds; build applicability is
checked from each export's `manifest.json`. Build strings must match
`replay_build` exactly (for example,
`++Ares-Core+release-13.05`). If both export IDs and builds are supplied, both
restrictions must match. Source scope documents the evidence
sample; claim applicability limits where that evidence may be counted. The
report includes the catalog SHA-256 and full source/claim definitions. A
duplicate claim/source ID, non-finite number, missing applicability, or a
criteria field absent from an export makes the report incomplete and returns
nonzero. Typed values and field names alone do not qualify a row for reviewed
semantic evidence.

| Script | What it does |
|---|---|
| `analyze_coverage.py` | Coverage analysis |
| `extract_ability_stats.py` | Validates a build-scoped Statistic/FText dictionary from exact cast/effect array slots, with main and checkpoint observations separate. Unknown IDs, changed names, missing partners and conflicts remain visible and return a nonzero exit. Counts are snapshots, not casts. |
| `extract_kill_observations.py` | Exports main and checkpoint KillData element snapshots with physical parent-row identity, independently checked raw values, nullable missing members and scoped reference status. Keeps all clocks separately; updates are not deduplicated kills. See [KILL_OBSERVATIONS.md](KILL_OBSERVATIONS.md). |
| `extract_kill_ledger.py` | Retains character-death events, projects component-local KillData state and links mutually unique same-round PlayerState identities. Preserves unmatched events and observations. See [KILL_LEDGER.md](KILL_LEDGER.md). |
| `extract_healing_observations.py` | Retains serialized heal amounts, section state, raw source rows and separate identity corroboration. Amount sums do not establish effective HP restored or player healing credit. See [HEALING_OBSERVATIONS.md](HEALING_OBSERVATIONS.md) for validation status. |
| `extract_fastarray_observations.py` | Writes numeric AbilitiesAndBuffs FastArray headers, deleted/changed item IDs and raw field offsets to NDJSON, retaining each input window and physical row identity. Field meanings remain unknown. See [the wire investigation](GAS_AND_PATCHVOLUME_INVESTIGATION.md). |
| `extract_section_observations.py` | Retains damage, healing, overheal-decay and reset section observations with raw parent/child checks. Distinguishes parentless records, known non-health sections and unresolved references. See [SECTION_OBSERVATIONS.md](SECTION_OBSERVATIONS.md). |
| `extract_section_timeline.py` | Builds observed section timelines with exact predecessors and explicit ordering/lifetime gaps; uses the pure `section_timeline.py` helper. See [SECTION_TIMELINE.md](SECTION_TIMELINE.md). |
| `extract_section_packet_timeline.py` | Retains the strict timeline and adds main packet-order comparisons with separate eligibility and arithmetic counters; uses the pure `section_packet_timeline.py` helper. See [SECTION_PACKET_TIMELINE.md](SECTION_PACKET_TIMELINE.md). |
| `kill_state.py` | Validates complete KillData bases and finisher revisions; compares checkpoint snapshots without counting them as new kills. Python module used by the ledger command. |
| `extract_match_observations.py` | Exports evidence-labelled ammo changes, equip/reload intervals, round balances, team loadouts, defuse observations and economic state. Money decreases and transaction snapshots remain separate; temporal association is not a verified purchase ledger. |
| `extract_ability_lifecycle.py` | Emits ability-path actor candidates with observed open/close/dormant events and explicit Owner/Instigator references. Player links are identity evidence, not proof of casts. Missing closes remain censored; no nearest-player attribution or fixed duration is used. |
| `analyze_raw_properties.py` | Streams a deterministic size-stratified corpus sample (or `--all`) one temporary export at a time and inventories preserved unnamed/raw replicated properties. Reports only build-level counts, bit widths, and anonymous recurrence ranks; it never prints replay paths/names, group/actor/object/handle/checksum identifiers, hashes, or payloads. Exits nonzero if an unnamed property row lacks exact-length `raw_bits`. Use `--format json` for a deterministic, versioned aggregate document. |
| `find_skips.py` | Finds skipped bits |
| `bench_export.py` | Times a full `export` against `tools/baselines/bench.json`. A smoke detector, not a profiler -- wall clock is noisy, so the default tolerance is 25% and it answers "did something get twice as slow", nothing finer. Reports a run *faster* than the baseline too: that means the recorded number no longer describes the code. |
| `extract_active_effects.py` | Derives an `active_effects.parquet` view from an export -- one row per persistent ability instance (smoke/wall/molly/slow/trap/recon/orb) with class, spawn position, and open/close lifetime. A `dormant` event does NOT end an instance -- a settled smoke that stops replicating has not despawned -- so those instances stay open-ended and the summary counts them. The data already lives in `actors.parquet`; this filters and pairs it. |
| `extract_spike_carrier.py` | Derives a `spike_carrier.parquet` view -- one row per spike custody interval, resolved through to the manifest `subject`. Reads `BombEquippable_C.Owner` on the spike's own channel rather than the inventory side, so it covers carrying-in-the-backpack and not just in-hand, and it follows proxy carriers (Gekko's Wingman) back through `Instigator`. |

```bash
python tools/extract_ability_stats.py --export <export-directory> --out ability-stats.json
python tools/extract_match_observations.py --export <export-directory> --out observations.json
python tools/extract_kill_observations.py --export <export-directory> --out kill-observations.json
python tools/extract_kill_ledger.py --export <export-directory> --out kill-ledger.json
python tools/extract_healing_observations.py --export <export-directory> --out healing.json
python tools/extract_fastarray_observations.py --export-dir <export-directory> --out-dir <new-output-directory>
python tools/extract_section_observations.py --export <export-directory> --out sections.json
python tools/extract_section_timeline.py --export <export-directory> --out timeline.json
python tools/extract_section_packet_timeline.py --export <export-directory> --out packet-timeline.json
python tools/extract_ability_lifecycle.py --export <export-directory> --out ability-lifecycle.json
```

Reload observations recognize `ReloadState` and `ReloadStateEmpty`. Each carries
packet endpoints, start/end boundary labels, and left/right censor flags.
`observed_span_ms` measures the retained interval; `duration_ms` is null when an
entry or exit is uncertain. Round resets and unknown state paths break intervals.
`reload_magazine_increases` links positive ammo transitions strictly inside an
observed interval through the same non-null weapon outer GUID. Boundary-packet
ordering is unresolved and excluded. These are supporting observations, not
completed reloads, shots, or a purchase ledger.

The reviewed semantic catalog accepts schema 1 exact field names and schema 2
literal indexed paths, such as
`Rounds[].Reports[].Interactions[].ParticipantSubject`. The latter requires an
exact group and build/export applicability; it does not accept arbitrary regex
or suffix matches. See [the measured evidence](SEMANTIC_CONTEXT_EXPANSION.md).

Repeat `--export` for the stat dictionary when comparing builds. The observed
13.01, 13.02 and 13.04 dictionaries contain 31 IDs each; 13.05 adds ID 27,
`TimeSprinting`, for a union of 32. The previous investigation's 33-ID headline
did not reproduce in the full 714-export scan. These names are wire FText keys,
not independently validated units or causal interpretations of their values.

The raw-property inventory defaults to six size-stratified replays per selected
build and holds only one temporary export at a time. Select builds explicitly,
and opt into an exhaustive run only when its cost is intended:

```bash
python tools/analyze_raw_properties.py ./target/release/vrfkit <corpus> \
  --build 13.02 --build 13.04 --limit-per-build 8
python tools/analyze_raw_properties.py ./target/release/vrfkit <corpus> \
  --build 13.04 --all --format json > raw-property-inventory.json
```

JSON schema version 1 contains `scope`, per-build counts and bit-width
histograms, anonymous recurrence totals/ranks, and the raw-payload integrity
verdict. `identifier_redacted: true` and
`typing_inference_performed: false` are explicit. It contains no replay or
group paths, filenames, actor/object/channel identifiers, field-handle values,
compatible checksums, payloads, or persistent hashes.

The exhaustive 2026-08-31 run covered all 527 then-available replays and 269,994,556
non-ClassNetCache replicated-property rows. Of 59,291,880 raw-only rows,
57,318,004 retained a wire name and 1,973,876 did not. Every unnamed row kept
exact-length `raw_bits` (missing, typed, wrong-length, checksum-attributed and
sentinel-handle violations were all zero); 90.6005% carried a non-zero payload
and 11.6311% were not byte-aligned. The anonymous inventory found 1,699 field
signatures and 1,029 update layouts. Release 13.04 has a larger genuinely new
shape tail: 73.71% of its unnamed rows used a cross-build signature and 77.78%
of its unnamed updates used a cross-build layout, versus approximately 100%
for the two older builds. This establishes preservation and schema drift, not
field meaning; the analyzer deliberately performs no type inference.

---

## 6. Validation suite

### Quick sweep -- after any change

```bash
cargo +1.86.0 test --workspace --locked                              # 714 passing
cargo +1.86.0 clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo +1.86.0 fmt --check
python -W error tools/check_ascii.py --check                         # 147 files
python -W error tools/check_effect_decoder.py --check                # 12 cases
python -W error -m unittest discover -s tools/tests -p "test_*.py"   # 891 tests
python -W error tools/check_docs.py --fast
python -W error tools/apply_type_corrections.py --check              # 187 corrections
python -W error tools/extract_checksum_types.py --export tools/fixtures/checksum_export --check
python -W error tools/extract_equippables.py --check
python -W error tools/check_baseline_schemas.py
# table.rs regenerates from the vendored descriptors (CI runs these too):
python -W error tools/extract_descriptors.py third_party/vrp/Replay.Valorant crates/vrf-decode/src/table.rs
python -W error tools/apply_type_corrections.py
cargo +1.86.0 fmt -p vrf-decode
git diff --exit-code -- crates/vrf-decode/src/table.rs
```

The CI interop gate sets `VRFKIT_INTEROP_DIR` to a private root before Rust's
`write_interop_files` test, then passes that root's exact `interop` child to
`crates/vrf-export/tests/python_interop.py`. The script never selects a
“newest” temp fixture from a different checkout.

**The ASCII rule is correctness, not style.** The Windows console is cp949, so a
single non-ASCII character in a format string truncates output at that point.
Rust sources are ASCII down to the comments.

### Regression guards -- after non-trivial changes

```bash
cargo +1.86.0 build --release -p vrfkit --features export --locked

python tools/check_export_baseline.py --baseline tools/baselines/export_02d4d478.json
# The checkpoint baseline needs --checkpoints; without it every checkpoint
# counter reads as missing and the check fails for that reason alone.
python tools/check_export_baseline.py --baseline tools/baselines/checkpoint_02d4d478.json --checkpoints
for b in 1210 1211 1300 1302 1304 1305; do
  python tools/check_corpus_baseline.py --baseline tools/baselines/build_$b.json
done
python tools/validate_corpus.py ./target/release/vrfkit.exe <corpus>
python tools/check_decode_errors_corpus.py ./target/release/vrfkit.exe <corpus>
python tools/check_metrics_baseline.py
./target/release/vrfkit.exe export <corpus>/02d4d478-1dfb-4412-9a77-29ca29105a9d.vrf --out out/nested
python tools/compare_combat_report.py

# Optional, opt-in: also decode every Checkpoint chunk and check its counters.
# The committed checkpoint check is the single pinned replay above; this is the
# corpus-wide one. Costs real extra time and disk per replay, so it is not part
# of the line above.
python tools/check_decode_errors_corpus.py ./target/release/vrfkit.exe <corpus> --checkpoints
```

`validate_corpus.py` and `check_decode_errors_corpus.py` also accept
`--redact-identifiers`. It replaces the corpus root and replay basenames in
diagnostics with run-local labels such as `replay-0001`; use it whenever logs
may leave the private analysis machine.

These read their inputs from `VRFKIT_CORPUS_DIR` and
`VRFKIT_VALPLAY_DIR` -- see
[Environment](../CONTRIBUTING.md#environment) for what each one points at.
**With the variable unset they print `SKIP` and exit 0**, so read the output
rather than the exit code.

`compare_combat_report.py` is the exception: it exits 2 when either input is
missing. Its C# side is `CliReader export` of the same replay, built from the
vendored descriptor commit and kept machine-local because it carries
per-player values (`compare_rpc_params.py` reads the same export). To produce
it, from a ValorantReplayParser clone that has commit `8824794`:

```bash
REF="$LOCALAPPDATA/vrfkit/csharp-reference/8824794/02d4d478-1dfb-4412-9a77-29ca29105a9d"
git -C <ValorantReplayParser> archive 8824794 Directory.Build.props src | tar -x -C <build-dir>
dotnet build <build-dir>/src/CliReader/CliReader.csproj -c Release -o <cli-dir>   # .NET 10 SDK
<cli-dir>/CliReader export <corpus>/02d4d478-1dfb-4412-9a77-29ca29105a9d.vrf --output "$REF"
grep CombatReportComponent "$REF/events.ndjson" > "$REF/combat_report.ndjson"
grep rpc_received "$REF/events.ndjson" | grep -E \
  'MulticastNotifyKilledEnemy|MulticastNotifyDamage_Point|MulticastEndRound' > "$REF/rpc_params.ndjson"
rm "$REF/events.ndjson" "$REF/movement.ndjson"   # 3.2 GB; only the lines above are read
```

Upstream (`b51d674`) will not do: it leaves CombatReport `Rounds` as a raw
payload, and its Gekko descriptor misses every RPC on Gekko's character (see
the README's C# comparison). On this replay `compare_combat_report.py` matches
all ten shapes; `compare_rpc_params.py` exits 1 on one damage record vrfkit
has and the C# export does not, documented there.

### What each check catches -- this is the point

| Check | Watches | Misses | Cost |
|---|---|---|---|
| `validate_corpus.py` | Framing (top level of the corpus dir; `--recursive` for subdirectories) | Type errors, broken semantics | ~30 s |
| `check_export_baseline.py` | 28 export counters + per-file rows/bytes | Other builds | 1 s |
| `check_decode_errors_corpus.py` | Overlay type errors + struct blob failures (top level; `--recursive` for subdirectories) | Broken semantics; Checkpoint chunks, unless `--checkpoints` | ~50 s |
| `check_decode_errors_corpus.py --checkpoints` | The same, plus every Checkpoint chunk's overlay and struct-blob decode | Broken semantics | slower: `vrfkit export` also decodes every Checkpoint chunk per replay |
| `check_metrics_baseline.py` | **Semantics** -- rounds, score, K/D/A (7 builds) | Errors in the metrics pipeline itself | ~46 s |
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

| Build | Clean/checked | Verified by |
|---|---:|---|
| 11.06 | 3/3 | Validation + checkpoints + typed/raw |
| 11.07 | 3/3 | Validation + checkpoints + typed/raw |
| 11.08 | 3/3 | Validation + checkpoints + typed/raw |
| 11.09 | 3/3 | Validation + checkpoints + typed/raw |
| 11.10 | 3/3 | Validation + checkpoints + typed/raw |
| 11.11 | 3/3 | Validation + checkpoints + typed/raw |
| 12.00 | 3/3 | Validation + checkpoints + typed/raw |
| 12.01 | 3/3 | Validation + checkpoints + typed/raw |
| 12.02 | 3/3 | Validation + checkpoints + typed/raw |
| 12.03 | 3/3 | Validation + checkpoints + typed/raw |
| 12.04 | 3/3 | Validation + checkpoints + typed/raw |
| 12.05 | 3/3 | Validation + checkpoints + typed/raw |
| 12.06 | 3/3 | Validation + checkpoints + typed/raw |
| 12.07 | 3/3 | Validation + checkpoints + typed/raw |
| 12.08 | 3/3 | Validation + checkpoints + typed/raw |
| 12.09 | 3/3 | Validation + checkpoints + typed/raw |
| 12.10 | 1/1 | Validation + checkpoints + typed/raw |
| 12.11 | 1/1 | Validation + checkpoints + typed/raw |
| 13.00 | 1/1 | Validation + checkpoints + typed/raw |
| 13.01 | 215/215 | Validation + checkpoints + typed/raw |
| 13.02 | 205/205 | Validation + checkpoints + typed/raw |
| 13.04 | 108/108 | Validation + checkpoints + typed/raw |
| 13.05 | 401/401 | Validation + checkpoints + typed/raw |
| 13.06 | 6/6 | Validation + checkpoints + typed/raw |

All rows use the [common 2026-09-25 audit](BUILD_VERIFICATION.md): 986 unique
replays, all 986 strictly clean after the two ActiveBlinds fixes. Every
replay passes block validation and checkpoint export; all observed evidence
values match the independent Python decoder. `Clean/checked` also requires
zero array and array-leaf errors. The report defines each denominator.

The following measurements retain their original dates and parser revisions.

The historical 714-file sweep passed ReplayData block validation and separately
reported zero checkpoint block loss. All 961,004 partial fragments now reassemble
with zero partial errors. Physical typed coverage is 70.8088% main and 78.2028%
checkpoint; [PARTIAL_HEADER_CORRECTION.md](PARTIAL_HEADER_CORRECTION.md) gives
exact denominators, the corrected header interpretation and remaining limits.
Historical measurements follow; their percentages are not current results. The 2026-09-07 full sweep exported all 714
files but found field-stream loss in every `validate` run. The main weighted
block preservation rate was 99.949053%; checkpoint was 99.589353%.

The earlier multi-build sweep (2026-08-31) reported:

```
527/527 oracle passes at 100%: 215 build 13.01 + 204 build 13.02 + 108 build 13.04
13.04 export/checkpoints: 108/108 readable, decode/struct/checkpoint failures 0
```

Build 13.05 landed later and was swept on its own (2026-09-07). Method:
`vrfkit validate` run once per file over the 51 files in the same corpus whose
branch header reads `++Ares-Core+release-13.05`, reading the `ORACLE PASS RATE`
line; the comparison figures come from four-file samples of each older build
validated the same day with the same binary.

```
13.05: 51/51 parsed, RepLayout oracle pass rate
       min 99.929554% / mean 99.950249% / max 99.962170%
same-day samples: 13.01 99.933-99.947%, 13.02 99.953-99.961%, 13.04 99.941-99.959%
```

Before this tail-preservation change, the uncapped audit found Ares accounted for
239,134 of the 240,679 main field-stream failures. The largest shape stops
after 9 bits (142,025 blocks); 185 bits is only one of several shapes.
The old checkpoint loss was 53,582 blocks: 52,201 at 185 bits, 105 at 217 and 1,276
at 233. These are
failure locations, not proof that CachedAttributeSet itself was broken.
The current implementation retains those tails; the measured decoded/raw split
is in FOLLOWUP.md. Unresolved RPC payloads are not additional lost blocks.

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

- **Untyped residual** -- the [`export`](#export) `Typed` is ~80.6% (denominator
  including RPC parameters). **Untyped != lost** (`raw_bits` preserved). Typing
  the rest needs the game binary or UE headers -- this is not a table-editing
  problem (archive/PROJECT_STATUS.md section 24).
- **`AbilitiesAndBuffsComponent`** -- the replay declares no ClassNetCache group
  for that class at all. The historical 13.01 checkpoint sweep confirmed it
  across 4,024 checkpoints. Its complete block payload is now preserved in one
  marked raw row, so this typing/attribution gap no longer lowers the current
  `validate` score. Separate field-stream failures still lower that score.
- **Scoreboard stats** -- release-13.02 replicates cumulative K/D/A through
  `BasicCombatStatsComponent` and cumulative combat score through
  `PlayerScoreComponent`. vrfkit types all four as `Int32`; valplay computes
  ACS as final Score divided by played rounds.
- **Team economy schemas** -- legacy `TeamEconomy` and newer `BaseTeamState`
  values require separate joins. Availability in vrfkit does not establish
  that a downstream metrics deployment consumes both schemas.
- **Non-Bomb game modes** -- five of the 215-corpus are Swiftplay, and **the
  parser side is done** (section 33): `GROUP_ALIASES` maps Swiftplay's
  GameState/PlayerState to the Bomb classes, so the fields all gain types. The
  five hardcoded class names in valplay's `compute_metrics.py` have been
  replaced with `is_game_state` / `is_player_state`, so `docs/swiftplay-metrics.patch`
  is applied and has been removed. The inspected valplay detail modules also
  use those shared helpers; the earlier claim that all three remained
  Bomb-only was stale. Deployment and UI verification belong to valplay.
- **Damage precision** -- vrfkit preserves the exact fractional wire damage.
  valplay additionally floors each final engagement segment before summing its
  scoreboard damage, which reproduces Tracker ADR without discarding the exact
  float total.

### Native transform evidence

`recover_native_binaries.py --binaries <archive-root> --output <recovered-root>`
recovers the seven protected 11.06--12.00 code images from SHA-256-pinned EXE
and `stub.dll` pairs. It needs optional `pefile`, `unicorn` and `numpy` packages.
Only analysis copies are written; both input and output hashes are checked.
Use `--build 11.06` to select one build, or omit it for all seven. See the
[offline recovery measurements](BUILD_RECOVERY_RESEARCH.md).

`capture_native_transforms.py --binaries <archive-root> --recovered-binaries <recovered-root> --check`
verifies all 1,264 committed 11.06--12.09 vectors against the pinned native
PE readers. It requires optional `pefile` and `unicorn` packages and the
executable layout documented in [LEGACY_BUILD_SUPPORT.md](LEGACY_BUILD_SUPPORT.md).
Without `--check`, it regenerates `crates/vrf-transform/tests/data/native_vectors.rs`.
Ordinary tests read the vectors without requiring proprietary binaries or an
emulator. Game binaries and replay exports are not committed.


### Uniform build verification

`verify_build_corpus.py` runs the same checks on every unique replay across
one or more recursive corpus roots. Inputs are deduplicated by SHA-256.
Each replay receives `validate`, `export --checkpoints`, required-counter
and Parquet row-count checks, and independent Python comparisons for the
observed fields in `public_fixture_type_evidence.json`. Missing counters,
nonzero framing/transform/array/type failures and changed inputs fail the
strict audit. Every build must also show positive checkpoint block and
decoded-value counts; otherwise `build_errors` makes the command fail. Unknown
RPCs preserved whole are counted separately from loss.
Unobserved evidence fields are reported as absent, never as verified values.

```powershell
python tools/verify_build_corpus.py --exe target/release/vrfkit.exe --corpus '<replay-root>' --corpus '<preserved-fixture-root>' --work-dir '<new-private-work-dir>' --output '<new-report.json>' --jobs 4
```

Work and report paths must be new. Logs, manifests and exports remain under
the private work directory. The JSON report contains build counts and input
hashes, without source filenames or player identifiers. An unsuccessful
strict audit still writes its results and exits nonzero. Read
[BUILD_VERIFICATION.md](BUILD_VERIFICATION.md) for the latest measured results.
`check_docs.py` checks both supported-build tables against that committed
report and the Rust registry, including the same verification wording and
clean/checked denominators. The native/upstream vector counts remain separate
arithmetic evidence; they do not substitute for any replay check.
