# vrfkit Project Status

Last updated: 2026-08-01. Reflects commit 21003aa (26th commit, master).
All numbers come from direct tool runs, not estimates.

Section 7-A was corrected on 2026-08-01 after its premise was disproved by
measurement. See NEXT_STEPS_FINDINGS.md for the evidence.

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
# Expected: 228 passed, 0 failed across all crates
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
#                  movement 1839607, decode errors 0
python tools\compare_combat_report.py
# Must print: ALL INTERESTING SHAPES MATCH
python tools\validate_corpus.py .\target\release\vrfkit.exe `
  "C:\Users\yakihyuk0728\Documents\GitHub\valplay\data\raw\vrf"
# Baseline: blocks 136,545,822  fields 98,883,979  rpcs 75,571,092
#           malformed 0  skipped 1,972,080,670
```

### What to do next (highest impact first)
See Section 7 for full detail, and NEXT_STEPS_FINDINGS.md for the measured
evidence behind the 7-A correction. The single most valuable next task is:

**7-A. Equippable (weapon) identity resolution** -- verified route, 100% hit.
  shot.firing_state -> NetGuidCache outer chain -> equippable actor GUID
  -> actors.parquet class_path -> display name.
  Needs one small Rust export addition (the netguid -> path/outer table is
  computed today but never written out), then adapter work.
  Verified on 02d4d478: 2,475 / 2,475 shots resolve, class_path identical
  to the C# reference on every one.
  Unlocks 4 metric sections: weapons, weapon_stats, spray_control, posture.

After that: 1ms timing alignment (Section 7-B) makes 5-6 more sections
byte-exact -- but note those sections are already numerically correct, so
7-B is cosmetic parity, not new capability.

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
manifest.json -- all written by `vrfkit export`. A Python adapter
(tools/to_valplay_bundle.py) converts these into the bundle shape that
valplay's compute_metrics.py already consumes, so the validated 21-section
analytics pipeline runs unchanged on our data.

---

## 2. Repository State (2026-08-01)

```
branch       : master
commits      : 26
tests        : 228 passing, 0 failed
clippy       : 0 warnings (--all-targets -- -D warnings)
fmt          : clean (--check)
working tree : clean
valplay repo : 0 modified files (user's uncommitted work untouched)
ValorantReplayParser : 0 modified files (instrumentation always reverted)
```

### Commit list

```
de24d6d  feat(decode, tools): decode shot EffectContainer blob
cc5dabd  feat(decode): decode RoundResults, TeamEconomy and RoundInfos blobs
df20d5b  feat(export): write actors.parquet with channel open/close events
b6947ee  feat(tools): adapter that feeds vrfkit output to the metrics pipeline
7c2faa1  docs: correct the README figures the honesty fix invalidated
6e6d544  feat(schema): resolve ClassNetCache groups from actor instance names
00dce40  test(net): update stale zero-function test
29b2936  fix(net): stop dropping ClassNetCache blocks for unresolved groups
90727ed  fix(net): clamp ClassNetCache handle read to a minimum of two
b531724  feat(oracle): name the class behind every payload-stage failure
bb797d2  fix(oracle): count payload-stage failures in the pass rate
0c2df40  docs: README with measured cross-parser comparison
070a953  test(tools): cross-parser verification harnesses
721f954  feat(cli): vrfkit inspect / validate / export
9ded7ae  feat(export): columnar Parquet output
157ed72  feat(movement): decode the remote-character update protocol
29aae8a  feat(decode): primitive decoders, nested arrays and a type overlay
f742245  feat(net): Unreal replication, framed with no skip path
33c4355  feat(schema): receive the replay's own dynamic field schema
5a634ae  feat(frame): DemoFrame iteration between container and replication
6f3cbcc  feat(container): .vrf container, chunk stream and Oodle decompression
8be1abc  feat(transform): five per-build payload transforms, golden-verified
7f3377d  feat(bitio): LSB-first bit reader and Unreal wire primitives
2df595d  chore: cargo workspace scaffolding and licensing
```

---

## 3. Crate Structure

```
vrfkit/
  crates/
    vrf-bitio       -- LSB-first bit reader, UE wire primitives (22 tests)
    vrf-transform   -- per-build payload transforms, golden-verified (31 tests)
    vrf-container   -- .vrf container, chunk stream, Oodle decompression (1 test)
    vrf-frame       -- DemoFrame iteration (0 unit tests, covered by integration)
    vrf-schema      -- dynamic field schema from replay wire (51 tests)
    vrf-net         -- Unreal replication pipeline, no skip path (31 tests)
    vrf-decode      -- primitive decoders, nested arrays, struct blobs (46 tests)
    vrf-movement    -- remote-character update protocol (5 tests)
    vrf-export      -- columnar Parquet writers (16 tests)
    vrfkit          -- CLI: inspect / validate / export (5 tests)
  tools/            -- Python generators and verification harnesses
```

Total: 228 tests.

---

## 4. Corpus Verification Numbers (215 replays, all ++Ares-Core+release-13.01)

