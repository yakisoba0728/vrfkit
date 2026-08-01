# vrfkit Project Status

Last updated: 2026-08-02. Includes the replay-coverage audit through 8eb5909,
the concurrent master audit corrections through 101c33a, the code audit fixes
in section 12, and the Codex needs-work results in section 14.
All numbers come from direct tool runs, not estimates.

Section 7-A was corrected on 2026-08-01 after its premise was disproved by
measurement, then implemented and verified at 100%. See
NEXT_STEPS_FINDINGS.md for the evidence trail.

Section 7-H was likewise disproved by measurement on 2026-08-01 -- but unlike
7-A it has no implementation on the other side. It is closed NOT SOLVABLE, with
no parser change, and section 8 carries the invariant it produced.

---

## QUICK START FOR THE NEXT SESSION

Read this section first. Everything else is supporting detail.

### Where things are
```
Parser (Rust)  : C:\Users\yakihyuk0728\Documents\GitHub\vrfkit
C# reference   : C:\Users\yakihyuk0728\Documents\GitHub\ValorantReplayParser
                 Tree is CLEAN, on branch local/vrfkit-descriptors (f67ea66).
                 The "17 uncommitted entries" warning that stood here until
                 2026-08-02 is obsolete: that work is committed as fe5343a.
                 `main` MUST STAY AT 2d2e05e. Published bundles stamp
                 parser_version 1.0.0+2d2e05e8, so moving it invalidates every
                 comparison figure in this document. The stamp records Git
                 HEAD, not a clean source tree: section 13-H explains the
                 descriptor provenance caveat. Treat the published bundles as
                 the immutable reference. Do not merge the branch into main,
                 regenerate the bundles, or pull.
                 Changing a descriptor there is allowed ON THE BRANCH, with
                 primary-source proof and the test that pins it (see 13-C).
                 This delegate branch's table was generated from that branch.
                 Current master also depends on local/pawn-descriptors at
                 d2b76f2 in the separate clean VRP-pawn-descriptors worktree.
                 During integration, use the feature branch's generator with
                 that newer C# worktree or 92 master entries disappear.
valplay        : C:\Users\yakihyuk0728\Documents\GitHub\valplay
                 Never modify. Run its scripts by absolute path only.
Corpus (.vrf)  : C:\Users\yakihyuk0728\Documents\GitHub\valplay\data\raw\vrf
                 215 files, all ++Ares-Core+release-13.01
Local 13.02    : %LOCALAPPDATA%\VALORANT\Saved\Demos\*.vrf
                 Game-owned rotating input; currently 3 files named 1/2/3.vrf.
                 Never point a baseline at this directory.
Older fixtures : C:\Users\yakihyuk0728\Documents\GitHub\ValorantReplayParser\tests\Test.Integration\Replays
                 One source fixture each for 12.10, 12.11, and 13.00.
                 READ ONLY; do not run baselines from this repository.
Local baselines: %LOCALAPPDATA%\vrfkit\baseline-corpora\build_*
```

### Verify the build before touching anything
```powershell
cd C:\Users\yakihyuk0728\Documents\GitHub\vrfkit
$env:CARGO_TARGET_DIR = $null
cargo test 2>&1 | Select-String "test result"
# Expected: 246 passed, 0 failed across all targets (243 regular + 3 doctests).
# Sum the per-target lines; the last line is one target, not the total.
# This figure has been stale twice. Re-measure before quoting it.
cargo clippy --all-targets -- -D warnings 2>&1 | Select-String "^error"
# Expected: no output (exit 0)
cargo fmt --check
# Expected: exit 0
python tools\check_effect_decoder.py --check
# Expected: OK: 12 live effect decoder cases
python tools\check_ascii.py --check
# Expected: OK: 61 tracked Rust file(s), ASCII only
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
#           malformed 0  skipped 1,972,080,670    (~30s, runs 16-wide)
python tools\check_corpus_baseline.py --baseline tools\baselines\build_1302.json
# This delegate branch predates master fix 3a4b04, so its old JSON reports the
# Saved\Demos input-set mismatch. Current master pins one stable copied replay
# under baseline-corpora and passes 1/1. Preserve that file during integration.
python tools\check_export_baseline.py --baseline tools\baselines\export_02d4d478.json
# Expected: OK ... 3 printed counters cross-check against their Parquet files.
# The strongest single guard: it pins all 21 export counters plus every Parquet
# file's rows AND bytes, and caught every counter move this session before
# anything else did. On DRIFT, explain each line before passing --update. The
# point is that a silent change is impossible, not that the numbers are sacred.
python tools\check_corpus_baseline.py --baseline tools\baselines\build_1210.json
python tools\check_corpus_baseline.py --baseline tools\baselines\build_1211.json
python tools\check_corpus_baseline.py --baseline tools\baselines\build_1300.json
# Expected for each older build: OK: 1 replays match the baseline
python tools\check_export_baseline.py --baseline tools\baselines\export_02d4d478.json
# Expected: OK: ... matches the baseline (NetGUID rows 16167, ...)
```

The last one guards the EXPORT path; the four above it guard the VALIDATE
path only. That distinction is why `NetGUID rows` went unread for the whole
project: `vrfkit validate` writes no Parquet, so the oracle never prints the
counter and validate_corpus.py's PATTERNS could not have had an entry for it.
check_export_baseline.py pins every counter the export summary prints, plus
each Parquet file's row count and byte size, and separately cross-checks the
three printed counters that ARE row counts (`NetGUID rows`, `Movement rows`,
`Actor opens + Actor closes`) against the files they name. Both halves were
driven to failure on a deliberately broken build before being committed; see
commit bfd0229 for the exact output of each.

For a change that is supposed to alter nothing at all -- a refactor, or a
performance change like 5-P -- the counters above are too coarse. Hash the
output instead. Delete `out\nested` first; a stale file makes a matching hash
meaningless.
```powershell
Get-ChildItem out\nested\*.parquet | Sort-Object Name |
  ForEach-Object { "{0}  {1}" -f (Get-FileHash $_ -Algorithm SHA256).Hash, $_.Name }
# 02d4d478, re-measured 2026-08-02 at HEAD:
#   84076CF7CA398C957C3E67148D0622F72E809CB4E2157F66CD4F18B197E65D7B  actors.parquet
#   43006C26AE546FB30AFFD9A36BA3C19649AE71322AEFB9121589521E69EB6856  fields.parquet
#   1242BBB15B29BE267BA4B0326BCBC508B5E2AC6C7CD8A1570035C335C04D9363  movement.parquet
#   501CABC678770431D0FEC9C37C4E21ED06193BB93263313959E87865625BBA0F  net_guids.parquet
```
The fields.parquet line said 2DDC81D8... and "unchanged since before 5-P"
until 2026-08-02. Three of the four were still right; that one had been stale
since 59700c5 (FName isHardcoded), which changed field values by design. A
hash pinned in prose goes stale silently -- which is the argument for
check_export_baseline.py above, where the numbers are pinned in a file a
script reads.

All four were confirmed byte-reproducible across two identical exports on
2026-08-02, so a hash comparison here is evidence and not noise.
manifest.json is deliberately not in that set: it records elapsed time, so it
differs on every run by design.

### What to do next (highest impact first)
See Section 7 for full detail, NEXT_STEPS_FINDINGS.md for the measured
evidence behind the 7-A correction, and Section 11 for the replay-coverage
audit. Non-Bomb mode coverage remains input-blocked and unmeasured: supply a
mode-labelled non-Bomb replay before making any claim about it.

7-A, 7-B, 7-D, 7-E, 7-G, 7-I, 7-J and 7-K are all DONE. The harness reports
**16 of 21 keys byte-identical on all 11 cross-validated replays**; excluding
the constant provenance `note`, the honest metric count is 15 of 20.

Verify it yourself:
```powershell
python tools\validate_metrics_corpus.py --jobs 3
# Expected: sections exact on ALL   : 16 / 21
python tools\check_corpus_baseline.py --baseline tools\baselines\build_1302.json
# Expected after integrating current master's 3a4b04: OK, 1 stable replay.
```

No section is BLOCKED. Most differences reflect data the C# parser drops:

  combat / kast             13 MulticastNotifyKilledEnemy RPCs from
                            character 576 that the C# parser never emits
  economy_detail            496 of 496 purchase buyers resolved vs its 151
  weapon_stats              one damage record commit 6e6d544 recovers

Tactical's root cause is now named -- a one-character typo in the C# Gekko
descriptor, section 13-C -- but five of its values are still higher in the
reference, including one opening_duels_won difference with its denominator
conserved. The mechanism is a non-monotonic kill-timeline derivation, not a
data-volume gain. Section 6 records the exact replay/value pairs. Do not
describe all five varying sections as understood or as monotonic gains.

What is actually left: **nothing in section 7 is open.** Every item is either
done, or closed with a measurement showing it cannot or should not be done.

  7-C  measured ceiling -- the game never declares the group. Read 7-C for
       the proportion before quoting the raw bit count: it is 1.05% of
       blocks and costs nothing measurable today
  7-F  measured -- the parallelisable slice of the DECODE path is 3.4%, the
       rest is order-dependent. The process-level win was taken instead
       (11x). The three hot spots 7-F named were then optimized in 5-P:
       an export is 1.73x faster with all four Parquet files byte-identical.
       Decode is still strictly sequential; only the writers are concurrent
  7-H  CLOSED NOT SOLVABLE. The export-gap check that fixed cf97ecf was run
       and came back negative, and so did every other structural route: the
       class of a stably-named subobject is never on the wire. Five
       measurements in 7-H. Do not reopen without new input data --
       checkpoint chunks are the only unexamined region.

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
Every field inside a walkable block emits (group_path, handle, name, bit_count,
raw_bits), even when its type is unknown. Overlay is additive. An unresolved
ClassNetCache block cannot be walked and emits no Parquet rows; it returns Err
and its skipped bits are counted and named, never hidden behind a guessed
capacity. Keep the source `.vrf` if future reinterpretation may be needed.


---

## 1. What This Project Is

