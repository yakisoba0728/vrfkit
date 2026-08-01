# vrfkit Project Status

Last updated: 2026-08-01. Reflects commit 3d37c68 (46th commit, master).
All numbers come from direct tool runs, not estimates.

Section 7-A was corrected on 2026-08-01 after its premise was disproved by
measurement, then implemented and verified at 100%. See
NEXT_STEPS_FINDINGS.md for the evidence trail.

---

## QUICK START FOR THE NEXT SESSION

Read this section first. Everything else is supporting detail.

### Where things are
```
Parser (Rust)  : C:\Users\yakihyuk0728\Documents\GitHub\vrfkit
C# reference   : C:\Users\yakihyuk0728\Documents\GitHub\ValorantReplayParser
                 HAS USER'S UNCOMMITTED WORK -- 17 entries in git status.
                 Never commit, stash, reset, or modify anything there.
                 If you need to instrument it: add ONE clean file, run, then
                 git checkout -- <that file>  and verify status is still 17.
valplay        : C:\Users\yakihyuk0728\Documents\GitHub\valplay
                 Never modify. Run its scripts by absolute path only.
Corpus (.vrf)  : C:\Users\yakihyuk0728\Documents\GitHub\valplay\data\raw\vrf
                 215 files, all ++Ares-Core+release-13.01
Local 13.02    : %LOCALAPPDATA%\VALORANT\Saved\Demos\*.vrf  (4 files)
```

### Verify the build before touching anything
```powershell
cd C:\Users\yakihyuk0728\Documents\GitHub\vrfkit
$env:CARGO_TARGET_DIR = $null
cargo test 2>&1 | Select-String "test result"
# Expected: 236 passed, 0 failed across all crates
cargo clippy --all-targets -- -D warnings 2>&1 | Select-String "^error"
# Expected: no output (exit 0)
cargo fmt --check
# Expected: exit 0
```

### Regression guard (run after any non-trivial change)
```powershell
cargo build --release -p vrfkit
.\target\release\vrfkit.exe export `
  "C:\Users\yakihyuk0728\Documents\GitHub\valplay\data\raw\vrf\02d4d478-1dfb-4412-9a77-29ca29105a9d.vrf" `
  --out out\nested
# Must NOT change: content blocks 608020, fields 429633, RPCs 342735,
#                  movement 1839607, NetGUID rows 16167, decode errors 0
python tools\compare_combat_report.py
# Must print: ALL INTERESTING SHAPES MATCH
python tools\validate_corpus.py .\target\release\vrfkit.exe `
  "C:\Users\yakihyuk0728\Documents\GitHub\valplay\data\raw\vrf"
# Baseline: blocks 136,545,822  fields 98,883,979  rpcs 75,571,092
#           malformed 0  skipped 1,972,080,670
```

### What to do next (highest impact first)
See Section 7 for full detail, and NEXT_STEPS_FINDINGS.md for the measured
evidence behind the 7-A correction.

7-A, 7-B, 7-D, 7-G, 7-J and 7-K are all DONE. **15 of 21 metric sections are
byte-identical to the C# reference on all 11 cross-validated replays**, up
from 3 sections on 1 replay at the start of the session.

Verify it yourself:
```powershell
python toolsalidate_metrics_corpus.py --jobs 3
# Expected: sections exact on ALL   : 15 / 21
```

No section is BLOCKED, and **no section differs for a reason that is not
understood**. The six that differ all do so because we carry data the C#
parser does not:

  combat / kast / tactical  13 MulticastNotifyKilledEnemy RPCs from
                            character 576 that the C# parser never emits
  economy_detail            496 of 496 purchase buyers resolved vs its 151
  weapons / weapon_stats    172 server-world effects it bins as "unknown",
                            plus one damage record commit 6e6d544 recovers

What is actually left:

**7-E. 13.02 corpus guard** -- the transform works on two local 13.02 demos
  but nothing pins it. A future transform change could break 13.02 silently.

**7-H. Instance-named component groups** -- still open, but check whether the
  data is merely unexported before treating it as a naming problem; that is
  what cf97ecf turned out to be.

**7-I** is a parity decision, not a defect. **7-C** is a measured ceiling --
do not spend time there. **7-F** is now closed the same way: the slice it
proposed to parallelise is 3.4% of an export, and the rest of what it called
"decode" is order-dependent. Section 7-F carries the timing breakdown, which
is the useful part -- it names Parquet writing (37%) and group-path
resolution (12%) as where the time actually goes.

### State of out/ directory (gitignored, safe to regenerate)
```
out\baseline\             -- regression baseline Parquet (do NOT delete)
out\nested\               -- latest export of 02d4d478
out\valplay_bundle\       -- latest adapter output + metrics.json
```
To regenerate everything from scratch:
```powershell
Remove-Item out\nested -Recurse -Force -EA SilentlyContinue
Remove-Item out\valplay_bundle -Recurse -Force -EA SilentlyContinue
cargo build --release -p vrfkit
.\target\release\vrfkit.exe export <vrf path> --out out\nested
python tools\to_valplay_bundle.py out\nested
python "C:\...\valplay\pipeline\metrics\compute_metrics.py" `
       (Resolve-Path out\valplay_bundle\02d4d478-...).Path
```

### Key invariant (never break)
Every content block emits (group_path, handle, name, bit_count, raw_bits)
even when nothing is known. Overlay is additive. A block whose group is
unresolved returns Err (counted, named) -- never Ok with a guessed capacity.
The oracle's honesty matters more than its pass rate.


---

## 1. What This Project Is

A from-scratch Rust VALORANT replay (.vrf) parser in a NEW repository
(C:\Users\yakihyuk0728\Documents\GitHub\vrfkit), built to replace the
C# parser (ValorantReplayParser, MIT) that the valplay Python analytics
pipeline depends on. The C# parser discards roughly 26% of content blocks
because it abandons any bunch whose payload has no registered descriptor;
this parser never discards: every field emits (group_path, handle, name,
bit_count, raw_bits) even when nothing is known about the type.

Primary outputs: fields.parquet, movement.parquet, actors.parquet,
net_guids.parquet, manifest.json -- all written by `vrfkit export`. A Python adapter
(tools/to_valplay_bundle.py) converts these into the bundle shape that
valplay's compute_metrics.py already consumes, so the validated 21-section
analytics pipeline runs unchanged on our data.

---

## 2. Repository State (2026-08-01)

```
branch       : master
commits      : 46
tests        : 236 passing, 0 failed
clippy       : 0 warnings (--all-targets -- -D warnings)
fmt          : clean (--check)
working tree : clean
valplay repo : 0 modified files (user's uncommitted work untouched)
ValorantReplayParser : 0 modified files (instrumentation always reverted)
```

### Commit list

