# vrfkit usage

The CLI, output schemas, library use, `tools/` scripts, and validation suite.

Design rationale is in [`../README.md`](../README.md).

## Table of contents

1. [Build](#1-build)
2. [CLI](#2-cli) -- [`inspect`](#inspect) / [`validate`](#validate) / [`diag`](#diag) / [`export`](#export)
3. [Output](#3-output) -- [`fields`](#fieldsparquet) / [`movement`](#movementparquet) / [`actors`](#actorsparquet) / [`net_guids`](#net_guidsparquet) / [`events`](#eventsparquet) / [`partials`](#partialsparquet) / [`checkpoint_fields`](#checkpoint_fieldsparquet) / [`manifest.json`](#manifestjson)
4. [Using it as a library](#4-using-it-as-a-library)
5. [`tools/` reference](#5-tools-reference) -- [Generators](#generators) / [Validation](#validation) / [Reading the installed game](#reading-the-installed-game) / [Downstream conversion](#downstream-conversion-tools) / [Analysis helpers](#analysis-helpers)
6. [Validation suite](#6-validation-suite)
7. [Supported builds](#7-supported-builds)
8. [Known limits](#8-known-limits)

---

## 1. Build

Build commands are in [CONTRIBUTING.md](../CONTRIBUTING.md#build). `export` is
a default feature: without it (`--no-default-features`) `arrow`/`parquet`/`zstd`
never enter the dependency tree (`cargo +1.86.0 tree -p vrfkit
--no-default-features --locked | grep -E "arrow|parquet|zstd"` prints nothing),
and the binary **refuses** `export` rather than exiting 0 having written
nothing.

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

ReplayInfo includes a free-form friendly name. When command output will be
shared or archived, pass `--redact-identifiers`; the command prints
`Friendly name: [redacted]` while leaving structural header and chunk fields
available for diagnosis.

```
=== Header ===
  Replay version:   5.3.2 (changelist 2152573997)
  Branch:           ++Ares-Core+release-13.01
  Platform:         LinuxServer
=== Chunks ===
  ReplayData:       19 chunks (43394226 bytes)
  Checkpoint:       18 chunks
  Event:           195 chunks
  Unknown:           0 chunks
  Header (extra):    0 chunks
```

`Unknown` counts chunks of a type no reader knows and `Header (extra)` header
chunks after the first; both print their zero.

### `validate`

Grammar oracle. It walks ReplayData payloads that reach content-block framing
and reports a block pass rate. **It writes no files.**

```
  Total content blocks: 608080
    Unopened channel:   0 bunches / 0 bits  <- whole bunches dropped: their channel had no open actor. Must stay zero
    Malformed framing:  0      <- blocks whose framing slipped. Nonzero is serious
    Transform failed:   0      <- payload transform failed. Signals an unsupported build
    Field stream failed:0
    RPC payload lost:   0      <- unresolved payloads that were not preserved. Must stay zero
    RPC unresolved/raw:6490    <- full decoded payload preserved, but inner handles cannot be named
  Unknown chunks:       0
  NOT COVERED:          18 Checkpoint chunk(s) were NOT walked - this verdict covers the ReplayData stream only (use `export --checkpoints` to decode them)

  ORACLE PASS RATE:     100.000000% (608071 / 608071 blocks passed)
  VERDICT: PASS - ReplayData block validation passed (exit 0)
```

(`02d4d478`; the 608,080 blocks include 9 deleted ones, which the pass rate
does not score.)

**Exit code**: `0` ReplayData block validation passed, `1`
validation found a counted failure, `2` there was nothing to validate (no ReplayData
blocks). Those are three different outcomes and are kept apart deliberately --
a file this command cannot read must not be reported as a file that passed.

`ReplayData unread` (`replay_data_trailing_bytes` in the manifest and the diag
JSON) counts ReplayData bytes no reader consumed: an outer chunk longer than
its inner size, and archive bytes the Oodle codec never read. Any nonzero
count fails the verdict.

The oracle walks the **ReplayData stream only**. Checkpoint chunks carry their
own replication framing and are not covered by this verdict; the count above
says how many were skipped. Use `export --checkpoints` to decode them.
Partial reassembly rejections discard payloads before block framing and are
also reported under `NOT COVERED`. They are excluded from the block score and
exit verdict; a pass does not establish end-to-end preservation. Other counted
transport failures, including unfinished partials, resource limits,
bunches dropped because their channel had no open actor and package-map export
bunches (`Package map exports:`; nothing after the exports is read), fail the verdict.
That holds even when the channel's open arrived in a rejected partial
fragment: the fragment itself stays unscored, but the complete bunches dropped
after it count as loss. The exception is a rejected fragment that reopened a
channel still holding a live actor: it retires nothing, so later bunches are
framed under that actor rather than dropped (see
[FOLLOWUP.md](FOLLOWUP.md)).

A pass means each measured block was decoded or explicitly preserved, not that
all values have known types or meanings; `RPC unresolved/raw` includes whole
unparsed tails as well as unresolved standalone RPC blocks.

`validate` always prints up to 32 stream-failure lines
(`=== Stream failures (N shown) ===`). `--diagnostics` adds every logged
diagnostic event, up to 16,384; the header gives the total, and the shown and
dropped counts once the cap is reached.

Rejected partial fragments and abandoned partial accumulators are preserved in
[`partials.parquet`](#partialsparquet); they remain unresolved and do not enter
the block validation numerator.

### `diag`

Walk ReplayData and checkpoint streams without writing Parquet:

```bash
vrfkit diag match.vrf --json failures.json
vrfkit diag match.vrf --json failure-samples.json --include-payloads
```

JSON schema version 4 separates main/checkpoint counters and aggregates by
stream kind, cause, resolved group, function count and handle. Each failure
aggregate carries `reconciled`: whether its totals agree with the pass's own
net counters, which the one-line `diag:` receipt on stderr also prints.
`chunks` and `checkpoint_meta` also carry the ExternalData blobs and bytes and
the GameSpecificFrameData bytes the DemoFrame walk skipped undecoded, and the
frames whose time was NaN or infinite (`replay_data_non_finite_frame_times` in
`chunks`, `non_finite_frame_times` in `checkpoint_meta`).
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
vrfkit export replay.vrf --out out/replay
vrfkit export replay.vrf --out out/replay --checkpoints
```

`--out` must be new, empty, or hold only what an export writes (the thirteen
tables and `manifest.json`). Anything else in it -- the replay itself, a
subdirectory, another tool's output, `desktop.ini` -- makes `export` exit 1
before decoding and name what it found: publishing replaces the whole
directory, which would delete them.

`--checkpoints` reads the Checkpoint chunks as well and **additionally** writes
`checkpoint_fields.parquet`, `checkpoint_actors.parquet`,
`checkpoint_net_guids.parquet`, `checkpoint_blocks.parquet`,
`checkpoint_guid_entries.parquet`, `checkpoint_export_groups.parquet`, and
`checkpoint_export_fields.parquet`.
It is off by default because it is a separate pass
that reads roughly 10% more of the file, and **with or without it, the five
original tables (`fields`, `movement`, `actors`, `net_guids`, `events`) are
byte-for-byte identical.** `partials.parquet` is shared by both passes: any
checkpoint partial rejections land there too, distinguished by `source`.

#### If an export is interrupted

`export` never writes into `--out` itself. Every table goes into a sibling
directory, `.<out>.vrfkit-staging-<pid>-<n>`, and only once the manifest --
the last file written -- is complete is that directory renamed to `--out`. A
prior `--out` is moved aside to `.<out>.vrfkit-previous-<pid>-<n>` for the
length of that rename and deleted after it. `--out` therefore always holds
either the previous complete export or the new one, never a mixture. If
something an export does not write appeared in `--out` while it ran, the
moved-aside copy is kept instead of deleted, and a `warning:` names it and
what it holds.

- **An error during the run** removes the staging directory and leaves
  `--out` as it was. A failed publication names the step, the paths, and
  whether the prior output is back in place.
- **A killed process** -- `Stop-Process -Force`, power loss, or Ctrl+C in a
  Windows console, whose default handler exits without unwinding -- cannot
  clean up, so the staging directory stays: Parquet files without their
  footers, and no `manifest.json`. A kill exactly between
  the two renames instead leaves `--out` missing and the complete prior
  export in the `previous` sibling.
- **The next export to the same `--out`** prints one `warning:` line per
  such sibling before it starts, and deletes nothing: a staging directory
  may belong to an export that is still running, and a `previous` sibling
  beside a missing `--out` may be the only copy of that output (the warning
  says so). Beside an existing `--out`, the warning names anything in a
  `previous` sibling that an export does not write. Delete a leftover
  yourself once no export to that destination is running, or move a
  `previous` sibling elsewhere to keep it.
- **The corpus tools never read a leftover as an export.**
  `validate_type_evidence.py`, `summarize_value_coverage.py` and
  `summarize_unresolved_fields.py` skip these names while discovering exports
  and list what they skipped under `skipped_generated_dirs` in their reports;
  `export_scan.py` is the one definition. They also refuse a discovered
  directory without `manifest.json`, since only a finished export has one.

#### Lines to actually watch in the summary

```
  Malformed pkts:   0        <- nonzero means framing is broken
  Struct blobs:     207 decoded / 0 failed
  Decode errors:    0        <- nonzero means an overlay type is wrong
  Sink tally:       1296660 fields / 342990 RPCs / 2028 opens / 1799 closes / 608080 content blocks
  Route children:   563 player info / 8527 rewards / 4100 selected / 1386 kills / 1364 active effects / 2670 ignore actors / 0 blinds / 0 projectile path
  RPC param walks:  113215
```

`Sink tally` counts what the sink wrote (its `fields` equals the
`fields.parquet` rows); `Route children` counts each measured array route's
child rows, and `RPC param walks` the RPC parameter walks that `Truncated
RPCs` is a share of. `--checkpoints` prints the same lines as `Checkpoint
route children:` and `Checkpoint RPC walks:`.

`Movement section trailers` and `Checkpoint movement section trailers` count
components and bits left outside a fitting, nonzero `movementBitCount` window.
They are separate from the unread bits inside a sized/open section and from
the byte-envelope trailers. Both lines print even at zero. The common build
audit and decode-error corpus guard reject nonzero section trailers.
The baseline guard validates and prints these fresh counts against the
manifest without adding guessed measurements to older baseline snapshots.

`Struct blobs` is the output of the dedicated decoders for `RoundResults` /
`TeamEconomy` / `RoundInfos`. These decoders are **additive**, so failing
completely does not move a single other counter -- when build 13.02 shifted a
handle, the entire summary looked healthy while the match score simply
disappeared. **`0 decoded` is an alarm
even if `failed` is 0.** On failure, the `Struct blob err:` line prints the
member and handle by name.

#### Reading the `Typed` ratio

```
  Typed:            83.1% (properties + RPC parameters)
```

(That figure is `02d4d478`'s, from `tools/baselines/export_02d4d478.json`:
`overlay_decoded_ok / overlay_rows_offered` = 822,251 / 988,995. It moves as
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
| `fields.parquet` | 1,296,660 | 12,692,351 | |
| `movement.parquet` | 1,844,147 | 19,984,802 | |
| `actors.parquet` | 3,827 | 76,830 | |
| `net_guids.parquet` | 16,167 | 114,423 | |
| `events.parquet` | 195 | 12,455 | |
| `partials.parquet` | 0 | 2,505 | main-only; with checkpoints: 0 rows, 2,505 bytes |
| `checkpoint_fields.parquet` | 352,089 | 1,193,006 | requires `--checkpoints` |
| `checkpoint_actors.parquet` | 3,014 | 25,848 | requires `--checkpoints` |
| `checkpoint_net_guids.parquet` | 74,270 | 175,916 | requires `--checkpoints` |
| `checkpoint_blocks.parquet` | 22,247 | 112,649 | requires `--checkpoints` |
| `checkpoint_guid_entries.parquet` | 74,270 | 219,662 | requires `--checkpoints` |
| `checkpoint_export_groups.parquet` | 8,307 | 16,481 | requires `--checkpoints` |
| `checkpoint_export_fields.parquet` | 49,314 | 106,370 | requires `--checkpoints` |
| `manifest.json` | -- | ~660,030 | varies: it records `elapsed_ms` |

Use [`bench_export.py`](#analysis-helpers) to measure runtime on your machine.
String columns are dictionary-encoded + ZSTD. Other columns are PLAIN + ZSTD
unless a dictionary measured smaller for that column; the per-table lists and
the measurement are in [PERFORMANCE_NOTES.md](PERFORMANCE_NOTES.md#dictionary-encoding-is-chosen-per-column).

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

Qualified map cursor/click vectors use `(x,y,z)` in `value_str`. The heal and
overheal-decay references (`HealCauser`, `DecayCauser`, and both RPCs'
`EventInstigator` and `EventInstigatorPawn`) use `value_i64`. A `DecayCauser`
of 0 is the null NetGUID -- no causer -- not an actor. `EventInstigator` is a
PlayerController reference that never joins to `actors.parquet`; that is
expected, not a decode fault. The player-state GUID words `A`..`D` are
`UInt32`, so their `value_i64` is never negative even when the high bit is
set. Multi-click vectors appear as additive indexed
children immediately before their raw parent. Their inner declaration handle
differs from the exported enclosing function handle.

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

The middle bucket is a work list, not a bug list -- some of it is deliberate
(`InputEventData`, a tagged union with no meaning source, is its largest
member). A raw bucket alone is not evidence that a field should be typed.

Note what separates the second and third buckets among RPCs, since both hold
`ClassNetCache` rows: an RPC whose parameters were resolved gets a checksum per
parameter and lands in the second, while an RPC whose payload could not be
split into parameters is emitted whole with no declared handle and lands in the
third. `ClientPlayOneShotEffectAtLocation.249` sits in the second because it is
a parameter -- one whose name the replay gives as a bare number (the hardcoded
FName index of `Rotation`), and whose sibling `248` this repo already types as
a `VectorDouble`.

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
look.

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
- `movement_state` and `move_type` were constant (0, 1) on every row of a
  527-replay 13.01-13.04 corpus; both bytes are exported verbatim in case a
  build breaks that.
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
| `spawn_vx` / `spawn_vy` / `spawn_vz` | f32? | Spawn velocity, world units per second; 0 when the spawn block sent none |

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
residual-zero count across the corpus; the sweep, tags and time bound are in
[the README](../README.md#the-event-chunk----the-servers-own-timeline)). The
nullable overlay is populated atomically only when arity, tag, name and a
1.001 ms time tolerance all match. For `characterDeath`, `(word0, word1)` is
the `(killer, killed)` NetGUID; for `roundStarted`, `word0` is the round
number. On any future layout mismatch the overlay stays null and the original
remains intact in `raw_payload`.

### `partials.parquet`

One row per rejected partial fragment or abandoned partial accumulator, with
its exact raw bits. These are preserved evidence, not reconstructed blocks or
RPCs, and they do not enter the block validation numerator. With
`--checkpoints`, both streams share this table.

| Column | Type | Description |
|---|---|---|
| `source` | str | `main` or `checkpoint` |
| `checkpoint_id` | str? | Original checkpoint wire ID; null on main rows. Each checkpoint numbers its packets independently |
| `payload_kind` | str | `current_fragment` (one fragment) or `accumulated_payload` (an assembly buffer) |
| `reason` | str | Rejection cause: `PartialPayloadReason` (`crates/vrf-net/src/pipeline/mod.rs`) in snake case, e.g. `missing_initial`, `channel_closed`, `end_of_stream` |
| `source_packet_id` | i32 | Packet of the source bunch |
| `source_payload_bit_offset` | i64 | Bit offset where the source bunch's payload begins within that packet |
| `rejection_packet_id` | i32? | Packet whose event rejected the payload; null for an accumulator still open at the end of the stream (`end_of_stream`) |
| `channel_index` | u32 | Channel |
| `channel_sequence` | i32 | The channel's reliable sequence, or the packet ID for an unreliable partial |
| `open` / `close` / `dormant` / `replication_paused` / `reliable` / `partial` / `partial_initial` / `partial_final` / `has_package_map_exports` / `has_must_be_mapped_guids` | bool | Source bunch header flags |
| `close_reason` | u8 | Source bunch close reason |
| `source_payload_bit_count` | i32 | Source bunch's payload size in bits |
| `bit_count` | u64 | Preserved bits |
| `raw_bits` | bytes | The preserved bits; bits past `bit_count` in the last byte are zero |

An accumulator's source columns describe its first fragment, while its
`bit_count` covers the assembled buffer. Main and checkpoint totals are in the
export summary (`Partial raw rows`, `Checkpoint partial raw`) and in the
manifest's `quality` object (`partial_rows` / `partial_bits`,
`checkpoint_partial_rows` / `checkpoint_partial_bits`).

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
| `character_net_guid` | The last non-zero `SpawnedCharacter` NetGUID |
| `character_net_guids` | Every non-zero `SpawnedCharacter` value in write order: a reconnect gives a player a second pawn |

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
| `unknown_chunks` | Chunks of a type no reader knows, printed as `Unknown chunks:` by `export` and `validate`. |
| `net`, `sink` (and under `checkpoints`) | Every replication and sink counter, zeros included: among them the nine `partial_*` cause counters and `fields_emitted` (the `fields.parquet` rows, the `Sink tally` figure). |
| `event_payloads_decoded` | Event payloads whose exact known arity, tag, public enum name and time relation populated the structural overlay. |
| `event_payload_unknown_groups` | Event groups outside that measured vocabulary; their raw payload remains preserved. |
| `event_layout_mismatches` | Known groups that failed any structural guard; all nullable overlay columns remain empty. |
| `movement_envelope_trailers`, `movement_envelope_trailer_bits` | In each `sink` block: byte-wrapped movement streams and the bits after their envelopes, which nothing reads. Printed as `Envelope trailers:` (`Checkpoint envelope trailers:`); 24 bits per stream on every measured replay, which `verify_build_corpus.py` requires. |
| `active_blinds_empty_trailers` | In each `sink` block: empty `ActiveBlinds` deltas whose one trailing zero byte the strict array walker was spared; the parent row keeps it. Printed as `ActiveBlinds trailers:` (`Checkpoint ActiveBlinds trailers:`). |
| `frame_non_finite_times` | DemoFrames whose time was NaN or infinite; their packets carry 0 ms. `checkpoints.checkpoint_frame_non_finite_times` counts the snapshot frames. Printed as `Frame times:` (`Checkpoint frame times:`), and by `validate`. |
| `checkpoints.checkpoint_trailing_bytes` | Checkpoint bytes no reader consumed: bytes after an archive inside its chunk, and archive bytes the Oodle codec never read -- the checkpoint twin of `replay_data_trailing_bytes`. Printed as `Trailing bytes:` under `=== Checkpoints ===`, and by `diag --json` as `checkpoint_meta.trailing_bytes`. 0 in all 19,166 checkpoint archives of 1,014 replays, 11.06-13.06; `verify_build_corpus.py` fails a replay where it is not. |

Beside `quality`, `dropped_field_exports`, `replaced_export_groups` and
`renamed_field_exports` count net-field exports the cache could not place and
declarations that replaced a group or renamed what later rows mean; all are
expected to be 0.

`content_blocks_lost` is `content_block_framing_failures + malformed_content_blocks +
transform_failures + field_stream_failures +
max(0, rpc_stream_failures - unresolved_rpc_payloads_preserved)`, computed by
`NetStats::lost_content_blocks` and shared with `validate`'s summary so the two
cannot drift.

Read this together with the oracle pass rate. `skipped_bits` includes
preserved, uninterpreted streams, so it is not a count of lost payload bits.

---

## 4. Using it as a library

Take only the layer you need. Every crate is `#![forbid(unsafe_code)]`, and
`vrf-bitio` is `no_std` + optional `alloc`.

| Layer | Crate | Feature flags |
|---|---|---|
| Bit reader / UE wire format | `vrf-bitio` | `alloc` (default; drop it for `no_std`) |
| Payload transform (26 builds) | `vrf-transform` | none |
| Container (info/header/chunk/event/checkpoint, Oodle) | `vrf-container` | `oodle` `event` `checkpoint` |
| DemoFrame traversal | `vrf-frame` | none |
| Dynamic schema + GUID cache + checkpoint tables | `vrf-schema` | `checkpoint` |
| Replication (packet/bunch/content block/field) | `vrf-net` | `diagnostics` (kept for compatibility; diagnostics are always on) |
| Field decoder + nested arrays + type overlay + effects | `vrf-decode` | `array` `effect` `overlay` `structs` |
| Movement decoder | `vrf-movement` | none |
| Parquet writer | `vrf-export` | `parquet` + per-table |
| Unified CLI | `vrfkit` | `export` (default) |

ZSTD is deliberately *not* feature-gated out -- every writer picks it, so
disabling it would produce files this crate could not explain.

CI compiles every crate's core-only build and each of its features alone, a
matrix derived from `cargo metadata` ([`CONTRIBUTING.md`](../CONTRIBUTING.md#before-you-open-a-pr)).

---

## 5. `tools/` reference

`pip install -r requirements.txt` first (pyarrow, numpy) -- everything below
needs it.

### Generators

**Never hand-edit the output.** The overlay table
`crates/vrf-decode/src/table.rs` (1,123 + 96 handles) and `equippable_table.py`
are not generated: they are maintained in the repository.

| Script | Produces |
|---|---|
| `apply_type_corrections.py` | Pins measured types in `table.rs` (sets, inserts, sorts, resyncs the length) |
| `extract_checksum_types.py` | `crates/vrf-decode/src/checksum_table.rs` -- `compatible_checksum` -> `FieldType`, learned from the fields the overlay table already declares. Needs an export directory, since checksums come from the replay. Checksums whose donors disagree are dropped, which is the safety property. Repeat `--export` to widen the basis; the run **merges** into the committed table rather than replacing it, because a checksum this basis did not happen to see is still correct. `--check` asks whether the two agree *where they overlap* -- not whether they are byte-identical, which a content-addressed table cannot be across different sets of replays. A committed type changes only on purpose: when a correction retypes a checksum's donors, the merge refuses the disagreement until `--retype CHECKSUM` names it (write mode only, and only for a real disagreement). |

Run `apply_type_corrections.py`. Each pin sets its entry's type, inserting an
absent exact key; a glob pin must match at least one entry. `--check` fails when
applying would change `table.rs` at all -- a type, a missing entry, the order or
the declared length -- and prints the difference.

```bash
python tools/apply_type_corrections.py           # apply (224 corrections)
python tools/apply_type_corrections.py --check   # fail unless applying changes nothing
```

CI runs the `--check` line.

Those 224 corrections are the live expectation set the script re-verifies.
`ADDITIONS` pins types nothing else declares:
currently 147 of them, each on complete wire evidence (one width on every row
and a value distribution a wrong type cannot produce) or a checksum or
sibling that fixes the type, recorded at its entry under the bar stated above
the list. `check_docs.py` checks both figures.

### Validation

| Script | What it watches |
|---|---|
| `validate_corpus.py` | Framing (preserved corpus, top level; `--recursive` for subdirectories), and every replay that still skips bits, most first |
| `validate_metrics_corpus.py` | Metrics pipeline passes; `--expect-exact FILE` fails a run whose summary differs from a pinned one |
| `check_corpus_baseline.py` | Per-build corpus baseline |
| `check_export_baseline.py` | Export counters + per-file rows/bytes/SHA-256 content identity, and the `Sink tally` fields against the `fields.parquet` rows; with `--checkpoints`, also every checkpoint GUID path against the main stream's own declaration of that GUID ([method](CHECKPOINT_PATH_RESOLUTION.md#cross-check-against-the-main-stream)) |
| `check_baseline_schemas.py` | All committed baseline schemas, measured SHA-256 hashes, and cross-file replay/counter/table identities. |
| `check_decode_errors_corpus.py` | Overlay type errors, struct blob failures, array/leaf/truncated-RPC/movement failures, unwalked CNC brute-force payloads and movement-section tails -- the same zero-required counters as `verify_build_corpus.py` -- and fails when a work counter behind them (decoded rows, struct blobs, array elements and fields, movement rows, `RPC param walks` for truncated RPCs) never moves; the unwalked-CNC gates, the main pass's sized movement-tail gate and the checkpoint movement gates have no work counter and are printed as unbacked (top level; `--recursive` for subdirectories, `--checkpoints` to also decode Checkpoint chunks) |
| `corpus_scan.py` | Not a check -- the `.vrf` discovery `validate_corpus.py` and `check_decode_errors_corpus.py` share, so the two can no longer glob a directory two different ways and disagree about what "the corpus" is without saying so. Non-recursive by default; read its docstring for why. |
| `summary_counters.py` | Not a check -- what the guard tools share: the export summary's counter lines as one table, read by `check_export_baseline.py`, `check_decode_errors_corpus.py` and `verify_build_corpus.py`, each pattern anchored on the whole line (a Python test reads every line and the argument behind each count back out of `summary.rs`); and the helpers that run vrfkit and resolve, skip or pin a baseline's input. |
| `export_scan.py` | Not a check -- the export discovery the tools that read a directory of exports share: it names the staging and backup directories an interrupted `vrfkit export` leaves behind ([section 2](#if-an-export-is-interrupted)), so none of those tools can count one as an export. A Python test reads the names back out of the Rust code that creates them. |
| `check_component_remaps.py` | Whether each component remap still matches. Needs only an export, so it works on a replay from a build that has no baseline -- which is the case a renamed component would otherwise slip through. Exits 1 on a broken pair or an entry of the Rust table that does not parse, 2 when the export has no `fields.parquet`. Re-derive a broken or renamed pair with `extract_component_classes` ([below](#reading-the-installed-game)). |
| `check_checksum_types.py` | Whether each overlay type hashes to the replay's own `compatible_checksum`. Recomputes Unreal's checksum from the C++ type vrfkit decodes and sorts every typed identity into match / mismatch / untestable -- enums, object references of an unknown class and struct members whose parent checksum is unknown (no parent chain, and no unambiguous agreement among their siblings) are untestable, never a match -- and checks every `checksum_table.rs` checksum under the names that carry it. Needs only manifests (`--export`, or `--corpus` for a directory of them). A mismatch vrfkit keeps on purpose is listed, with its reason and evidence, in `tools/fixtures/checksum_types_expected.json`, keyed on its exact shape (checksum, wire name, parent chain, vrfkit's type, the C++ type the checksum names). Exits 1 on a mismatch no item names (the corpus exits 0, only the listed `249` quaternions mismatching) and on an item that applies to the input (its checksum is declared) but covers nothing, STALE; exit 2 on a malformed list. The method, the provenance of the formula and its limits are in [CHECKSUM_TYPES.md](CHECKSUM_TYPES.md). |
| `check_entry_survival.py` | Whether every name-keyed entry -- `table.rs` (names and handles), `scoped_types.rs`, `checksum_table.rs`, the measured array routes, the component-remap targets and the group aliases -- is still declared build after build, read from the `manifest.json` and checkpoint declaration tables of a directory of exports. Separates a field the group stopped declaring, a group that moved (naming the successor, and whether the successor's field is still typed), and a class nobody used; judges each absence by the chance that it is sampling, so a three-replay build can never fail. Fails on an evidenced loss that is neither still typed nor listed with its reason in `tools/fixtures/entry_survival_expected.json`. Run it on every new build ([section 7](#checking-a-new-build-still-matches-every-entry)). |
| `check_rep_movement_levels.py` | Each class's `ReplicatedMovement` location level, from the first update joined to its spawn ([DATA.md](DATA.md#replicatedmovementlocation-is-world-units-at-a-per-class-level)), plus a speed check and the rotator width that consumes each row. Exits 1 when a class the overlay types measures another level, no clean level, or a row its rotator width does not consume; 2 on no `ReplicatedMovement` row |
| `overlay_mirror.py` | Not a check -- the Python mirror of the overlay that `check_checksum_types.py` and `check_entry_survival.py` share: it parses `table.rs` and the generated `scoped_types.rs` and `checksum_table.rs` and the `overlay.rs` resolution constants, refusing any table that does not parse whole, and follows `overlay::resolve_entry`'s order. |
| `check_metrics_baseline.py` | **Semantics** -- rounds, score, K/D/A |
| `compare_combat_report.py` | Metrics-input multiset |
| `compare_rpc_params.py` | RPC parameters and records against the C# export, with its listed expected differences |
| `compare_with_csharp.py` | Diff against the C# parser |
| `check_effect_decoder.py` | Effect decoder (12 cases), run by `tools/tests/test_check_effect_decoder.py`; it has no command line of its own |
| `check_ascii.py` | Rust source ASCII sweep |
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

`check_docs.py` checks this document and the others it lists: every `tools/`
script mentioned, every crate in the table, every link and `#anchor`
resolving, and the live value of each number it can re-derive, in the
phrasings it reads -- the overlay and handle table sizes (in Rust doc comments
and `Cargo.toml` too), the test counts, the reference replay's printed overlay
counters, `Typed` ratio and export rows/bytes, the build table's clean/checked
counts, and the counts its `MEASURED_RE` names. Any other figure, DATA.md's
measurements among them, is a measurement nothing re-runs, and nothing checks
it. A stale sentence compiles and passes every test.

```bash
python tools/check_docs.py           # also runs the test suites to compare counts
python tools/check_docs.py --fast    # skip the count comparison
```

### Reading the installed game

`tools/extract_component_classes/` is a workspace member Rust tool that reads
the installed game's IoStore containers. It lists the class of every component
template in an installed game's IoStore containers, which is where
`KNOWN_SUBOBJECT_CLASS_PATHS` in `crates/vrfkit/src/sink/paths.rs` comes from.
It opens the files for reading only, shares them with every other handle, and
writes nothing except `--out`.

```bash
cargo +1.86.0 build --release -p extract-component-classes --locked
target/release/extract-component-classes \
    "<VALORANT>/live/ShooterGame/Content/Paks" --out classes.tsv
# one lookup, JSON with provenance and every counter:
target/release/extract-component-classes \
    "<VALORANT>/live/ShooterGame/Content/Paks" --format json --name ZoomStateMachine
```

One row per component template, sorted: `instance` (the name the replay
sends), `kind` (`gen_variable` for a Blueprint-added component,
`cdo_subobject` for one the C++ class creates), `class`, `class_kind`
(`script_import`, `package_import` for a Blueprint class, or an `_unresolved`
form), `native_class` (the first `/Script` ancestor), `asset` (the owning
package), `export`, `outer`, `class_ref` and `container`. `--kind` keeps one
shape; `--name` (repeatable) keeps named instances and reports the ones not
found. Anything that cannot be resolved prints as `?`.

The summary goes to stderr and prints every counter, zeros included -- among
them `indexed_files_dropped`, directory-index files that cannot be attached to
a chunk (an entry past the TOC's end, or one a later file also names) -- and
each container's id, size and modification time (Unix seconds,
`ucas_modified_unix` in JSON), `global` included; a size or time that cannot be
read prints as `?` (`null` in JSON). Exit 0 means every
package was read, each compressed block to its last byte, and every self-check
held -- each rebuilt script path hashes back to the index the game stores, each
package name to its chunk id; 1 means something could not be read or a check
failed (the readable rows are still written); 2 is a usage or setup error. The
legacy `.pak` files beside the containers have encrypted indexes and are not
read; the summary lists them.

Procedure and what the output does and does not establish:
[`DATA.md`](DATA.md#reading-component-classes-out-of-the-game).

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
python tools/generate_scoped_types.py --check
```

`extract_match_observations.py` gives each ammo decrease
`weapon_rpc_within_300ms`: how many weapon-scoped continuous-effect RPCs on the
magazine's outer NetGUID lie within 300 ms, tallied with zeros. It does not
classify the RPC as a shot.

`generate_scoped_types.py` regenerates `scoped_types.rs` from the reviewed
`tools/fixtures/scoped_type_evidence.json`, and owns that fixture's layout:
write mode rewrites the fixture into it, and `--check` fails on any other
layout. These types require the exact
group, field name and compatible checksum. They never propagate to an
unobserved class alias or globally by checksum, and they are not checksum
donors. Besides the primitives, the fixture accepts `VectorDouble`,
`FTextTree`, `EnumRemainingBits`, `RotationShort`, `VectorNetQuantize100`,
`RepMovementByte` and `RepMovementShort`, each with an independent decoder in
`validate_type_evidence.py` (`test_generate_scoped_types.py` fails on a type
name without one). A `RepMovement` entry must also state its
location level, `location_quantization` (`RoundWholeNumber`, `RoundOneDecimal`
or `RoundTwoDecimals`), measured by the spawn join in
[DATA.md](DATA.md#replicatedmovementlocation-is-world-units-at-a-per-class-level):
exact consumption cannot see a wrong level, and the Rust test
`every_rep_movement_entry_carries_its_measured_location_level` fails on a
`RepMovement` type whose group it does not list. The ordinary
`validate_type_evidence.py` specification also accepts an optional `checksum`
to independently verify this narrower scope.

### Downstream conversion tools

| Script | What it does |
|---|---|
| `to_valplay_bundle.py` | Parquet -> NDJSON bundle (events/movement/manifest). The format valplay's `compute_metrics.py` consumes |
| `equippable_table.py` | Weapon class path -> display name and category, maintained in the repository. `CompactPistol_C` stays "Compact Pistol", though the 13.06 game calls it "Bandit" |

#### What the bundle manifest carries

The bundle's `manifest.json` forwards the export's `quality` object and its
`net_field_export_groups` table **verbatim**, and adds an `adapter` object of
its own measurements -- `events_written`, `events_time_ms_regressions`,
`field_rows_read`, the movement/net_guid/Event row counts read back against the
ones the export declared, and the conversion `losses` tally.
`server_timeline_rows_read` and `server_timeline_events_written` make the Event
path independently recountable. `bundle_schema_version`
names the shape; valplay's resume marker records it and rebuilds when it moves.

`losses` carries every counter, zero included, so a clean conversion reads as
zeros rather than as missing keys; the console summary prints every counter
too. `property_key_collisions` counts property values a same-named row
of the same event overwrote, typed values and raw blobs alike -- rows the
export tells apart only by `handle`, so the lost value is a different property,
not an older copy (28,845 on 02d4d478). `payload_shape_conflicts` counts only
rows that disagree about a key's shape, members against a row's own value (0 on
02d4d478: an array's container row is skipped at any depth, its decoded
elements carry the value). `non_finite_movement_rows` counts movement lines holding a
non-finite position, velocity, yaw or pitch. Those are written `Infinity` /
`-Infinity` / `NaN`, as every non-finite float in the bundle is spelled: Python's
`json` reads them, a strict parser such as orjson rejects the line. None occurs
on the 1,018-export corpus, and the decoder does not rule them out.

The blobs a consumer decodes itself -- `RoundInfos`, a damage RPC's
`LifeChangeEvents`, the shot effect arrays -- are built from `raw_bits` whether
or not the overlay also typed the row, so typing one upstream cannot change or
drop it. `raw_blobs_unavailable` counts those that could not be built: the row
had no raw bits (its typed value, if any, is published in the blob's place), or
only decoded members arrived. `damaged_bone_undecoded` counts `DamagedBone`
values the parser could not decode; they are published as `null`, never
rendered from the raw bytes. All three counters are 0 on 11 exports of builds
11.06-13.06.

`players` is deliberately **not** forwarded: valplay derives the same table
from the same `BombPlayerState` rows and its version keeps a *set* of character
GUIDs, which is what attributes a resurrected player's kills. Forwarding the
single-character copy would add account UUIDs to a second file and offer a
lossy alternative to the richer table.

An export with no `quality` object is forwarded as `"quality": null`, never as
zeroes: a consumer must be able to tell "nothing was lost" from "nobody
counted".

RPC grouping retains packet, actor, nullable subobject, group and function
handle. `rpc_received` and shot events retain `object_net_guid` as null for
actor rows, distinct from a real subobject GUID; consumers needing an effective
target must make their actor fallback explicit. Repeated invocations on the
same object in one packet remain a separate, counted collision limitation.
Each effect-array tag and value is read inside its declared field window.
A short field cannot consume a neighboring field; unreadable halves enter
`effect_half_read_pairs`.

`events.parquet` crosses as additive `server_timeline_event` rows through a
deliberately narrow allowlist: server `event_group`, `time_ms`, `time2_ms`, and
only `word0`/`word1` for groups whose fixed payload arity Rust already
validated. The neutral `payload_tag`, `payload_name` and `payload_seconds`
fields cross only as a complete tuple after the adapter independently checks
the exact public tag/name allowlist, finiteness and the same 1.001 ms time
tolerance; a mismatching row omits the tuple. Replay
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

**The slowest stage is valplay's `compute_metrics.py`; the slowest one
vrfkit owns is `to_valplay_bundle.py`.** On `02d4d478`, three sequential runs
each: `vrfkit export` 0.84 s, `to_valplay_bundle.py` 6.02 s, `compute_metrics.py`
13.15 s (medians). Each replay is independent, so **parallelizing across
replays is the biggest lever**; timings move +/-10% on one machine, and only an
A/B says whether a slower number is a regression.

### Analysis helpers

`extract_player_effects.py --export <export-directory> --out player-effects.json`
extracts `BlindManagerComponent.ActiveBlinds` updates and
`EffectManagerComponent` continuous-effect start/stop observations, including
the nearsight effect containers. Each record carries `target_identity` and
the original typed members. Only a `SpawnedCharacter` value admits
`player_body`: any value in the manifest's `character_net_guids`, the pawn a
player had before reconnecting included. `identity_provenance` says which, and
`player_body_via_non_final_spawned_character` counts the second kind. A
controlled camera, drone or targeting form does not become a player through
possession, ownership or its own `PlayerState`. Missing/conflicting identities
remain explicit. Non-player observations stay in the output, preserving flash
source evidence even when no player was affected.

Totals count observations, not unique hits. Array re-replication is retained;
there is no inferred cast attribution, explosion timing or start/stop interval
join. Only the main stream is read. Repeated RPC parameter names split adjacent
same-packet invocations; the export has no explicit invocation ID, so these
groups are not proof of unique effects. Untyped members and ambiguous identity
counts are printed even when zero.

`validate_type_evidence.py <export-or-parent> <specifications.json>` independently
reads raw payloads against explicit type proposals: the primitives and the
scoped geometry/enum shapes above, decoded from Unreal's wire layout rather
than by the Rust readers. Each specification
names an exact exported group and field, and the decoder requires full payload
consumption. Its recursive search skips the leftovers of an interrupted export
and refuses a table without `manifest.json` beside it. This checks structure and observed numeric ranges, not gameplay
meaning. Use it before adding overlay types and when comparing their emitted
values after export (`--compare-typed`). Besides the byte-aligned primitives (`UInt32`
read unsigned, and `VectorDouble` as exactly 192 bits of three finite
little-endian doubles, compared by parsing the exported `(x,y,z)` back into
doubles) it reads eight bit-level types -- `EnumByte` (a 1..8-bit
payload), `EnumRemainingBits`, `FName`, `FTextTree` (the full FText history
tree, compared by parsing the exported JSON), `RotationShort`,
`VectorNetQuantize100`, `RepMovementByte` and `RepMovementShort` -- with its own
LSB-first reader rather than `vrf-bitio`'s; those also require zero padding above `bit_count`, and a
`ReplicatedMovement` value is compared by parsing the exported JSON, so `1` and
`1.0` are the same number there. Its `location` is compared at either scale the
packed integers allow -- divided by 100 or in whole units -- and the report
says which (`location_scales`). The reader divides by each class's measured
level, so a whole-unit class reports `/1` and a two-decimal pawn `/100`; the
per-class level itself is pinned by the Rust test
`every_rep_movement_entry_carries_its_measured_location_level`, not here.

The shipped `tools/fixtures/type_evidence.json` covers the `ADDITIONS` and
the checksum-scoped wire entries of the table, one entry per exported group
that carries each; they were checked on every row of the 1,018-replay audit
exports and with `--compare-typed` on fresh exports of all 24 builds. The last
seven entries are the bomb-objective and ult-orb RPC parameters
(`BombPlantedRPC.BombPlanter` / `PlantSite`, `BombDefusedRPC.DefusingCharacter`,
`OrbPickedUpRPC.Orb Gatherer` / `Collectable Orb` on the Bomb and Swiftplay game
states), checked with `--compare-typed` on a 76-replay sample of the 21 archived
builds instead; the Swiftplay pair appears in one of those replays.
`tools/fixtures/type_evidence_aliases.json` covers the Swiftplay class-alias
propagation. Run on a sample, the `missing` list names every entry that sample
lacks and the exit status is 1 for that reason alone; `--allow-missing` lists
them without failing, but still exits 1 when no entry is observed at all. The
evidence file may also be `tools/fixtures/scoped_type_evidence.json` (run it
with `--allow-missing`): an RPC parameter's `Class:Function` group is matched
as the exported `Class_ClassNetCache` group and `Function.field` name. A
passing primitive check does not make a specimen gameplay semantics.

`validate_ability_array_evidence.py <export-directory> [...] --compare-typed
--require-routes` checks the measured `ActiveBlinds` and
`MulticastSetPath.NetworkedProjectilePath` routes. It independently reads each
parent's raw bits, requires explicit terminators and exact consumption, and
compares child paths, context, raw bits and typed values. `--require-routes`
rejects an aggregate with no sample of either route; an individual replay may
legitimately contain neither. The tool reads main `fields.parquet`; checkpoint
behavior is separately covered by the export comparison and corpus guards.

`summarize_value_coverage.py <export-or-parent> [--jobs 4]` reads the physical
`value_i64/f64/bool/str` columns and emits JSON to stdout. It counts each row
once if any typed column is non-null, including zero, false and empty strings.
Main and checkpoint denominators stay separate; missing checkpoint tables are
reported by the number of exports containing them. Malformed inputs produce
`complete: false` and a nonzero exit, rather than an apparently complete total.
This measures value presence, not semantic understanding or block preservation.

| Script | What it does |
|---|---|
| `extract_kill_observations.py` | Exports main and checkpoint KillData element snapshots with physical parent-row identity, independently checked raw values, nullable missing members and scoped reference status. Keeps all clocks separately; updates are not deduplicated kills. Accepts 11.06-12.09, 13.01-13.06, 13.08 and 13.09 (`MEASURED_BUILDS`) and refuses 12.10, 12.11, 13.00 and any other build before reading a row. See [KILL_OBSERVATIONS.md](KILL_OBSERVATIONS.md#measured-builds). |
| `extract_kill_ledger.py` | Retains character-death events, projects component-local KillData state and links mutually unique same-round PlayerState identities. Preserves unmatched events and observations. Reads the same measured builds as the observation extractor. See [KILL_OBSERVATIONS.md](KILL_OBSERVATIONS.md#ledger). |
| `extract_healing_observations.py` | Retains serialized heal amounts, section state, raw source rows and separate identity corroboration. Amount sums do not establish effective HP restored or player healing credit. See [SECTION_OBSERVATIONS.md](SECTION_OBSERVATIONS.md#healing). |
| `extract_fastarray_observations.py` | Writes numeric AbilitiesAndBuffs FastArray headers, deleted/changed item IDs and raw field offsets to NDJSON, retaining each input window and physical row identity. Reads two exported routes, `_cnc_h1` under `AbilitiesAndBuffsComponent` and `__vrfkit_chained_cnc_h1__` under `/Script/ShooterGame.AresAbilitySystemComponent`, and labels each record with its `route` and stream (`population`). Accepts what was measured: `_cnc_h1` main rows on 22 builds (12.10 and 12.11 have none) and chained rows in both streams on all 24. Any other route, stream or build keeps its raw bits with a rejection reason, and the exit is nonzero. The receipt counts every route and stream, zeros included. Field meanings remain unknown. See [the wire investigation](GAS_AND_PATCHVOLUME_INVESTIGATION.md). |
| `extract_ground_volumes.py` | Decodes the cells of ground-area volumes (GroundVolumeComponent `FragmentInfo` items, including the bare `PatchVolume` rows) with the names and checksums each replay declares: world-space polygon, floor, ceiling, grid cell, travel distances, status, owner actor class. Writes items, every source window with its raw bits and status, and a receipt with counts including zeros; exits nonzero on any rejected window. Measured builds only. The receipt maps `253` to `ID` and `X`/`Y` to `GridPos` by exact (name, checksum); `Status` gets its enumerator name in 13.06 replays only, the build the names were read from. See [GROUND_VOLUMES.md](GROUND_VOLUMES.md). |
| `extract_section_observations.py` | Retains damage, healing, overheal-decay and reset section observations with raw parent/child checks. Distinguishes parentless records, known non-health sections and unresolved references. See [SECTION_OBSERVATIONS.md](SECTION_OBSERVATIONS.md). |
| `section_timeline.py` | Builds the main-stream section timeline: each section state's exact predecessor with explicit ordering/lifetime gaps, a strict (time-ordered) and a packet-ordered continuity decision, and separate eligibility and arithmetic counters. See [SECTION_OBSERVATIONS.md](SECTION_OBSERVATIONS.md#timeline). |
| `kill_state.py` | Validates complete KillData bases and finisher revisions; compares checkpoint snapshots without counting them as new kills. Python module used by the ledger command. |
| `wire_bits.py` | Python module, not a command: the bit reader, replicated-array and FastArray-header parsers and the Arrow-filtered Parquet row reader that the section, healing, kill, ground-volume and FastArray extractors share. |
| `extract_match_observations.py` | Exports evidence-labelled ammo changes, equip/reload intervals, round balances, team loadouts, defuse observations and economic state. Money decreases and transaction snapshots remain separate; temporal association is not a verified purchase ledger. A decrease between `switchTeams` and the next `roundStarted` (the team-switch credit reset) is published as `money_decreases_in_team_switch_window` instead and is never a snapshot's nearest decrease; the window count prints with its zero. |
| `extract_ability_lifecycle.py` | Emits ability-path actor candidates with observed open/close/dormant events and explicit Owner/Instigator references. Player links are identity evidence, not proof of casts, and reach every `SpawnedCharacter` pawn of a player (`player_reference_provenance` names an earlier pawn). Missing closes remain censored; no nearest-player attribution or fixed duration is used. |
| `player_identity.py` | Python module used by `extract_player_effects.py`, `extract_spike_carrier.py`, `extract_ability_lifecycle.py`, `extract_healing_observations.py` and `extract_damage_events.py`, not a command. Admits every non-zero `SpawnedCharacter` value in main `fields.parquet` (BombPlayerState and its Swiftplay alias) as a player body, joined to the manifest `subject` -- not only the manifest's last value, which drops the pawn a player had before reconnecting. A pawn claimed by two PlayerStates or subjects is a conflict. Its counts (earlier pawns, conflicts, manifest disagreements, untyped or foreign rows) are reported with their zeros by every tool that uses it. |
| `analyze_raw_properties.py` | Streams a deterministic size-stratified corpus sample (or `--all`) one temporary export at a time and inventories preserved unnamed/raw replicated properties. Reports only build-level counts, bit widths, and anonymous recurrence ranks; it never prints replay paths/names, group/actor/object/handle/checksum identifiers, hashes, or payloads. Exits nonzero if an unnamed property row lacks exact-length `raw_bits`. Use `--format json` for a deterministic, versioned aggregate document. |
| `bench_export.py` | Times a full `export` against `tools/baselines/bench.json`; `--checkpoints` times `export --checkpoints` against its own `export_checkpoints` slot. A smoke detector, not a profiler -- wall clock is noisy, so the default tolerance is 25% and it answers "did something get twice as slow", nothing finer. Reports a run *faster* than the baseline too: that means the recorded number no longer describes the code. |
| `extract_active_effects.py` | Derives an `active_effects.parquet` view from an export -- one row per persistent ability instance (smoke/wall/molly/slow/trap/recon/orb) with class, spawn position, and open/close lifetime. A `dormant` event does NOT end an instance -- a settled smoke that stops replicating has not despawned -- so those instances stay open-ended and the summary counts them. The data already lives in `actors.parquet`; this filters and pairs it. Each row carries `actor_kind` (`projectile`/`game_object`/`zone`/`patch`/`pawn`/`other`, from the class leaf prefix): a projectile and the zone it places are two rows by design, and the kind lets a consumer count either. Weapons (`Gun_` leaves, such as Chamber's ult gun) and Breach's through-wall flash are not effects; Brimstone's orbital strike is a `damage_zone`. Every type and kind prints with its zero. `--tracks <path>` also writes `actor_tracks.parquet`: one row per `ReplicatedMovement` update (location, velocity, yaw) or `MulticastAddSmokeScreenPoint.Translation` wall point, with the class and a `source` column; an untyped row is kept with null coordinates and counted by class. |
| `extract_spike_carrier.py` | Derives a `spike_carrier.parquet` view -- one row per spike custody interval, resolved through to the manifest `subject`. Reads `BombEquippable_C.Owner` on the spike's own channel rather than the inventory side, so it covers carrying-in-the-backpack and not just in-hand, and it follows proxy carriers (Gekko's Wingman) back through `Instigator`. A carrier is any `SpawnedCharacter` pawn of a player, including one from before a reconnect; `carrier_identity_provenance` says which. |
| `minimap.py` | Minimap projection (`project`, crossed axes: u from `pos_y`, v from `pos_x`) and park-slot mask (`parked`, on both x and z), importable. As a command, `--export <dir> --maps <valorant-api /v1/maps JSON>` prints the share of live movement rows inside [0,1]² and the u/v range, and exits 1 for a map the file lacks or leaves at zero, or a share below 99%. The constants file is the user's; nothing in the repo carries it. See [DATA.md](DATA.md#minimap-projection). |
| `extract_damage_events.py` | Derives a `damage_events.parquet` view -- one row per `MulticastNotifyDamage_{Point,Base}` invocation: time, victim actor, class and (for a `SpawnedCharacter` pawn) subject, damager subject from `DamagerPlayerState`, weapon class and name, origin/impact/direction, dealt/taken/falloff, bone, regional damage, killed, wallbang. The victim is the row's actor, not the `Character` parameter (0 on a kill against utility); the `NetTimestamp`/`RespawnNumber` sentinels and GUID 0 are null. Invocations sharing a bunch split on a repeated parameter (counted). Killing blows on a player body must pair one to one with `characterDeath`, or the run exits 1 and writes nothing. |
| `extract_rounds.py` | Derives a `rounds.parquet` view -- one row per played round: reset, round start, buy phase end (`buy_end_ms`, the `CastTime` epoch), the unnamed post-round phase and the side switch from the `MulticastSetPhase` RPC, each checked against the named RPC of its frame (a difference exits 1 and writes nothing); `round_number` from `roundStarted`; winner, role, result and attacker team from `RoundResults` by `RoundNumber`; plant/defuse/explode times and the replicated plant site. A surrender pads `RoundResults` with awarded rounds, which are counted, never rows. A missing phase stays null. |

```bash
python tools/extract_match_observations.py --export <export-directory> --out observations.json
python tools/extract_kill_observations.py --export <export-directory> --out kill-observations.json
python tools/extract_kill_ledger.py --export <export-directory> --out kill-ledger.json
python tools/extract_healing_observations.py --export <export-directory> --out healing.json
python tools/extract_fastarray_observations.py --export-dir <export-directory> --out-dir <new-output-directory>
python tools/extract_ground_volumes.py --export-dir <export-directory> --out-dir <new-output-directory>
python tools/extract_section_observations.py --export <export-directory> --out sections.json
python tools/section_timeline.py --export <export-directory> --out timeline.json
python tools/extract_ability_lifecycle.py --export <export-directory> --out ability-lifecycle.json
python tools/extract_rounds.py --export <export-directory> --out rounds.parquet
python tools/extract_damage_events.py --export <export-directory> --out damage_events.parquet
python tools/extract_active_effects.py --export <export-directory> --out active_effects.parquet --tracks actor_tracks.parquet
python tools/minimap.py --export <export-directory> --maps maps.json
```

Reload observations recognize `ReloadState` and `ReloadStateEmpty`. Each carries
packet endpoints, start/end boundary labels, and left/right censor flags.
`observed_span_ms` measures the retained interval; `duration_ms` is null when an
entry or exit is uncertain. Round resets and unknown state paths break intervals.
`reload_magazine_increases` links positive ammo transitions strictly inside an
observed interval through the same non-null weapon outer GUID. Boundary-packet
ordering is unresolved and excluded. These are supporting observations, not
completed reloads, shots, or a purchase ledger.

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

The analyzer performs no type inference, so it establishes preservation and
schema drift, not meaning.

---

## 6. Validation suite

The pre-PR sweep, and what CI runs, is in
[CONTRIBUTING.md](../CONTRIBUTING.md#before-you-open-a-pr); the suites have
739 Rust tests (Windows), 738 Rust tests (non-Windows), and 1074 Python tests.

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
for b in tools/baselines/build_*.json; do
  python tools/check_corpus_baseline.py --baseline "$b"
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
missing. Its C# side is a CliReader export of the same replay, kept
machine-local because it carries per-player values (`compare_rpc_params.py`
reads the same export). To produce it:

```bash
REF="$LOCALAPPDATA/vrfkit/csharp-reference/02d4d478-1dfb-4412-9a77-29ca29105a9d"
<cli> export <corpus>/02d4d478-1dfb-4412-9a77-29ca29105a9d.vrf --output "$REF"
grep CombatReportComponent "$REF/events.ndjson" > "$REF/combat_report.ndjson"
grep rpc_received "$REF/events.ndjson" | grep -E \
  'MulticastNotifyKilledEnemy|MulticastNotifyDamage_Point|MulticastEndRound' > "$REF/rpc_params.ndjson"
rm "$REF/events.ndjson" "$REF/movement.ndjson"   # 3.2 GB; only the lines above are read
```

Keep `$REF/manifest.json`: `compare_rpc_params.py` reads the replay's SHA-256
from it. Without it no expected difference can be applied or checked, so the
run cannot pass: it exits 2, or 1 if anything differs.

The export must decode CombatReport `Rounds` and the RPCs on Gekko's
character. On this replay `compare_combat_report.py` matches all ten shapes,
and `compare_rpc_params.py` matches every parameter and every record but one:
a damage record only vrfkit emits, because the reference skips its block
([FOLLOWUP.md](FOLLOWUP.md#the-damage-record-only-vrfkit-emits)). The tool
lists that record as an expected difference and exits 0; it exits 1 on any
other difference, and on that one if it stops occurring exactly as listed
(`STALE`).

### What each check catches -- this is the point

| Check | Watches | Misses | Cost |
|---|---|---|---|
| `validate_corpus.py` | Framing (top level of the corpus dir; `--recursive` for subdirectories) | Type errors, broken semantics | ~30 s |
| `check_export_baseline.py` | Every export counter + per-file rows/bytes/SHA-256 | Other builds | 1 s |
| `check_decode_errors_corpus.py` | Overlay type errors, struct blob failures, array/leaf/truncated-RPC/movement failures, unwalked CNC brute-force payloads, movement-section tails, and a decoder whose work counter never moves (top level; `--recursive` for subdirectories) | Broken semantics; a decoder stopped on one build or route while the rest keep its corpus total up; a CNC brute force that never runs (`attempted` is legitimately 0 on some builds, so it backs nothing); a sized movement window (none measured; no work counter backs `Movement tails sized`); Checkpoint chunks, unless `--checkpoints` | ~50 s |
| `check_decode_errors_corpus.py --checkpoints` | The same failure counters for every Checkpoint chunk too (overlay, struct blobs, array truncations/residual bits, leaf errors, truncated RPCs, movement, unwalked CNC payloads, movement tails), and the checkpoint work counters (decoded fields, blobs, array elements and fields) | Broken semantics; checkpoint movement, movement-tail, CNC brute-force and truncated-RPC failures in practice -- no checkpoint RPC reached those decoders on the 1,018-replay audit (each was a RepLayout tail), so their zero is printed as unbacked | slower: `vrfkit export` also decodes every Checkpoint chunk per replay |
| `check_metrics_baseline.py` | **Semantics** -- rounds, score, K/D/A (8 builds) | Errors in the metrics pipeline itself | ~46 s |
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

### Baseline updates

Every baseline check takes `--update`. **When DRIFT appears, explain each line
before** you use it. The point is not that the numbers are sacred but that
silent change must be impossible.

---

## 7. Supported builds

The per-build table is the README's
([Supported VALORANT builds](../README.md#supported-valorant-builds)), and every
row uses the [common audit](BUILD_VERIFICATION.md): 1,018 unique replays, all
1,018 strictly clean. Every
replay passes block validation and checkpoint export; all observed evidence
values match the independent Python decoder. `Clean/checked` also requires
zero array and array-leaf errors. The report defines each denominator.

Adding a new build takes one `transforms!` line -- its branch,
`SEED_ADDEND`, signed `INIT_A_OFFSET` and its step list
([README](../README.md#supported-builds-and-the-cost-of-a-new-build)).

### Checking a new build still matches every entry

A build that decodes cleanly can still have moved a class the overlay names:
the rows then arrive untyped with every counter at zero. Cypher's tripwire did
exactly that between 13.00 and 13.01 (`.../Gumshoe/S0/Ability_E/` became
`.../Ability_4/`); the tool reports such a move `covered` once the new path is
typed. After
the new build exports, compare its declarations with the builds before it:

```bash
# Export the new build's replays beside the earlier builds' exports.
for f in <new-build-replays>/*.vrf; do
  ./target/release/vrfkit export "$f" --checkpoints --out "<exports>/$(basename "$f" .vrf)"
done
python tools/check_entry_survival.py --root <exports>
python tools/check_entry_survival.py --root <exports> --show 'Gumshoe'   # one entry's history
```

The previous build must be in the same root: each build is judged against the
builds before it, gathered back to at least 30 replays. Read three lists. A
structural finding is a field its group stopped declaring; a move names the
group that took over and says, entry by entry, whether the successor's field
still resolves to the same type (`covered`) or not (`lost`); a vanished group
had no successor and is reported only. An absence fails only with evidence:
at least 10 replays of the new build declaring the group (for a move, 10
replays at all) and a sampling probability of 0.001 or less, so a build
exported from a handful of replays reports everything as weak. Fix a `lost`
entry by typing the successor (`apply_type_corrections.py`, or the checksum or
scoped tables), not by listing it; list a finding in
`tools/fixtures/entry_survival_expected.json` only when the loss is correct,
with the reason and the evidence. A listed item that stops matching fails as
`STALE`.

**When pinning a replay as a baseline, do not point at
`%LOCALAPPDATA%\VALORANT\Saved\Demos`.** The game owns and rotates that
directory. Preserved copies live in `%LOCALAPPDATA%\vrfkit\baseline-corpora`.

---

## 8. Known limits

- **Untyped residual** -- the [`export`](#export) `Typed` is ~83.1% (denominator
  including RPC parameters). **Untyped != lost** (`raw_bits` preserved). Each
  remaining field is typed only on its own evidence (CONTRIBUTING.md, "Type
  corrections are conservative").
- **`AbilitiesAndBuffsComponent`** -- the replay declares no ClassNetCache group
  for that class at all (4,024 of 4,024 checkpoints of a 13.01 sweep). Its
  complete block payload is preserved in one marked raw row, so the gap does
  not lower the `validate` score.
- **Scoreboard stats** -- release-13.02 replicates cumulative K/D/A through
  `BasicCombatStatsComponent` and cumulative combat score through
  `PlayerScoreComponent`. vrfkit types all four as `Int32`; valplay computes
  ACS as final Score divided by played rounds.
- **Team economy schemas** -- legacy `TeamEconomy` and newer `BaseTeamState`
  values require separate joins. Availability in vrfkit does not establish
  that a downstream metrics deployment consumes both schemas.
- **Non-Bomb game modes** -- `GROUP_ALIASES` maps Swiftplay's
  GameState/PlayerState to the Bomb classes; valplay
  selects them with `is_game_state` / `is_player_state`.
- **Damage precision** -- vrfkit preserves the exact fractional wire damage.
  valplay additionally floors each final engagement segment before summing its
  scoreboard damage, which reproduces Tracker ADR without discarding the exact
  float total.

### Native transform evidence

`recover_native_binaries.py --binaries <archive-root> --output <recovered-root>`
recovers the seven protected 11.06--12.00 code images from SHA-256-pinned EXE
and `stub.dll` pairs. It needs optional `pefile`, `unicorn` and `numpy` packages.
Only analysis copies are written; both input and output hashes are checked.
Use `--build 11.06` to select one build, or omit it for all seven.

`capture_native_transforms.py --binaries <archive-root> --recovered-binaries <recovered-root> --check`
verifies all 1,422 committed vectors (11.06--12.09, 13.08 and 13.09) against the
pinned native PE readers. It requires optional `pefile` and `unicorn` packages and the
executable layout documented in [LEGACY_BUILD_SUPPORT.md](LEGACY_BUILD_SUPPORT.md).
Without `--check`, it regenerates `crates/vrf-transform/tests/data/native_vectors.rs`.
Ordinary tests read the vectors without requiring proprietary binaries or an
emulator. Game binaries and replay exports are not committed.

### Windows release packaging

`package_release.py` creates and verifies the versioned Windows x64 ZIP used
by CI and tagged releases. It rejects invalid tags, wrong executable formats,
changed checksums, unexpected ZIP entries and mismatched source provenance.
Creation also runs the extracted binary with `--help`; use `--smoke` to repeat
that execution when verifying on Windows. Verification without `--smoke` is
portable and does not execute the Windows binary.

```powershell
python tools/package_release.py create --exe target/x86_64-pc-windows-msvc/release/vrfkit.exe --directory '<new-package-directory>' --tag v0.1.0 --commit '<full-40-character-commit-sha>'
python tools/package_release.py verify --directory '<package-directory>' --tag v0.1.0 --commit '<full-40-character-commit-sha>' --smoke
```

The ZIP contains the executable, license, third-party notice and
`build-info.json`; a separate `.zip.sha256` file records the ZIP checksum.
Only tag pushes publish a GitHub Release, after the complete CI run and a
packaged-binary replay audit. See [release maintenance](../CONTRIBUTING.md#tagged-windows-releases)
for the workflow and how to validate a candidate without publishing.

### Uniform build verification

`verify_build_corpus.py` runs the same checks on every unique replay across
one or more recursive corpus roots. Inputs are deduplicated by SHA-256.
Each replay receives `validate`, `export --checkpoints`, required-counter
and Parquet row-count checks, and independent Python comparisons for the
observed fields in `public_fixture_type_evidence.json`. Missing counters,
nonzero framing/transform/array/type failures and changed inputs fail the
strict audit, as does a movement envelope trailer total that is not 24 bits
per stream. Every build must also show positive checkpoint block and
decoded-value counts; otherwise `build_errors` makes the command fail. Unknown
RPCs preserved whole are counted separately from loss.
Unobserved evidence fields are reported as absent, never as verified values.
Each export's checkpoint GUID paths are also rebuilt from the raw declaration
record and compared with the main stream's independent GUID registry. A path
or outer difference, or an export where no indexed entry joined, fails that
replay; every `guid_crosscheck_*` count reaches the report per build, zeros
included. See [the cross-check](CHECKPOINT_PATH_RESOLUTION.md#cross-check-against-the-main-stream).

```powershell
python tools/verify_build_corpus.py --exe target/release/vrfkit.exe --corpus '<replay-root>' --corpus '<preserved-fixture-root>' --work-dir '<new-private-work-dir>' --output '<new-report.json>' --jobs 4
```

Work and report paths must be new. Logs, manifests and exports remain under
the private work directory. The JSON report contains build counts and input
hashes, without source filenames or player identifiers. An unsuccessful
strict audit still writes its results and exits nonzero. Read
[BUILD_VERIFICATION.md](BUILD_VERIFICATION.md) for the latest measured results.
`check_docs.py` checks README's supported-build table against that committed
report and the Rust registry (the same verification wording and clean/checked
denominators), and this document's `Payload transform (N builds)` count
against the registry. The native and golden vector counts remain
separate arithmetic evidence; they do not substitute for any replay check.