A from-scratch Rust VALORANT replay (.vrf) parser in a NEW repository
(C:\Users\yakihyuk0728\Documents\GitHub\vrfkit), built to replace the
C# parser (ValorantReplayParser, MIT) that the valplay Python analytics
pipeline depends on. The C# parser discards roughly 26% of content blocks
because it abandons any bunch whose payload has no registered descriptor.
vrfkit preserves every field it can walk, including raw bits for unknown
types; an unresolved ClassNetCache block is instead counted as a loud stream
failure and emits no field/RPC rows.

Primary outputs: fields.parquet, movement.parquet, actors.parquet,
net_guids.parquet, manifest.json -- all written by `vrfkit export`. A Python adapter
(tools/to_valplay_bundle.py) converts these into the bundle shape that
valplay's compute_metrics.py already consumes, so its 20 metric sections plus
the constant provenance `note` run unchanged on our data.

---

## 2. Repository State (2026-08-02)

```
verification : codex/needs-work in an isolated worktree; master was not moved
               or merged by the delegate
commits      : run `git rev-list --count HEAD`. No number is written here
               on purpose: the two that were had both gone stale, and this
               one would be wrong the moment the line was committed
tests        : 246 passing, 0 failed (243 regular + 3 doctests)
clippy       : 0 warnings (--all-targets -- -D warnings)
fmt          : clean (--check)
working tree : clean
valplay repo : 0 modified files (never written to; scripts run by absolute
               path, and compute_metrics.py is always pointed at a directory
               under vrfkit's out/ so it cannot write metrics.json into it)
ValorantReplayParser : clean, on branch local/vrfkit-descriptors at f67ea66.
               main untouched at 2d2e05e -- the commit the reference bundles
               were built from. Do not move it (QUICK START says why)
```

### Commit list

```
b10467b fix(tools): respect descriptor category overrides
a0ea2b4 chore: enforce ASCII Rust sources
b68baaa fix(decode): complete descriptor handle fallback
e1eb220 fix(decode): preserve explicit descriptor handles
fb41b96 test(tools): guard live effect decoder
14a9e93 test(export): prove the offloaded writers cannot fail silently
2012c51 perf(sink): lend the record buffers to the sink instead of rebuilding them
f70781a perf(sink, schema): memoise the RPC parameter group lookup
e08665b perf(export): move the fields and movement Parquet writers off the packet loop
a026a7f docs: task brief for delegating the three remaining items to Codex
bb21b82 docs: handoff brief for the three remaining tasks
8cc83a1 docs: state what 7-C actually costs, not just how many bits it is
c055ee5 docs: close out section 7 and record the session's corrected claims
ef9a521 Merge branch 'worktree-agent-a6abf41017a8780d8'
a1e9943 docs: record the throughput win 7-F's measurement pointed at
ae3b83f perf(tools): run corpus validation N replays at a time
ef6e0c2 docs: close 7-H as not solvable, with the five measurements that prove it
b299a86 Merge branch 'worktree-agent-ab447b5e87d427c27'
601a447 docs: name the right reordering hazard in 7-F
9ec24e7 docs: close 7-E and 7-I, and record the vacuous malformed counter
de0ca29 docs: close 7-F after measuring where the export time actually goes
9cb7a24 fix(tools): the corpus malformed counter was never actually read
6a73475 fix(tools): stop filtering out effects that carry no firing state
e0c5bd8 docs: record cross-validation, the movement defect, and three audited claims
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
    vrfkit          -- CLI: inspect / validate / export (2 tests; the driver is
                       otherwise covered by the regression guard. The two are
                       the writer-thread failure guards from 5-P, which the
                       regression guard cannot reach because it only exercises
                       the success path)
  tools/            -- Python generators and verification harnesses
```

Total: 246 tests, measured at b10467b (243 regular targets plus 3 doctests).
The earlier 242 figure omitted one existing test; Task B then added three.
DO NOT trust the per-crate rows above: they were
taken excluding doc-tests for some crates and including them for others, so
they do not sum to the total. Known wrong even before 5-P: vrf-frame is 5 not
3, vrf-export is 19 not 18 (0 unit + 17 integration + 2 doc). Re-measure per
crate before quoting any single row.

An earlier version of this paragraph said the previous breakdown "was wrong
for six of the ten crates even though its total happened to be right" -- the
replacement breakdown was wrong too, in the same way, which is why the rows
now carry this warning rather than a correction.

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
                                   MEASURED for the first time on 2026-08-01.
                                   validate_corpus.py matched on "Malformed:"
                                   while the oracle prints "Malformed
                                   framing:", and a non-matching pattern was
                                   silently skipped -- so this had always been
                                   a Counter default, not a reading. Fixed in
                                   9cb7a24; the value is genuinely 0, and a
                                   counter that stops printing now warns
                                   instead of reading as zero.
  unattributed bits: 1,972,080,670 (~246 MB; 97.283437% is
                     AbilitiesAndBuffsComponent)
                     That 97.283437% is a share of the FAILURES, not of the
                     replay. Per replay it is ~2.1% of bits and ~1.05% of
                     blocks, and no metric depends on it -- see 7-C.
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
typed (value_*)    : 374,143 rows with any value_* column set
                     30.162% of all 1,240,444 rows; 37.831% of the
                     988,979 rows offered to the overlay. The distinct
                     overlay decoded-ok counter is 363,478; do not conflate
                     that counter with non-null Parquet rows.
oracle pass rate   :  98.95%
```

13.02 replays (4 local demos, pinned by tools/check_corpus_baseline.py):
  all 4 parse, malformed 0, pass rate 97.959041% - 99.329325%
  blocks 3,117,920  fields 2,279,512  rpcs 1,713,576  skipped 74,573,628

Earlier measurement of two of them:
```
2a09e682  55 MB   686,559 blocks  malformed 0  transform 0  pass 97.96%
43d0f434  85 MB 1,004,465 blocks  malformed 0  transform 0  pass 99.18%
```
The C# parser that valplay currently uses REJECTS 13.02 replays outright.

Older supported builds (one machine-local fixture per build, pinned by
tools/check_corpus_baseline.py):

```
build  blocks  malformed  fields  RPCs   skipped  oracle pass rate
12.10  13,679          0   7,924  9,605   12,915       99.203158%
12.11   6,505          0   4,700  3,593   11,052       98.478094%
13.00   8,859          0   4,558  5,722   18,104       98.679309%
```

All three inspect and validate with exit 0. Full export also reaches exit 0 and
writes its output files, but it is not decode-clean: the builds report 9, 18,
and 19 FName SourceID decode errors respectively. Walkable rows retain raw bits;
unresolved ClassNetCache streams remain counted as skipped bits and emit no row.
These gaps are recorded, not hidden by the zero malformed count.

The adjacent 12.08 C# fixture is intentionally unsupported. A real end-to-end
`validate` run exits 1, names `++Ares-Core+release-12.08`, and lists the known
branches. This confirms that an unknown build fails loudly rather than silently
selecting a transform.


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

An uncapped corpus audit later measured 97.283437% of unattributed bits as
AbilitiesAndBuffsComponent, for which the replay declares no cache group.
No lookup can reach it; see the corrected breakdown in 7-C.

### 5-E. README correction (commit 7c2faa1)

README still claimed 100.000000% pass rate with 3,671 skipped bits.
Corrected to an honest non-100% range with 1,972,080,670 unattributed bits,
plus an explanation that framing is exact everywhere and the shortfall is
attribution rather than parsing. The final uncapped corpus measurement is
97.487010%-99.681958%, with median 99.323286%.

Also corrected: overlay figures (106 groups/929 fields -> 123/1054; the
table was 1,058 entries as of section 13-C/13-D and is now superseded by
section 14's generated 1,100-name/84-handle result),
RPC comparison (334,641 -> 342,735 vs C# 230,893), typed coverage.

First-ever measurement of 13.02 replays documented here: two local demos
parse with malformed framing 0 and transform failures 0.

During this work: PowerShell Get-Content -Raw read as cp949 then wrote
as UTF-8, corrupting all Korean text. Recovered with git checkout --
and re-applied edits using the write tool only.

### 5-F. valplay adapter (commit b6947ee)

tools/to_valplay_bundle.py: reads vrfkit export (fields.parquet,
movement.parquet, manifest.json) and writes a bundle that valplay's
compute_metrics.py consumes unchanged. Reusing the 20 validated metric
sections plus its constant provenance note is the point; reimplementing would
discard the validation.

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

### 5-O. Closing out section 7

7-E, 7-F, 7-H and 7-I were finished in parallel; 7-F and 7-H ran as isolated
worktree agents and both came back with a measured "do not do this", which is
the outcome that saves the most time.

  7-I  the 172 server-world effects are now emitted, not filtered.
       weapons became EXACT. Dropping them had hidden information valplay's
       "unknown" bucket and shots_without_equippable diagnostic exist to
       report -- a silent drop made at the adapter layer, where the parser's
       own invariants were not being applied.
  7-E  tools/check_corpus_baseline.py pins the four 13.02 demos and was
       proven to fail on a perturbed baseline before being trusted.
  7-F  closed by measurement: the transform it wanted to parallelise is 3.4%
       of an export, and the decode half is order-dependent because a
       block's group path -- and therefore its handle bit width -- depends on
       cache state mutated by earlier blocks. The process-level win it
       pointed at was taken instead: 11x on the corpus, zero risk.
  7-H  closed as not solvable from replay data. The class of a stably-named
       subobject is never transmitted; five independent measurements, and the
       cf97ecf export-gap check was run first and came back negative.

The malformed counter was the session's sharpest lesson. validate_corpus.py
matched on "Malformed:" while the oracle prints "Malformed framing:", and a
non-matching pattern was silently skipped -- so the figure quoted as the
primary evidence for exact framing had never been read. It is genuinely 0,
but for the whole project's history that was luck rather than knowledge.

Six claims were corrected this session by measuring something that had been
asserted: two premises (7-B, 7-D), two figures (combat.per_player's
tolerance, the per-crate test counts), one scope error (7-G's "only one
reference bundle" -- there were eleven), and one counter that was never read
at all.

### 5-P. Export path optimization (commits e08665b, f70781a, 2012c51, 14a9e93)

7-F ended by naming three places the time was: Parquet writing (37%),
`on_content_block` group-path resolution (12%) and `try_parse_rpc_params`
(10%). Two of the three were taken -- Parquet and the RPC lookup. Group-path
resolution was not, and is now the largest single slice; the breakdown at the
end of this section says where it went instead. The largest win after Parquet
turned out to be a fourth thing 7-F's table had folded into `process_packet`
and never named: `ExportSink` construction. The constraint throughout was
**bit-identical, order-identical output**, checked by SHA-256 of all four
Parquet files after every step -- all four are unchanged from the
pre-optimization baseline:

```
actors.parquet     F9D21B325B8C8F426CE758F000DBF3B5E412ABFE23CBCB862D8BCA522CA82CE5
fields.parquet     2DDC81D8C3EBB58931BF9C667D0C505A608F6F73C2CB097A461EB738E087B59A
movement.parquet   1242BBB15B29BE267BA4B0326BCBC508B5E2AC6C7CD8A1570035C335C04D9363
net_guids.parquet  501CABC678770431D0FEC9C37C4E21ED06193BB93263313959E87865625BBA0F
```

Result, interleaved A/B against the pre-change binary (alternating runs, so
machine drift cancels), in-process elapsed on 02d4d478:

```
              baseline           patched          speedup