```
3d37c68 fix(tools): collapse intra-packet sub-moves and stop printing f32 artefacts
38ca3fe feat(tools): cross-validate metrics against every available reference bundle
279770a docs: close 7-B and 7-D, and record what their premises got wrong
cf97ecf feat(export, tools): carry the subobject GUID through to the bundle
fc24b63 fix(tools): emit spawn paths and coordinates in the reference's shapes
bea59d9 fix(tools): stop rounding shot locations to two decimals
bff712a fix(frame): round frame timestamps like the reference instead of truncating
50fc3ab docs: correct the oracle pass-rate median and max to measured values
9b99017 docs: record the custom-decoder audit and promote its lesson to an invariant
059713e feat(decode, tools): decode the damage geometry vectors
2764428 docs: close 7-J, and reclassify 7-B as the largest remaining gap
e7414d9 fix(tools): correct the RegionalDamage enum ordinals
90a50e1 fix(decode, tools): type EquippableUsed as a net GUID (7-J)
0869b3c docs: reconcile the combat row and sharpen the 7-J handoff notes
c2a3f4d docs: record the 7-A outcome and the two gaps its verification exposed
1f3afe4 fix(tools): classify fire mode from the firing-state name, not ammo counters
b258dfd feat(tools): resolve weapon identity for every shot
47849d2 feat(export): write net_guids.parquet with the NetGUID containment chain
391ee2e docs: correct section 7-A after measurement disproved its premise
21003aa docs: add quick-start section to PROJECT_STATUS.md for next session
ed4415f docs: PROJECT_STATUS.md -- full session record, remaining work and tradeoffs
de24d6d feat(decode, tools): decode shot EffectContainer blob and emit valorant_shot_received
cc5dabd feat(decode): decode RoundResults, TeamEconomy and RoundInfos struct blobs
df20d5b feat(export): write actors.parquet with channel open and close events
b6947ee feat(tools): adapter that feeds vrfkit output to the existing metrics pipeline
7c2faa1 docs: correct the README figures the honesty fix invalidated
6e6d544 feat(schema): resolve ClassNetCache groups from actor instance names
00dce40 test(net): update the zero-function case left stale by the loud-failure change
29b2936 fix(net): stop dropping ClassNetCache blocks for unresolved groups
90727ed fix(net): clamp the ClassNetCache handle read to a minimum of two
b531724 feat(oracle): name the class behind every payload-stage failure
bb797d2 fix(oracle): count payload-stage failures in the pass rate
0c2df40 docs: README with measured cross-parser comparison
070a953 test(tools): cross-parser verification harnesses
721f954 feat(cli): vrfkit inspect / validate / export
9ded7ae feat(export): columnar Parquet output
157ed72 feat(movement): decode the remote-character update protocol
29aae8a feat(decode): primitive decoders, nested arrays and a type overlay
f742245 feat(net): Unreal replication, framed with no skip path
33c4355 feat(schema): receive the replay's own dynamic field schema
5a634ae feat(frame): DemoFrame iteration between container and replication
6f3cbcc feat(container): .vrf container, chunk stream and Oodle decompression
8be1abc feat(transform): five per-build payload transforms, golden-verified
7f3377d feat(bitio): LSB-first bit reader and Unreal wire primitives
2df595d chore: cargo workspace scaffolding and licensing
```

---

## 3. Crate Structure

```
vrfkit/
  crates/
    vrf-bitio       -- LSB-first bit reader, UE wire primitives (22 tests)
    vrf-transform   -- per-build payload transforms, golden-verified (22 tests)
    vrf-container   -- .vrf container, chunk stream, Oodle decompression (32 tests)
    vrf-frame       -- DemoFrame iteration (3 tests)
    vrf-schema      -- dynamic field schema from replay wire (47 tests)
    vrf-net         -- Unreal replication pipeline, no skip path (31 tests)
    vrf-decode      -- primitive decoders, nested arrays, struct blobs (53 tests)
    vrf-movement    -- remote-character update protocol (5 tests)
    vrf-export      -- columnar Parquet writers (18 tests)
    vrfkit          -- CLI: inspect / validate / export (0 tests; the driver is
                       covered by the regression guard, not unit tests)
  tools/            -- Python generators and verification harnesses
```

Total: 236 tests. Counts measured per crate on 2026-08-01; the previous
breakdown in this document was wrong for six of the ten crates even though
its total happened to be right.

---

## 4. Corpus Verification Numbers (215 replays, all ++Ares-Core+release-13.01)

```
succeeded          : 215 / 215
failed             : 0

oracle pass rate  (measured 2026-08-01 from validate_corpus.py output;
                   the median and max previously recorded here, ~98.9% and
                   99.99%, were never measured and were both wrong -- the
                   tool also reports "below 99.99%: 215", i.e. no replay
                   reaches 99.99%)
  min              : 97.487010%  (936a0967-7a14-46bf-ab7e-b33f7e228cc4.vrf)
  median           : 99.323286%
  max              : 99.681958%

corpus totals
  content blocks   : 136,545,822
  fields emitted   :  98,883,979
  RPCs emitted     :  75,571,092
  malformed framing:           0   <-- container/bunch/block framing perfect
  unattributed bits: 1,972,080,670 (~246 MB, 91.7% is AbilitiesAndBuffsComponent)
```

Reference replay 02d4d478-1dfb-4412-9a77-29ca29105a9d.vrf:
```
content blocks     : 608,020
  malformed framing:       0
  RPC stream failed:   6,365  (unresolved group, function_count=0)
fields emitted     : 429,633
RPCs emitted       : 342,735
movement rows      : 1,839,607
actors.parquet rows:   3,827  (2028 opens + 1799 closes)
net_guids.parquet  :  16,167  (14,480 carry an outer GUID)
decode errors      :       0
typed (value_*)    :    36.0%  (356,290 decoded, up from 35.7% / 353,334
                                after the 7-J and geometry type corrections)
oracle pass rate   :  98.95%
```

13.02 replays (local demos, first measured in this session):
```
2a09e682  55 MB   686,559 blocks  malformed 0  transform 0  pass 97.96%
43d0f434  85 MB 1,004,465 blocks  malformed 0  transform 0  pass 99.18%
```
The C# parser that valplay currently uses REJECTS 13.02 replays outright.


---

## 5. What Was Done in This Session (chronological)

All work was verified by direct tool runs. Where an agent reported a result,
it was re-checked independently before being accepted as fact.

### 5-A. Oracle honesty fix (commits bb797d2, b531724)

The oracle computed its pass rate from malformed_content_blocks alone.
A block can fail at three depths: framing, the payload transform, and
walking the field/RPC stream inside it. Only the first was counted.

Consequence: a block could consume 3,386 bits worth of payload, fail
mid-stream, and the oracle would still report 100.000000%.

Fix: added transform_failures, field_stream_failures, rpc_stream_failures
to NetStats. All three now fold into the verdict and print separately.
Each skip site increments something visible.

The four blocks that were failing loudly:
  - Deadlock Ability_X  GameObject_Spline                3,386 bits
  - Clove    Ability_X  GameObject_Cashew_X_SegmentManager 281 bits x2
  - Clove    Ability_E  GameObject_Cashew_E_MapMissileMarker 2 bits

Also added on_stream_failure hook to ReplicationSink so the class name
travels to the oracle output. Without this the counters said "one block
failed"; with it they say which class.

### 5-B. Capacity-1 handle read fix (commit 90727ed)

Root cause of the four failures above: Unreal's ReadFieldHeaderAndPayload
passes FMath::Max(NetFieldExports.Num(), 2) to SerializeInt. When a group
declares exactly 1 function slot, SerializeInt(1) would consume ZERO bits
(ceil(log2(1)) = 0), but the wire payload always contains the 1-bit handle
written by SerializeInt(2) on the server. One bit of desync, cumulative.

Proof by exhaustive search (same technique that found the velocity bug):
offsets 0..64 x handle widths 0/1/2 -- only start=1 lands each block on
its exact end:
  1 + IntPacked(3801)=16 + 3801 = 3818 bits  exact
  1 + IntPacked(441)=16  +  441 =  458 bits  exact
  1 + IntPacked(89)=8    +   89 =   98 bits  exact
Inner payloads walk as clean RepLayout streams, zero bits remaining.

The C# parser reads the same declared 1 and fails on the same four blocks.
This is not a divergence from the reference; it is a place we exceed it.

