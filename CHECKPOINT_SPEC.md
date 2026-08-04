# `.vrf` Checkpoint chunk -- format specification

**STATUS: IMPLEMENTED.** This began as a read-only investigation and is now the
reference for shipped code. The parser lives in `vrf-container`'s `checkpoint`
module (chunk header, decompression) and `vrf-schema`'s (the two tables), and
runs behind `vrfkit export --checkpoints`. See PROJECT_STATUS.md section 23 for
what was built and 22-I for the measurement that justified building it.

Two things in here are worth reading even now that the code exists: section 9
records the hypotheses that were ruled out, and section 10 records what is still
unknown. Neither is in the code.

Investigation date: 2026-08-04. The probe referenced below was a standalone
cargo project in a session scratchpad and is gone; the production parser
reproduces every figure in this document -- 4,024 checkpoints, 17,186,645 guid
entries, 1,955,988 group records, 11,529,869 exported slots, 2,967,025,362 bytes
of plaintext, zero errors -- which is how the two were cross-checked.

---

## 0. Headline

**The whole checkpoint chunk is now decoded, byte-exact, with zero unexplained bytes.**
Verified over the **entire corpus: 215 files, 4,024 checkpoints, 0 violations.**

**Answer to question 4: NO.**
No `_ClassNetCache` export group appears in a checkpoint that ReplayData does not also declare.
`AbilitiesAndBuffsComponent_ClassNetCache` — and any near spelling — is **absent from all 4,024
checkpoints in all 215 corpus files**. Checkpoints do not unblock `AbilitiesAndBuffsComponent`.
Details and the exact measurement in §5.

What checkpoints *do* contain that ReplayData does not: **46–51 additional export-group
*paths* per replay** (all ordinary RepLayout groups, none `_ClassNetCache`, and essentially all
of them declared with **zero** named field slots), plus a complete **NetGUID → path table**
(~1,000–5,000 entries) and **one full-state DemoFrame per checkpoint**.

---

## 1. Chunk-level layout (unchanged from what the main session established — confirmed)

`ChunkType == 2` (`ChunkType::Checkpoint`). Payload:

| Offset | Type | Field | Verified value |
|---|---|---|---|
| 0 | `FString` | `Id` | `"checkpoint0"`, `"checkpoint1"`, … (0-based, matches chunk order) |
| … | `FString` | `Group` | always `"checkpoint"` |
| … | `FString` | `Metadata` | `"1"`, `"2"`, … (1-based counter) |
| … | `u32` | `Time1` | ms |
| … | `u32` | `Time2` | **always equal to `Time1`** |
| … | `i32` | `SizeInBytes` | byte count of the archive that follows |
| … | `[u8; SizeInBytes]` | archive | Oodle framing (below) |

These three `FString`s are **UTF-8** (positive length), unlike every `FString` inside the archive
(§2), which is UTF-16LE. That is why the header size varies:

| Chunk | `Id` | `Metadata` | header bytes |
|---|---|---|---|
| `checkpoint0`…`checkpoint8` | 11 ch → 4+12 | `"1"`…`"9"` → 4+2 | 16+15+6+12 = **49** |
| `checkpoint9` | 11 ch → 4+12 | `"10"` → 4+3 | 16+15+7+12 = **50** |
| `checkpoint10`+ | 12 ch → 4+13 | `"11"`+ → 4+3 | 17+15+7+12 = **51** |

The 49/50/51 variation the main session saw is fully explained by string length; there is no
hidden field.

Archive framing, identical to ReplayData:

```
[i32 decompressed_size][i32 compressed_size][oodle bytes]
```

`compressed_size + 8 == SizeInBytes` and `decompressed_size == plaintext length`:
**verified on all 4,024 corpus checkpoints, 0 violations.**

> Correction to a prior note: the main session verified `compressed_size + 8 == SizeInBytes` on
> the reference replay only. It now holds corpus-wide.

---

## 2. Decompressed archive layout