export        2.840 s median     1.640 s median    1.73x
              2.760 s min        1.580 s min       1.75x
validate      1.580 s median     1.350 s median    1.17x
              1.520 s min        1.290 s min       1.18x
```

`validate` moves less because it never wrote Parquet; its 1.17x is the sink
work alone. The export-minus-validate gap -- which 7-F established IS the
Parquet write -- went from 1.26 s to 0.29 s. That is the offload measured
from the outside, and it agrees with 7-F's 1045 ms.

Three optimizations, each provably output-preserving rather than
tested-into-confidence, plus the guard that keeps the first one honest:

  **e08665b -- fields and movement writers moved off the packet loop.**
  Each is an independent file whose writer reads no replay state. They now
  run on their own threads, fed 16,384-row batches over a bounded channel.
  The writers still see every record exactly once in stream order and the
  row-group flush still falls on the same cumulative row counts, so the
  encoder input is identical. Batched rather than per packet because a
  replay is ~530 k packets carrying 0.8 field rows and 3.5 movement rows
  each; per-packet messages would cost more than the encoding they hide.
  std::thread + sync_channel, no new dependency.

  **f70781a -- the RPC parameter group lookup is memoised.**
  `find_rpc_param_group_path` fell back to scanning every declared export
  group with `ends_with(":<function>")`, once per RPC: 113,214 calls against
  475 groups. It is a pure function of (block group path, function name,
  set of declared group paths), and only the third can change mid-replay, so
  `NetGuidCache` gained a `schema_generation` counter bumped by
  `add_export_group` and `clear` -- the only operations that add or remove a
  path or alias. A memo stamped with it is exactly equivalent to
  recomputing. The counter deliberately does not track field mutations, and
  says so; only path-set queries may key on it.

  **2012c51 -- the record buffers are lent to the sink, not rebuilt.**
  `ExportSink` is constructed once per packet -- 530,401 times -- and
  allocated two `Vec::with_capacity(256)` each time. Instrumentation put
  construct-and-drop at 356 ms, larger than the whole movement decoder. A
  discriminating probe confirmed it was the allocation and not the timers:
  `Vec::new()` moved the slice to 66 ms while pushing 69 ms back into
  `process_packet` (the vectors then regrow every packet). The buffers now
  live in a caller-owned `RecordBuffers`; `ExportSink::new` clears them, so
  a caller that never drains them -- the oracle is one -- cannot accumulate.

  **14a9e93 -- the offloaded writers are proven unable to fail silently.**
  Threading moved the writers' errors off the `?` path. Both the error and
  the panic branch were driven deliberately and confirmed to fail for the
  right reason when the `match` on the join result is replaced with
  `let _ = join(); Ok(())`. Test count 236 -> 238; these two are the only
  additions.

**Tried and measured as not worth it.** Returning `Option<&str>` instead of
two `to_owned()` calls from `resolve_actor_package_and_archetype`, which runs
once per content block. Interleaved A/B: median 1.580 s vs 1.590 s, min
1.470 s vs 1.470 s -- no effect, and it was reverted. The remaining
allocation in that path is the `Vec<String>` from
`replay_path_lookup_keys` / `class_net_cache_lookup_keys`, up to four calls
of up to six strings per block; removing it needs a borrowing or callback API
in `vrf-schema`, which was not attempted.

Where the time is **now**, same method as 7-F (temporary `Instant` timers,
reverted before commit). Instrumented total 1.81 s against 1.64 s clean, so
~11% of timer overhead is spread across these rows:

```
  oodle decompress            165 ms
  DemoFrame iteration          22 ms
  phase 2 (packet loop)      1529 ms
    process_packet           1366 ms
      resolve_group_path      371 ms   <- now the single largest slice
      try_parse_rpc_params    220 ms
      movement decode         192 ms
      on_field total          214 ms
        apply_overlay          66 ms
        resolve_field_name     33 ms
        raw_bits copy          22 ms
        record push            22 ms
      resolve_function_count   19 ms
      residual (bunches, payload transform, framing)  ~350 ms
    sink construct + drop      38 ms   (was 356 ms)
    append to writer threads   89 ms
```

Parquet no longer appears as a slice: it is overlapped with the packet loop,
which is the whole point. The next target, if there is one, is
`resolve_group_path` at 371 ms over 608,011 blocks.

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
weapons          EXACT        after emitting the 172 server-world effects
                              the reference bins as "unknown" (7-I)
weapon_stats     OURS BETTER  by_weapon identical for all 23 weapons;
                              region_source, hp_tracking and
                              shots_without_equippable byte-identical.
                              Differs only on non_player_victim_hits,
                              212 vs 211 -- the damage record commit 6e6d544
                              recovers and the C# parser discards
---------------------------------------------------------------------------
```

EXACT: identical Python object equality. The harness prints 16 of 21, but
       one of those keys is `note` -- a fixed provenance string
       compute_metrics writes for any input, structurally incapable of
       failing. The honest figure is 15 of 20 real metric sections, and the
       same 15 on all 11 cross-validated replays (section 6-A).
       NOTE ALSO: the table above lists `combat` twice; it is one section.
OURS BETTER: our value is more complete/correct than the C# reference.
BLOCKED: the data is present but a named defect prevents it being used.
         No section is BLOCKED.

CORRECTION 2026-08-01. This block previously claimed "no section differs for
a reason that is not understood" and "every remaining difference is a case
where we carry data the C# parser does not". An audit refuted both. Three
fields exist where the REFERENCE is higher than us:

  2c9e88a0  tactical.clutch_attempts     ref 4   ours 1
  45758459  tactical.clutch_attempts     ref 7   ours 5
  500ce1a8  tactical.clutch_attempts     ref 6   ours 3
  500ce1a8  tactical.clutch_wins         ref 2   ours 1
  02d4d478  tactical.opening_duels_won   ref 11  ours 10
            (with opening_duels_played conserved at 18)

opening_duels_won is a strict subset of opening_duels_played, and the
denominator is conserved -- so that one is a disagreement about a single
duel's OUTCOME, not a data-volume difference. These are derived from the kill
timeline, whose derivation is not monotonic in kill count, so carrying 13
extra kills COULD lower a clutch count. No mechanism has been established.
Treat this as an open question, not as understood.

kast survived the same check cleanly: zero reference-higher values on any
replay.

Scoreboard metrics that Tracker.gg validated (K/D/A, ADR, HS%, KAST,
FK/FD, MK, rank): reproduced exactly for all 10 players from vrfkit data.



### 6-A. Cross-validation across every available reference bundle

Section 6 used to rest on 02d4d478 alone. Eleven replays have BOTH a source
.vrf and a C# reference metrics.json -- the claim in 7-G that only fd816a35
was cross-validated was wrong; fd816a35 is simply the one whose .vrf is
missing.

    python tools\validate_metrics_corpus.py --jobs 3

runs the full pipeline over all eleven and prints a section x replay matrix.

Result (2026-08-01): the harness reports **16 of 21 keys byte-identical on all
11 replays**. One is the constant provenance `note`, so 15 of 20 real metric
sections are exact across all 11.

  ability_detail  ability_usage  economy      movement_detail
  movement_summary  objective    objective_detail  players
  posture         rounds         shot_rays    side_winrate
  spray_control   ultimate       weapons      (+ note)

The five that vary are combat, economy_detail, kast, tactical, and weapon_stats.
Most differences align with data we recover and the C# parser drops, but five
tactical values are reference-higher and their mechanism is not established;
the correction and exact values above supersede the earlier direction claim.

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

### 7-C. Unattributed ClassNetCache blocks [MEASURED; PRODUCTION DECISION PENDING]

Read the proportion before the raw number, because the raw number misleads.
"1,972,080,670 bits" and the old "91.7% AbilitiesAndBuffsComponent" figure
both sound alarming and have been quoted that way in this document; measured
against what we DO read, on 02d4d478:

    named and decoded   822,744,224 bits   97.9%
    unattributed         17,507,210 bits    2.1%
    blocks failed             6,365 of 608,020   1.05%

The old 91.7% was intended as a share OF THE FAILURES, not of the replay;
the uncapped current measurement is 97.283437%, reported below. Either way,
the replay-level proportion is roughly one block in a hundred.

WHAT IT COSTS TODAY: nothing measurable in the current 20 real metric sections.
Fifteen are byte-identical to the C# reference on 11 replays; the five varying
sections do not consume this missing ability-state stream. Their tactical
direction discrepancy is a separate open question. No current consumer asks
for this data, and the C# parser cannot read it either.

Ability behaviour is already covered through other groups (30,493 field rows
on 02d4d478: Wraith smoke zones, Smonk smoke, melee, the ability statistics
replicator, Hunter bolts, ...), which is why ability_usage and ability_detail
are both EXACT.