Fix: function_count.max(2) in parse_class_net_cache.
Capacities >= 2 are unchanged because max(N,2) == N.
Corpus effect: skipped bits from stream failures 3,671 to 0.
RPCs 73,742,672 to 73,778,191 (capacity-1 ghost records replaced by real).

### 5-C. Silent skip path exposed (commits 29b2936, 00dce40)

parse_class_net_cache returned Ok(0) when function_count was zero (group
resolution failed). Zero does not mean "no functions"; it means "unknown".
The payload disappeared without touching any counter.

Making it return Err instead revealed:
  - 14,459 blocks / 18,831,872 bits in 02d4d478 alone
  - 2,276,559,577 bits (~284 MB) across the 215-replay corpus

The oracle had been reporting 100% over silently discarded data.
Pass rate fell to ~97.6-98.9% per replay. The number got worse because
it was wrong before.

Diagnostics named the cause: actor instance names (BombDestination_A,
WindowShieldA1, AudDeadeyeVOComponent) instead of _ClassNetCache paths.
BombDestination_C_ClassNetCache existed in the schema with capacity 3;
the lookup simply was not reaching it.

Stale test (class_net_cache_zero_functions_skips) was also fixed here.
The original commit 29b2936 went in with that test broken -- the grep for
FAILED in cargo output did not catch it because cargo had already halted
and the summary total was 104 instead of 205. Lesson: check the total,
not just the absence of a FAILED line.

### 5-D. Instance-name-to-ClassNetCache resolution (commit 6e6d544)

Added resolve_cnc_for_instance_name to NetGuidCache: walks from an actor
instance name to its class cache group using the schema the replay itself
declares. No hardcoded names anywhere (a hardcoded list would break on
new agents or new maps).

Result: 8,094 blocks recovered. RPC stream failures 14,459 to 6,365.
Skipped 18,831,872 to 17,507,210. Pass rate 97.62% to 98.95%.
RPCs +8,094 in 02d4d478. Corpus RPCs 73,778,191 to 75,571,092.
Skipped corpus-wide: 2,276,559,577 to 1,972,080,670 (13.4% fewer).

MulticastNotifyDamage_Point: 580 to 581 records, all 581 distinct by
(packet, time, actor, value) -- not a duplicate, a genuinely recovered
event the C# parser discards.

Remaining 91.7% of unattributed bits: AbilitiesAndBuffsComponent, for
which the replay declares no cache group. No lookup can reach it.

### 5-E. README correction (commit 7c2faa1)

README still claimed 100.000000% pass rate with 3,671 skipped bits.
Corrected to measured 97.49%-99.99% range with 1,972,080,670 unattributed
bits, plus an explanation that framing is exact everywhere and the shortfall
is attribution rather than parsing.