```
+--------------------------------------------------------------+ 0
| u32  DemoFrameOffset     (see §2.1 — frame starts at this+8)  |
| u32  0                                                        |
| u32  0                                                        |
| u32  0                                                        |
| u32  NumGuidCacheEntries                                      | 16
+--------------------------------------------------------------+ 20
| GuidCacheEntry x NumGuidCacheEntries        (§2.2)            |
+--------------------------------------------------------------+ gc_end
| u32  NumNetFieldExportGroups                                  |
| NetFieldExportGroup x NumNetFieldExportGroups   (§2.3)        |
+--------------------------------------------------------------+ map_end
| exactly ONE DemoFrame, byte-identical grammar to ReplayData   |
+--------------------------------------------------------------+ end of buffer
```

All reads are **byte-aligned** (`FBinaryArchive` semantics), same as ReplayData's DemoFrame.

### 2.1 The 20-byte prologue

- `u32 @0` — call it `w0`. **`map_end == w0 + 8` in all 4,024 corpus checkpoints, 0 exceptions.**
  So `w0` is the offset of the DemoFrame *measured from byte 8 of the archive*, not from byte 0.
- `u32 @4`, `u32 @8`, `u32 @12` — **all zero in all 4,024 checkpoints** (checked explicitly).
- `u32 @16` — the guid-cache entry count. Confirmed by exact closure: parsing exactly this many
  entries lands the cursor on a plausible `NumNetFieldExportGroups`, which in turn closes on
  `w0 + 8`.

**Verified**: `w0 + 8 == map_end == DemoFrame start`; words at 4/8/12 are zero.
**Inferred, not verified**: bytes 0..8 are probably a single `int64` (offset) and bytes 8..16 a
second `int64` (likely a deleted-startup-actor / delta-checkpoint field that is always 0 for
VALORANT). The two readings cannot be distinguished because the high words are zero.
**Practical rule for an implementer: do not use `w0` at all.** Parse the two tables; the frame
begins exactly where the export-group map ends. `w0` is only useful as a consistency assertion.

### 2.2 GuidCacheEntry (§ answers question 1)

```
NetGUID     : IntPacked
OuterGUID   : IntPacked
PathIsString: u8            -- 0 or 1 only
  if 1:  PathName  : FString       (UTF-16LE, always negative length; NO trailing i32 number)
  if 0:  NameIndex : IntPacked     (hardcoded-name-table index)
Flags       : u8            -- 0x00 or 0x03 only
```

Worked example, checkpoint 0 of `02d4d478-…`, decompressed offsets:

| Off | Bytes | Decode |
|---|---|---|
| 20 | `0e` | `IntPacked` → NetGUID **7** |
| 21 | `00` | `IntPacked` → OuterGUID **0** |
| 22 | `01` | PathIsString = 1 |
| 23 | `e7 ff ff ff` | FString length **−25** → 25 UTF-16 units |
| 27..76 | … | `"/Game/Maps/Ascent/Ascent"` |
| 77 | `03` | Flags = 0x03 — **end of entry 0** |
| 78 | `0a` | NetGUID **5** |
| 79 | `0e` | OuterGUID **7** ← the package above |
| 80 | `01` | PathIsString = 1 |
| 81 | `f9 ff ff ff` | −7 → `"Ascent"` |
| … | | Flags, then next entry |

Chain evidence that the field order is `(NetGUID, OuterGUID)` and not the reverse:
`"Ascent"` (guid 5) has outer 7 = the package `/Game/Maps/Ascent/Ascent`;
`"PersistentLevel"` (guid 3) has outer 5 = the world `Ascent`;
`"Default__BaseReplayController_C"` (guid 9) has outer 11 = the package
`/Game/Characters/_Core/BaseReplayController`. Reading the pair the other way produces
nonsense values (641/385/1409) with no such chain.

Measured field-value distributions — **note the differing scopes, they are not the same claim**:

| Field | Observed values | Scope |
|---|---|---|
| `PathIsString` | only `0` and `1`, **never any other byte** | **corpus-wide: 215 files, 17,186,645 entries.** The parser hard-errors on any third value and `verify` reported 0 violations |
| `PathName` FString length | **always negative (UTF-16LE); zero positive/UTF-8 lengths** | **corpus-wide: 25,038,008 FStrings** across guid paths, group paths and field names |
| `PathIsString` split | `1` ×1,134,611 (75.7 %), `0` ×365,091 (24.3 %) | 20 files / 1,499,702 entries |
| `Flags` | `0x00` ×998,076, `0x03` ×501,626 — no third value seen | **20 files only.** The parser does *not* validate this byte, so a third value elsewhere in the corpus would not have been caught |
| `OuterGUID == 0` | 498,762 entries (33 %) | 20 files |
| GUID parity | odd (static) 1,120,038; **even (dynamic) 379,664** | 20 files |