WHAT IT WOULD ADD: this component carries ability and buff/debuff STATE --
charge counts over time, who was blinded or slowed and for how long, ult
gauge between casts, heal and shield application. That is the difference
between "this ability was used" (which we have) and "this ability affected
these players for this long" (which we do not). Interesting for coaching or
pro analysis; irrelevant to replacing the C# parser.

LOST, NOT MERELY UNNAMED. This section previously claimed "the raw bits are
written to Parquet regardless ... replays already archived can be
reinterpreted". That is FALSE and was never checked.

`crates/vrf-net/src/field.rs:119` returns `Err` when `function_count == 0`
BEFORE reading any bits. The caller only invokes `on_stream_failure`, which
pushes a diagnostic string capped at 32 lines. No `on_field` or `on_rpc`
fires, so no Parquet row is written. Measured: AbilitiesAndBuffsComponent
accounts for 17,264,706 skipped bits on 02d4d478 but only 4,960 bits / 160
rows in fields.parquet -- and all 6,365 failures are RPC-kind, so those 160
rows are its RepLayout property path, which parses normally.

Consequence: an archived Parquet export CANNOT be reinterpreted if a future
build declares the group. You would have to re-run against the .vrf. Keep
the source replays.

This also qualifies NO SKIP PATH as written in section 8. "Every field emits
(group_path, handle, name, bit_count, raw_bits) even when nothing is known"
holds for fields inside a walkable block. A ClassNetCache block whose group
cannot be resolved emits nothing at all -- it is counted and named in the
oracle, which is the honest part, but its bits are not preserved.

These blocks frame correctly (malformed framing 0); the group resolution
returns function_count=0 and they are counted as failures.

PRESERVATION COST, measured 2026-08-02 before any production change. A
temporary instrumented build wrote each unresolved whole-block payload as one
sentinel row; it never fabricated a per-field split. Each replay used one
warmup and 10 interleaved OFF/ON pairs with the same instrumented release
binary. Clean HEAD and instrumented OFF were byte-identical for all four
Parquet files, and 14,755 sentinel rows were audited in order from the parser
boundary through Parquet, including exact metadata, bit counts, raw bytes,
byte lengths, and zero high padding bits.

```text
replay    rows added   fields.parquet ZSTD bytes added   paired timing result
08aec1e1          928                            37,136   not measurable
02d4d478        6,365                           229,274   not measurable
252168ae        7,462                           245,938   not measurable
```

The median paired deltas were -6.4944 ms, -0.1204 ms, and +19.3544 ms;
fixed-seed bootstrap 95% confidence intervals all included zero. Existing
parser/overlay counters and every non-fields Parquet file were invariant.

This measurement is NOT a production implementation. No source change or
baseline update from Task C was committed. If preservation is approved later,
the adapter must explicitly ignore the sentinel row before metrics processing,
and `skipped_bits` must keep its current meaning of "not parsed". On the
post-Task-B 02d4d478 export, the expected production-only change is fields
rows 1,240,444 -> 1,246,809 and bytes 13,255,044 -> 13,484,318. Do not mix
those forecast values into the current baseline before that decision.

CORRECTED 2026-08-01: the previous breakdown was not derivable from a committed
tool. MAX_STREAM_FAILURE_RECORDS capped diagnostics at 32 lines, and the quoted
percentages had been inferred from that truncated sample. A temporary uncapped
aggregation of all 1,047,182 stream failures across the 215-replay corpus
accounts for every one of the 1,972,080,670 skipped bits and measures:

  97.283437%  AbilitiesAndBuffsComponent  (1,918,507,857 bits / 752,483 blocks)
   1.545398%  PatchVolume                  (   30,476,488 bits /   3,432 blocks)
   0.319715%  AttachedDamageSection        (    6,305,042 bits /  99,002 blocks)
   0.224846%  DefenderAnnouncer            (    4,434,144 bits /  10,868 blocks)
   0.181710%  AttackerAnnouncer            (    3,583,464 bits /   8,783 blocks)
   0.160508%  MapTargetingState            (    3,165,345 bits /  23,632 blocks)

On 02d4d478, AbilitiesAndBuffs is 98.61%, PatchVolume is 0.66%, and
RespawningWallPlate2_7 is 0.02% of skipped bits. The previously quoted 91.7%
was not current. MeleeAttackState is absent from the failure set on both scopes:
0 blocks and 0 bits corpus-wide; its already-resolved path emits 473 field rows
on 02d4d478. There is nothing there to recover.

If this breakdown needs to become routinely reproducible, first add a committed
uncapped aggregation mode rather than drawing conclusions from the 32-line
diagnostic. The wrong breakdown survived precisely because that mode is absent.

AbilitiesAndBuffsComponent is the real ceiling. Until the game server
declares its ClassNetCache group in the schema, no lookup can reach it.
This may change in a future build.

CONFIRMED 2026-08-01: searched all 475 declared export groups in
02d4d478's manifest -- zero contain the substring "AbilitiesAndBuffs".
This is now a measured fact rather than an assumption. Do not spend
time trying to recover those bits.

MeleeAttackState1/2/3/4/_Alt were already recovered by the schema-driven
instance-name resolver in commit 6e6d544. The replay declares exactly one
ClassNetCache for all five names, not five distinct function tables:

  /Script/ShooterGame.MeleeAttackStateComponent_ClassNetCache
    num_exports = 2; slot 0 empty; slot 1 = MulticastHitImpact

On 02d4d478, 467 non-empty blocks parse through that group: State1 201,
State2 45, State3 32, State4 23, and _Alt 166. They carry 211,441 content
bits in total (91,452 / 20,159 / 14,397 / 10,069 / 75,364 respectively),
of which 187,995 bits are RPC parameter payload. All 54 instance GUIDs emit
successfully resolved rows. The 475-group manifest contains only the shared
ClassNetCache and its MulticastHitImpact parameter group; variant-specific
ClassNetCache groups declared by the replay: zero.

The numeric names reach the shared group by the existing trailing-digit
fallback followed by the replay-declared `Component_ClassNetCache` candidate;
`_Alt` reaches it through underscore-segment stripping. No hardcoded name or
new lookup rule is needed. Corpus skipped bits before and after this audit are
identical at 1,972,080,670 because there is no parser change to make.

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

### 7-E. 13.02 regression guard [DONE 2026-08-01]

Done in commit 9cb7a24. tools/check_corpus_baseline.py pins per-file and
total oracle figures in JSON and fails on any difference:

    python tools\check_corpus_baseline.py --baseline tools\baselines\build_1302.json

tools/baselines/build_1302.json covers the four local 13.02 demos --
blocks 3,117,920, fields 2,279,512, rpcs 1,713,576, malformed 0,
skipped 74,573,628, pass rate 97.96-99.33%.

Those replays live under %LOCALAPPDATA% and are machine-local, so an absent
corpus SKIPs with a message rather than failing. A guard that fails on
someone else's machine gets disabled, and a disabled guard protects nothing.
The guard was proven to fail: perturbing the baseline produced the expected
DRIFT lines and exit 1.

This work also uncovered a worse problem -- see the malformed-counter note
in section 4.

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
risk. The counter totals would survive reordering -- `skipped_bits` is a
u64 sum and addition commutes -- but the ordered records would not: the
`DiagnosticEvent` vector behind `validate --diagnostics`, and which 32
lines survive the first-32-wins stream-failure cap. Both are load-bearing
under NO SILENT SUCCESS. Do not reopen without new measurements.

Where the time actually is, if a future session wants throughput: Parquet
writing (37%), `on_content_block` group-path resolution (12%), and
`try_parse_rpc_params` (10%).

**Two of those three were actioned in 5-P** -- Parquet writing and
`try_parse_rpc_params`. Group-path resolution was not, and is now the largest
single slice. An export is 1.73x faster, byte-identically. 7-F itself stays
CLOSED: nothing in 5-P made the decode half concurrent. Read 5-P before
treating the table above as current -- it is the pre-5-P breakdown.

The process-level win was taken (commit ae3b83f). validate_corpus.py ran the
215 replays as 215 *sequential* subprocesses; each already owns its own
output and shares nothing, so running them N-wide is near-linear with no
bit-identity risk at all:

  325.4s -> 29.4s, an 11x speedup, every number byte-identical
  (blocks 136,545,822, fields 98,883,979, rpcs 75,571,092, malformed 0,
   skipped 1,972,080,670, pass rate min/median/max unchanged, 215/215)

Default workers are cores-2 capped at 16; set VRFKIT_JOBS to override.

### 7-G. Reproduce metrics.json for other replays [DONE 2026-08-01]

Done in commit 38ca3fe. This section claimed the Tracker.gg cross-validation
replay fd816a35 had no .vrf and implied it was the only reference bundle.
Eleven others have both a bundle and a .vrf; only fd816a35 is missing its
source. tools/validate_metrics_corpus.py now runs all eleven -- see 6-A.

### 7-H. Instance-named component groups [NOT SOLVABLE FROM REPLAY DATA]

Several component groups arrive under an actor instance name and never reach
their declared class group, so their fields stay unnamed. The bits are
captured -- no-skip-path holds -- but no field_name is attached. 33,529 of
429,633 field rows in 02d4d478 (7.8%) are affected; the export's
"No field name" counter is the headline number for this gap.

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

INVESTIGATED AND CLOSED 2026-08-01. The premise stated here previously --
"resolving it needs structure the replay provides, most likely the subobject's
outer chain in guid_to_outer, leading to the owning actor's class" -- was
disproved by measurement, the same way 7-A's premise was. The owning actor's
class is the CHARACTER class (Terra_PC_C), not the component's class
(AresInventory); the outer chain cannot produce the latter.