Also corrected: overlay figures (106 groups/929 fields -> 123/1054),
RPC comparison (334,641 -> 342,735 vs C# 230,893), typed coverage.

First-ever measurement of 13.02 replays documented here: two local demos
parse with malformed framing 0 and transform failures 0.

During this work: PowerShell Get-Content -Raw read as cp949 then wrote
as UTF-8, corrupting all Korean text. Recovered with git checkout --
and re-applied edits using the write tool only.

### 5-F. valplay adapter (commit b6947ee)

tools/to_valplay_bundle.py: reads vrfkit export (fields.parquet,
movement.parquet, manifest.json) and writes a bundle that valplay's
compute_metrics.py consumes unchanged. Reusing the 21 validated metric
sections is the point; reimplementing would discard the validation.

Result: combat.per_player reproduces EXACTLY -- 27 fields x 10 players =
270 comparisons, 0 mismatches. K/D/A/ADR/HS%/wallbangs/multikill/kd/
hit-region breakdown/damage_dealt/rounds_played/team all identical.

players, rounds, ultimate, movement_detail, movement_summary also match.
tactical and kast DIFFER -- and ours is more correct: both consume the
kill timeline; ours has 132 kills vs C#'s 119 (character-576 blind spot
documented in valplay's own notes; vrfkit recovers the 13 missing RPCs).

### 5-G. actors.parquet (commit df20d5b)

actor_writer.rs records one row per channel event (open or close):
time_ms, packet_id, channel_index, actor_net_guid, event_kind,
class_path, archetype_path, spawn location and rotation.
Null written when genuinely unknown; countable rather than silent.

02d4d478: 3,827 rows (2028 opens + 1799 closes). Matches validate totals.

### 5-H. Struct blob decoders (commit cc5dabd)

structs.rs: decodes BombGameState.RoundResults, TeamEconomy.LoadoutValue
and AverageLoadoutValue, OwnerExclusivePlayerInfo.RoundInfos.EndOfRoundMoney.
Wire layout derived from C# ValorantPayloadDecoders.cs, validated against
real bits from the corpus and the C# events.ndjson reference.

Effect on metrics:
  objective.round_results : 18/18 entries identical to reference
  side_winrate section    : byte-identical to reference
  economy section         : byte-identical to reference
  team_score              : 13:5 correct

### 5-I. EffectContainer blob decoder (commit de24d6d)

effect.rs: decodes ClientPlayOneShotEffectAtLocation RPC's EffectContainer
into shot data -- firing player, attack vectors, ammo, burst position.
17,818 invocations in 02d4d478, all previously raw.

Adapter now emits valorant_shot_received events.
shot_rays.ray_count: 2,475/2,475 exact. aim_deviation identical.

Weapon identity (equippable resolution) is NOT done yet -- see Section 7.

### 5-J. 7-A premise disproved by measurement (commit 391ee2e)

Section 7-A claimed the shot EffectContainer carries the equippable net GUID
and that the join needed no Rust change. Both were checked against real data
before any code was written:

  effect_equippable set on   0 of 2,647 reference shots
  firing_state GUIDs matching 0 of 2,475 actors.parquet rows

Reading the C# resolver (ValorantShotEventEnricher.cs:123) showed three
tiers, and the one 7-A described is the one that never fires. Tier 2 walks
the FiringState GUID's outer chain to the owning equippable.

A temporary instrumented build (added, run, reverted; export totals
unchanged) proved tier 2 before committing to it:

  firing_state GUIDs in guid_to_outer : 175 / 175
  shots resolving to a weapon         : 2,475 / 2,475
  class_path equal to the reference   : 2,475 / 2,475

A first scoring pass showed 28 mismatches; counting join-key collisions
found exactly 28 (time_ms, actor_net_guid) keys carrying two shots with
different weapons in the same millisecond. The mismatches were the join, not
the resolution.

Also measured here: 0 of 475 declared export groups mention
AbilitiesAndBuffs, converting 7-C's ceiling from assumption to fact.

### 5-K. net_guids.parquet and weapon identity (commits 47849d2, b258dfd)

NetGuidCache::net_guid_entries plus a NetGuidWriter export the containment
chain the parser had always computed and thrown away. 16,167 rows for
02d4d478, sorted by net_guid for byte-reproducibility.

The adapter walks that chain per shot, consulting actors.parquet (channel
opens) and net_guids.parquet (every registered GUID) at each hop because the
two cover different populations -- the weapon is in the first, its
FiringState only in the second.

tools/extract_equippables.py generates the display-name table from the C#
resolver's Define() list rather than anyone retyping 24 paths. It stays on
the Python side so the parser's no-hardcoded-names invariant holds.

Result: 2,475 / 2,475 shots resolved, weapon name and category counts
identical to the reference across all 19 weapons.

### 5-L. Fire mode classified from the firing-state name (commit 1f3afe4)

Found while checking whether the four target sections actually improved.
The adapter had inferred alternate fire from FiringState.BurstShotNumber
being non-zero. That counter indexes shots within any spray, so every
full-auto shot after the first was labelled alternate -- 1,462 of 2,475 --
and fire_mode_evidence was a hardcoded string.

The cost was invisible until weapon identity landed: spray_control drops
alternate-fire shots outright, so it was scoring 1,013 shots instead of
2,304 without anything looking wrong.

The real signal is the firing-state subobject's name, which net_guids.parquet
now carries. Reproducing ValorantShotFireModeResolver gives every bucket
identical to the reference (2273 / 130 / 31 / 22 / 19) and makes
spray_control EXACT.

Lesson worth keeping: the aggregate weapon counts matched perfectly while a
second field was wrong in a way that silently halved a downstream section.
Verifying the thing you built is not the same as verifying the sections it
was supposed to unblock.

Follow-on items surfaced during verification, both new sections:
7-J (EquippableUsed.NetGuid decodes wrong, blocks weapon_stats) and
7-I (172 events the reference emits and we do not, now classified as
server-world effects rather than dropped shots).

### 5-M. EquippableUsed and RegionalDamage (commits 90a50e1, e7414d9)

7-J closed. Two bugs, the second only visible once the first was fixed.

EquippableUsed was FieldType::Raw because the C# descriptor hides it behind
a custom .Decode(...) the extractor cannot read. The adapter, given no type,
read the bits as a fixed little-endian uint16. IntPacked is 8/16/24 bits
wide, so that only ever saw 272 of 632 occurrences, and IntPacked's
continuation flag sits in the low bit of the first byte, so every multi-byte
value came out odd -- while the engine requires dynamic NetGUIDs to be even.
All 115 values we produced were odd and none was a real actor.

The diagnosis came from two cheap discriminating checks rather than from
staring at the values: parity (115 odd vs the reference's 115 even) and
"does it resolve to an actor" (1/115 vs 114/115). Rank-order pairing between
the two value sets looked suggestive and was worthless -- the implied ratios
were 1.41 / 1.71 / 1.82 / 3.97 and two entities tied on frequency.

With the GUIDs correct, hits/damage/kills matched but head and body were
swapped. REGIONAL_DAMAGE_MAP had ordinals 0 and 1 reversed and put invalid
at 3; EAresRegionalDamage.cs has Normal=0, Headshot=1, Legshot=2,
RegionCount=3, Invalid_Radial=4, Invalid=5. The 18 genuine "no hit region"
events at ordinal 5 had been falling through to unknown_5.

Both fixes verified against the reference: 116 distinct GUIDs all even,
115 of 116 resolving to an actor (the extra is the record 6e6d544 recovers),
by_weapon identical for all 23 weapons, region_source byte-identical.


### 5-N. Movement, cross-validation, and three corrected claims

Cross-validation (commit 38ca3fe) changed what could be claimed at all.
Eleven replays have a reference bundle AND a source .vrf, not one, and
running all of them showed the 02d4d478 figures generalise exactly -- the
same section set is byte-identical on every replay. It also crashed on
1d898bfb, exposing a sparse-array padding bug one replay could never have
shown.

Movement (commit 3d37c68): the "+2,387 intermediate frames" note was hiding
a real defect. posture.distance_m was LOW for 10 of 10 players, which finer
sampling cannot cause. Our intra-packet sub-moves share a time_ms and defeat
posture.py's `0 < dt` guard, so a leg of every duplicated pair was dropped.
See 7-K.

Three documented claims were audited and two needed correcting:

  combat kill timeline   CONFIRMED, but the framing was wrong. "132 kills vs
                         119" reads as the C# parser undercounting kills. It
                         does not -- both bundles report combat_report_credits
                         132 and identical per_player kills. Only the
                         MulticastNotifyKilledEnemy stream is affected, and
                         all 13 extras are corroborated by lethal damage RPCs
                         in the reference's own bundle.
  kast                   CONFIRMED exactly as documented, 3 players +1.
  tactical               "3 players differ" was wrong: 8 of 10 differ, and it
                         is a reshuffle rather than a gain -- first_bloods and
                         first_deaths net to zero.
  combat.per_player      Numbers right, wording wrong. The 21 non-exact
                         fields are not "JSON float precision"; they are
                         genuine numeric differences from float32
                         accumulation, all under float32 epsilon.

Causation for the kill-derived claims was established by injecting the 13
RPCs into a copy of the reference bundle and re-running valplay's own
compute_metrics: the unmodified copy reproduces the reference, the injected
copy reproduces ours.

---

## 6. metrics.json Reproduction Status (02d4d478 vs reference)

Reference: valplay/pipeline/exports/02d4d478-.../metrics.json
  (produced by C# parser, bundle was slimmed: ~97% of rpc_received removed)

Our bundle: out/valplay_bundle/02d4d478-.../metrics.json
  (produced by vrfkit export + to_valplay_bundle.py + compute_metrics.py)

```
Section          Status       Notes
---------------------------------------------------------------------------
players          EXACT        10 players, PUUID/character/tier identical
side_winrate     EXACT        byte-identical after struct blob fix
economy          EXACT        byte-identical after struct blob fix
combat           MATCH*       per_player 270/270 within float32 epsilon
                              (249 byte-exact; the other 21 differ only in
                              JSON float precision, worst relative delta
                              3.6e-8 -- the earlier "270/270 exact" in this
                              document omitted that tolerance).
                              kill_timeline_check differs because OURS IS
                              MORE COMPLETE (132 kills vs ref 119; ref
                              missing char-576). Those two keys are the only
                              ones in combat that differ at all.
rounds           EXACT        after the 7-B timestamp fix
objective        EXACT        after the 7-B timestamp fix
ultimate         EXACT        after the 7-B timestamp fix
shot_rays        EXACT        after 7-B plus dropping coordinate rounding
spray_control    EXACT        69 cells, 2304 shots, zero differing cells
ability_usage    EXACT        after emitting package paths and f32-shortest
                              spawn coordinates
ability_detail   EXACT        after carrying the subobject GUID (cf97ecf)
objective_detail EXACT        same cause as ability_detail
movement_detail  EXACT        after collapsing intra-packet sub-moves
movement_summary EXACT        same, plus f32-shortest coordinates
posture          EXACT        same; distance_m had been LOW for 10/10
                              players, see 7-K
combat           OURS BETTER  two keys differ. kill_timeline_check: we carry
                              13 MulticastNotifyKilledEnemy RPCs the C#
                              parser never emits, all from character 576,
                              each corroborated by a lethal damage RPC in
                              the REFERENCE's own bundle at the same ms.
                              per_player: 249/270 byte-exact; the other 21
                              are the four damage-sum fields, differing by
                              at most 3.59e-8 relative -- under float32
                              epsilon. Every other field is exactly equal
kast             OURS BETTER  exactly 3 players +1 KAST round, caused by
                              the same 13 kills (proven by injecting them
                              into the reference bundle and reproducing our
                              output exactly)
tactical         OURS BETTER  8 of 10 players differ, not 3. It is a
                              reshuffle, not a gain: first_bloods and
                              first_deaths net to zero, trade_kills +7,
                              traded_deaths +5
economy_detail   OURS BETTER  credits and loadout identical for all 10
                              players. We resolve 496 of 496
                              PurchasedItemComponent buyers, the reference
                              151. All 496 buyers are real player states,
                              all 496 item classes resolve, and the
                              reference's set is a strict subset
weapons          OURS BETTER  all 19 weapons + shot counts identical; the
                              reference additionally bins 172 server-world
                              effects as "unknown" (7-I)
weapon_stats     OURS BETTER  by_weapon identical for all 23 weapons;
                              region_source and hp_tracking byte-identical.
                              Differs only on `excluded`: the +1 recovered
                              damage record and 7-I's 172
---------------------------------------------------------------------------
```

EXACT: identical Python object equality. 14 of 20 metric sections here,
       and the same 14 on all 11 cross-validated replays (section 6-A).
OURS BETTER: our value is more complete/correct than the C# reference.
BLOCKED: the data is present but a named defect prevents it being used.
         No section is BLOCKED, and no section differs for a reason that is
         not understood. Every remaining difference is a case where we carry
         data the C# parser does not.

Scoreboard metrics that Tracker.gg validated (K/D/A, ADR, HS%, KAST,
FK/FD, MK, rank): reproduced exactly for all 10 players from vrfkit data.



### 6-A. Cross-validation across every available reference bundle

Section 6 used to rest on 02d4d478 alone. Eleven replays have BOTH a source
.vrf and a C# reference metrics.json -- the claim in 7-G that only fd816a35
was cross-validated was wrong; fd816a35 is simply the one whose .vrf is
missing.

    python toolsalidate_metrics_corpus.py --jobs 3

runs the full pipeline over all eleven and prints a section x replay matrix.

Result (2026-08-01): **15 of 21 sections byte-identical on all 11 replays** --
the same set measured on 02d4d478, so those figures generalise.

  ability_detail  ability_usage  economy      movement_detail
  movement_summary  objective    objective_detail  players
  posture         rounds         shot_rays    side_winrate
  spray_control   ultimate       (+ note)

The six that vary do so per replay in the expected direction: kast and
tactical are EXACT on the replays where the C# parser missed nothing (7 and
8 of 11 respectively), and differ only where we recover kills it dropped.
combat, economy_detail, weapons and weapon_stats differ on every replay,
always because we carry more.

This is also what found the sparse-array crash: 1d898bfb produced no metrics
at all until the padding fix. One replay could not have surfaced it.

---

## 7. What Remains and Why (named gaps, ordered by impact)

### 7-A. Equippable (weapon actor) identity resolution [DONE 2026-08-01]

Implemented in commits 47849d2 (net_guids.parquet), b258dfd (adapter) and
1f3afe4 (fire mode, found while verifying the sections this unblocked).

  shots with a resolved weapon : 2,475 / 2,475  (100.00%)
  weapon name + category counts: identical to the C# reference for all
                                 19 weapons, zero differences

Section outcomes:
  spray_control  EXACT
  posture        by_weapon EXACT for all 10 players
  weapons        shot counts identical; differs only by the reference's
                 "unknown": 172 bucket (7-I) and a 1-RPM delta on two
                 weapons (7-B)
  weapon_stats   still zero hits/damage/kills -- blocked by 7-J, which is
                 unrelated to weapon identity

Historical detail follows, kept because the premise correction matters.

CORRECTED 2026-08-01. The earlier version of this section said the shot
EffectContainer carries the equippable net GUID and that the join needed no
Rust change. Both were wrong. Measured against the reference bundle:
effect_equippable is set on 0 of 2,647 shots, and firing_state GUIDs appear
in 0 of 2,475 actors.parquet rows. Full evidence in NEXT_STEPS_FINDINGS.md.

The route that actually works, verified end to end at 100%:
  shot.firing_state (adapter already emits it)
    -> NetGuidCache guid_to_outer      <- NOT currently exported
    -> equippable actor GUID
    -> actors.parquet class_path       <- already exported and correct
    -> weapon display name via a lookup table

Probe result on 02d4d478 (temporary instrumentation, since reverted):
  firing_state GUIDs present in guid->outer : 175 / 175  (100%)
  shots resolved to a weapon class_path     : 2,475 / 2,475  (100.00%)
  class_path equal to the C# reference       : 2,475 / 2,475
  reference equippable GUIDs in actors.parquet: 157 / 157, byte-identical

Three sub-tasks:
  a) Export the netguid table (Rust, vrf-export + a NetGuidCache accessor).
     Suggested: net_guids.parquet with (net_guid, path, outer_net_guid).
     16,167 rows for this replay. The data already exists in guid_to_outer
     (cache.rs:89) and guid_to_path (:88); the exporter never emits it.
     Export path as well as outer -- path is what distinguishes FiringState
     from ZoomedFiringState, and the C# fallback uses it.
  b) Walk the outer chain in the adapter (Python). Mirrors the C# tier-2
     resolver, ValorantShotEventEnricher.cs:163.
  c) Build the weapon display name table. The C# parser hardcodes this in
     ValorantEquippableResolver.cs:20 (130 lines of
     Define(class_path, name, category)). Keep it in the Python adapter so
     the Rust parser stays free of hardcoded names -- see section 8.