Notes:
- The table is **not** static-GUID-only: 25 % of entries carry even (dynamic) GUIDs. Any
  implementation that filters on `is_dynamic()` would drop a quarter of the table.
- `NameIndex` (the `PathIsString == 0` branch) is a **name-table index, not a path**. Evidence:
  checkpoint 0 has 157 such entries but only **25 distinct values**, and the values repeat in a
  fixed pattern across sibling subobjects — e.g. `(guid 38, outer 34, 18)`, `(40, 34, 155)`,
  `(42, 34, 156)`, then `(48, 44, 18)`, `(50, 44, 155)`, `(52, 44, 156)`, …. checkpoint 17 has
  1,204 such entries with 159 distinct values. This is the same "hardcoded FName" mechanism
  `vrf-schema`'s `read_fname` already handles (`crates/vrf-schema/src/reader.rs:57-67`), which
  renders such names as the decimal index. **Verified**: value distribution and reuse pattern.
  **Inferred**: that the table is the engine/game global name table (its contents are not in the
  replay, so the index cannot be resolved to text from the file alone).
- **Note the polarity.** In `read_fname` a leading `1` byte means *hardcoded index*. Here a
  leading `1` means *string*. The two bytes are not the same field; do not reuse `read_fname`
  for the guid table.
- `Flags` — **inferred** to be UE's `bNoLoad | (bIgnoreWhenMissing << 1)` (0x03 = both set).
  Only the two-value distribution is verified.
- **Unknown**: whether `NetworkChecksum` exists anywhere in this record. It does not: the record
  closes exactly with no room for it. If UE's `SerializeGuidCache` writes one, this build does not.

### 2.3 NetFieldExportGroup record

```
u32 NumNetFieldExportGroups
repeat NumNetFieldExportGroups times:
   PathName          : FString    (UTF-16LE)
   PathNameIndex     : IntPacked
   NumNetFieldExports: IntPacked          <-- IntPacked, NOT u32
   repeat NumNetFieldExports times (slot index i = 0..N):
       bExported : u8      -- 0 or 1
       if bExported:
           Handle             : IntPacked   (always == i; 11,529,869 exported slots corpus-wide, 0 violations)
           CompatibleChecksum : u32
           ExportName         : FName
                                  u8 bHardcoded
                                  if 1: IntPacked NameIndex
                                  if 0: FString Name (UTF-16LE); i32 Number
```

Worked example (checkpoint 0, group 0):

| Off | Bytes | Decode |
|---|---|---|
| 139140 | `60 00 00 00` | `NumNetFieldExportGroups` = **96** (u32) |
| 139144 | `bd ff ff ff` | −67 → 66-char path |
| 139148..139281 | | `"/Game/Characters/_Core/BaseReplayController.BaseReplayController_C"` |
| 139282 | `02` | `PathNameIndex` = 1 |
| 139283 | `2a` | `NumNetFieldExports` = **21** (IntPacked; `0x2a >> 1`) |
| 139284..286 | `00 00 00` | slots 0,1,2 not exported |
| 139287 | `01` | slot 3 exported |
| 139288 | `06` | handle = **3** (== slot) |
| 139289 | `85 51 f9 f4` | checksum `0xf4f95185` |
| 139293 | `01` | FName hardcoded |
| 139294 | `b1 02` | name index 216 |
| 139296..303 | `00`×8 | slots 4..11 |
| 139304 | `01 18 …` | slot 12, handle 12, name index 215 |
| … | | slot 14 → FString `"PlayerState"` + `i32 0`; slot 18 → `"SpawnLocation"` |
| 139401 | | next group's FString length — record closed at exactly 21 slots |