Root cause, from crates/vrf-net/src/content.rs: `read_content_block_header`
returns as soon as `is_stably_named` is set, BEFORE `classNetGuid` is read.
A default subobject is name-stable, so its class is never transmitted. Unreal's
own receiver recovers it by resolving the name inside the already-spawned
outer actor and calling `Object->GetClass()` -- that is asset data (the owning
class's CDO), not replay data.

Five measurements, all on 02d4d478 unless noted:

  1. HEADER BITS. Instrumented `on_content_block` and dumped every subobject
     block. All of them are `is_stably_named = true, class_net_guid = 0`:
     InventoryComponent 5342 blocks, MagazineAmmo 3642, CalloutRegionTracker
     1837, ReserveAmmo 1112, VisionComponent 680, AresAttributeSet_2 76,
     ZoomStateMachine 4255 RepLayout + 70 ClassNetCache. Not one block for
     these objects ever carried a class GUID.

  2. NO EXPORT GAP (the cf97ecf check, run and negative). Grouped every
     `object_net_guid` in fields.parquet by the set of group_paths it appears
     under. ZERO object GUIDs appear under BOTH an unnamed instance-name path
     and a resolved class path. There is no earlier class-bearing block to
     memoize from, so a per-object `object_net_guid -> group` cache -- which
     would go beyond C#, whose cache is keyed on ClassNetGuid only -- has
     nothing to learn from.

  3. OUTER CHAIN TERMINATES. GUID 582 = "InventoryComponent", outer 576; GUID
     576 is a dynamic actor GUID with no path and no outer. Same shape for all
     40 InventoryComponent GUIDs. The chain ends at a pathless actor.

  4. NO CLASS GUID EXISTS. Only one GUID carries a literal /Script path --
     GUID 15 = "/Script/ShooterGame" -- because UE exports an object's leaf
     name plus its outer GUID, not its full path, so class objects appear as
     bare leaves parented to it. GUID 15 has exactly 19 children --
     DefaultPlayspace, Default__OwnerExclusivePlayerInfo,
     RoundBasedAFKDetectionComponent, AresAttributeSet, ItemSlot,
     MultiItemSlot, ShooterCharacterHitRegDebugComponent,
     AutoEquipTransitionContext, NetworkedRandomNumberGeneratorComponent,
     PurchasedItemComponent, AresEquippableDataTracker, GameStateHUDConfig,
     AbilityTrackingDelegateComponent, TeamRoleComponent,
     Default__FootstepsComponent, AnimTriggeredStateContinueTransitionContext,
     ActorListTransitionContext, Default__FiringStateComponent,
     TransformTransitionContext. AresInventory is not among them. A class
     object is assigned a NetGUID only when it is referenced on the wire, and
     these classes never are.

  5. NETWORK CHECKSUM IS ABSENT. `internal_load_object` reads and discards a
     `NetworkChecksum` when `ExportFlags` bit 2 is set. That checksum is
     `GetClassNetworkChecksum(Obj->GetClass())` and would be an exact,
     replay-declared class token. It is never sent: across 16,648 GUID export
     records in 02d4d478 the flags histogram is {1: 3440, 3: 13208} -- only
     HasPath and HasPath|NoLoad, bit 2 never set, 0 checksums. Confirmed on a
     second replay (03c60af4): 6,857 records, {1: 2696, 3: 4161}, 0 checksums.
     Even if present it would need a (checksum -> class path) pairing, and
     measurement 4 shows the only source of such pairings does not contain
     AresInventory.

Two near-misses, both recorded so they are not re-litigated:

  HANDLE-RANGE COINCIDENCE -- NOT A MECHANISM. The declared group
  /Script/ShooterGame.AresInventory has max populated handle 31 and
  InventoryComponent's unnamed rows have max handle 31;
  /Script/ShooterGame.AresAttributeSet has max populated handle 285 and
  AresAttributeSet_2 has max handle 285. Consistency is not a declaration.
  Turning it into a rule means searching 475 groups for one whose handle set
  is a superset of the observed handles -- that is guessing a group to make a
  number look better, which NO SILENT SUCCESS exists to forbid. Worse, on the
  RepLayout path a wrong group has no failure signal (see below), so the
  corruption would be silent.

  CNC-DERIVED REPLAYOUT PATH -- MEASURED AND DECLINED. Running
  `resolve_cnc_for_instance_name` on the unnamed RepLayout paths and stripping
  `_ClassNetCache` from any hit recovers 156 of the 33,529 rows (0.47%), and
  they are Switch_BlackMarket_2 (103) and WindowShieldA1 (53) -- static
  ACTORS, 7-C territory, not 7-H components. It resolves none of the ten
  offenders above. Declined on top of the low yield because of an asymmetry
  that matters generally: on the ClassNetCache path a wrong group desyncs
  loudly through function_count, but RepLayout handles are IntPacked and
  independent of group capacity, so a wrong group there silently mislabels
  fields and nothing fails.

INSTANCE NAME -> CLASS IS MANY-TO-ONE, so no string rule can be correct even
in principle. Demonstrated from replay data alone, on groups that DO resolve:
/Script/ShooterGame.MeleeAttackStateComponent_ClassNetCache is reached from
five distinct object names (MeleeAttackState1/2/3/4/_Alt);
GrenadeExplodeIndicator_C_ClassNetCache from three; and
DamageableComponent_ClassNetCache from two ("Damageable" and
"DamageHandlerComponent") -- the second only because it is one of the four
entries in KNOWN_SUBOBJECT_CLASS_PATHS, i.e. hardcoded. MagazineAmmo and
ReserveAmmo are likewise two names for what the declared schema offers only
one candidate class for (/Script/ShooterGame.AmmoComponent).

The C# reference does not solve this either: ResolveSubobjectClassPath returns
null when ClassNetGuid is invalid, except for its 4-entry
KnownSubobjectClassPaths dictionary. That dictionary is the hardcoding this
project's invariant forbids, and vrfkit mirrors it only to preserve parity.

SCOPE OF THE CLAIM. All of the above is measured over ReplayData chunks, which
is everything vrfkit ingests (driver.rs skips every chunk whose type is not
ReplayData). 02d4d478 also has 18 Checkpoint chunks that are never parsed.
UDemoNetDriver::SerializeGuidCache writes (NetGUID, OuterGUID, PathName,
NetworkChecksum) for every ObjectLookup entry there, so a checkpoint reader is
the ONLY unexamined place a class token could still live. It is unlikely to
help -- ObjectLookup holds only GUIDs that were actually assigned, and
measurement 4 shows these classes were never referenced, so they were never
assigned one -- but it is the one honest caveat on "the replay does not
declare it".

7-A does not depend on this, and no metric section does: the affected fields
are ammo-level detail in weapon_stats and posture / fire-mode refinement, none
of which currently feed a section that is not already exact.

### 7-I. Effects with no firing state [DONE 2026-08-01]

Resolved in commit 6a73475 by emitting them, which made `weapons` EXACT.

The adapter had filtered out any effect RPC without
FiringState.FiringPlayerState -- 172 of 02d4d478's 2,647 -- plus 7 more that
carried no blob at all. The 172 are server-world effects
(source_id = DedicatedServerWorldSourceID) with no player, weapon or attack
vectors, so dropping them looked like the clean choice.

It was the wrong one. valplay's weapons section has an "unknown" bucket and
weapon_stats has a shots_without_equippable diagnostic, both built precisely
to receive these. Filtering them hid information the consumer was designed to
report -- the same silent-drop mistake the parser invariants exist to
prevent, made at the adapter layer where those invariants were not being
applied.

Nothing downstream is distorted: every section that would be already guards
on firing_player_state or attack_vectors, and spray_control, posture,
shot_rays and movement_* are unchanged and still EXACT.

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
  Every field inside a walkable block emits (group_path, handle, name,
  bit_count, raw_bits) even when its type is unknown. Overlay is additive:
  typed values fill value_* columns; decode failure leaves them null with raw
  bits intact. An unresolved ClassNetCache block cannot be walked, emits no
  row, and must remain a loud counted failure; retain the source `.vrf`.
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

A STABLY-NAMED SUBOBJECT'S CLASS IS NOT ON THE WIRE
  read_content_block_header returns as soon as is_stably_named is set, before
  classNetGuid is read, so default subobjects (InventoryComponent,
  MagazineAmmo, ZoomStateMachine, ...) never declare their class. Unreal
  recovers it by resolving the name inside the already-spawned outer actor and
  reading Object->GetClass() -- asset data, not replay data. No amount of
  outer-chain walking, leaf matching, or checksum recovery substitutes for it
  (7-H documents all five routes and why each fails). Treat "this component's
  fields are unnamed" as expected, not as a bug to be closed by a name rule.

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
  constraint for the diagnostics path. (Confirmed 2026-08-02: `chcp` on this
  machine reports codepage 949.)

  Before Task D, a complete inventory found 61 tracked Rust files, with 44
  files / 510 physical lines / 8,984 non-ASCII Unicode scalars across 28 code
  points. The inventory scans complete file contents, including comments,
  doc comments, literals, BOMs, and malformed encoding bytes.

  `python tools/check_ascii.py --check` now enumerates tracked `*.rs` files
  with `git ls-files -z`, scans their raw bytes, and rejects every byte above
  0x7f. It reports stable file/line/column/byte diagnostics and fails loudly
  if Git enumeration or a read fails. The post-cleanup inventory is 61 files,
  0 affected files, 0 affected lines, and 0 non-ASCII bytes.

  This guard was observed failing twice: first on the real 510-line tree, then
  after planting `o` with an umlaut in a tracked Rust file (1 line / 2 UTF-8
  bytes, exit 1). The exact original SHA-256 was restored and the default
  scan returned exit 0. A separate `--self-test` exercises the same scanner.

  ASCII `??` corruption is outside the guard's detection domain. Task D also
  restored the damaged vrf-net field/lib/pipeline docs and vrfkit sink mapping
  comments from Git history, removed three BOMs, and verified zero remaining
  literal `??` sites in the four affected files.

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
  validate_metrics_corpus.py -- metrics.json parity across all 11 replays
                               that have a C# reference bundle
  check_corpus_baseline.py  -- pins the VALIDATE path per build
  check_export_baseline.py  -- pins the EXPORT path (counters + Parquet
                               shape) and cross-checks the printed row
                               counts against the files they name
  check_effect_decoder.py  -- 12-case guard for the live Python shot-effect
                               decoder, including two C# bundle cases
  check_ascii.py           -- complete tracked-Rust raw-byte ASCII guard
  find_skips.py             -- finds which replays still have skipped bits
  to_valplay_bundle.py      -- vrfkit Parquet -> valplay bundle adapter
                               ALSO holds the live shot-effect blob decoder.
                               crates/vrf-decode/src/effect.rs is a Rust
                               implementation of the same format that nothing
                               calls, with a different failure contract; see
                               its module docs before assuming they are
                               interchangeable.

### Path references
  Parser repo   : C:\Users\yakihyuk0728\Documents\GitHub\vrfkit
  C# reference  : C:\Users\yakihyuk0728\Documents\GitHub\ValorantReplayParser
                  Instrumentation: only in clean files, always reverted.

                  TABLE.RS DEPENDS ON A BRANCH THERE, NOT ON origin/main.
                  Generating from origin/main yields 680 overlay entries;
                  from local main 666; the current committed table has 1,100
                  name entries plus 84 explicit-handle aliases.
                  The difference is the descriptor work on
                  branch `local/vrfkit-descriptors` (fe5343a, 2026-08-02):
                  weapons, ItemSlot, PurchasedItemComponent,
                  OwnerExclusivePlayerInfo, EquippablePickup, TimedBomb
                  and the effect manager. Credits, purchases,
                  inventory-slot identity and shot effect data all rest
                  on it.

                  Check out that branch before running
                  extract_descriptors.py. Generating from main and
                  shipping the result would silently cut typed coverage
                  by a third -- apply_type_corrections.py now fails
                  loudly if that happens, which is how this was found.

                  That work was uncommitted until 2026-08-02, so the
                  table was reproducible on one machine only. A backup
                  of the pre-commit state is under
                  Documents/vrp-uncommitted-backup/20260802-011146.
  valplay       : C:\Users\yakihyuk0728\Documents\GitHub\valplay
                  Never modify.
  Corpus        : valplay\data\raw\vrf  (215 x .vrf, all 13.01)
  C# ref output : valplay\pipeline\exports\02d4d478-...\
                  SLIMMED: 97% of rpc_received removed, several keys stripped
  Local 13.02   : %LOCALAPPDATA%\VALORANT\Saved\Demos\*.vrf
                  Game-owned rotating input; currently 3 files, not a baseline.

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
The 20 real metric sections (plus a constant provenance note) were validated
against Tracker.gg scoreboard data for 10 players. Rewriting them would discard
that validation. The adapter
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

### No parallel DECODE within a replay (measured, closed)
The decode pipeline is sequential within a replay and stays that way. This
entry used to name the blocker as atomic oracle counters; that was wrong, and
it made the problem sound mechanical. The real blocker is that a content
block's resolved group path -- and therefore its `function_count` and the
bit width of its handle read -- depends on cache and channel state mutated
by earlier blocks in the same phase-2 walk. Only the payload transform is
pure, and it is 3.4% of an export (97 ms of 2.83 s instrumented). The
tradeoff is now a measured one rather than a deferral: the gain is capped
below what a rayon dependency and the reordering risk cost. See 7-F for
the full per-slice breakdown and for the process-level alternative that
does pay off.

Be precise about what 5-P did and did not change. It put the `fields` and
`movement` **Parquet writers** on their own threads. It did not make anything
in the decode path concurrent: `process_packet`, the sink, the `NetGuidCache`
and `ChannelState` all still run strictly sequentially on the main thread, in
stream order. 7-F's hazard is therefore not triggered -- the `DiagnosticEvent`
vector and the first-32-wins `stream_failures` cap are produced by the same
single-threaded walk in the same order as before, and the writers receive
records in the order the walk emits them. What is concurrent is only the
encoding of records that have already been decided.

### Blob decoders in sink.rs vs vrf-decode
The struct blob decoders (RoundResults etc.) are wired in sink.rs rather
than as a layer in vrf-decode, because they need access to the resolved
group path to know which blob format to apply. A cleaner architecture would
pass the group path through to vrf-decode, but that would require changing
the decode trait signature. Current approach works; refactoring is optional.

---

## 11. Delegate Coverage Audit (2026-08-01)

This audit addressed the two live input-coverage questions in CODEX_TASK_BRIEF.md
and independently confirmed why its original resolver task was withdrawn.
Search and measurements were read-only except for copying three fixtures into
vrfkit-owned machine-local baseline directories and adding their generated JSON
baselines. The dirty C# reference repository and valplay were not modified.

### 11-A. Non-Bomb mode coverage [INPUT-BLOCKED, UNMEASURED]

Recursive searches of all three scopes below found the same four physical
replays and no additional `.vrf` files:

```
%LOCALAPPDATA%\VALORANT\Saved\Demos   4
%LOCALAPPDATA%\VALORANT\Saved         4
%LOCALAPPDATA%\VALORANT               4
```

All four are 13.02, inspect/export/validate successfully, have malformed,
transform, and field-stream failures of zero, and exactly reproduce the pinned
build_1302 totals in section 4. Their runtime schemas and emitted replay events
contain BombGameState, BombPlayerState, BombDestination, TimedBomb, and spike
plant/defuse/explosion evidence.

That is positive evidence for Bomb mechanics, not a reliable official playlist
label. The replay header's `game_specific_data` contains serializedVersion and
playerLoadouts but no mode, queue, or playlist key, and modes such as Spike Rush,
Swiftplay, or Premier may reuse Bomb assets. The CLI has no independent game-mode
detector. Therefore the defensible inventory is the task brief's four Bomb-labelled
inputs and **zero mode-labelled non-Bomb inputs**.

No non-Bomb baseline was created and no claim about non-Bomb parsing is made.
To close this item, supply at least one replay per desired non-Bomb mode together
with a trustworthy external mode label; then run inspect, validate, full export,
and a mode-specific baseline on those inputs.

### 11-B. Older supported builds [DONE]

A wider search found one unique source fixture for every previously unmeasured
supported build under the read-only C# integration-test directory:

| Build | Source filename | Bytes | SHA-256 |
|---|---|---:|---|
| 12.10 | `9f8b32c5-c243-41ec-bbbb-832582edf652.12_10.vrf` | 525,616 | `A4CE1B72F9BDF99492162013C1C909E6994A0D22BEF1899E687FDE71FBC86606` |
| 12.11 | `5c673443-5bdc-4576-b416-aab3f62471a5.12_11.vrf` | 410,628 | `7A7A5492DDF286BB04413DA96F0D3B216F91150E8174A3A4397493529E17EBDD` |
| 13.00 | `12974d2b-848f-490d-80ba-5f03a033c2d5.13_00.vrf` | 431,908 | `FD49091DD43171BB060EB6BBAE50ED6677AA1077344572C5BF65F0C6FE2B4C1A` |

The search covered valplay data, Documents, Downloads, Desktop, VALORANT Saved,
and 34 user-profile directories named archive/archives/backup/backups, including
archive member listings without extraction. It enumerated 236 physical `.vrf`
files, 226 unique SHA-256 values; all 236 inspect successfully. One directory,
`%LOCALAPPDATA%\Temp\WinSAT`, was inaccessible. The 215-file valplay corpus is
entirely 13.01; the four Saved demos are 13.02; the Downloads replay duplicates a
13.01 valplay input.

One hash-verified copy of each old fixture now lives under:

```
%LOCALAPPDATA%\vrfkit\baseline-corpora\build_1210
%LOCALAPPDATA%\vrfkit\baseline-corpora\build_1211
%LOCALAPPDATA%\vrfkit\baseline-corpora\build_1300
```

Commit 8f7375e adds `tools/baselines/build_1210.json`, `build_1211.json`, and
`build_1300.json`. Each positive guard passes 1/1. Each guard was also pointed at
the wrong build corpus and observed to report seven DRIFT differences with exit
1, proving that the guards detect change rather than merely run.

The nearby real 12.08 fixture provides the unknown-build negative case. Unit
tests already cover the selector and ReplicationReader constructor; the real CLI
run additionally proves the process boundary rejects it loudly with exit 1 and
the unsupported branch name. No fallback transform is selected.

### 11-C. MeleeAttackState resolver premise [WITHDRAWN; CONFIRMED FALSE]

The proposed missing resolver work was already implemented. All five instance
names reach the one replay-declared shared ClassNetCache, all measured rows emit,
and an uncapped 215-replay failure aggregation contains zero MeleeAttackState
blocks and zero MeleeAttackState bits. No parser rule or hardcoded name was added.

Section 7-C contains the resolver path, exact per-variant counts and bit totals,
and the corrected 97.283437% failure-share measurement. Commit 458f8e0 records
the corrected documentation and the clarified function-count comments; total
skipped bits remain exactly 1,972,080,670 before and after the audit.

---

## 12. Code Audit Fixes (2026-08-02)

Four findings from a read-only audit of the Rust crates, plus the ASCII
sweep. No export figure moved: all four Parquet files for 02d4d478 hash
identically before and after the whole series, on a clean re-export, and the
corpus totals are exact.

### 12-A. Non-finite frame times [FIXED, commit e83f99f]

`vrf-frame` converted `timeSeconds` with `(f64::from(t) * 1000.0).round() as
u32` and a comment asserting the cast saturates so non-finite input "yields 0
as the reference does". Measured: NaN -> 0, -inf -> 0, **+inf -> 4294967295**.
ReplayEventJsonWriter.cs:194 has an explicit `float.IsFinite(seconds)` guard,
which is now written out here. `time_seconds` is a raw `read_f32` with no
validation, so any bit pattern is representable; one +inf frame would have
stamped 4294967295 ms on every packet in it.

Another comment on `DemoPacket::time_ms` still said "truncated", from before
7-B changed it to round. Corrected in the same commit.

### 12-B. object_net_guid filtered to None [FIXED, commit a2b8343]

The sink recorded a subobject GUID as
`Some(header.object_net_guid.0).filter(|&g| g != 0)`. The reference reads the
field unconditionally (ContentBlockFramer.cs:436-437) and branches on
`!header.ObjectNetGuid.IsValid` (ContentBlockPathResolver.cs:100), so it
treats the invalid GUID as reachable. Folding it to `None` did not discard a
zero -- `None` means "actor block" downstream, the adapter substitutes the
actor GUID, and the block collapsed onto the actor. That is exactly the merge
cf97ecf existed to undo, and it contradicted `FieldRecord`'s own doc comment.

The case does not occur on 02d4d478 (all four hashes unchanged), and it
cannot move any corpus counter: the change only ever replaces `None` with
`Some(0)`, and blocks/fields/rpcs/malformed/skipped are counts.

### 12-C. NetGUID row count unguarded [FIXED, commit bfd0229]

See the regression-guard block in QUICK START. `check_export_baseline.py` and
`tools/baselines/export_02d4d478.json` are new.

### 12-D. vrf-decode/src/effect.rs is dead code [KEPT WITH A NOTE, commit a28072b]

Nothing in Rust calls it; the live decoder is a Python port in
`tools/to_valplay_bundle.py`. Not wired in, because the two have opposite
failure contracts (Rust returns `Err` on a malformed blob and discards the
array; Python breaks and returns a partial list), the consumer reads Parquet
so wiring it in means a schema change, and the Python path currently matches
the reference on all 2,647 shots. Not deleted, because its nine executable
examples remain a useful independent Rust specification: six non-empty blobs
and three empty-array cases.

`tools/check_effect_decoder.py --check` now exercises the live Python path
with all nine Rust examples, two cases whose expected values come from the C#
reference bundle, and one malformed-input case that pins Python's partial-list
contract. The 12-case guard was observed failing after deliberate byte
corruption (exit 1) and passing after restoration (exit 0).

### 12-E. Non-ASCII in string literals [FIXED, commits e8f40cb and the cli.rs follow-up]

27 in total, of which 22 were the whole of `print_diagnostic_event`. The
27th was the CLI's USAGE banner, which a per-line literal scan cannot see and
which truncates on every no-argument invocation. Detail, the enforcement
scope, and the scan that actually works are in section 8.

The audit's line numbers for `vrf-container/tests/corpus.rs` (91, 108) were
wrong; the glyphs are on 112 and 129.

---

## 13. Data-Loss Fixes (2026-08-02)

Five places where a value the wire carries, and the parse recovers, was lost,
mangled or invented on the way out. None of them was a parsing failure -- every
one was a serialization or lookup decision downstream of a correct decode,
which is why the corpus totals never moved and no counter ever complained.

### 13-A. A cleared optional bit means "default", not "absent" [FIXED, 2637808]

`ArchiveVectorReaders.ReadOptionalQuantizedVector` returns `defaultVector` when
the leading bit is clear -- `(0,0,0)` for spawn location and velocity, `(1,1,1)`
for scale (`NewActorSerializer.cs:56-72`). vrfkit returned `None`, collapsing
that into the genuinely-absent case: a static actor never enters the spawn block
at all, so its location is unknown, while a dynamic actor with the bit clear has
a known location of exactly the origin.

On 02d4d478 that is **66 actors** -- game state, player state, surrender-vote
and mission actors, which really do sit at the origin -- reported as having no
location alongside the **27** that truly have none. All 2,028 `actor_spawned`
locations now match the reference key-for-key, including the 27/66 split.

This one is worth remembering as a process failure, not just a bug. The
preceding commit had changed the *adapter* to stop fabricating `{0,0,0}`, on the
stated premise that "there are zero genuine (0,0,0) spawns". The premise was
never checked against the reference; it is false. That change traded 66 wrong
values for 66 wrong nulls and cost `ability_detail` and `ability_usage` their
EXACT status -- 16/21 fell to 14/21 with nothing in the test suite noticing.
Fixing the parser instead restored both. **A claim about what the data contains
is not established by the code that produces it.**

### 13-B. ReplicatedMovement shipped a debug string [FIXED, 2637808]

`FRepMovement` decodes all eight members correctly. Its `Display` wrote
`mov(loc=..,rot=..,vel=..)`, which has nowhere to put
`simulated_physics_sleep` or `server_physics_handle`, so they were dropped;
`value_str` is one column and there is no struct column to hold them.
14,377 rows on 02d4d478 shipped that string where the reference
(`ReplayJsonNormalizer.cs:255`) emits an eight-member object.

Now serialized as a JSON object with the reference's member names and order.
Joined against the reference on (time_ms, group path, actor GUID, object GUID):
8,610 shared keys, zero reference-only, **all eight members agree on every
one**. 551 more that we decode and the reference does not emit. 5,216 stay raw
because 17 ability/projectile classes have no `RepMovement` entry in the
generated table -- the reference emits nothing for those either.

Both recovered members are `false`/`0` throughout this replay, so no new value
is recovered here. What changed is that they are representable at all.

Both `RepMovement` tests now assert the whole string. Substring assertions could
not see the members carrying no data, which is exactly where the loss was.

### 13-C. Gekko's descriptor path had a one-character typo [FIXED, f67ea66 + 4f78f6d]

`AggrobotAgentDescriptor` declared `/Game/Characters/Aggrobot/Aggrobot_PC...`;
the replays declare `AggroBot` -- capital B. Riot mixes casing inside Gekko's
own content (`Ability_Aggrobot_C_ExplodeyPatch` really is lowercase; the
character directory and asset are not) and the descriptor picked the wrong one.
Lookup is ordinal (`DescriptorCatalogIndex.cs:7`, `BoundExportStore.cs:5`), so
the class bound nothing.

Gekko is the only agent whose descriptor string differed from the replay string.
`AgentClassNetCacheDescriptors.cs:14` builds each agent's cache path as
`agent.Path + "_ClassNetCache"` and registers exactly one function, which is why
only `MulticastNotifyKilledEnemy` was lost among that actor's RPCs -- every
other one resolves through subobject class paths that do not depend on the
character path. The larger half of the loss was Gekko's replicated character
property group, unbound for the whole match.

**The reference's own export summary reported it all along**: AggroBot is the
sole `was_decoded: false` among the match's eight agent classes.

Fixed at source on `local/vrfkit-descriptors` (f67ea66), together with the test
that pinned the typo (`ValorantDescriptorsTests.cs:16`). Regenerating moves
3,605 rows off "not in table": 528 decode to typed values, 3,077 resolve to
fields the descriptor declares Raw or Skip.

This is the named root cause behind the `tactical`/`kast` divergence recorded in
section 6. It does **not** make those sections converge -- the published
reference bundles were built by the parser *with* the typo, so they are still
missing Gekko's kills, and clutch derivation is not monotonic in kill count.

Measured, not inferred: all five values section 6 pins were recomputed after the
fix and every one is unchanged (2c9e88a0 clutch_attempts 4/1, 45758459 7/5,
500ce1a8 6/3 and clutch_wins 2/1, 02d4d478 opening_duels_won 11/10). Section 6's
table is current. On 02d4d478 the per-player breakdown now shows the mechanism
directly: Gekko is player 264, and the reference credits him 0 first bloods,
0 opening duels and 0 trade kills against our 2, 2 and 4.

Do not pursue parity there; regenerating the reference bundles would invalidate
every comparison figure in this document.

### 13-D. The extractor could not read a factored handle run [FIXED, 4f78f6d]

`AddPropertyHandle`'s handle argument had to be a literal. A descriptor may
instead factor a run of handles into a helper that takes the first one:
`MulticastNotifyDamageBaseParameters.cs:24` declares
`AddDeathFields(uint firstHandle)` and calls it as `AddDeathFields(32)`, so its
six statements read `firstHandle` and `firstHandle + 5`.

The table is keyed on `(group_path, field_name)` and never reads the handle, so
the value needs no resolving -- only the shape has to be recognised. The
trailing comma every caller writes is what stops the looser pattern swallowing a
lambda: `x => x.Prop` has no comma after `x`.

`MulticastNotifyDamage_Base` regains four fields its `_Point` twin -- which
spells the same handles inline -- has had all along. All 51 invocations now
decode `KillsForKiller`, `KillsForVictim`, `DeathAnimMontage` and
`DeathMontageEffectOverrideIsQueued`, and all 51 events match the reference on
all four with zero events on either side the other lacks.

Four module-level regexes encoding the old literal-only assumption were dead
code. Removed -- left in place they invite putting the assumption back.

### 13-E. `payload: null` meant two different things [FIXED, 2637808]

An RPC row whose `field_name` carries no dot is the function itself, not one of
its parameters. Usually that means a zero-parameter RPC and there is nothing to
carry -- but 608 rows on 02d4d478 arrive with the whole parameter block as
undecoded bits, because the descriptor bound no property handles for that
function. They were dropped, so "no parameters at all" and "parameters we could
not read" were indistinguishable downstream.

Now keyed under the function's own name, using the same `{BitCount, Data}` blob
shape as every other raw payload. The reference emits none of these functions
(they sit in its 241 unbound groups), so there is no key to match; this is a
vrfkit-only convention. Null RPC payloads 230,160 -> 229,552.

Measured, not assumed: **zero** of those rows carry a decoded value. An earlier
note called them "608 real values dropped"; they are 608 undecoded blobs.

### 13-I. A static actor has no class path, and no archetype either [FIXED, ea08a83]

Same shape as 13-A, found the same way -- by widening a comparison that had been
passing. `NewActorSerializer.cs:29` returns before reading the spawn block for
anything that is not dynamic, so the reference leaves both
`ReplicationClassPath` and `ArchetypePath` null for static actors. We filled
both in.

`sink.rs` fell back to the actor GUID's own path, with a comment asserting that
"for static actors the actor GUID path itself is the class". It is not -- that
path is the level's instance name. 27 opens on 02d4d478 shipped `Ascent_C_0`,
`AresWorldSettings` and `WindowShieldA1` as replication class paths. The adapter
then derived an archetype from it, and since the class path was empty by that
point, all 27 came out as the literal string `Default__`.

Nothing is lost: all 27 paths are byte-identical to the `path` column
`net_guids.parquet` already carries for the same GUID, checked row by row.

All 2,028 `actor_spawned` events now match the reference on **all three**
fields. The earlier check compared `location` alone, which is why the other two
stayed wrong through two rounds of "spawns match". **A comparison only defends
the fields it reads.**

### 13-F. What is still untyped, and why it is not a bug

30 `ReplicatedGravityDirection` rows across four classes with **no descriptor on
either side**: `Smonk_PostDeath_PC` (14), `Pawn_Hunter_E_Drone` (8),
`Pawn_Aggrobot_SeekerNade` (6), `Pawn_Aggrobot_RollyPolly` (2). The reference
decodes none of them. Writing those descriptors is new upstream work, not a fix.

5,216 `ReplicatedMovement` rows stay raw for the same reason, across 17
ability/projectile classes.

### 13-G. Verification run for this session

    cargo test --workspace              243 passed, 0 failed (corrected later)
    cargo clippy -- -D warnings         clean
    cargo fmt --check                   clean
    validate_corpus.py                  215/215, malformed 0, five totals exact
    check_export_baseline.py            OK, 3 counters cross-check their files
    check_corpus_baseline.py x4         OK (12.10, 12.11, 13.00, 13.02)
    validate_metrics_corpus.py          16/21 sections exact on all 11 replays

The export baseline was updated twice in this session -- both times because the
guard caught a counter move unprompted, and both times the move was explained
before the baseline was rewritten. Row counts never changed; only byte sizes and
overlay counters did.

### 13-H. Stale figure corrected

The C# repo's "17 uncommitted entries" figure in the brief is stale. That work
was committed as fe5343a; the tree is now clean at f67ea66 on
`local/vrfkit-descriptors`, with `main` still untouched.

The published bundle stamp needs one more qualification. It records Git HEAD
`2d2e05e`, but does not prove that the working tree was clean.
`EffectManagerComponentDescriptors.cs` is absent from clean `2d2e05e` and was
first committed in fe5343a; that commit records that the descriptor work had
previously lived uncommitted. Published bundle behavior is consistent with the
descriptor being present. Therefore a clean checkout of `2d2e05e` alone is not
a complete reproduction recipe. Keep `main` pinned, but treat the published
bundle artifact -- not an inferred clean tree -- as the immutable reference.

---

## 14. Codex needs-work results (2026-08-02)

This section records the four delegated items. Work was committed on the
isolated `codex/needs-work` branch; master, valplay, and the C# source tree were
not modified or merged by the delegate.

### 14-A. Live effect decoder guard (fb41b96)

The brief's count of eight Rust examples was stale: `effect.rs` contains nine.
The new `tools/check_effect_decoder.py --check` runs those nine through the
live Python decoder, adds two independently expected C# reference-bundle cases,
and pins Python's partial-list malformed-input contract. All 12 pass. A
deliberately corrupted byte produced exit 1; restoring it produced exit 0.

### 14-B. Untyped-row investigation and descriptor extraction (e1eb220, b68baaa)

Every count below uses the explicit denominator: 871,595 rows with every
`value_*` column null, out of 1,240,444 total fields.parquet rows on 02d4d478.
The requested descriptor-present/descriptor-absent binary was itself too
coarse; several groups contain a mixture of intentional raw data, movement
markers, undescribed functions, and a real handle/name mismatch.

```text
group                         pre no-value   evidence-backed result
BaseReplayController              333,022   descriptor is extracted; 225,808
                                            movement markers and 107,214 C#-
                                            undescribed function rows remain
LocationalEffectManager           124,744   no C# descriptor
EffectManager                     110,508   descriptor/extractor work; residue
                                            is raw, skipped, or undescribed
ReplayEffect                       23,275   5,294 recovered; 17,981 intentional
                                            raw/undescribed rows remain
BombPlayerState                    20,898   20,888 absent from the C# descriptor;
                                            10 UniqueId rows intentional Raw
```

ReplayEffect supplied the real fix. Its descriptor binds handles 26/27 to
Location/Rotation while runtime manifest names are 248/249. The overlay now
tries direct name, the existing `b`-prefix rule, then an explicit descriptor
handle alias. Both RPC and RepLayout sinks pass the handle, including when no
field name exists.

The generator also learned three previously invisible C# declaration shapes:
11 `AddRaw` wrapper entries, 2 called BombGameState helper entries, and 29
runtime agent-cache entries. Current generated output is 152 groups / 1,100
name entries: Raw 164, Skip 154, Typed 782, plus 84 separately sorted
explicit-handle aliases. No prior name key was deleted or type-changed.

Fix round 2 (b10467b) makes an explicit descriptor category override take
precedence over an inherited Agent category, matching the C# catalog's
effective `HasFlag(Agent)` filter. This prevents three Ability subclasses on
current master's pawn-descriptor branch from receiving fabricated runtime
ClassNetCaches. Unknown categories and unsupported override syntax now fail
loudly. The f67 input remains byte-identical to the tracked 1,100-entry table.

Fresh 02d4d478 export measurement:

```text
measure                    before       after       delta
Parquet rows             1,240,444   1,240,444           0
all value_* null           871,595     866,301      -5,294
overlay decoded OK         358,184     363,478      +5,294
overlay Raw/Skip            71,427      73,351      +1,924
overlay not in table       525,839     518,621      -7,218
overlay no field name       33,529      33,529           0
overlay rows offered       988,979     988,979           0
fields.parquet bytes    13,187,104  13,255,044     +67,940
```

The `not_in_table` reduction is exactly 5,294 newly decoded rows plus 1,924
newly classified deliberate Raw/Skip rows. Structural counters and every
Parquet row count are unchanged. The byte increase comes from the 5,294 newly
populated string values. All 2,647 C# Location values matched exactly and all
2,647 Rotation values matched within 5e-5. The adapter accepts both the legacy
raw representation and the typed representation with identical geometry.

### 14-C. Whole-block payload preservation measurement

Section 7-C contains the three-replay cost table, timing protocol, exact
14,755-row round-trip audit, and the production decision gate. No Task C
production source or baseline change was committed.

### 14-D. Complete Rust ASCII enforcement (a0ea2b4)

Task D translated every tracked Rust comment/doc/diagram to meaning-preserving
ASCII, removed the three BOMs, and restored the net/sink mojibake and `??`
damage from history. Pre-cleanup: 61 tracked Rust files, 44 affected files,
510 affected lines, 8,984 non-ASCII scalars over 28 code points. Post-cleanup:
61 files, zero violations. `tools/check_ascii.py` scans complete raw file
contents, not line-local string patterns. The real dirty tree and a planted
tracked violation both failed before the restored tree passed.

### 14-E. Explained export baseline drift

Before any baseline update, the final release export was checked against
`tools/baselines/export_02d4d478.json`. It reported exactly four differences:

```text
overlay_decoded_ok    358,184 -> 363,478
overlay_raw_skip       71,427 ->  73,351
overlay_not_in_table  525,839 -> 518,621
fields.parquet bytes 13,187,104 -> 13,255,044
```

These are precisely the Task B reclassification identity and its populated
values described in 14-B. Task A and Task D do not affect export data; Task C
was measurement-only and its ON values are excluded. After documenting this
attribution, the baseline was updated. Its JSON diff contains exactly those
four values, and an immediate ordinary check passes with all three printed
counter/Parquet row identities intact. Any additional drift is a failure.

### 14-F. Final verification

Fresh final sweep on the delegate branch after the Task A/B/D commits and the
documented export-baseline update:

```text
cargo test --workspace                  243 regular + 3 doctests, 0 failed
cargo clippy --workspace --all-targets  clean with -D warnings
cargo fmt --check                       clean
cargo build --release                   exit 0
Python descriptor/adapter tests          10/10
effect decoder guard                     12/12
ASCII guard                              61/61 tracked Rust files
validate_corpus.py (13.01)              215/215; malformed 0
  totals                                blocks 136,545,822
                                        fields 98,883,979
                                        RPCs 75,571,092
                                        skipped 1,972,080,670
check_export_baseline.py                PASS after the explained 4-line update
check_corpus_baseline.py 12.10/12.11/13.00  PASS
compare_combat_report.py                ALL INTERESTING SHAPES MATCH
validate_metrics_corpus.py --jobs 3     16/21 exact on all 11 replays
```

The same five metric sections remain non-universal: combat, economy_detail,
weapon_stats, tactical, and kast. That set and the 16/21 count did not move.

The branch's old 13.02 JSON still points at the game-owned Saved/Demos
directory and expects four UUID-named files. During this run that directory
contained three newer files named 1.vrf/2.vrf/3.vrf, so the old check correctly
reported an input-set mismatch; it was not updated. The three available 13.02
replays independently validate 3/3 with malformed 0. Concurrent master commit
3a4b04 already corrected this stale guard to one stable copied replay under
`%LOCALAPPDATA%\vrfkit\baseline-corpora\build_1302`; running the delegate
binary against that current master baseline passes 1/1 with malformed 0.

While this isolated worktree was active, master advanced independently from
the merge base 9865c29 to f7bcdb9. The delegate did not merge, rebase, or
cherry-pick it. A read-only merge audit finds conflicts in PROJECT_STATUS.md,
table.rs, and export_02d4d478.json. Neither version of the two generated/data
files should be selected manually.

Current master depends on a separate clean C# worktree at
`C:\Users\yakihyuk0728\Documents\GitHub\VRP-pawn-descriptors`, branch
`local/pawn-descriptors@d2b76f2`. It is a descendant of f67ea66 and contributes
92 name entries across 20 ability groups that do not overlap Task B's 42 new
name entries. After obtaining merge authority, preserve master's fixed
build_1302 baseline and decode-error guard. First preserve b10467b, which
applies the nearest explicit category override and prevents three phantom
Ability ClassNetCaches. Then run that extractor against the d2b76f2 worktree,
followed by type corrections and rustfmt. A temporary clean generation with
b10467b produced exactly 1,192 names / 172 groups / Raw 164 / Skip 164 / Typed
864 / 84 handle aliases, retained all 29 real runtime Agent caches, and omitted
the three phantom paths. Regenerate -- never hand-merge -- the tracked table.

Then build a combined release and re-measure the export baseline. Counter
arithmetic is only a sanity check; Parquet ZSTD bytes, hash, typed-row count,
and final baseline must not be predicted or added across branches. Re-run the
215-replay decode-error guard, corpus validation, 02d4 export, all baselines,
combat comparison, and 11-replay metrics before resolving the documentation
conflicts with measured combined values.

At final delegate verification, the primary C# repository was clean at
`local/vrfkit-descriptors@f67ea66`, the separate pawn-descriptor worktree was
clean at d2b76f2, C# main remained `2d2e05e`, and valplay was clean at
`main@4578a5a`.