```
succeeded          : 215 / 215
failed             : 0

oracle pass rate
  min              : 97.49%  (worst file, attribution failures)
  median           : ~98.9%
  max              : 99.99%

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
decode errors      :       0
typed (value_*)    :    35.7%  (35.7% of 1,239,406 fields.parquet rows)
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
combat           MATCH*       per_player 270/270 exact; kill_timeline_check
                              differs because OURS IS MORE COMPLETE
                              (132 kills vs ref 119; ref missing char-576)
rounds           ~1ms         round_intervals off by exactly 1ms
objective        ~1ms         side_switch_ms off by 1ms; round_results 18/18
ultimate         ~1ms         cast_times_ms off by 1ms
tactical         OURS BETTER  3 players differ; vrfkit recovered 13 kill RPCs
kast             OURS BETTER  3 players +1 KAST round; same root cause
shot_rays        ~1ms         ray_count 2475/2475 exact; sample .ms off 1ms;
                              weapon field "unknown" (equippable not resolved)
movement_summary MINOR        1,839,607 vs 1,837,220 samples (+2387 rows,
                              vrfkit captures intermediate move frames)
movement_detail  MINOR        per-character floats <0.1 cm/s speed delta
ability_usage    MINOR        spawn counts correct; class names differ
                              (C# uses display names; we use wire class paths)
objective_detail MINOR        18 rounds correct; timing off 1ms
economy_detail   MINOR        18 rounds, correct credits; weapon display names
ability_detail   MINOR        events correct; timing off 1ms
weapons          BLOCKED      shots 2475 but all "unknown" weapon
weapon_stats     BLOCKED      same equippable resolution gap
spray_control    BLOCKED      requires weapon name to group
posture          BLOCKED      requires equippable identity
---------------------------------------------------------------------------
```

EXACT: identical Python object equality.
~1ms: numerically correct; systematic -1ms offset from bunch timestamp choice.
OURS BETTER: our value is more complete/correct than the C# reference.
MINOR: correct data, cosmetic naming or sample-count difference.
BLOCKED: requires equippable (weapon actor) resolution -- see Section 7.

Scoreboard metrics that Tracker.gg validated (K/D/A, ADR, HS%, KAST,
FK/FD, MK, rank): reproduced exactly for all 10 players from vrfkit data.


---

## 7. What Remains and Why (named gaps, ordered by impact)

### 7-A. Equippable (weapon actor) identity resolution [HIGHEST IMPACT]

Blocks: weapons, weapon_stats, spray_control, posture (4 metric sections).

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

### 7-B. 1ms timing alignment [LOW IMPACT, COSMETIC]

All timestamp differences are exactly -1ms. This is not random noise; it
is a systematic choice of which packet timestamp to use (start vs end of
the UE4 bunch). No metric threshold operates at 1ms granularity, so this
has zero effect on any computed value.

Fix: in pipeline.rs, shift the time_ms attribution by +1ms, or verify
which boundary the C# parser uses and match it.

Unlocks: rounds, objective.side_switch_ms, ultimate.cast_times_ms,
shot_rays[].ms go from ~1ms to EXACT. Probably 5-6 more EXACT sections.

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

### 7-D. Ability/item class display names [LOW IMPACT]

ability_usage and ability_detail use wire class paths (e.g. Ability_E_C)
instead of display names (e.g. "Smoke"). This causes MINOR differences
but the underlying data is correct. A class-path-to-display-name table
exists in the C# parser; extracting it closes this gap.

### 7-E. 13.02 golden vector coverage [MAINTENANCE]

Two 13.02 replays parse with malformed framing 0 and transform failures 0,
so the transform is working. However the 215-replay corpus is entirely
13.01, so 13.02 has no corpus-level regression guard. If a future session
modifies the transform path, 13.02 could silently break.

Fix: add a small 13.02 replay to the test corpus, or at minimum run
validate_corpus.py over the local demos and pin the output.

### 7-F. Parallelization [OPTIONAL PERFORMANCE]

Content block headers and their declared bit lengths are plaintext (before
the transform). Framing can stay sequential while transform+decode goes
wide. For the current 1.4s/replay speed this is not urgent, but for a
production pipeline ingesting hundreds of new replays per day it matters.

### 7-G. Reproduce metrics.json for a replay other than 02d4d478

The Tracker.gg cross-validation was done on fd816a35, but that replay's
.vrf does not exist in data/raw/vrf (only the parsed bundle). Validation
on 02d4d478 is strong (270 comparisons, 0 mismatches for scoreboard
metrics) but it is still one replay. Running the adapter on 3-4 more
replays and spot-checking K/D/A would raise confidence.

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

### 7-I. Verify the 2,647 vs 2,475 shot gap [SMALL, VALIDATION HYGIENE]

The reference bundle has 2,647 valorant_shot_received events and resolves
equippable on exactly 2,475 of them -- the same count our adapter emits.
Our adapter filters on FiringState.FiringPlayerState being present. If that
filter is what produces both numbers, then "ray_count 2475/2475 exact" is a
self-selecting comparison and reads stronger than it is.

Classify the 172: pull their source_id / fire_mode_evidence from the
reference events.ndjson and determine whether they are gun shots we drop or
ability/melee effects that were never gun shots.

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

GENERATED FILES ONLY VIA GENERATORS
  crates/vrf-decode/src/table.rs    -- only via tools/extract_descriptors.py
  crates/vrf-transform/src/sbox.rs  -- only via tools/extract_sboxes.py
  crates/vrf-transform/tests/data/golden_vectors.rs -- only via tools/extract_golden.py
  Hand-editing these is how subtle bugs enter.

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

### No parallelization yet
The current pipeline is sequential within a replay. Adding rayon-based
parallelism over content blocks would require either (a) making the oracle
counters atomic or (b) collecting per-block results and merging. The gain
at 1.4s/replay is marginal for batch work but could matter for a streaming
pipeline. Deferred because the correctness work is more urgent.

### Blob decoders in sink.rs vs vrf-decode
The struct blob decoders (RoundResults etc.) are wired in sink.rs rather
than as a layer in vrf-decode, because they need access to the resolved
group path to know which blob format to apply. A cleaner architecture would
pass the group path through to vrf-decode, but that would require changing
the decode trait signature. Current approach works; refactoring is optional.