**This is the single most important detail and it is what defeated the first parse attempt:**
`NumNetFieldExports` is `IntPacked`, so `0x2a` is **21**, not the 42 you get from reading a u32.
`NumNetFieldExportGroups` at the head of the section *is* a plain u32. The two counts use
different encodings in the same section.

### 2.4 DemoFrame

Byte-identical to the ReplayData DemoFrame grammar. `vrf_frame::iter_demo_frames` parses it
**unmodified**.

Measured over all 4,024 corpus checkpoints:

| Property | Result |
|---|---|
| DemoFrames per checkpoint archive | **exactly 1** (4,024 frames / 4,024 checkpoints, 0 exceptions) |
| `timeSeconds × 1000` vs chunk `Time1` | equal within 1 ms in **all** frames |
| frames with a non-zero net-field-export count | **0** |
| frames with a non-zero export-GUID count | 3,809 of 4,024 |
| total packets | 904,891 |

The frame carries **no** net-field-export declarations at all. Its entire schema comes from the
export-group map that precedes it in the same archive.

---

## 3. Question 2 — "what are the 7 bytes between 169,586 and 169,593?"

**The premise was wrong; there are no such bytes and 169,593 is not the frame start.**

`w0 = 169,586` is not a section boundary — it lands *inside the UTF-16 null terminator of the
last field name in the export-group map*. The map's last bytes are:

```
0x29670  74 00 00 00 | 00 00 00 00 | 00 00 | 00 00 00 00  a2 20 …
         ^t  ^hi ^-- NUL --^  ^ i32 Number = 0 ^  ^slots^  ^-- DemoFrame --
         169584                169588          169592     169594
```

- 169,584–169,587: last two UTF-16 units of a field name (`…Event` + NUL).
- 169,588–169,591: the FName's `i32 Number` = 0.
- 169,592, 169,593: two more `bExported = 0` slot bytes, closing the last group.
- **169,594**: `i32 currentLevelIndex = 0`, then `f32 timeSeconds = 0.047638…` (chunk `Time1` = 47).

So the correct rule is `frame_start = w0 + 8 = map_end`. The "7 bytes" were an artifact of
guessing `w0 + 7`.

---

## 4. Question 3 — why ExportData failed with "unknown path name index 0"

**Answer: neither hypothesis (a) nor (b). The frame start offset was one byte early.**

Measured directly — `iter_demo_frames` on checkpoint 0 with a **fresh, empty** `NetGuidCache`,
starting at `w0 + delta`:

| Start | Result |
|---|---|
| `w0+0` (169586) | Err — packed integer did not terminate within 5 bytes |
| `w0+1` | Err — unexpected end of archive: needed 50331648 bits |
| `w0+2` | Err — invalid length 4014880 |
| `w0+3` | Err — bit-level read failed |
| `w0+4` | Err — **unknown path name index 16** |
| `w0+5` | Err — **unknown path name index 3873** |
| `w0+6` | Err — **unknown path name index 0** |
| `w0+7` | Err — **unknown path name index 0**  ← the exact error the main session reported |
| **`w0+8`** (169594) | **`Ok(64)` — 64 packets, 41,114 bytes** |
| `w0+9` | Err — export GUID payload size is negative: −25 |
| `w0+10` | Err — packed integer did not terminate |

- **Hypothesis (a) — "the cache must be pre-seeded from ReplayData": RULED OUT.** A fresh empty
  cache works. Corpus-wide, **0 of 4,024** checkpoint frames contain any net-field-export
  record, so there is nothing for a seed to satisfy.
- **Hypothesis (b) — "the checkpoint uses a variant frame encoding": RULED OUT.** The stock
  `iter_demo_frames` with the stock `flags` parses every checkpoint frame with zero changes.
- **Hypothesis (c) — misalignment: CONFIRMED.** `UnknownPathIndex { index: 0 }` is simply what
  `read_net_field_exports` (`crates/vrf-schema/src/reader.rs:76-97`) emits when handed misaligned
  bytes: it reads `path_name_index = 0`, sees `is_exported != 1`, and finds no group 0.
  It is a misalignment signature, not a schema-availability signature.