Effort: a Rust export addition plus adapter work, not the 1-2 hours the
earlier estimate claimed. No parser resolution redesign is needed.

Unlocks: 4 metric sections from BLOCKED to MATCH.

NOT needed: resolving InventoryComponent -> /Script/ShooterGame.AresInventory.
That is the C# tier-3 fallback and tier 2 already covers 100% of shots. It
remains interesting for other sections -- see 7-H.

### 7-B. 1ms timing alignment [DONE 2026-08-01]

Fixed in commit bff712a. rounds, objective and ultimate became EXACT, and
weapon_stats.hp_tracking's timeline went from every entry off by -1ms to
zero differences.

The diagnosis in this document was wrong in a way worth recording. It said:

> All timestamp differences are exactly -1ms. This is not random noise; it
> is a systematic choice of which packet timestamp to use (start vs end of
> the UE4 bunch).

It was not a boundary choice, and it was not systematic. vrf-frame computed

    let time_ms = (time_seconds * 1000.0) as u32;

which truncates and multiplies in f32, against the reference's
(ReplayEventJsonWriter.cs:194)

    (long)Math.Round(seconds * 1000d, MidpointRounding.AwayFromZero)

Only frames whose fractional millisecond was >= 0.5 landed early -- roughly
half of them. That is exactly why the differences that existed were always
-1 while many timestamps matched: the "systematic -1ms" reading came from
looking only at the rows that differed.

Lesson: "the differences are all -N" does not imply "everything is shifted
by N". Check how many values match before inferring a constant offset.

### 7-C. Unattributed ClassNetCache blocks [MEDIUM IMPACT, BOUNDED]

1,972,080,670 bits across 215 replays cannot be attributed to any export
group. These blocks frame correctly (malformed framing 0) but the group
resolution returns function_count=0 and they are counted as failures.

Breakdown:
  91.7%  AbilitiesAndBuffsComponent  -- no cache group declared in schema
  ~5%    MeleeAttackState1/2/3/4     -- digit suffix is a class variant,
                                        not an instance suffix; each has
                                        a distinct function table
  ~2%    RespawningWallPlate2_7      -- "2_7" too aggressive to strip
  ~1%    Various (space-in-name, etc.)

AbilitiesAndBuffsComponent is the real ceiling. Until the game server
declares its ClassNetCache group in the schema, no lookup can reach it.
This may change in a future build.

CONFIRMED 2026-08-01: searched all 475 declared export groups in
02d4d478's manifest -- zero contain the substring "AbilitiesAndBuffs".
This is now a measured fact rather than an assumption. Do not spend
time trying to recover those bits.

The MeleeAttackState variants are tractable: if MeleeAttackState1_C_ClassNetCache
etc. are added to the schema lookup logic, those would be recovered.

### 7-D. Ability/item class display names [DONE 2026-08-01]