> **Do not read the above as "no seeding needed."** An empty cache suffices for frame
> **framing** — `iter_demo_frames` reaches the packets because the frame declares no exports.
> Seeding a `NetGuidCache` from the checkpoint's own export-group map (§2.3) and guid table
> (§2.2) is **required** for anything downstream: `ReplicationReader` resolves group paths and
> field names out of that cache, and since the frame carries zero export records the map is the
> **only** schema source in the archive. The §6 measurements were produced with a seeded cache;
> an unseeded run would frame the same packets and name nothing.

---

## 5. Question 4 — does any export group appear in a checkpoint that ReplayData never declares?

### 5.1 The `_ClassNetCache` answer: NO

| Measurement | Result |
|---|---|
| Checkpoints scanned | **4,024 across all 215 corpus files** |
| Export-group records parsed | 1,955,988 |
| Group paths containing `AbilitiesAndBuffs` | **0** |
| Group paths containing `Buff` | **0** (reference replay, both RD and CP) |
| `_ClassNetCache` groups, reference replay | ReplayData **147**, checkpoints **147**, checkpoint-only **0** |
| `_ClassNetCache` checkpoint-only, 4 sampled files | **0, 0, 0, 0** |

`AbilitiesAndBuffsComponent` **does** occur in these files — but only as a **NetGUID object path**
(a subobject instance name), in the guid tables of *both* ReplayData and the checkpoints. It is
not an export-group declaration in either. That is exactly the region-classification distinction
that matters: a hit in the GUID table unlocks nothing, because the guid table is a
NetGUID→object-path namespace, not the `NetFieldExportGroup` namespace, and vrfkit already has
that mapping from ReplayData's export-GUID bunches.

**Checkpoints do not unlock `AbilitiesAndBuffsComponent`.** The 97.3 % unattributed-bits problem
is unchanged by this work. The server simply never sends that class's ClassNetCache layout
anywhere in the file.

### 5.2 What checkpoints *do* add

Union over all checkpoints vs. the whole ReplayData stream, per file:

| File | RD groups | CP groups | CP-only | RD-only | CP-only `_ClassNetCache` |
|---|---|---|---|---|---|
| `02d4d478-…` (reference) | 475 | 522 | **48** | 1 | 0 |
| `03c60af4-…` | 418 | 466 | **51** | 3 | 0 |
| `3e835083-…` | 467 | 510 | **46** | 3 | 0 |
| `b261cc25-…` | 499 | 543 | **47** | 3 | 0 |

The 48 checkpoint-only groups in the reference replay are all ordinary RepLayout classes, e.g.
`/Script/ShooterGame.DamageableComponent`, `.ForceModuleManagerComponent`,
`.BlindManagerComponent`, `.UltPointsComponent`, `.DownedComponent`, 11 `…Modifier_C`
blueprints, `/Script/Engine.AnimInstanceReplicationComponent`.

**But they are almost entirely empty declarations.** Field-name coverage, reference replay:

| Measurement | Value |
|---|---|
| ReplayData `(group, handle) → name` pairs | 3,226 |
| Checkpoint `(group, handle) → name` pairs | 3,223 |
| Pairs in checkpoints but not ReplayData | **15** |
| Pairs in ReplayData but not checkpoints | 18 |
| Pairs where the two disagree | 379 — **all cosmetic** (`#216` vs `216`: my probe prefixes hardcoded-name indices; `vrf-schema` does not) |
| Groups whose declared length differs between RD and CP | **0** |

So the checkpoint's export-group map is, for practical purposes, **the same schema ReplayData
already delivers**. It contributes 15 new named handles out of 3,226 (0.5 %) in the reference
replay. The 46–51 extra *paths* carry declared capacities but no exported field names.

---

## 6. Question 5 — what the frames contain and how they relate to ReplayData

Reference replay, measured with `vrf_net::ReplicationReader` over the checkpoint frame packets
(cache seeded from the checkpoint's own two tables):

```
ReplayData (whole match): packets=530,401  bytes=112,887,672  actor opens=2,028
                          rep-layout blocks=258,882  CNC blocks=349,119  fields=429,627

cp0  t=47       1 frame  64 packets   41,114 B  opens= 64  rep=  300  cnc= 0  fields= 2,010
cp1  t=91,927   1 frame 187 packets   82,471 B  opens=159  rep=1,035  cnc= 8  fields= 4,730
cp5  t=587,447  1 frame 203 packets  118,570 B  opens=158  rep=  901  cnc=10  fields= 4,192
cp17 t=1,697,092 1 frame 261 packets 201,462 B  opens=169  rep=  974  cnc=10  fields= 4,214
```

- Every checkpoint frame is a **full-state snapshot**: it re-opens an actor channel for every
  actor alive at that instant (~160 actors mid-match) and re-sends that actor's complete
  RepLayout property state. That is why the archive grows monotonically with match time.
- **The actor *set* is a subset of ReplayData's.** On cp0 all 64 actor GUIDs, and on cp5 all 158,
  also appear as actor opens in the ReplayData stream (100 % overlap, both spot checks).
  **This is set membership only.** Whether the property *values* a checkpoint carries agree with
  what ReplayData carried at the same timestamp is **not measured** (§10). Do not treat
  "redundant" as established — it is the expectation, not a finding, and the emission decision
  in §7 point 5 rests on it.
- Volume relative to ReplayData for the reference replay: **2.2 % of packet bytes**
  (2,477,136 / 112,887,672) and **18.1 % of decoded fields** (77,812 / 429,627). The field ratio
  is much higher than the byte ratio because snapshot packets are dense with property data and
  carry almost no movement/RPC traffic.
- Almost all content is RepLayout. ClassNetCache blocks are 0–10 per checkpoint (vs. 349,119 in
  ReplayData). **Caveat: my probe sink returns `function_count = 0`, so it does not decode RPCs;
  the block counts are trustworthy, the RPC count is not measured.**
- The genuinely non-redundant content is the *completeness* of the snapshot: any property that
  was replicated once before the first ReplayData chunk the parser reads, and never again, is
  present in every checkpoint. Whether that is worth anything depends on whether vrfkit is
  already reading the stream from the very beginning (it is), so the expected value is low.
  **Unknown / not measured**: whether any individual `(actor, handle)` value in a checkpoint
  differs from what ReplayData carried at the same timestamp.

---

## 7. Question 6 — smallest correct implementation

### Reused unchanged (no edits needed)

| Component | Why it just works |
|---|---|
| `vrf_container::ChunkIterator` | already yields `ChunkType::Checkpoint` |
| `vrf_container::decompress_replay_data` | the archive body is identical Oodle framing |
| `vrf_frame::iter_demo_frames` | parses checkpoint DemoFrames with **zero** changes, stock `flags` |
| `vrf_schema::NetGuidCache` / `NetFieldExportGroup` / `NetFieldExport` | the checkpoint tables populate exactly these structures via existing public setters (`add_export_group`, `set_field_on_group`, `set_net_guid_path`) |
| `vrf_net::ReplicationReader` | consumes the emitted packets unchanged |
| `vrf_bitio::BitReader` | `read_int_packed`, `read_fstring` (handles negative/UTF-16), `read_u8/u32/i32` cover every primitive needed |

### Genuinely new code

**4 files to change / 1 to add.**

1. **`crates/vrf-container/src/lib.rs`** (~60 lines added)
   `pub struct CheckpointMeta { id, group, metadata, time1, time2, size_in_bytes, archive_offset }`
   plus `pub fn parse_checkpoint_meta(payload: &[u8]) -> Result<CheckpointMeta, ContainerError>`
   and `pub fn decompress_checkpoint(payload: &[u8], compressed, encrypted) -> Result<Vec<u8>, …>`.
   The last one should *not* use the synthesised-16-byte-prefix trick — factor the Oodle body out
   of `decompress_replay_data` into a private `decompress_oodle_archive(bytes, expected_len)` and
   have both call it. Also add `ContainerError` variants for checkpoint-specific size mismatches.

2. **`crates/vrf-schema/src/checkpoint.rs`** (NEW, ~150 lines + tests)
   - `pub fn read_checkpoint_guid_cache(reader, cache) -> Result<u32>` — §2.2, returns entry count.
   - `pub fn read_checkpoint_export_group_map(reader, cache) -> Result<u32>` — §2.3.
   - `pub fn read_checkpoint_tables(data: &[u8], cache) -> Result<CheckpointTables>` returning
     `{ guid_count, group_count, frame_offset }` where `frame_offset` is the post-map cursor.
     Assert `frame_offset == u32::from_le(data[0..4]) + 8` and error if not — that is a free
     integrity check, backed by 4,024 samples with zero exceptions.
   New `SchemaError` variants: `UnexpectedPathKind { byte }`, `CheckpointOffsetMismatch { .. }`.
   Export from `crates/vrf-schema/src/lib.rs`.

3. **`crates/vrfkit/src/driver.rs`** (~30 lines)
   Add a `ChunkType::Checkpoint` arm. Because the checkpoint tables are self-contained, the
   correct shape is a **separate `NetGuidCache` per checkpoint**, seeded from that checkpoint's
   own tables, and a **separate `ReplicationReader`** — do **not** feed checkpoint exports into
   the main ReplayData cache. Reasons: (a) `path_name_index` values in the checkpoint map are
   drawn from the same numbering as ReplayData's, so merging is *probably* safe but is not
   verified, and (b) actor-channel state is per-connection; replaying channel opens through the
   live reader would corrupt the main stream's channel table.
   Gate the whole thing behind a CLI flag (`--checkpoints`) since it is off the critical path.

4. **`crates/vrfkit/src/cli.rs`** (~5 lines) — the flag.

5. **Sink/exporter** — `crates/vrfkit/src/sink.rs` / `crates/vrf-export/*`: decide what to emit.
   Given §6's finding that checkpoint content is ~100 % redundant, the highest-value emission is
   probably *not* the fields but the **NetGUID→path table** (17.2 M entries corpus-wide, giving
   the outer-chain for every static and dynamic object at 18 sample times) and the extra
   46–51 group paths. Everything else is duplicate state. **This is a product decision, not a
   format question, and I have not made it.**

### Cost / scale figures for planning

| Quantity | Corpus total (215 files) |
|---|---|
| Checkpoint chunk payload bytes | 1,062,150,914 (matches the brief's figure exactly) |
| Decompressed archive bytes | 2,967,025,362 (2.97 GB — 2.8× expansion) |
| Checkpoints | 4,024 |
| GuidCache entries | 17,186,645 |
| Export-group records | 1,955,988 |
| DemoFrames | 4,024 (exactly 1 per checkpoint) |
| DemoFrame packets | 904,891 |
| Wall-clock to decompress + fully parse all tables + walk all frames | **~24 s** single-threaded release build |

---

## 8. Verification method, per claim

| Claim | How verified |
|---|---|
| Chunk header layout, `Time1 == Time2`, id/group/metadata naming | asserted per checkpoint over all 215 files — `cp2 verify`, 0 violations / 4,024 |
| `compressed_size + 8 == SizeInBytes`, `decompressed_size == len` | same run, 0 violations |
| Prologue words at 4/8/12 are zero | same run, 0 violations |
| Guid-cache entry layout | (a) exact closure: `NumGuidCacheEntries` entries land on a valid `NumNetFieldExportGroups`; (b) the outer chain resolves to sensible package/world/CDO relationships; (c) `PathIsString` ∈ {0,1} enforced by the parser over 17,186,645 corpus entries, 0 violations; (d) `Flags` ∈ {0,3} observed over 1,499,702 entries (20 files, unenforced) |
| All archive `FString`s are UTF-16LE | sign of every length prefix counted during `verify`: 25,038,008 negative, **0 positive**, corpus-wide |
| `bExported` ∈ {0,1} in group records | parser hard-errors on a third value; 0 violations over 1,955,988 group records |
| Export-group record layout | exact closure on `map_end == w0 + 8` for 4,024/4,024, and `Handle == slot index` in 11,529,869/11,529,869 exported slots corpus-wide |
| `NumNetFieldExports` is IntPacked | decisive: with u32 the first group claims 42 slots and overruns into the next group's FString at offset 139,401; with IntPacked it claims 21 and closes exactly there |
| Exactly one DemoFrame per checkpoint | independent frame walker in the probe (`walk_frames`, mirrors `vrf-frame`'s grammar), 4,024 frames / 4,024 checkpoints |
| Frame time == chunk `Time1` | same walker, 0 frames off by more than 1 ms |
| Frame parses with `iter_demo_frames` and an empty cache | run per checkpoint on the reference replay: `Ok(64)`…`Ok(261)` |
| Off-by-one diagnosis | swept `w0+0 … w0+10`, tabulated in §4 |
| Q4 negative result | `AbilitiesAndBuffs` substring over 1,955,988 parsed group *paths* (not a raw byte scan) across all 4,024 checkpoints → 0 |
| Checkpoint schema ≈ ReplayData schema | `(group, handle) → name` set comparison, reference replay |
| Actor-set overlap | `ReplicationReader` actor-open GUID sets, cp0 and cp5 vs whole ReplayData stream |

Probe commands (all in `cp2`): `list`, `hex`, `gaps`, `gc`, `map`, `full`, `fullall`, `verify`,
`stats`, `kind0`, `offbyone`, `q4b`, `delta`, `q5`.

---

## 9. Ruled-out hypotheses and corrections

Recorded so nobody repeats them.

1. **"The frame starts at `w0 + 7` (169,593)."** Wrong. `w0 + 8`. The `+7` came from a
   brute-force scan for `(i32 in 0..8, f32 in 0.1..3600)`, which has many false positives in
   200 KB. Derive the offset from the end of the export-group map instead.
2. **"The export cache must be pre-seeded from ReplayData up to the checkpoint's time."** Wrong.
   Checkpoint frames contain zero net-field-export records (0/4,024).
3. **"The checkpoint export section uses a variant encoding."** Wrong. Stock `iter_demo_frames`.
4. **"`u32 @16` counts something other than guid entries"** — considered when the first parse
   stopped after 159 entries; that stop was caused by mis-handling the `PathIsString == 0`
   variant, not by a wrong count.
5. **The pair `(a, b)` at the head of a guid entry is `(NetGUID, OuterGUID)`, not `(Outer, GUID)`
   and not a single 2-byte packed value.** Reading `03 0a` as one `IntPacked` = 641 produces an
   unresolvable outer chain; the correct split has the previous entry's `Flags` byte first.
6. **`NumNetFieldExports` is not a u32.** Reading it as u32 gives exactly 2× the true value for
   small counts (because `IntPacked` shifts left by 1) and silently overruns.
7. **Guid-cache entries are not static-GUID-only.** 25 % have even (dynamic) GUIDs.
8. **The C# reference parser does not implement checkpoints.** `ReplayChunkDispatcher.cs`
   (`src/Replay.Unreal/Chunks/ReplayChunkDispatcher.cs`, the `case ReplayChunkType.Checkpoint:`
   arm) logs `"Skipping checkpoint chunk {ChunkIndex}."` and does nothing else. There is no
   primary-source implementation to cite; everything above is derived from the bytes.
   (`ArchiveCheckpoint.cs` in `Replay.Encoding` is an unrelated archive save/restore helper.)
9. **No Unreal Engine source was consulted or relied on.** Field *names* used above
   (`bNoLoad`, `NetworkChecksum`, …) are labels of convenience; only the byte layout and the
   value distributions are evidence.

---

## 10. Open / unknown

- The meaning of the `IntPacked` `NameIndex` in `PathIsString == 0` guid entries. It is a
  name-table index (verified by reuse pattern); the table is not in the file, so the text is
  unrecoverable from the replay alone. Same limitation already exists for hardcoded FNames in
  ReplayData's net-field exports, so this is not a regression.
- Whether bytes 0..16 are two `int64`s or four `u32`s. Undecidable — the high words are zero.
- Whether checkpoint `path_name_index` values are drawn from the same numbering space as
  ReplayData's. Not tested; the implementation plan avoids depending on it.
- Whether any individual property value in a checkpoint differs from ReplayData's value at the
  same timestamp (i.e. whether checkpoints ever *correct* the incremental stream). Not measured.
- RPC content of the 0–10 ClassNetCache blocks per checkpoint frame. Not decoded (probe sink
  returns `function_count = 0`).