Fixed in commit fc24b63. ability_usage is EXACT; ability_detail became
EXACT once the subobject GUID landed (commit cf97ecf).

The premise here was also wrong. It said the sections needed a
class-path-to-display-name table "extracted from the C# parser". The
reference does not use display names for abilities either -- it uses class
names. The entire difference was that our replication_class_path carried the
full object path ("Foo.Foo_C") where the reference emits the package path
("Foo"), and the two consumers that matter take path.split("/")[-1]
verbatim rather than splitting on the dot.

No name table was needed. A second, unrelated formatting difference in the
same sections was spawn coordinates: Float32 widened to Python float printed
2382.199951171875 where the reference shows 2382.2.

### 7-E. 13.02 golden vector coverage [MAINTENANCE]

Two 13.02 replays parse with malformed framing 0 and transform failures 0,
so the transform is working. However the 215-replay corpus is entirely
13.01, so 13.02 has no corpus-level regression guard. If a future session
modifies the transform path, 13.02 could silently break.

Fix: add a small 13.02 replay to the test corpus, or at minimum run
validate_corpus.py over the local demos and pin the output.

### 7-F. Parallelization [CLOSED 2026-08-01 -- MEASURED, NOT WORTH IT]

The premise was that framing could stay sequential while "transform+decode"
went wide. The framing observation is correct -- headers and declared bit
lengths really are plaintext -- but the payoff is not there: the transform
is 3.4% of an export, and the decode half cannot go wide at all. No code
changed; the measurement is the deliverable.

Measured on 02d4d478, release build, warm file cache, median of three
runs, on a 24-core i9-13900KS:

```
vrfkit export    2.60 s
vrfkit validate  1.48 s
```

Note which subcommand is which. `validate` runs the identical container +
DemoFrame + replication + sink path and omits only the Parquet writers, so
the 1.12 s gap IS the Parquet write. The "1.4s/replay" this section used
to quote is a `validate` figure; an export is 1.8x that, and the two were
being compared as if they were the same number.

Per-slice breakdown, from a temporary instrumented build (Instant timers
around each slice, reverted before commit). The instrumented total was
2.83 s against 2.60 s clean, so roughly 8% of timer overhead is spread
across these rows; the shares are of the instrumented total:

```
  oodle decompress            148 ms    5.2%
  DemoFrame iteration          21 ms    0.7%
  process_packet             1350 ms   47.7%
    read_packet (bunches)     115 ms    4.1%
    payload transform          97 ms    3.4%   <-- all 7-F can parallelise
    on_content_block          347 ms   12.3%   (group path resolution)
    field/rpc parse           681 ms   24.1%
      try_parse_rpc_params    272 ms    9.6%
      movement decode         167 ms    5.9%
      apply_overlay            62 ms    2.2%
      resolve_field_name       30 ms    1.1%
      raw_bits copy            20 ms    0.7%
      record push              21 ms    0.7%
  drain -> Parquet           1045 ms   36.9%
    fields.parquet            570 ms   20.1%
    movement.parquet          450 ms   15.9%
  writer finish                31 ms    1.1%
  net_guids write               6 ms    0.2%
```

The transform slice is 867,835,037 bits over 608,011 blocks, about
1.1 GB/s. It is genuinely pure -- the seed is
`seed_for(bit_count, actor_net_guid)` and nothing else -- so it is the one
slice that could be handed to workers.

Why the decode half cannot. Content blocks are not independent:

- `handle_channel_open` (pipeline.rs, three `internal_load_object` calls)
  passes the sink through, and `register_path` writes to the NetGuidCache.
  That is a phase-2 cache mutation, in stream order, on the actor-spawn
  path every replay exercises -- 2,028 opens on 02d4d478. Package-map
  export bunches would mutate it too but never fire on this replay
  (`exported_guids` is 0), so the spawn path is the load-bearing one.
- `on_actor_open` writes `ChannelState::archetypes`, which
  `on_content_block` reads to resolve the group path.
- That resolved group path selects the export group whose `num_exports`
  becomes `function_count`. Section 9 records why this is destructive to
  get wrong: the handle read is `ReadSerializedInt(max(num_exports, 2))`,
  so a block decoded against a stale cache reads its handles at a
  different bit width and desynchronises from its first field onward.

A block's meaning therefore depends on every earlier block. Only the
transform is order-free.

Ceiling: perfect N-way parallelism over the transform alone saves at most
3.4% of an export and about 6.5% of a validate. Against that: a rayon
dependency, per-worker scratch buffers, a gather step, and a bit-identity
risk -- reordering would change `skipped_bits` accumulation order, the
diagnostics vector, and which 32 lines survive the stream-failure cap, all
of which are load-bearing under NO SILENT SUCCESS. Do not reopen without
new measurements.

Where the time actually is, if a future session wants throughput: Parquet
writing (37%), `on_content_block` group-path resolution (12%), and
`try_parse_rpc_params` (10%). For batch work the real win is at process
level -- `validate_corpus.py` runs the 215 replays as 215 *sequential*
subprocesses, and each subprocess already owns its own output, so running
them N-wide is near-linear with no bit-identity risk at all.

### 7-G. Reproduce metrics.json for other replays [DONE 2026-08-01]

Done in commit 38ca3fe. This section claimed the Tracker.gg cross-validation
replay fd816a35 had no .vrf and implied it was the only reference bundle.
Eleven others have both a bundle and a .vrf; only fd816a35 is missing its
source. tools/validate_metrics_corpus.py now runs all eleven -- see 6-A.

### 7-H. Instance-named component groups [MEDIUM IMPACT, DESIGN WORK]

Several component groups arrive under an actor instance name and never reach
their declared class group, so their fields stay unnamed. The bits are
captured -- no-skip-path holds -- but no field_name is attached.

Top unnamed group_paths in 02d4d478's fields.parquet:
```
13043  InventoryComponent          (declared as /Script/ShooterGame.AresInventory)
 8042  ZoomStateMachine            fire mode / posture
 3124  MagazineAmmo                weapon_stats
 1782  CalloutRegionTracker
  746  MapTargetingState
  693  HealthDamageSection
  564  ReserveAmmo                 weapon_stats
  516  PMAimToolingPointsTarget
  470  VisionComponent
  464  AresAttributeSet_2
```

This is NOT a mechanical extension of resolve_cnc_for_instance_name
(cache.rs:301). That function strips _SEGMENT stems and tries
stem_ClassNetCache / stemComponent_ClassNetCache / stem_C_ClassNetCache.
No stem of "InventoryComponent" produces "AresInventory" -- the class carries
an "Ares" prefix the instance name does not have. Resolving it needs
structure the replay provides (most likely the subobject's outer chain in
guid_to_outer, leading to the owning actor's class) rather than string
manipulation.

7-A does NOT depend on this. It matters for ammo-level detail in weapon_stats
and for posture / fire-mode refinement.

Still open, but narrower than it looks. A separate subobject problem that
LOOKED like this one turned out to be an export gap, not a resolution gap:
content blocks describing a subobject carry its GUID, vrf-net parsed it, and
the sink discarded it, so every ItemSlot on a character collapsed onto the
actor GUID. Fixed in commit cf97ecf by adding fields.object_net_guid, which
made ability_detail and objective_detail EXACT. Before treating any remaining
"component" gap as a naming problem, check whether the data is simply not
being exported.

### 7-I. Verify the 2,647 vs 2,475 shot gap [SMALL, VALIDATION HYGIENE]

The reference bundle has 2,647 valorant_shot_received events and resolves
equippable on exactly 2,475 of them -- the same count our adapter emits.
Our adapter filters on FiringState.FiringPlayerState being present. If that
filter is what produces both numbers, then "ray_count 2475/2475 exact" is a
self-selecting comparison and reads stronger than it is.

Classify the 172: pull their source_id / fire_mode_evidence from the
reference events.ndjson and determine whether they are gun shots we drop or
ability/melee effects that were never gun shots.

CLASSIFIED 2026-08-01, and it is NOT a defect. All 172 carry
source_id = "DedicatedServerWorldSourceID" and fire_mode_evidence = None,
i.e. no firing state at all. They are server-world effects, not weapon
shots -- which is exactly why the reference cannot resolve an equippable for
them either and files them under "unknown".

Our adapter filters on FiringState.FiringPlayerState being present, which
excludes them correctly. The only consequence is cosmetic: the reference's
weapons.shots_by_weapon carries an "unknown": 172 bucket and reports
distinct_weapons 20 where we report 19.

Decide whether to emit them for byte-parity or keep the cleaner output. If
kept, this stops being a gap and becomes a documented divergence.

### 7-J. EquippableUsed.NetGuid decoded wrong [DONE 2026-08-01]

Fixed in commits 90a50e1 (type correction) and e7414d9 (RegionalDamage enum,
a second bug the first fix exposed).

weapon_stats.by_weapon is now identical to the reference for all 23 weapons,
and region_source is byte-identical. Remaining deltas are the +1 recovered
damage record, the 7-I server-world effects, and the 7-B 1ms offset on
hp_tracking timestamps -- no unexplained difference remains.

Root cause: DamageParameters.cs:51 attaches a custom decoder
(.Decode(ValorantPayloadDecoders.Equippable)) that extract_descriptors.py
cannot see through, so the field landed in table.rs as FieldType::Raw. That
decoder is exactly archive.ReadIntPacked(), which FieldType::ObjectNetGuid
already implements. With no type, the adapter guessed a fixed little-endian
uint16 -- wrong both because IntPacked is 8/16/24 bits wide depending on the
value, and because the low bit of the first byte is IntPacked's continuation
flag, which made every multi-byte value odd when dynamic NetGUIDs must be
even.

Generalisable lesson: any C# field with a custom .Decode(...) is invisible to
the extractor and silently becomes Raw, and Raw reads as a deliberate choice
rather than an unknown.

AUDITED 2026-08-01 (commit 059713e). Every .Decode() call site in the C#
descriptors was checked. EquippableUsed was not the only casualty -- five
damage geometry fields hit the same trap while vrf-decode already implemented
their exact quantization, and all five are now typed:

  DamageOrigin                      VectorNetQuantize100
  DamageImpactLocation              VectorNetQuantize
  DamageImpactBoneRelativeLocation  VectorNetQuantize
  DamageDirection                   VectorNetQuantizeNormal
  DamageImpactNormal                VectorNetQuantizeNormal

Unlike EquippableUsed these were not producing wrong values, just undecoded,
so no metric section moved. Verified against the reference on 258 damage
records: all five vectors identical on every one.

The remaining 153 Raw entries are genuinely raw -- RawPayload("...") blob
types (TArray<FEffectDataFloat>, FTransform, ...) that the struct and effect
blob decoders handle downstream. Re-run the audit when new descriptors land.

The original investigation notes follow.

Blocked: weapon_stats hits / regions / damage / kills (all reported 0).

Found 2026-08-01 while verifying 7-A. weapon_stats resolves the gun behind
each hit from MulticastNotifyDamage_*.EquippableUsed.NetGuid. Our values do
not match the reference and resolve to nothing:

```
                distinct  total   range          overlap with ref
  reference     115       631     0 - 35,346     --
  ours          115       625     961 - 61,261   0
```

The structure is right and the entity set is right: 115 distinct on both
sides, and the per-entity frequencies line up in rank order (42, 40, 30, 19,
16, 16 on both). Only the GUID *values* differ, and not one of ours appears
in the reference. The payload key set is otherwise byte-identical, so this
is isolated to how the object-reference field itself is read.

Two measurements pin it down:

```
                        lands in actors.parquet     parity
  reference             114 / 115                   115 even, 0 odd
  ours                    1 / 115                     0 even, 115 odd
```

Section 9 records the engine rule: IsDynamic => IsValid && (Value & 1) == 0.
Weapon instances are dynamic actors, so a correct GUID here must be EVEN.
Every one of ours is ODD, and they resolve to no actor. The reference's are
all even and 114 of 115 resolve to a weapon class path.

So we are not mis-mapping a correct GUID; we are producing a value that is
not a valid dynamic NetGUID at all. Start from how the object-reference RPC
parameter is read (a missing shift or an off-by-one-bit read is the shape
that matches) and compare against the C# ValorantPayloadDecoders path.

The rank-order pairing between the two value sets is NOT evidence of a
transform: the implied ratios are 1.41 / 1.71 / 1.82 / 3.97, and two entities
tie at 16 occurrences, so the pairing is not even well defined.

Two traps for whoever picks this up:
  - Exactly one of our 115 values does land in actors.parquet. With the
    other 114 missing, treat that as coincidence, not partial correctness.
  - The reference emits GUID 0 for 22 unresolved hits (valplay's own notes
    record this). A correct decode must be able to produce 0. Ours never
    does, which is a second independent signal rather than a rounding
    difference.

Our total is 625 vs the reference's 631; the six-record gap should be
explained as part of the same investigation.

### 7-K. Intra-packet sub-moves [DONE 2026-08-01]

Fixed in commit 3d37c68. Opened when movement_summary / movement_detail /
posture were the last sections differing for an unverified reason: a note
said we emit 2,387 more samples than the reference because "vrfkit captures
intermediate move frames".

Measured, that phrasing was directionally right and materially incomplete.
The extras are not extra frames in time -- every one lands on a
(time_ms, packet_id, character) triple the reference already has. Our
decoder walks the marker-chained move sequence inside a packet and emits
each sub-move; the reference keeps only the last. 1,687 of the 2,387 carry
distinct positions (genuine intermediate detail), the other 700 are adjacent
wire-level resends. Zero reference rows are missing from ours.

The part the note missed: posture.distance_m was WRONG, low for 10 of 10
players by 3.1-5.2 m. posture.py requires 0 < dt before adding a distance
step but updates last_sample unconditionally, so two sub-moves at the same
ms make it add the first leg and silently discard the second. A shorter
distance cannot come from finer sampling, which is what made it findable.

movement.parquet still carries every sub-move. Only the bundle collapses to
the last per (time_ms, character), which is the shape the consumer was
written against: dropping exactly those rows reproduces the reference's
movement_detail on 60/60 values with no rounding.

Lesson: "we emit more rows than the reference" is not self-evidently
harmless. Check the direction of every derived metric -- a value that moved
the way extra data cannot move it is the tell.

---

## 8. Design Invariants (do not break)

These are load-bearing. Breaking any one silently corrupts downstream
consumers without any test failing.

NO SKIP PATH
  Every field emits (group_path, handle, name, bit_count, raw_bits) even
  when nothing is known. Overlay is additive: typed values fill value_*
  columns; failure leaves them null with raw bits intact.
  Rationale: a parser that silently drops data cannot be trusted even when
  it looks correct. The oracle's honesty matters more than its pass rate.

NO SILENT SUCCESS
  A block whose group cannot be resolved fails loudly (function_count=0
  returns Err, counted in rpc_stream_failures). Never guess a capacity to
  make the number look better; that is silent corruption.

A CUSTOM C# DECODER MEANS THE TYPE IS UNKNOWN, NOT RAW
  extract_descriptors.py cannot see through .Decode(...) in the C#
  descriptors, so any field with a custom decoder lands in table.rs as
  FieldType::Raw. That is indistinguishable from a field we deliberately
  keep raw. Two real bugs came from this (7-J and the damage geometry
  fields). When new descriptors land, diff the .Decode() call sites against
  the Raw entries in table.rs before trusting them.

GENERATED FILES ONLY VIA GENERATORS
  crates/vrf-decode/src/table.rs    -- only via tools/extract_descriptors.py
  crates/vrf-transform/src/sbox.rs  -- only via tools/extract_sboxes.py
  crates/vrf-transform/tests/data/golden_vectors.rs -- only via tools/extract_golden.py
  tools/equippable_table.py         -- only via tools/extract_equippables.py
                                       (check staleness: --check)
  Hand-editing these is how subtle bugs enter.

NO HARDCODED NAMES IN THE PARSER
  The Rust crates emit class paths, never display names. Weapon display
  names ("Vandal") exist nowhere in the wire format -- the game ships them
  as client assets -- so a table is unavoidable, but it lives in the Python
  adapter (tools/equippable_table.py) where labelling is a presentation
  concern. Moving it into a Rust crate would break this invariant.

ASCII ONLY IN CODE AND COMMENTS
  The Windows cp949 console truncates output at the first non-ASCII byte
  in a Rust format string. This is not a style rule; it is a correctness
  constraint for the diagnostics path.

NO UNSAFE
  #![forbid(unsafe_code)] everywhere. Oodle decompression is the only
  case that needed unsafe; it is behind a C FFI in a separate crate.

---

## 9. Key Technical Facts (for a new session starting from this document)

### Wire format facts
- IntPacked: max 5 bytes, value |= (b>>1) << shift, low bit = continue
- ReadSerializedInt(max): value_bits = max.ilog2(); reads that many bits,
  then one more conditional bit. ReadSerializedInt(1) consumes ZERO bits.
  Unreal uses FMath::Max(N, 2) to avoid the degenerate case.
- FString: positive length = UTF-8, negative = UTF-16 (x2 bytes)
- isFieldExported: 1 BYTE, not 1 bit
- UE GUID: IsDynamic => IsValid && (Value & 1) == 0  (EVEN is dynamic)
- Bunch header bit layout documented in crates/vrf-net/src/pipeline.rs

### Transform constants
  12.10  0x12fd0ee5 / 0x1b / subtract / no sbox
  12.11  0x409d36a3 / 0x23 / ADD      / no sbox
  13.00  0x2949b6ef / 0x11 / subtract / sbox
  13.01  0xe62fcd5c / 0x24 / subtract / no sbox
  13.02  0x9e81a37c / 0x04 / subtract / sbox
  TAIL_XOR == SEED_ADDEND & 0xFF for all 5 builds (pinned as test)
  S-boxes are shared across 13.00 and 13.02

### ClassNetCache handle read (critical)
  The handle uses ReadSerializedInt(FMath::Max(group.num_exports, 2)).
  WITHOUT the max-2 clamp, single-export groups consume 0 bits for the
  handle and desync the stream. This is confirmed from Unreal Engine
  source (DataChannel.cpp) and independently verified against corpus data.

### Corpus baseline (regression values for 02d4d478)
  content blocks  608,020    RPCs emitted    342,735
  fields emitted  429,633    movement rows 1,839,607
  decode errors         0    actors.parquet    3,827
  oracle pass rate  98.95%

  combat.per_player: 27 fields x 10 players = 270 comparisons, 0 mismatches

### Tools directory
  extract_sboxes.py      -- generates sbox.rs
  extract_equippables.py -- generates equippable_table.py (weapon names)
  extract_golden.py      -- generates golden_vectors.rs
  extract_descriptors.py -- generates table.rs (type overlay)
  apply_type_corrections.py -- wire/declaration mismatches
  compare_combat_report.py  -- CombatReport cross-check vs C#
  compare_rpc_params.py     -- RPC parameter cross-check vs C#
  compare_with_csharp.py    -- structural cross-check
  analyze_coverage.py       -- field coverage analysis
  validate_corpus.py        -- full 215-replay batch validation
  find_skips.py             -- finds which replays still have skipped bits
  to_valplay_bundle.py      -- vrfkit Parquet -> valplay bundle adapter

### Path references
  Parser repo   : C:\Users\yakihyuk0728\Documents\GitHub\vrfkit
  C# reference  : C:\Users\yakihyuk0728\Documents\GitHub\ValorantReplayParser
                  HAS USER'S UNCOMMITTED WORK (17 entries). Never modify.
                  Instrumentation: only in clean files, always reverted.
  valplay       : C:\Users\yakihyuk0728\Documents\GitHub\valplay
                  Never modify.
  Corpus        : valplay\data\raw\vrf  (215 x .vrf, all 13.01)
  C# ref output : valplay\pipeline\exports\02d4d478-...\
                  SLIMMED: 97% of rpc_received removed, several keys stripped
  Local 13.02   : %LOCALAPPDATA%\VALORANT\Saved\Demos\*.vrf (4 files)

---

## 10. Tradeoffs Made and Why

### Parquet over NDJSON
Measured on 02d4d478:
  fields:   Parquet 12.6 MB vs NDJSON ~318 MB  (25x smaller)
            Parquet read ~0.05s vs NDJSON ~1.1s (22x faster)
  movement: Parquet 30.7 MB vs NDJSON ~566 MB  (18x smaller)
Parquet is the clear winner for a pipeline that reads the same data many
times. Downside: not human-readable without a viewer.

### Adapter over rewriting compute_metrics.py
The 21 metric sections were validated against Tracker.gg scoreboard data
for 10 players. Rewriting them would discard that validation. The adapter
adds a translation layer (~600 lines) but keeps the proven analytics code
unchanged. Downside: any schema mismatch between vrfkit output and what
the adapter produces causes a silent wrong value rather than an error.

### No hardcoded names anywhere
Resolution rules use runtime schema data, not lists of agent names or
map names. A hardcoded list breaks on every new agent or map. Downside:
harder to debug when resolution fails (the failure is "no group found"
rather than "this name is not in the table").

### Loud failures over silent drops
parse_class_net_cache returns Err for unresolved groups instead of Ok(0).
This reduced the corpus pass rate from an inflated 100% to an honest
~98-99%. The tradeoff is that the oracle number looks worse. The gain is
that every discarded bit is counted and the class is named, which is what
allows the gaps to be investigated and closed.

### No parallelization within a replay (measured, closed)
The pipeline is sequential within a replay and stays that way. This entry
used to name the blocker as atomic oracle counters; that was wrong, and it
made the problem sound mechanical. The real blocker is that a content
block's resolved group path -- and therefore its `function_count` and the
bit width of its handle read -- depends on cache and channel state mutated
by earlier blocks in the same phase-2 walk. Only the payload transform is
pure, and it is 3.4% of an export (97 ms of 2.83 s instrumented). The
tradeoff is now a measured one rather than a deferral: the gain is capped
below what a rayon dependency and the reordering risk cost. See 7-F for
the full per-slice breakdown and for the process-level alternative that
does pay off.

### Blob decoders in sink.rs vs vrf-decode
The struct blob decoders (RoundResults etc.) are wired in sink.rs rather
than as a layer in vrf-decode, because they need access to the resolved
group path to know which blob format to apply. A cleaner architecture would
pass the group path through to vrf-decode, but that would require changing
the decode trait signature. Current approach works; refactoring is optional.
