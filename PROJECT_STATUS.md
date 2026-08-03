# vrfkit Project Status

Last updated: 2026-08-04. Includes the replay-coverage audit through 8eb5909,
the concurrent master audit corrections through 101c33a, the code audit fixes
in section 12, the Codex needs-work results through 45223c9 in section 14,
production whole-block ClassNetCache payload preservation in section 7-C, and
the five-agent session in section 22 -- Event chunks, movement timestamps,
manifest metadata, the effect decoder, and the checkpoint investigation that
closed the AbilitiesAndBuffs door for good. All numbers come from direct tool
runs, not estimates.

Section 22-D retires a hope this document carried since 7-H: checkpoint chunks
are NOT the key to the unattributed-bits ceiling. Measured over all 4,024
checkpoints in the corpus, not argued.

Section 22-I then measured the other question about them and got the opposite
of the expected answer: checkpoints are NOT redundant with ReplayData. 6-11% of
their property values disagree with the incremental stream at the same
timestamp. The parser is now recommended, and CHECKPOINT_SPEC.md specifies it.

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
                 The GAME OWNS THIS DIRECTORY AND ROTATES IT. Do not pin a
                 baseline at it -- build_1302.json did, and on 2026-08-02 all
                 four pinned replays were gone, replaced by three unrelated
                 ones (verified: no subset of the pinned four sums to the new
                 total). Re-pinned against a preserved copy, below.
Older fixtures : C:\Users\yakihyuk0728\Documents\GitHub\ValorantReplayParser\tests\Test.Integration\Replays
                 One source fixture each for 12.10, 12.11, and 13.00.
                 READ ONLY; do not run baselines from this repository.
Local baselines: %LOCALAPPDATA%\vrfkit\baseline-corpora\build_*
                 One preserved replay per build: 12.10, 12.11, 13.00 (test
                 fixtures, ~1 MB each) and 13.02 (a real 62 MB demo, copied
                 2026-08-02). Every build_*.json now points here, at files
                 nothing else writes to. THIS is what makes the pins mean
                 something -- a baseline over a directory some other program
                 owns guards nothing.
```

### Verify the build before touching anything
```powershell
cd C:\Users\yakihyuk0728\Documents\GitHub\vrfkit
$env:CARGO_TARGET_DIR = $null
cargo test 2>&1 | Select-String "test result"
# Expected: 287 passed, 0 failed across all targets (284 regular + 3 doctests).
# Sum the per-target lines; the last line is one target, not the total.
# Per crate at 5c46851: bitio 22, transform 22, container 41, frame 6,
# schema 47, net 32, decode 75, movement 5, export 25, vrfkit 12.
# This figure has been stale FIVE times now -- it read 252 while HEAD had 257,
# then 257 while four agents had taken it to 287. Re-measure before quoting.
cargo clippy --all-targets -- -D warnings 2>&1 | Select-String "^error"
# Expected: no output (exit 0)
cargo fmt --check
# Expected: exit 0
python tools\check_effect_decoder.py --check
# Expected: OK: 12 live effect decoder cases
python tools\check_ascii.py --check
# Expected: OK: 63 tracked Rust file(s), ASCII only
```

### Regression guard (run after any non-trivial change)
```powershell
cargo build --release -p vrfkit
.\target\release\vrfkit.exe export `
  "C:\Users\yakihyuk0728\Documents\GitHub\valplay\data\raw\vrf\02d4d478-1dfb-4412-9a77-29ca29105a9d.vrf" `
  --out out\nested
# Must NOT change: content blocks 608020, fields 429637, RPCs 342735,
#                  movement 1839607, NetGUID rows 16167, decode errors 0,
#                  Event rows 195, Effect blobs 53908
# The last two are new at section 22. Effect blobs is the ONLY counter that
# moves when the effect decoder changes -- the overlay buckets are decided
# before that pass runs, so Decoded OK and Not in table stay put either way.
python tools\compare_combat_report.py
# Must print: ALL INTERESTING SHAPES MATCH
python tools\validate_corpus.py .\target\release\vrfkit.exe `
  "C:\Users\yakihyuk0728\Documents\GitHub\valplay\data\raw\vrf"
# Baseline: blocks 136,545,822  fields 98,884,839  rpcs 75,571,092
#           malformed 0  skipped 1,972,018,965    (~30s, runs 16-wide)
# `fields` and `skipped` moved at db42e6a: the controller's opening bunch was
# framed nine bits early (17-A), so 215 replays x 4 handles and x 287 bits came
# back. That commit refreshed every baseline FILE and left this comment stale
# for two commits -- it read 429633 / 98,883,979 / 1,972,080,670, so a new
# session would have compared a green run against the wrong numbers and gone
# hunting for a regression that was not there. Dated figures further down are
# historical measurements at their own commits and stay as they are; this block
# is the one that has to track HEAD.
python tools\check_decode_errors_corpus.py .\target\release\vrfkit.exe `
  "C:\Users\yakihyuk0728\Documents\GitHub\valplay\data\raw\vrf"
# Expected: OK: every replay reported Decode errors: 0   (~70s, 8-wide)
# RUN THIS AFTER ANY table.rs CHANGE. `vrfkit validate` does not print the
# overlay counters -- only `export` does -- so validate_corpus.py cannot see
# a decode error and never could. A wrong overlay type moves NONE of the
# counters validate_corpus.py reads: the row still emits, the block still
# walks, blocks/fields/rpcs/malformed/skipped are all identical. This is
# also the only check that can catch a per-class type whose two candidate
# readings are indistinguishable on 02d4d478 but not on some other replay.
python tools\check_corpus_baseline.py --baseline tools\baselines\build_1302.json
# Expected: OK: 1 replays match the baseline
# It used to pin FOUR replays living in %LOCALAPPDATA%\VALORANT\Saved\Demos,
# which the GAME owns and rotates. On 2026-08-02 all four were gone, replaced
# by three unrelated matches (verified: no subset of the pinned four sums to
# the new total). The guard reported it correctly -- seven DRIFT lines naming
# each missing and each unexpected replay -- but a baseline over a directory
# another program writes to guards nothing. Re-pinned at one preserved 62 MB
# demo under baseline-corpora, like the other three builds. Do not point it
# back at Saved\Demos.
python tools\check_export_baseline.py --baseline tools\baselines\export_02d4d478.json
# Expected: OK ... 4 printed counters cross-check against their Parquet files.
# Production fields baseline: 1,246,812 rows, 13,742,379 bytes. 6,364 of those
# rows preserve unresolved whole blocks. This line read 1,246,809 / 13,564,140
# for a whole session while the pinned JSON said 1,246,812 / 13,586,343 -- the
# FILE was right and the comment was not, which is the direction that matters
# least but is still how a session ends up hunting a phantom regression.
# The strongest single guard: it pins all 23 export counters plus every Parquet
# file's rows AND bytes, and caught every counter move this session before
# anything else did. On DRIFT, explain each line before passing --update. The
# point is that a silent change is impossible, not that the numbers are sacred.
# It grew a fifth file at section 22: PARQUET_FILES drives the whole script, so
# events.parquet was invisible -- not failing, unguarded -- until it was added.
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
#   D14CD5D548C0C452885E91BA54ADCBE1E0BB09C4D113E316ED974554A09A0DD9  fields.parquet
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

  7-C  whole-block payload preservation is implemented. The semantic ceiling
       remains because the game never declares the function table. The adapter
       excludes preservation rows, so current metrics are unchanged
  7-F  measured -- the parallelisable slice of the DECODE path is 3.4%, the
       rest is order-dependent. The process-level win was taken instead
       (11x). The three hot spots 7-F named were then optimized in 5-P:
       an export is 1.73x faster with all four Parquet files byte-identical.
       Decode is still strictly sequential; only the writers are concurrent
  7-H  CLOSED NOT SOLVABLE. The export-gap check that fixed cf97ecf was run
       and came back negative, and so did every other structural route: the
       class of a stably-named subobject is never on the wire. Five
       measurements in 7-H. Do not reopen without new input data --
       checkpoint chunks were the only unexamined region, and section 22-D
       examined them: 4,024 checkpoints across all 215 files, 1,955,988
       export-group records, ZERO mentioning AbilitiesAndBuffs. There is no
       unexamined region left in the file. The ceiling stands until a game
       build declares the group

Where the open work is now, after section 22:

  - IMPLEMENT THE CHECKPOINT PARSER. 22-I measured the question 22-D left
    open and the answer went the other way: 6-11% of a checkpoint's RepLayout
    values disagree with what ReplayData carried at the same timestamp, the
    median differing key was last written ~77 s earlier, and 0.5-2% are keys
    ReplayData never sent at all. Checkpoints are not redundant. The format is
    fully specified in CHECKPOINT_SPEC.md and sized at 4 files + 1 new
    (~150 lines); reuse decompress_replay_data, iter_demo_frames, NetGuidCache
    and ReplicationReader unchanged. Seed a SEPARATE cache and reader per
    checkpoint -- do not feed checkpoint exports into the live ReplayData
    cache, and do not replay its channel opens through the live reader
  - Event payload words are raw for 5 of 7 groups (22-H). characterDeath is
    solved; characterUltimateUsed's single word is not
  - the 26M untyped bits that are NOT effect blobs and NOT AbilitiesAndBuffs
    are missing overlay entries, not missing decoders -- MulticastStop-
    ContinuousEffect's SourceID/EffectID are the worked example in 22-E

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
ClassNetCache block cannot be split into fields. It still returns Err and its
skipped bits remain counted, while its exact whole payload is preserved once as
an explicitly marked Parquet row. No field or RPC structure is fabricated.


---

## 1. What This Project Is

A from-scratch Rust VALORANT replay (.vrf) parser in a NEW repository
(C:\Users\yakihyuk0728\Documents\GitHub\vrfkit), built to replace the
C# parser (ValorantReplayParser, MIT) that the valplay Python analytics
pipeline depends on. The C# parser discards roughly 26% of content blocks
because it abandons any bunch whose payload has no registered descriptor.
vrfkit preserves every field it can walk, including raw bits for unknown
types; an unresolved ClassNetCache block remains a loud stream failure and
emits no fabricated field/RPC rows, but its whole payload is preserved as one
explicitly marked row.

Primary outputs: fields.parquet, movement.parquet, actors.parquet,
net_guids.parquet, manifest.json -- all written by `vrfkit export`. A Python adapter
(tools/to_valplay_bundle.py) converts these into the bundle shape that
valplay's compute_metrics.py already consumes, so its 20 metric sections plus
the constant provenance `note` run unchanged on our data.

---

## 2. Repository State (2026-08-04)

```
measured at  : master, 2026-08-04, after the five-agent session (section 22)
               was integrated and its worktrees removed
branches     : master only. No worktrees, no stashes, no remote
commits      : run `git rev-list --count HEAD`. No number is written here
               on purpose: the two that were had both gone stale, and this
               one would be wrong the moment the line was committed
tests        : 287 passing, 0 failed (284 regular + 3 doctests: vrf_export 2,
               vrf_transform 1). This line has been stale FIVE times -- 238,
               246, 249, 252, 257. Re-measure with `cargo test --workspace`
clippy       : 0 warnings (--all-targets -- -D warnings)
fmt          : clean (--check)
ascii        : 63 tracked Rust files, clean; --self-test passes
working tree : clean
corpus       : 215/215, malformed 0, decode errors 0 across all 215
overlay      : 369,743 decoded / 73,984 raw-skip / 511,916 not-in-table /
               33,340 no-field-name; typed 37.4%; table 1,185 entries.
               UNCHANGED by the effect decoder, by construction -- see 22-F.
               Real coverage: 1,246,812 rows, 64.5% still untyped (was 68.8%)
effect blobs : 53,908 decoded, 0 failures on 02d4d478; 0 failures corpus-wide
guards       : export baseline OK; build_1210/1211/1300/1302 OK;
               compare_combat_report ALL INTERESTING SHAPES MATCH;
               validate_metrics_corpus 16/21 with all 231 cells stable
valplay repo : 0 modified files (never written to; scripts run by absolute
               path, and compute_metrics.py is always pointed at a directory
               under vrfkit's out/ so it cannot write metrics.json into it)
ValorantReplayParser : clean, on branch local/vrfkit-descriptors at 8824794.
               main untouched at 2d2e05e -- the commit the reference bundles
               were built from. Do not move it (QUICK START says why)
```

### Commit list

```
45223c9 fix(tools): reject lexical descriptor shadows
81d4f88 fix(tools): scope descriptor source resolution
519de0b fix(tools): harden descriptor source parsing
7e0051f fix: anchor ASCII scan to repository root
23fb6aa fix(tools): detect empty effect corruption
4ddac84 docs: record safe category parsing
b5b74db fix(tools): parse category overrides safely
7515cfc docs: align the rotating 13.02 path reference
7e0a8de docs: clarify concurrent integration requirements
b10467b fix(tools): respect descriptor category overrides
aef30de docs: record needs-work measurements and baseline
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

Total: 246 tests, measured at 45223c9 (243 regular targets plus 3 doctests).
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

oracle pass rate  (re-measured 2026-08-02 at 8be0b8d; the median and max once
                   recorded here, ~98.9% and 99.99%, were never measured and
                   were both wrong -- the tool also reports "below 99.99%:
                   215", i.e. no replay reaches 99.99%. The 2026-08-01 figures
                   97.487010 / 99.323286 / 99.681958 were correct at the time
                   and were lifted by 17-A, which recovered 287 bits per
                   replay.)
  min              : 97.487378%  (936a0967-7a14-46bf-ab7e-b33f7e228cc4.vrf)
  median           : 99.323434%
  max              : 99.682485%

corpus totals   (2026-08-02 at 8be0b8d)
  content blocks   : 136,545,822
  fields emitted   :  98,884,839
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
  unattributed bits: 1,972,018,965 (~246 MB; 97.283437% is
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
  whole-block rows:    6,365  (preserved, still counted as skipped)
fields emitted     : 429,633
RPCs emitted       : 342,735
movement rows      : 1,839,607
actors.parquet rows:   3,827  (2028 opens + 1799 closes)
net_guids.parquet  :  16,167  (14,480 carry an outer GUID)
decode errors      :       0
fields.parquet rows: 1,246,809
typed (value_*)    : 380,060 rows with any value_* column set
                     30.483% of all 1,246,809 rows; 38.430% of the
                     988,979 rows offered to the overlay. The distinct
                     overlay decoded-ok counter is 369,395; do not conflate
                     that counter with non-null Parquet rows.
oracle pass rate   :  98.95%
```

Historical 2026-08-01 snapshot (four then-local 13.02 demos):
  all 4 parse, malformed 0, pass rate 97.959041% - 99.329325%
  blocks 3,117,920  fields 2,279,512  rpcs 1,713,576  skipped 74,573,628

Those four files are no longer the current corpus or a valid live baseline
source. `Saved\Demos` rotates and now contains 1.vrf/2.vrf/3.vrf; this delegate
branch's old JSON therefore reports an input-set mismatch. Current master
commit 3a4b04 supersedes it with one stable copied replay under vrfkit's own
baseline directory. Preserve that master version during integration.

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
unresolved ClassNetCache streams remain skipped failures and emit one marked
whole-block preservation row.
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

### 7-C. Unattributed ClassNetCache blocks [IMPLEMENTED AND VERIFIED; PAYLOAD PRESERVED, STILL UNPARSED]

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

PRE-PRODUCTION BEHAVIOUR (fixed 2026-08-02): `parse_class_net_cache` returned
`Err` when `function_count == 0` before reading any payload bits. The caller
only invoked `on_stream_failure`, which retained a diagnostic capped at 32
lines. No `on_field` or `on_rpc` fired, so the decoded payload disappeared.

PRODUCTION BEHAVIOUR: the parser now exposes that exact decoded whole-block
payload before reporting the same failure. Export writes exactly one row with

    field_name == "__vrfkit_unresolved_class_net_cache_payload__"

The row is not a field or RPC: `handle == u32::MAX` is only a secondary
invariant, all `value_*` columns are null, and no per-field split is invented.
The exact reserved field name is the sole consumer predicate because ordinary
array-truncation rows may also use `u32::MAX`. A pre-change export and scan of
all 215 corpus replays (281,557,231 existing rows) found zero exact marker
matches. The self-checking proof is `out/task3_collision_proof`.

These blocks still frame correctly (malformed framing 0), still return the
same failure, and still count every payload bit as skipped/not parsed. The
32-line diagnostic cap is unchanged; it is not the data path.

PRODUCTION ROUND-TRIP AUDIT: three replay exports compared the parser-boundary
tuple and payload to marker-filtered Parquet rows in original order. The tuple
covered time, packet, channel, actor/object GUIDs, group path, bit count, and
raw bytes. It also checked `ceil(bit_count/8)` lengths, every final-byte high
padding bit, marker/handle/value invariants, ON-OFF row deltas, and non-fields
Parquet hashes.

```text
replay       rows         bits   raw bytes   non-byte rows
08aec1e1       928      629,141      79,053             885
02d4d478     6,365   17,507,210   2,191,622           6,252
252168ae     7,462    7,400,586     927,439           7,322
TOTAL       14,755   25,536,937   3,198,114          14,459
```

All 14,755 rows matched exactly. The aggregate bit-remainder buckets were
`[296, 2338, 973, 811, 593, 2453, 4694, 2597]`; padding violations were zero.

PRODUCTION COST: the timing build used the current production row path behind
an experimental OFF/ON switch in one release binary. Each replay had one
warmup per mode and 10 adjacent, alternating OFF/ON pairs. The fixed-seed
10,000-resample paired-median bootstrap confidence intervals all include zero.

```text
replay    rows added   fields ZSTD bytes added   paired median   bootstrap 95% CI
08aec1e1          928                    38,544        -6.481 ms   [-22.023,  8.154]
02d4d478        6,365                   230,008        -8.949 ms   [-63.254, 50.843]
252168ae        7,462                   247,651        +8.347 ms   [-23.564, 87.695]
```

Evidence caveat: the raw timing summary pins the measurement executable as
SHA-256 `8140184A...D5D1E`, but a later same-source relink replaced the saved
file with `780CE881...5339`; no copy of the measured executable remains to
rehash. The 60 timed rows and bootstrap calculation were independently
recomputed from the preserved CSV, but the timing bundle is not fully
self-contained.

The export wall-clock effect is therefore not measurable. Existing parser and
overlay counters were invariant, and actors.parquet, movement.parquet, and
net_guids.parquet were byte-identical on all three replays.

ADAPTER GUARD: removing its exact-name exclusion changed 02d events.ndjson
from 576,246 to 582,611 lines. All 6,365 preservation rows leaked as fake
`export_group_received` events. Restoring the predicate returned it to 576,246
lines; events, movement, and manifest were byte-identical to the pre-change
bundle. The 11-replay metrics matrix stayed identical in all 231 section cells.

The production 02d4d478 baseline is now 1,246,809 rows / 13,564,140 bytes,
from 1,240,444 / 13,334,132 (+6,365 rows / +230,008 bytes). No other baseline
line changed.

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
This is now a measured fact rather than an assumption. No schema-driven
semantic decode is possible today, but the preserved whole-block rows support
offline investigation and future reinterpretation.

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
identical at 1,972,080,670. The later preservation path also leaves that
"not parsed" counter unchanged.

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

Historical state at commit 9cb7a24: tools/check_corpus_baseline.py pinned
per-file and total oracle figures in JSON and failed on any difference:

    python tools\check_corpus_baseline.py --baseline tools\baselines\build_1302.json

At that time, tools/baselines/build_1302.json covered four local 13.02 demos --
blocks 3,117,920, fields 2,279,512, rpcs 1,713,576, malformed 0,
skipped 74,573,628, pass rate 97.96-99.33%.

That design is superseded. `Saved\Demos` is a rotating game-owned directory;
this delegate JSON expects four vanished UUID files and now reports the
input-set mismatch against 1.vrf/2.vrf/3.vrf. Concurrent master commit 3a4b04
pins one stable copied replay under `%LOCALAPPDATA%\vrfkit\baseline-corpora`
instead. The historical guard was proven to fail: perturbing its baseline
produced the expected DRIFT lines and exit 1.

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

### 7-H. Instance-named component groups [43.9% SOLVABLE FROM REPLAY DATA -- NO METRIC IMPACT]

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

CORRECTION 2026-08-02. The tag on this section was
`[NOT SOLVABLE FROM REPLAY DATA]`. That is refuted for 43.9% of the affected
rows, including the single largest offender. The mechanism findings above
(measurements 1-5) survive intact -- a stably-named subobject's class GUID
really is never on the wire. What was wrong was the leap from "the class is not
declared" to "the class cannot be determined".

THE ROUTE THIS SECTION DESCRIBED AND THEN DECLINED TO RUN. The
"HANDLE-RANGE COINCIDENCE" near-miss above dismisses the idea after checking
only the MAX handle, and calls the general form "guessing a group to make a
number look better". The general form is not a guess when it is unique. Match
the FULL observed handle SET against the declared handle set of every
RepLayout-shaped group, and require exactly one compatible candidate:

    InventoryComponent observes 25 handles: [1..17, 21..25, 29, 30, 31]
    compatible groups among all 244 RepLayout-shaped groups: exactly 1
      -> /Script/ShooterGame.AresInventory

That is the class this section already names as correct, reached by
elimination. The argument closes: AresInventory's 27 handles were exported, so
the server replicated one; every resolved group's observed handles fall within
its declared set (0 exceptions), so no resolved group absorbed that data; the
only unattributed RepLayout data are the 70 bare-name groups; and
InventoryComponent is the only handle-compatible one among them.

Corroborated on a second axis -- observed bit widths against AresInventory's
declared field names line up field by field (handle 1 `bIsActive` is 1 bit x40;
handles 2-17 are the 16 `ItemSlots`; handle 23 `NetTimestamp` is 32 bits x4,860;
handle 21 `NewCurrentEquippable` shows 8/16/24-bit IntPacked NetGUID widths).
Two declared handles are never observed and zero observed handles are
undeclared.

SAFETY. Applied to groups whose class is ALREADY known, across all 11
cross-validated replays: 87 spoke, 87 correct, 0 mismatches. The rule is
self-policing -- unique candidate binds, ambiguity stays silent -- so NO SILENT
SUCCESS holds. This matters because the objection raised above is real: on the
RepLayout path a wrong group has no failure signal. Uniqueness, not
plausibility, is what makes it safe.

YIELD, by evidential strength, over the 11 cross-validated replays:

    route                          rows    share   nature
    A  handle-set uniqueness    174,077    43.3%   determination
    B  plain unique_leaf_match    1,183     0.3%   existing rule, now called
                                                   (FIXED, section 17)
    C  stem-strip leaf            3,413     0.8%   name heuristic (weakest)
    union                       176,585    43.9%   0 conflicts between routes

The remaining 56% genuinely resists. `ZoomStateMachine` observes only handles
{2, 4}; 21 groups are compatible. A small handle set carries no information.

ROUTE B IS AN ASYMMETRY IN OUR OWN CODE, not a data problem. `sink.rs:433`
(subobject path) and `sink.rs:401` (class path) both call `unique_leaf_match`.
The actor-GUID fallback at `sink.rs:301-310` does not -- it tries the lookup
keys and then `return actor_path.to_owned()`. `AresWorldSettings` ->
`/Script/ShooterGame.AresWorldSettings` is an exact unique leaf the parser
already trusts on the other two paths. Fixing it is a few lines.

FIXED 2026-08-02, and the 1,183 figure reproduced exactly. See section 17.

VALUE (of the remaining routes A and C): STILL DO NOT DO IT. No metric section
changes -- and section 17 now confirms that empirically for route B, which was
done anyway because it was our own asymmetry rather than a new rule.
Every section is EXACT,
MATCH or OURS BETTER and none is BLOCKED; 7-A's tier-2 resolver already covers
100% of shots and says explicitly that resolving InventoryComponent is not
needed. The correct verdict is "solvable for 43.9%, buys nothing measurable" --
which is a different statement from "cannot be solved", and the difference
matters to whoever reads this next.

FOUR MORE STATEMENTS IN THIS SECTION ARE WRONG:

1. Measurement 4: "A class object is assigned a NetGUID only when it is
   referenced on the wire, and these classes never are." The generalization is
   false. `AresAttributeSet` IS assigned one -- GUID 1947, outer 15
   (`/Script/ShooterGame`) -- and 163 dynamic instances carrying 18,850 rows
   reference it as their `classNetGuid`. This section's own 19-child list of
   GUID 15 contains `AresAttributeSet`. True for AresInventory; false as a
   general rule.

2. "INSTANCE NAME -> CLASS IS MANY-TO-ONE, so no string rule can be correct
   even in principle." Many-to-one is a function, which is exactly what a
   lookup consumes. The example given (MeleeAttackState1/2/3/4/_Alt -> one
   component) is the parser doing this successfully today, 473 rows. What would
   break a string rule is ONE-to-many, which this section never demonstrates.

3. "33,529 of 429,633 field rows in 02d4d478 (7.8%)". Arithmetically right for
   the denominator named. The pre-preservation file had 1,240,444 decoded
   field/RPC rows; production fields.parquet has 1,246,809 total rows including
   6,365 non-field preservation rows. Measured replacement: 2.69% of all
   current Parquet rows, 7.80% of RepLayout field rows. Quote the denominator.

4. "the affected fields are ammo-level detail in weapon_stats and posture /
   fire-mode refinement." The largest block is 13,043 rows -- 38.9% of the gap
   -- and it is inventory and loadout state (`ItemSlots` x16,
   `CurrentEquippable`, `NewCurrentEquippable`, `SlotModifiers`,
   `RespawnNumber`). Ammo is MagazineAmmo (3,124) and ReserveAmmo (564) only.

WHAT WAS NOT RE-MEASURED: measurements 1 and 5 need parser instrumentation and
were left alone. Measurement 1 is consistent with everything else observed.
Measurement 5's flag histogram is untested. Checkpoint chunks remain
unexamined -- this section's own caveat stands.

---

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
  bits intact. An unresolved ClassNetCache block cannot be walked into fields.
  It remains a loud counted failure, while one distinguished row preserves the
  exact whole payload for later reinterpretation.
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
  reading Object->GetClass() -- asset data, not replay data. Outer-chain
  walking, leaf matching and checksum recovery were each measured and each
  fails (7-H). This sentence used to close over ALL routes; that was wrong.
  The replay's own net field export declares each group's handle set, and for
  some components exactly one declared group is handle-compatible, which
  determines the class by elimination rather than by a name rule -- 43.9% of
  the affected rows, see 7-H's 2026-08-02 correction. Treat "this component's
  fields are unnamed" as expected and as buying nothing to close, NOT as
  impossible to close.

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

  `python tools/check_ascii.py --check` now anchors itself to the repository
  root, enumerates tracked `*.rs` files with `git -C <root> ls-files -z`, scans
  their raw bytes, and rejects every byte above 0x7f. It reports stable repo-
  relative file/line/column/byte diagnostics and fails loudly if Git
  enumeration or a read fails. Root and nested-crate invocations both scan all
  61 files. The post-cleanup inventory is 61 files, 0 affected files, 0
  affected lines, and 0 non-ASCII bytes.

  This guard was observed failing twice: first on the real 510-line tree, then
  after planting `o` with an umlaut in a tracked Rust file (1 line / 2 UTF-8
  bytes, exit 1). The exact original SHA-256 was restored and the default
  scan returned exit 0. A separate `--self-test` exercises the same scanner.

  ASCII `??` corruption is outside the guard's detection domain. Task D used
  clean historical revisions to restore vrf-net field and vrfkit sink mapping
  comments. No clean pre-damage revision exists for vrf-net lib/pipeline, so
  those two were reconstructed from their surviving labels and local execution
  context. It also removed three BOMs and verified zero remaining literal `??`
  sites in the four affected files.

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
  check_decode_errors_corpus.py -- asserts Decode errors: 0 corpus-wide.
                            The ONLY check that sees overlay decode failures:
                            `validate` does not print those counters at all
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
                  from local main 666. The committed table is regenerated
                  from `local/vrfkit-descriptors` at 8824794, whose history merges
                  the pawn/projectile descriptors (13-J) into the Gekko
                  casing fix (13-C). Both delegate branches are merged and
                  their worktrees removed; regenerating from that branch
                  reproduces the committed table exactly.
                  (An earlier note cited ced9379 for the pawn branch; that
                  commit was amended away and is unreachable. The real
                  commit was d2b76f2, now merged as f0dd7e7.)
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

Historical audit snapshot on 2026-08-01: recursive searches of all three
scopes below found the same four physical replays and no additional `.vrf`
files:

```
%LOCALAPPDATA%\VALORANT\Saved\Demos   4
%LOCALAPPDATA%\VALORANT\Saved         4
%LOCALAPPDATA%\VALORANT               4
```

At that time all four were 13.02, inspect/export/validate succeeded, had malformed,
transform, and field-stream failures of zero, and exactly reproduced the pinned
build_1302 totals in section 4. Their runtime schemas and emitted replay events
contain BombGameState, BombPlayerState, BombDestination, TimedBomb, and spike
plant/defuse/explosion evidence.

That is positive evidence for Bomb mechanics, not a reliable official playlist
label. The replay header's `game_specific_data` contains serializedVersion and
playerLoadouts but no mode, queue, or playlist key, and modes such as Spike Rush,
Swiftplay, or Premier may reuse Bomb assets. The CLI has no independent game-mode
detector. Therefore the defensible inventory is the task brief's four Bomb-labelled
inputs and **zero mode-labelled non-Bomb inputs**.

This is retained as dated evidence, not a current inventory. `Saved\Demos`
has since rotated to 1.vrf/2.vrf/3.vrf, the delegate's four-file JSON no longer
matches, and master 3a4b04 now guards one stable copied 13.02 replay.

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

CORRECTION 2026-08-02, from a read-only audit. The recommendation holds --
keep as is, wire nothing in -- but two of the three reasons above are wrong,
and the "port the Rust vectors" suggestion is dangerous as written.

REASON 2 IS REFUTED. "The consumer reads Parquet so wiring it in means a
schema change" is false. Two precedents in this repo already carry a decoded
multi-member structure through the EXISTING columns: `ReplicatedMovement`
serializes as a JSON object in `value_str` (13-B), and `decode_struct_array`
at `sink.rs:1133` emits one row per leaf with a nested `field_name` such as
`Rounds[3].Reports[1].DamageDealt`. No new column is needed on either route.
The real cost is bytes and baseline updates: +143,105 bytes (+1.08%) adding
decoded arrays to `value_str`, or +12,828 (+0.10%) if the now-redundant
`raw_bits` are dropped; zero rows added on that route; about 0.47% of export
wall-clock.

REASON 1 IS CONFIRMED AS CODE BUT MISDESCRIBED. The contracts are not mirror
images. Python is MORE permissive in two places -- `effect.rs` caps payloads at
65,536 bits and 8 fields per element and Python caps neither. One divergence is
on WELL-FORMED input, not malformed: on a repeated element index Rust mutates
in place and accumulates while Python rebuilds and clobbers, which is a data
model difference, not a failure contract. And the two are not even fed the same
input -- `_decode_effect_blob` computes `bit_count = len(raw_bytes) * 8` while
the call site discards the `bit_count` column, where `effect.rs` takes an exact
bit length. Decisively: `effect.rs`'s `Err` HAS NO CONSUMER. Nothing outside
the file calls it. A failure contract only differs observably if something
reads it.

REASON 3 IS CONFIRMED AND WIDENED, with its vacuity stated. Reproduced from a
fresh export rather than quoted, comparing all 22 keys of the `shot` object on
the union of keys, across all 11 replays: 34,762 shot events, 0 mismatches,
0 one-sided keys. But `effect_equippable` matches vacuously -- null on both
sides for all 2,647 shots on 02d4d478 -- and `tracer_option` has exactly one
distinct value. The load-bearing evidence is `random_seed` and
`attack_vectors`, 2,475 distinct values each.

NO DIVERGENT BRANCH IS REACHABLE. Every input where the two decoders disagree
maps to a branch counter that is zero corpus-wide: 2,008,409 blobs across the
215 replays of build 13.01, plus 37,019 across three local 13.02 demos, take
the terminator branch and nothing else. Every blob is byte-aligned and every
one has `len(raw_bits) * 8 == bit_count`, so even the input-length difference
has no reachable instance. A direct Python-vs-Rust differential over every real
blob in 11 replays -- 100,997 compared, floats as IEEE-754 bit patterns -- found
0 disagreements.

WHAT THE REFERENCE DOES WITH A MALFORMED BLOB IS UNKNOWN, and that is the
finding, not a gap in the audit. No blob in 2,045,428 takes a divergent branch,
so the reference exhibits no behaviour to observe. Do not settle the question
from our own decoders' synthetic output.

THERE IS A THIRD DECODER FOR THIS FRAMING and none of the three reasons
mentions it. `crates/vrf-decode/src/array.rs` is wired in at `sink.rs:1133`
and, on hitting a limit, stops that branch, preserves the remaining raw bits
and increments a truncation counter. Ranked against section 8's invariants:
the live Python path is a SILENT partial with no counter, which fails NO SILENT
SUCCESS; `effect.rs` would be counted but throws away recoverable elements;
`array.rs` is counted, raw-preserving and keeps the good elements. The
implementation whose contract the invariants actually endorse is the one
already wired in.

DO NOT GENERATE THE PYTHON TEST VECTORS FROM THE RUST IMPLEMENTATION. The
paragraph above suggests porting the Rust vectors, and that is safe only for
the eight existing ones, which are all well-formed and byte-aligned by
construction (three were confirmed byte-identical to real `raw_bits` rows). The
vectors that would matter are the malformed ones, and generating those from
`effect.rs` would write Rust's contract into the live path's tests on 12 of 18
constructed cases -- a behaviour change wearing a test-addition costume. Pin
what Python DOES, and record the corpus census as the reason those branches are
unreachable.

WHEN effect.rs CAN BE DELETED. Its only remaining value is as an executable
spec, and that value moves to the Python side the moment a test port lands.
The eight vectors cannot pin the one live-path decision with a latent-bug
shape: that the adapter passes `len(raw_bits) * 8` rather than the `bit_count`
column. If the port pins that too, the keep-reason dissolves and deleting
`effect.rs` is defensible. If it does not, that decision is unpinned on both
sides, so keeping `effect.rs` still buys nothing.

Evidence log: `out/audit_effect/measurements.txt`, 303 lines, every command's
raw output. Not committed -- `out/` is gitignored.

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

### 13-F. What is still untyped, and why it is not a bug [SUPERSEDED by 13-J]

30 `ReplicatedGravityDirection` rows across four classes with **no descriptor on
either side**: `Smonk_PostDeath_PC` (14), `Pawn_Hunter_E_Drone` (8),
`Pawn_Aggrobot_SeekerNade` (6), `Pawn_Aggrobot_RollyPolly` (2). The reference
decodes none of them. Writing those descriptors is new upstream work, not a fix.

5,216 `ReplicatedMovement` rows stay raw for the same reason, across 17
ability/projectile classes.

That upstream work was done on 2026-08-02; all 5,246 rows now decode. See 13-J.
One count above is worth keeping straight: the two lists overlap.
`Pawn_Aggrobot_SeekerNade` carries **both** fields, so the union is **20**
distinct classes, not 21.

### 13-G. Verification run for this session

    cargo test --workspace              243 passed, 0 failed (corrected later)
    cargo clippy -- -D warnings         clean
    cargo fmt --check                   clean
    validate_corpus.py                  215/215, malformed 0, five totals exact
    check_export_baseline.py            OK, 3 counters cross-check their files
    check_corpus_baseline.py x4         OK (12.10, 12.11, 13.00, 13.02)
    validate_metrics_corpus.py          16/21 sections exact on all 11 replays

Re-run at 13-J (2026-08-02), same list plus the new decode-error guard:

    cargo test --workspace              243 passed, 0 failed  (242 was stale)
    cargo clippy -- -D warnings         clean
    cargo fmt --check                   clean
    validate_corpus.py                  215/215, malformed 0, five totals exact
    check_decode_errors_corpus.py       215/215, decode errors 0
    check_export_baseline.py            4 explained drift lines, then updated
    check_corpus_baseline.py x4         OK (12.10, 12.11, 13.00, 13.02)
                                        13.02 re-pinned at a preserved demo
                                        after the game rotated Saved\Demos
    compare_combat_report.py            ALL INTERESTING SHAPES MATCH
    validate_metrics_corpus.py          16/21 sections exact on all 11 replays

"16/21 held" is the weak form of that last line and is not what was checked.
`out/xval_summary.json` was diffed cell by cell against the run from before
this change: all **231** cells (11 replays x 21 sections) are identical, and
the only key that moved anywhere in the file is `elapsed_s`. A section
flipping exact -> non-exact and another flipping back would leave the count at
16; it cannot leave the matrix identical. Diff the file, not the total.

The export baseline was updated twice in this session -- both times because the
guard caught a counter move unprompted, and both times the move was explained
before the baseline was rewritten. Row counts never changed; only byte sizes and
overlay counters did.

### 13-H. Stale figure corrected

The C# repo's "17 uncommitted entries" figure in the brief is stale. That work
was committed as fe5343a; the tree is now clean at f67ea66 on
`local/vrfkit-descriptors`, with `main` still untouched.

### 13-J. The ability pawns and projectiles got descriptors [DONE 2026-08-02]

Closes 13-F. Twenty actor classes replicated `ReplicatedMovement` or
`ReplicatedGravityDirection` with no descriptor on either side. They now have
one, on C# branch `local/pawn-descriptors` (ced9379, based on f67ea66).

Nothing here is a new layout. Every (wire name, type) pair is copied from a
descriptor that already declares that name, and the field-name census from
`fields.parquet` was checked against the candidate descriptor per class first:

| wire names on the class | descriptor reused |
|---|---|
| bReplicateMovement, Owner, Instigator, PlayerState, Controller, ReplayLastTransformUpdateTimeStamp, ReplicatedGravityDirection, ReplicatedMovementMode, bCrouchHeld | `GenericAgentDescriptor` |
| ReplicatedMovement + Owner + Instigator | `MageWallDescriptor` / `NeonTunnelDescriptor` (Byte) or `DarkCoverAbilityDescriptor` / `CoveAbilityDescriptor` (Short) |

Counters on 02d4d478, before -> after, and they close exactly:

```
Decoded OK      358,184 -> 364,101   +5,917
Raw/Skip         71,427 ->  72,060     +633
Not in table    525,839 -> 519,289   -6,550   = 5,917 + 633
No field name    33,529 ->  33,529        0
Rows offered    988,979 -> 988,979        0
Decode errors         0 ->       0
```

The +5,917 is `ReplicatedMovement` 5,216, `ReplicatedGravityDirection` 30,
`Owner` 267, `Instigator` 265, `ReplicatedMovementMode` 41, `Controller` 33,
`bReplicateMovement` 30, `PlayerState` 24, `bCrouchHeld` 11. The +633 is
`ReplayLastTransformUpdateTimeStamp`, which `GenericAgentDescriptor` declares
`Ignore()`, on the four pawn classes: 506 + 57 + 54 + 16. Joined row by row,
the change is purely additive -- **0 rows lost a value and 0 changed one**.

**The rotator quantization is the one thing not on the wire.** It is a
per-class descriptor choice and the two readings differ by 8 bits per set
rotator axis, so a wrong choice makes the strict decoder EOF or leave residual
bits. Decided per class by which reading consumes every payload to its exact
end: 13 classes ByteComponents (short-wide fails on 6%-100% of payloads),
1 class ShortComponents (`Pawn_Aggrobot_SeekerNade`; byte-wide fails on 4 of 6).

Three classes -- Clove's `GameObject_Smonk_NewSmoke`, `_PDS` and
`GameObject_Smonk_Q_DecayExplosion` -- **never replicate a rotation at all**, so
both readings consume the same bits and produce the same values. Say that
plainly rather than claiming the wire chose: on this corpus the choice is
unobservable. It is bounded, not guessed. Flipping all three to ByteComponents
and re-running the whole 215-replay corpus leaves **every overlay counter
identical** (decoded OK 83,467,121, decode errors 0) and 02d4d478's
`fields.parquet` **byte-identical** (SHA-256 4c9f02f8...). They take the C#
builder default.

An earlier draft of this section justified that by "all three sibling
smoke/zone descriptors take the default". **That was false**, and this
repository had already proved it false: `apply_type_corrections.py` rewrites
`ProjectileSmokeScreenDescriptor` (Viper) from Short to **Byte**, because a
Short read runs off the end of 137 payloads, and its comment calls the bare
`.ReplicatedMovement()` call "an oversight rather than a deliberate
difference". The real state of the precedent is split: `DarkCover` (Omen) is
Short but unobservable itself (0 of 7,007 rotator flags), `CoveAbility` (Astra)
is Short and contradicted by nothing in the corpus but absent from 02d4d478,
and `ProjectileSmokeScreen` is Short and **wrong**. The builder default is the
only clean tiebreaker, and if these three classes are ever seen replicating a
rotation, that correction is the precedent to check first.

Two independent checks that the values are real, not merely bit-exact:

  * **gravity.** All 30 newly-covered rows decode to exactly `(0,0,-1)`,
    magnitude 1.0, on all four classes -- identical to all 10 rows the seven
    agent classes already decoded. Exact consumption proves nothing here
    (`FVectorNetQuantizeNormal` is a fixed 48 bits and every row is 48 bits),
    so the value is the only evidence.
  * **movement location.** Decoded per class and compared against the spawn
    coordinate `actors.parquet` already carries for the same class. The
    *undivided* quantized integer lands on the spawn coordinate class by class:
    `Projectile_Wraith_4_Smoke` raw x [-4197, 6635] against spawn x
    [-4196.9, 6634.6]; `GameObject_Smonk_NewSmoke` [-1242, 6119] against
    [-1241.9, 6118.6]. Two decoders derived independently -- vrf-decode's Rust
    and a Python reimplementation written for this task -- agree to the row:
    the Python probe predicted 553 failures for a ShortComponents read of
    `Projectile_Wushu_4_Smoke` (495 EOF + 58 residual) and the Rust build
    reported exactly 553.

**A pre-existing scale quirk this surfaced, deliberately NOT changed.** The C#
`ReplicatedMovementDecoder` reads the location with `VectorNetQuantize100`,
i.e. an unconditional divide by 100. The wire's 7-bit vector header carries the
component width and an is-integer flag but *not* the scale, and the two are not
consistent across classes: 18 of the 19 classes here quantize to whole units,
so the divide makes `location` **100x smaller than the true world coordinate**,
while `Pawn_Aggrobot_SeekerNade` quantizes to two decimals (component width 21
instead of 14) and the divide lands correctly. This is not introduced here --
`Zone_Wraith_4_Smoke` and `EquippablePickupProjectile` already decoded this way
and were matched member-for-member against the reference on 8,610 shared keys
in 13-B. Changing it would break that agreement, and no metric section consumes
the field. Recorded as a known divergence from world coordinates, not a bug to
fix silently.

**Left alone, deliberately:**

  * `215` / `216` on all 20 classes (1,099 + 1,112 rows). These are the
    anonymous actor bookkeeping handles, and the wire sends them **3 bits**
    wide, not 32 -- `FlameWallDescriptor` and `EquippablePickupProjectile`
    declare them `Int32()` and `apply_type_corrections.py` already rewrites
    exactly those to `EnumRemainingBits`. Declaring them here would mean
    extending that correction list to 13 more group paths, which is a separate
    change with its own evidence. Not attempted.
  * `bAIControlled`, `Started Planting`, `SeekingActive` on
    `Pawn_Aggrobot_SeekerNade` (1 bit each). No existing descriptor declares
    these names, so there is nothing to reuse and typing them would be
    invention. Left undeclared.
  * The 359 group paths in `fields.parquet` with zero overlay entries -- 93 of
    them `/Game/Characters/*_C` -- are untouched apart from these 20. The
    brief's framing as "21 actor classes" double-counted
    `Pawn_Aggrobot_SeekerNade`, which carries both fields.

**New guard, and it was driven to failure before being trusted.**
`tools/check_decode_errors_corpus.py` exports every replay and asserts
`Decode errors: 0`, because `vrfkit validate` never prints the overlay counters
and `validate_corpus.py` therefore could not see a decode error and never
could. Proven both directions: with `Projectile_Wushu_4_Smoke` flipped to
ShortComponents it exits **1** naming 16 offending replays on a 20-replay slice
(7,801 errors; 75,286 over the full corpus, 160 of 215 replays affected), and
exits **0** on the restored build. `Decoded OK` fell by exactly the error count
in both runs.
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

### 14-A. Live effect decoder guard (fb41b96, 23fb6aa)

The brief's count of eight Rust examples was stale: `effect.rs` contains nine.
The new `tools/check_effect_decoder.py --check` runs those nine through the
live Python decoder, adds two independently expected C# reference-bundle cases,
and pins Python's partial-list malformed-input contract. All 12 pass. Review
found that flipping bit 0 in the three one-byte empty arrays was observationally
unchanged by that partial-list contract. Commit 23fb6aa flips the second bit
(mask 0x02) for those empty payloads and pins every named case. All 12 deliberate
corruptions now produce exit 1; the unmodified set produces exit 0.

### 14-B. Untyped-row investigation and descriptor extraction (e1eb220, b68baaa, b10467b, b5b74db, 519de0b, 81d4f88, 45223c9)

This is a historical pre-preservation measurement. Every count below uses the
explicit denominator: 871,595 rows with every
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
runtime agent-cache entries. Fresh raw generator output before the 24 pinned
type corrections is 152 groups / 1,100 name entries: Raw 164, Skip 154, Typed
782, plus 84 separately sorted explicit-handle aliases. Task B deleted no
previously generated name key.

Fix round 2 (b10467b) makes an explicit descriptor category override take
precedence over an inherited Agent category, matching the C# catalog's
effective `HasFlag(Agent)` filter. This prevents three Ability subclasses on
current master's pawn-descriptor branch from receiving fabricated runtime
ClassNetCaches. Unknown categories and unsupported override syntax now fail
loudly. The f67 input retains the tracked 1,100-entry canonical table.

Fix round 3 (b5b74db) closes the parser boundary exposed by review. A
same-length C# code view masks comments and literal bodies before class and
category discovery, including nested interpolated expressions and braces in
comments. Qualified and `global::` category type/member names are supported;
real unknown or unsupported overrides still fail loudly. Regression tests pin
qualified Ability suppression, `Agent | Ability`, `All`, comment/string
decoys, and class-boundary braces. At b5b74db the complete Python tool suite
was 13/13. Both live generations retained 29 real Agent caches and excluded
the same three phantoms.

Fix round 4 (519de0b) closes the fail-silent source-parser boundaries then
found by whole-branch review. It recognizes delimiter-counted plain and
interpolated C# raw strings; limits category, Path, and Configure discovery to
direct class members; bounds block and expression-bodied Configure methods;
anchors literal handles to AddPropertyHandle or the discovered handle wrapper;
and requires runtime-cache structural captures to be live code rather than
comments or strings. Qualified/global identifiers remain supported. Alias and
escaped-alias category return types are deliberately unsupported but now fail
loudly with the owning class instead of silently inheriting.

Fix round 5 (81d4f88) binds those structural matches to their actual C# owner.
Raw-wrapper discovery is live-code-only and follows only the declaring class's
base chain. Field names are extracted from aligned raw/code statements, so
comment or string decoys cannot supply a name or type. Runtime caches are
anchored to a live `ClassNetCacheDescriptor` constructor in the innermost
owning type; their unique direct-member factory, returned `RpcDescriptor`
initializer, complete `Name` RHS, and direct owning-class constant must all be
unambiguous. Local factory shadowing, unsupported RHS expressions, and
multi/trailing factory lists fail loudly instead of emitting or omitting data.
Escaped class identifiers and the escaped runtime descriptor type are
normalized. The first bounded review reported Critical 0 / Important 0 /
Minor 0; two exact cross-reviews then found five additional lexical-boundary
cases in the same modified surface.

Fix round 6 (45223c9) pins those five cases. A direct factory-local constant or
containing-method `Func<RpcDescriptor>` that shadows a class member now fails
loudly. Runtime factory arrays find their closing bracket in the live aligned
code view, so `]` inside a comment cannot truncate a multi-factory list. Only a
direct factory-body return can supply the returned `RpcDescriptor` initializer,
and duplicate declarations of an actual raw-wrapper owner cannot merge wrapper
semantics across namespaces. All five fixtures failed before the fix and pass
after it. The single bounded implementation review reported Critical 0 /
Important 0 / Minor 0; the two original cross-reviewers then re-ran only their
exact repros against the committed fix.

This extractor is still a source-subset tool, not a general C# front end. This
round did not add arbitrary escaped member spellings such as `@Path` or
`@Configure`, qualified base-name resolution, same-name raw/typed wrapper
overload resolution, or general ClassNetCache helper/comment/brace grammar.
None of those shapes occurs in either pinned f67 or d2 descriptor input. If a
future source tree introduces one, add a fixture and either support it or fail
loudly before regenerating the table.

The final Python tools suite is 53/53. Fresh raw f67 generation still yields
1,100 names / 152 groups / Raw 164 / Skip 154 / Typed 782 / 84 aliases / 29
runtime caches. After all 24 pinned type corrections, the ordered f67 entries
and aliases are semantically identical to the tracked table. Raw d2 generation
still yields 1,192 / 172 / 164 / 164 / 864 / 84, retains 29 real caches, and
omits the three phantom paths. The raw artifact uses a one-struct-per-line,
pre-correction layout and is not itself the canonical byte format; the tracked
canonical Git blob remains unchanged at SHA-256
`1E9BF29DA6B1B1618CEED8637FBB2628DBEC160976B228039B52228BDAA2DE69`.

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
14,755-row round-trip audit, and the historical pre-production measurement.
Production preservation was later implemented and verified as documented in
section 7-C and the current export baseline.

### 14-D. Complete Rust ASCII enforcement (a0ea2b4, 7e0051f)

Task D translated every tracked Rust comment/doc/diagram to meaning-preserving
ASCII and removed the three BOMs. It restored field/sink text from clean
history and reconstructed lib/pipeline text from surviving context because no
clean historical revision exists for those two files. Pre-cleanup: 61 tracked
Rust files, 44 affected files,
510 affected lines, 8,984 non-ASCII scalars over 28 code points. Post-cleanup:
61 files, zero violations. `tools/check_ascii.py` scans complete raw file
contents, not line-local string patterns. Commit 7e0051f anchors enumeration
and reads to the repository root; root and nested-crate invocations both scan
61 files. The real dirty tree and a planted tracked violation both failed
before the restored tree passed.

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
values described in 14-B. At that delegate checkpoint, Task A and Task D did
not affect export data and Task C was measurement-only, so its ON values were
excluded. After documenting this
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
Python tools tests                       53/53
effect decoder guard                     12/12
ASCII guard                              61/61 tracked Rust files
validate_corpus.py (13.01)              215/215; malformed 0
  totals                                blocks 136,545,822
                                        fields 98,883,979
                                        RPCs 75,571,092
                                        skipped 1,972,080,670
master check_decode_errors_corpus.py     215/215; unreadable 0; decode errors 0
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
build_1302 baseline and decode-error guard. Preserve b10467b, b5b74db,
519de0b, 81d4f88, and 45223c9: together they apply the nearest explicit
category override, safely discover qualified overrides outside
comments/literals/raw strings and nested types, scope wrapper/factory/name
resolution to live owning declarations, reject ambiguous lexical shadows, and
prevent three phantom Ability ClassNetCaches. Then run that extractor against
the d2b76f2 worktree,
followed by type corrections and rustfmt. A temporary clean generation with
45223c9 produced exactly 1,192 names / 172 groups / Raw 164 / Skip 164 / Typed
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

---

## 15. The untyped tail, triaged (2026-08-02)

860,384 of 02d4d478's 1,240,444 `fields.parquet` rows carry no decoded value,
across 406 groups. Section 14-B triaged the five largest (756,053 rows). This
covers the remaining **386 groups / 104,331 rows**, which nobody had looked at.

### 15-A. Bottom line: nothing in the tail is an extractor bug

**Zero case-1 findings.** No group in the tail is one the C# describes and our
generator fails to read. No near-miss spellings, no dead handle aliases, no
cross-replay anomalies.

    verdict                                          groups     rows
    no descriptor for these FIELDS (group is bound;      57   36,294
      residue is C# Ignore(), .Decode()-Raw, or
      names C# never declares)
    no C# descriptor for the class                     215   34,723
    no ClassNetCacheDescriptor for the class            46   20,870
    stably-named subobject, class not on the wire       68   12,444
      (7-H territory)

The hunt was run two independent ways and both came back negative:

- **Name diff, C# to table.rs**, using a from-scratch extractor rather than
  `extract_descriptors.py` so a bug in the generator could not hide itself.
  142 paths / 1,125 name pairs; only 3 names absent from the table, none
  recovering a row. That independent extractor was validated against the two
  known shapes first -- it recovers `AddDeathFields(32)`'s four names (13-D)
  and base-class `AddSharedFields()`.
- **Method-agnostic**, comparing the reference's emitted payload keys against
  our all-null groups. Two apparent hits, both false: `TeamEconomy` and
  `RoundResults` on `BombGameState_C`, where we emit the array container as Raw
  *and* fully decoded children (`RoundResults[0..17].{RoundNumber, RoundResult,
  WinningTeam, WinningTeamRole}`, 72 rows, 0 null). A representation
  difference, not a loss.

A second apparent class -- "the reference decodes a key we have no row for", 16
pairs -- is entirely the reference's JSON dropping Unreal's `b` prefix. Its
`CrouchHeld` is our `bCrouchHeld`: 326 rows on Hunter_PC, zero null.

**The `AggroBot` casing bug (13-C) is not present again.** 386 tail paths
against 158 C# path literals, normalised to lowercase with separators stripped:
zero groups match only after normalisation. Fuzzy match at ratio >= 0.93: zero.

### 15-B. One dead table entry, and it was a real upstream gap [FIXED, 8824794]

`/Script/ShooterGame.AresGameStateBase:MulticastResetForRespawn` declares
`SpawnTransform` as `FieldType::Transform`. It is the **only** `Transform`
entry among all 1,192 and it is never hit on any of the 11 replays.

The descriptor's model of the wire is wrong.
`MulticastResetForRespawnParameters.cs:16-22` declares one `FTransform?
SpawnTransform` via `AddProperty(...).Transform()`. The replay's net field
export declares **four** handles for that group:

    handle 0  ShooterCharacter    173 rows, all null (Raw on both sides)
    handle 1  249                 173 rows, all null
    handle 2  Translation         173 rows, all null
    handle 3  Scale3D             173 rows, all null

So the transform is replicated as separate named components, not as one
`FTransform`. `SpawnTransform` matches nothing, and 519 rows stay raw.

**This is fixable and the evidence for it already exists in the table.**
`MulticastAddSmokeScreenPoint` declares both `Translation` and `Scale3D` as
`FieldType::VectorDouble` -- the same names, with a settled type. That is the
13-J pattern exactly: the wire supplies the names and an existing descriptor
already declares them.

`249` is unnamed and unknown. An `FTransform` is rotation + translation +
scale, and the other two are accounted for, so rotation is the obvious
hypothesis -- but it is a hypothesis, and the falsifiable test is the strict
one this repository already has: a wrong type produces `Err` and moves
`Decode errors` off zero. Do not type it on the strength of the reasoning
alone.

The reference loses the same three fields: its `events.ndjson` emits
`MulticastResetForRespawn` 173 times with a payload of `{"ShooterCharacter":
{BitCount, Data}}` and nothing else. This is upstream absence, not our
regression.

FIXED 2026-08-02 on `local/vrfkit-descriptors` (8824794). `SpawnTransform`
is replaced by `Translation` and `Scale3D`, both `.FVector()`, copied from the
smoke-screen declaration rather than chosen. Handle 249 is left unnamed, for
the reason above.

Downstream: overlay decoded 369,395 -> 369,741, exactly +346 = 2 fields x 173
invocations, not-in-table down by the same 346, raw/skip unchanged. Decode
errors stay 0 across all 215 replays -- the falsifiable half, since a wrong
type would fail to consume the payload.

The values are physically sensible, not merely well-formed. `Scale3D` is
`(1,1,1)` on all 173, which is what a respawn should carry, and `Translation`
has 48 distinct map coordinates, all 173 of which fall inside the bounding box
of the spawn coordinates `actors.parquet` independently carries.

The C# test that pinned this function now pins both new names, and was driven
to failure first: removing the `Translation` declaration fails it by name,
restoring it passes 86/86.

Vacuity, stated so the figure is not read as stronger than it is: `Scale3D`
has exactly one distinct value. `Translation`'s 48 carry the evidence.

### 15-C. Bomb_CombatReportComponent [CLOSED -- not a gap]

Flagged as an unverified lead when this triage was written, then checked. It is
the same verdict as the rest of the tail: upstream absence, and we are ahead of
the reference rather than behind it.

SUPERSEDED 2026-08-02 by f5feb82 and ce02f1a, see 18. Both halves of the next
paragraph are now false: the null count fell from 11,641 to 3,032 when the
walker started asking the overlay table for leaf types, and "a handle we have no
name for" no longer describes anything -- the REPLAY names all 70 handles this
group declares, and the leaf label now comes from that declaration. What
survives is the verdict: those handles are ones the C# reference does not read,
and we keep their bits where it discards them.

11,641 of its 21,268 rows carry no decoded value. They are not overlay-table
misses -- this group is decoded by `decode_struct_array` against
`COMBAT_ROUNDS_SCHEMA`, and `array.rs:355` labels any handle the schema does not
name as `_h{handle}`. So every null row is a handle we have no name for.

**The decisive check is whether those are handles the C# reference names and we
failed to transcribe.** They are not. `CombatRoundReports.cs` dispatches on an
explicit handle set at each nesting level, and the unnamed handles are exactly
the complement of it, with **zero overlap at every level**:

    level                   handles we render as _hN        C# reads
    Rounds[].Reports[]      6,7,8,9,99..102,104..108        5,10,98,103
    ...Interactions[]       14,15,16,17                     11,12,13,18..26,61,96
    (one level deeper)      27..35,62..70                   -- (none)

Our schema names every handle the C# names and no more. The reference does not
read these fields either: the member names in its own `events.ndjson` are the
28 the dispatch table produces, and `_h`-style entries appear nowhere in it.

Where we differ is that **we keep the bits and the reference does not**: all
10,671 unnamed leaves carry their `raw_bits`, 303,448 bits in total, so this
component is reinterpretable from an archived Parquet the day someone maps
those handles. The reference discards them.

That also explains `compare_combat_report.py` reporting ALL INTERESTING SHAPES
MATCH without contradiction -- both sides decode the same named set, and the
comparison covers exactly that set.

The typed overlay entries that go unhit on this group are its second decode
path, superseded by the struct-array path. They are dead weight, not a defect.
No count is given: three measurements of it produced 17, 22 and 27 depending on
the matching rule, so the figure is not reliable. See 16-A.

### 15-D. What this triage does NOT establish

Recorded because the strength of the evidence is uneven across the table, and
reading it as uniform would be wrong.

**Direct reference evidence covers 35 of 386 groups / 29,606 of 104,331 rows.**
The other 351 groups rest on a reading of the C# source. In particular: the
reference emits only 41 distinct `export_group_path` values under
`parse_profile: viewer` against 475 declared groups, so "absent from both the
emitted set and the filtered summary" -- 143 groups, 31,869 rows -- means the
profile never asked for it, **not** that the reference tried and failed. That
bucket is not parity evidence.

**The reference bundle is `parser_version 1.0.0+2d2e05e`**, which predates both
`f67ea66` (13-C) and `f0dd7e7` (13-J). Its `was_decoded: false` set still
contains AggroBot and the pawn classes current source covers. Every verdict
above rests on the current C# source; the bundle is corroboration only.

**The 20 head groups (756,053 rows) were out of scope here**, including the 15
that 14-B did not reach.

**Unhit-entry sweep**: 195 of 1,192 entries plus 84 aliases go unhit across 11
replays. 102 are `Skip` and 26 are `Raw`, so they are unhit by construction.
All 84 `OverlayHandleEntry` aliases resolve to `(group, handle)` pairs the
replays really declare; 2 are load-bearing, none dead.

RETRACTED 2026-08-02: this paragraph used to end "the typed remainder are
struct-member names reached under nested spellings; `SpawnTransform` is the
only genuinely dead typed entry." **That was relayed from a subagent report and
written down without being measured, and it is false.** Between 166 and 175
typed entries never resolve in groups the wire does present, depending on how a
descriptor path is matched to its wire group, and nobody has separated "dead"
from "optional RPC parameter that no invocation carried". See 16-A. What
survives is the narrower fact in 15-B: `FieldType::Transform` occurs exactly
once and never resolves.

---

## 16. Falsification pass over this session's own claims (2026-08-02)

Ten claims made today were handed to a dedicated adversarial audit with
instructions to break them. Nine survived. One did not, and it is one this
document asserted rather than one the code does.

The audit's method is worth recording because it is reusable: it rebuilt the
Rust overlay classifier in Python from `overlay.rs` and `sink.rs` -- including
`find_rpc_param_group_path`, since the Parquet `group_path` is **not** the
overlay key for RPC rows -- and reproduced **six** pinned counter sets exactly
from raw Parquet plus the manifest, with no rebuild. That is an independent
implementation agreeing with the parser, not the parser agreeing with itself.

### 16-A. REFUTED: "SpawnTransform is the only genuinely dead typed entry"

Section 15-D said that. It is false, and worse, **it was never measured -- it
was relayed from a subagent report and written down as fact.** That is the
exact failure this document keeps recording in other people's work.

Measured across the 11 cross-validated replays, with exact `(group_path,
field_name)` matching plus nested and array-index spellings:

    typed entries in table.rs                     871
    never resolved on any of the 11 replays       273
      of which in groups the wire does present    166   (method-dependent)

SUPERSEDED IN PART, 2026-08-02: the table has since gone 872 -> 864 typed
entries (19-A removed 8 that could never match), and the leaf-name and
nine-bit-framing fixes moved rows into groups. The never-resolved counts above
were measured before all of that and have NOT been re-derived. Treat the
*method* finding -- that the count depends on an unsettled matching rule -- as
the durable part, and re-measure before quoting any number here.

The audit, using a different rule for mapping RPC descriptor paths to wire
groups, measured 175 never-resolved and 90 in exercised groups. **The
disagreement is the finding**: the count depends on how a descriptor path like
`DamageableComponent:MulticastNotifyDamage_Point` is matched to the wire group
`DamageableComponent_ClassNetCache`, and neither rule is obviously right.

Much of the residue is legitimate. An RPC parameter that no invocation happens
to carry is absent, not dead -- the two largest buckets under my rule are
`MulticastNotifyDamage_Point` (37) and `_Base` (27), which are exactly that
shape. So the honest statement is:

**The number of genuinely dead typed entries is unknown, larger than one, and
nobody has separated "dead" from "optional and unexercised". Do not quote a
figure for it until someone does.**

The commit's narrower sentence survives: `FieldType::Transform` occurs exactly
once in the table and never resolves (15-B).

Also retracted: 15-C and the tail triage both say "22 typed entries" on
`Bomb_CombatReportComponent`. Three measurements give 17, 22 and 27 depending
on the matching rule. The figure is not reliable and has been removed rather
than replaced.

### 16-B. Vacuity disclosures for the parity claims

All nine surviving claims were confirmed, but several "N of N match" figures
rest on values with almost no variety. Recorded so nobody quotes them as
stronger evidence than they are:

    claim                          non-trivial evidence
    spawn locations                1,441 distinct locations, 1,120 rotations
    ReplicatedMovement             location 6,581 distinct, velocity 1,603,
                                   rotation 1,595 -- but 5 of the 8 members are
                                   single-valued across all 8,610 payloads
    static actor paths             148 distinct class paths, 148 archetypes
    MulticastNotifyDamage_Base     4 fields, distinct values 2 / 2 / 2 / 1
                                   (IsQueued is false on all 51)
    gravity "all 40 read (0,0,-1)" 1 distinct value, 369/369 across 11 replays

And on 13-C: 3,077 of the 3,077 rows that moved to raw/skip are
`ReplayLastTransformUpdateTimeStamp`, which the descriptor declares `Ignore()`.
85% of that "recovery" is data the descriptor deliberately discards. The
commit says so; the summary figure alone does not.

### 16-C. Two gaps nobody claimed [OPEN]

**The reference emits an export group we emit nothing for.** On
`/Game/Characters/_Core/BaseReplayController.BaseReplayController_C` -- the
RepLayout group, not the `_ClassNetCache` -- the reference emits 20
`export_group_received` events. We emit **zero rows**. `table.rs` already holds
`PlayerState` and `SpawnLocation`; they are dead for want of rows, so this is a
parser-side gap, not an extractor one.

Not a regression: the count is 0 in exports from Jul 31, Aug 1 and Aug 2 alike.
It is the **only** reference-only export group in the whole bundle.

INVESTIGATED 2026-08-02, narrowed but not closed.

**The gap is two fields, not twenty events.** 19 of the 20 reference events
carry `payload: {}`. The reference emits one event per content block; we emit
one row per field, so a block with no readable fields is zero rows here by
design and the two models cannot agree on those 19. Only the event at t=8
carries anything: `PlayerState: 4` and
`SpawnLocation: {2382.22, -10417.90, 400.00}`.

**CORRECTED 2026-08-02, later the same day. Everything from here to the end of
this entry was wrong, and the mechanism is now known at bit precision. See 17-A.
The short version: the byte IS consumed, our framing was NOT correct, and the
experiment below failed because it was one bit short of nine.**

**One hypothesis was tested and refuted by measurement.** The replay declares
four handles on that group (216, 215, PlayerState, SpawnLocation), so the data
is on the wire. `pipeline.rs`'s net-player-index byte is consumed only when
`pc_guids` already contains the archetype or actor GUID, and for GUID 2 -- the
first dynamic actor, which opens before any package-map export declares the
controller's path -- that set is still empty. `read_dynamic_actor_spawn_data`
has an explicit `|| actor_net_guid.0 == 2` fallback for exactly that reason and
the index-byte check does not, which looked like the same asymmetry.

Adding the fallback **made it worse**: 7 real subobject rows disappeared
(`ViewTargetComponent.bIsActive`, `ScreenTransitionComponent.bIsActive`,
`FreeCamComponent.bIsActive`, `ReplayEffectComponent.bIsActive`,
`SpectateInOrder`, `FreeCamPlayspaceComponent`, and one preserved payload) and
one garbage row appeared. So the byte must NOT be consumed there, and our
current framing of that bunch is correct. Reverted.

**Worth recording: `skipped` went DOWN and that was corruption, not progress.**
The broken build read 236 more bits and had 4 fewer not-in-table rows, while
`malformed` stayed 0. Every headline counter moved in the direction that reads
as an improvement. Only a row-level diff against the previous export showed the
loss. Do not accept a drop in skipped bits as evidence of anything.

**What is still unexplained.** The reference decides the index byte from a
static descriptor registry -- `ReadNetPlayerIndexStage.cs` asks
`GetExportGroupKind(path) == PlayerController` against
`ReplicationClassPath`/`ArchetypePath`/`ActorPath` -- where we ask whether a
GUID was seen in a package-map export. Different mechanisms that happen to
agree on the byte. Why the reference then reads a property block on that
channel and we do not is not established.

Size check before anyone spends more on this: the recoverable data is one
`PlayerState` reference and one spawn location for the spectator camera. No
metric consumes either. `SpawnLocation` is a value `actors.parquet` already
carries independently.

**`spawn_scale` and `spawn_velocity` are read and then dropped.**
`pipeline.rs:617/634` reads both; nothing writes them to any artifact. The
reference carries 466 non-zero velocities and 3 non-unit scales on 02d4d478.
So 13-A's fix is correct at source for all three vectors, and observable for
one. Adding the other two is a schema change; worth doing only if a consumer
wants them.

---

## 17. The controller's property block, found (2026-08-02)

### 17-A. vrfkit frames one bunch nine bits early [MECHANISM FOUND, FIX PENDING]

16-C asked why we emit zero rows on the `BaseReplayController` RepLayout group
and concluded "the byte must NOT be consumed there, and our current framing of
that bunch is correct". **Both clauses are false.** That entry now carries the
retraction; this one carries the mechanism.

**Two independent divergences, nine bits total, on the one bunch that opens the
controller channel. Either alone is fatal.**

1. `pipeline.rs:638-643` gates the spawn-velocity read behind `is_pc`. The
   reference reads it **unconditionally** -- `NewActorSerializer.cs:69-72` has
   no condition around `SpawnVelocity` at all. **Our comment's premise, that
   "PlayerController actors set bReplicateMovement == false so their spawn data
   omits velocity entirely", is a fabricated invariant.** The bit is on the
   wire with value 0, which is exactly why the reference reports
   `velocity: (0,0,0)` rather than null. Cost: **1 bit**.
2. `pipeline.rs:489-495` gates the net-player-index byte on `pc_guids`, which is
   empty for GUID 2 -- the controller opens before any package-map export
   declares its path. Cost: **8 bits**.

**Why it stayed invisible: the misframed header re-synchronises.** Starting nine
bits early, `content.rs:84-109` reads the velocity bit as `has_rep_layout=false`,
the index byte's bit 0 as `is_actor=false`, the next eight bits as an object GUID
(`0x80` -> 64), and the reference's `is_actor` bit as `is_stably_named=true` --
which returns early *before* the `is_deleted` check, and that is what lets the
ghost header survive. It consumes 3 + 8 = 11 bits where the correct header
consumes 2, so both parsers arrive at the content-bit count at the identical
offset 119, read the same 287-bit payload, and frame blocks 1-9 at identical
offsets. The bunch consumes to exactly 831 bits with zero remainder either way.

So the block is not dropped. It is routed to the ClassNetCache path, where
`function_count == 0` and `field.rs` refuses to walk it -- and since Task C it
is preserved as an unresolved payload under `<unknown:64>`.

**Confirmed independently, without any code change.** The reference reports
`SpawnLocation = (2382.2236328125, -10417.9013671875, 399.999267578125)`. Packed
as three little-endian f64 -- which is what `table.rs` declares `SpawnLocation`
to be on that group -- that byte sequence occurs in the `<unknown:64>` payload at
**bit offset 87**. The same triple as three f32 occurs nowhere. The bits we throw
away are the property block, and the type we already have is the right one.

**Why 16-C's experiment misled me.** It added `|| actor_net_guid.0 == 2` to the
index-byte check alone: eight bits of a nine-bit correction. That lands one bit
off, destroys seven real subobject rows, and produces one garbage row -- which I
read as "the byte must not be consumed" when it actually meant "eight is not
nine". The six subobject rows it destroyed are real; that part of 16-C was right
and its explanation was wrong.

**Ghost plausibility, which is the transferable lesson.** In **6 of 11 replays**
the misframed block is attributed to a named, real-looking component --
`TotemLoadoutComponent`, `SprayLoadoutComponent`, `BasicCombatStatsComponent` --
because GUID 64 resolves to a different real object in each. A wrong frame did
not produce garbage; it produced a plausible component name. `field.rs` already
carries a test warning that plausibility is not evidence of correctness. This is
an instance of it, and it is the third time today that a well-formed value hid a
defect.

**A trap for anyone comparing block counts.** The reference bundle is a filtered
projection, not a census: `ReplayExportSink.cs:96` drops every
`ExportGroupReceived` whose payload is null or undecoded. The reference frames
all ten blocks in that bunch and exports one. Do not diff block counts against
`events.ndjson`.

**The general defect behind the specific one.** `pc_guids` is a parallel,
incomplete mirror of path knowledge the cache already holds:
`vrf-schema/src/reader.rs:204` writes GUID paths straight into the cache from
chunk-level exports, bypassing the intercept sink that fills `pc_guids`. The
cache knows GUID 9 -> `Default__BaseReplayController_C` at packet 0 while
`pc_guids` does not. Any check keyed on `pc_guids` inherits that blind spot.

**Measured effect of the fix** (run and reverted): per replay **-1 row** (the
`<unknown:64>` preserved payload) and **+4 rows** on the controller's RepLayout
group, handles 3, 12, 14 and 18. `PlayerState` and `SpawnLocation` match the
reference exactly on all 11 replays. `actors`, `movement` and `net_guids`
unchanged. All tests pass -- nothing pinned the old behaviour.

NOT YET APPLIED. The proper form of divergence 2 replaces `pc_guids` with a
cache path lookup and `is_player_controller_path`, which needs a lookup method on
`GuidPathSink` and therefore touches `sink.rs`, which another session was
rewriting when this landed. Apply both together -- one without the other is the
one-bit-off failure above. Do not apply divergence 1 alone.

### 16-D. Corrections to this session's own supporting text

**A fourth class never replicates a rotation.** 13-J enumerates three classes
whose rotator quantization is unobservable because both readings are
bit-identical. `Zone_Wraith_4_Smoke` is a fourth -- 27,438 rows, one distinct
rotation value, all zero -- and it is declared `ShortComponents`. The safety
argument for the other three is otherwise confirmed: every `ByteComponents`
class has 7 to 22,844 distinct rotations, so the discriminator really does fire
where it can.

**13-J's coverage is per-replay, not corpus-wide.** "0 untyped" holds on
02d4d478. Across the other 10 replays 290 of 659 gravity rows and 34,765 of
126,973 movement rows are still untyped, in classes those replays contain and
02d4d478 does not.

**The ReplicatedMovement join key is not unique.** `(time_ms, group path, actor
GUID, object GUID)` collides 17 times on our side with differing payloads, zero
times on the reference's. All 17 fall outside the 8,610 shared keys so the
result stands, but a last-wins dict join was luckier than it was careful.

**`check_corpus_baseline.py` SKIPs on a missing corpus and exits 0.** So the
13.02 failure mode that 3a4b04a fixed -- a game-owned directory changing under
the baseline -- would have passed *silently* had the directory been emptied
rather than repopulated. It was caught because the game left three different
files behind. That is luck, not the guard working.

### 16-E. What the audit did not check

- The tail triage's "zero extractor bugs in 386 groups" (section 15-A) was not
  independently reproduced; doing so means redoing that triage's own work.
- The merge's "231/231 cells identical" was not re-derived; only the current
  16/21 state was confirmed.
- The 215-replay corpus runs were substituted with 11-replay evidence.

---

## 18. `Ping` -- encoding settled, deliberately not typed (2026-08-02)

`Ping` on `/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C` is the
largest single wire-declared field with no table entry: **222,855 rows** across
the 11 cross-validated replays, declared in 11 of 11 manifests, `bit_count == 16`
on every row. **The string `Ping` appears nowhere in the C# reference**, so
unlike every field typed so far there is no existing declaration to copy from.

### 18-A. The encoding

**A fixed 16-bit little-endian unsigned integer**: `raw[0] + 256 * raw[1]`.

Not IntPacked, refuted on two independent axes:

    test                            Ping                 16-bit NetGUID controls
    byte 0 continuation bit set     111,438 / 222,855    8,774 / 8,774
                                    50.00%               100.00%

A coin flip against a field that is genuinely IntPacked in the same group at the
same width. The second axis is stronger still: UE RepLayout only sends a
property that **changed**, so a correct reading must give nonzero consecutive
deltas. Zero-deltas per channel are 0.010% under little-endian and **32.7%**
under IntPacked -- the wire says the value changed on a third of the updates
where the IntPacked reading says it did not.

Byte order is settled by **continuity across the 256 boundary**, not by variance
analysis. Big-endian is ~256x little-endian on 99.985% of rows, which is nearly
an affine transform, so F and eta-squared cannot discriminate it -- do not cite
them for byte order. Across the 47 crossings, little-endian traces smooth spike
ramps (`18, 85, 157, 262, 460, 550, 237, 365, 17`) where big-endian jumps
incoherently (`4608, 21760, 40192, 1537, 52225, ...`) over the identical rows.

"A byte plus another field" is refuted too: the 33 high-byte rows sit in 14
channels across 10 replays, and rows adjacent to them have byte-0 p90 of 157
against 22 corpus-wide. The high byte only fires inside latency spikes, so it is
the top of the same number.

The surviving candidates are one candidate, not four. `SerializeInt(ValueMax)`
for any max in (35017, 65541] reads exactly 16 bits and yields the identical
integer as uint16 LE, so uint16 / int16 / SerializedInt / a 16-bit quantized
read are indistinguishable here. Bit 15 is never set (max 2249), so **signed
versus unsigned is unobservable on this corpus.**

### 18-B. It behaves like latency in milliseconds

    min 5   p5 9   p50 14   p90 22   p99 36   p99.9 79   max 2249
    191 distinct values, mod-4 histogram uniform (so NOT ms/4 compressed)
    per-replay medians 11-18, minima 5-9, consistent across all 11
    10 actors per replay, ~1,700-2,500 updates each, dt_ms p50 626
    65.1% of updates change by exactly +/-1; 91.2% by <= 3

One actor's opening series -- `39, 34, 21, 12, 11, 12, 11, 12, 13, 12, 11, 9` --
is a connection settling to a stable 12.

**The units are inferred, not established.** The shape is milliseconds and ms/4
is ruled out, but nothing in the corpus or the reference bundle pins the scale:
the reference carries no latency figure at all. Per-replay eta-squared ranges
0.021-0.669; quote the range, not the pooled F.

### 18-C. A reusable lever: checksums as a type-equality test

The manifest's `compatible_checksum` is a CRC chain over the property name and
type. CRC is affine, so `ck_A ^ ck_B == M_L(c_A ^ c_B)` is an **exact
type-equality test that needs no knowledge of the type string**. Validated on
positive controls: `PlayerId`/`CompetitiveTier`/`NumUltimatePoints` match at
L=24; the `b`-prefixed booleans at L=24; six object references at L=32.

`Ping`'s checksum (1482599241, identical in all 11 replays) matches **none** of
them at any L in 0..400. So it is not int32, not float, not bool, not uint8, not
an object reference, not `FRepMovement` or `FUniqueNetIdRepl`. The base rate was
checked before leaning on this -- 979 of 1,768 pairs are singletons, so "Ping is
a singleton" alone would be weak; the *elimination* is what carries.

This technique generalises to any untyped field and nobody had used it before.

### 18-D. Not typed, and why

`FieldType::SerializedInt { max: 65536 }` would work -- verified by reading
`read_serialized_int`, which computes `value_bits = max.ilog2()` = 16, reads 16
bits LSB-first, and satisfies `decode_field`'s `NotFullyConsumed` guard. No new
enum variant needed.

It is still the wrong move today:

- It breaches the standing rule twice. The rule is that a field gets a type when
  the wire supplies the name **and** an existing descriptor declares that name's
  type. Neither half holds: no descriptor declares `Ping`, and its meaning is
  inferred from behaviour.
- `table.rs` is generated, so this needs a C# descriptor entry on the branch
  (the 13-J pattern) or a new `apply_type_corrections.py` rule. Real cost.
- **No metric consumes it.** The reference has no latency figure to compare
  against, so there is nothing to be right or wrong about downstream.

The encoding is recorded here as established. Typing it becomes a short job the
day something wants a per-player latency series -- and the thing that would
settle the units is an external latency source to correlate against, not more
analysis of these bits.

### 18-E. Not checked

The other 204 replays -- "declared nowhere else" is scoped to 5,445 export
groups across 11 manifests. Non-Bomb modes. Signedness, which this corpus
cannot observe. The literal UE type string: meet-in-the-middle on the checksum
saturates, with 68^6 candidates against 2^32 giving ~23 expected random
collisions.

---

## 19. Generator and array-walker fixes (2026-08-02)

### 19-A. The extractor never read `ExportGroupKind` [FIXED, 18dce16]

`tools/extract_descriptors.py` flattened every C# descriptor into
`(path, property_name)` overlay rows as if all of them were `Kind = Actor`. Two
kinds cannot be:

- **`FastArray`** describes an array element struct, never an export group at
  all -- `RemoteCharacterUpdate`, 2 entries.
- **`AttributeSet`** declares one property per attribute where the wire
  replicates a generic `BaseValue`/`CurrentValue` pair per attribute subobject
  -- `AresAttributeSet`, 6 entries.

8 entries that could never match anything. **The only generator bug among
16-A's 122 dead entries**; the other 114 are C# build drift the reference
cannot decode either.

**The brief for this task was wrong where it mattered, and the fix caught it.**
It asserted "every C# descriptor declares a Kind". Four
`ExportGroupDescriptor` subclasses with live paths never override it and take
`Unknown` from the parameterless constructor's default -- `CoveAbility`,
`DarkCoverAbility`, `ProjectileSmokeScreen`, `SmokeScreenManager` -- and their
fields decode on the corpus today. **A dispatch that defaulted to "drop" would
have deleted them, and every headline counter would still have looked fine.**
It is now a policy table with one row per enum member, and an unclassified Kind
is a hard failure rather than a silent default.

All seven enum members appear in the C#: Actor 35, Component 19, ClassNetCache
17, PlayerController 1, Unknown 1, AttributeSet 1, FastArray 1.

Table 1,193 -> 1,185 entries, Typed 872 -> 864; Raw 157, Skip 164 and all 84
handle aliases untouched. Regenerating from the C# reproduces 1,185 exactly.

**The proof is that nothing moved.** All four Parquet files are SHA-256
identical to a pre-change export and every overlay counter is unchanged. That
is load-bearing rather than incidental: `not_in_table` holding at 511,916 is
exactly what a removed entry that HAD been matching would have disturbed. The
export baseline drifted on nothing; there was no `--update` to explain.

The argument for each removal is **structural, not statistical** -- which is
what separates it from the trap 16-A records. An unexercised RPC parameter is
absent, not dead; these 8 cannot be overlay keys by construction.

9 tests added, each driven to failure first: 8 mutations, 8 caught, generator
byte-identical after each restore.

Disclosed rather than changed: phase 3c iterates Kind-bearing classes but gates
on the Agent category instead of Kind. No effect today -- both filtered
descriptors are Movement/Ability, and the delta being exactly 8 proves it.

### 19-B. The array walker asked a second copy for its types [FIXED, f5feb82]

The combat-report walker typed leaves from a hardcoded handle->type match in
`sink.rs` -- a duplicate of what the generated table already holds, and
incomplete. It now asks the table first, keyed on the name **the replay
declares** for that handle, since UE flattens an array's element members into
consecutive handles on the enclosing group and `resolve_field_name` was already
reading exactly that. The hardcoded match remains as a floor.

8,609 leaves gained a value on 02d4d478; null leaves on that group fell from
11,641 to 3,032. Row-level diff: identity columns identical, 8,609 value cells
newly filled, **0 lost and 0 altered**, all inside
`Bomb_CombatReportComponent`.

**I suspected this was wrong before trusting it, and the suspicion was
misplaced.** The declared names repeat -- `StateRemainingTime` at handles 7, 15,
32 and 67 -- and game-clock names on a per-report leaf looked like a lookup
landing in the wrong handle space. It is not: UE flattens the same struct at
several nesting positions, so one member name legitimately maps to one type at
several handles.

What settled it is consumption, not plausibility. Per handle the bit count is
single-valued and matches the declared type -- 32 for the floats, 8 for the
enum byte, 192 for `DeathLocation`'s three doubles -- with **zero decode
failures**. A wrong type here does not produce a wrong value; it fails to
consume the payload and the row stays null. The three `HUDConfig` handles still
read that way (16 bits, 1,043 rows, 0 decoded), unchanged from before.

## 20. Route B closed: the actor path now does the leaf lookup (2026-08-02)

`sink.rs`'s three group-path resolvers were asymmetric. The class path
(`resolve_subobject_group_path`, primary) and the subobject path (same
function, secondary) each fell back to `NetGuidCache::unique_leaf_match`
before giving up and returning the raw NetGUID path. The actor-GUID fallback
in `resolve_actor_group_path` did not: it tried the lookup keys and then
`return actor_path.to_owned()`. So a static actor whose own path is an exact,
unique leaf of a declared group -- `AresWorldSettings` against
`/Script/ShooterGame.AresWorldSettings` -- shipped as a bare instance name
with every field it carried unnamed. 7-H measured that gap at 1,183 rows over
the 11 cross-validated replays and called it route B.

The fix is the same four lines the other two call sites already have,
including the `!is_cnc || ends_with("_ClassNetCache")` guard.

### 20-A. What moved, at row level

`out/xval` was exported before and after and the two `fields.parquet` trees
compared POSITIONALLY, not by join: same row count, and every one of
`time_ms`, `packet_id`, `channel_index`, `actor_net_guid`, `object_net_guid`,
`handle`, `bit_count`, `raw_bits` identical at every position in all 11
replays. That proves the 1:1 row correspondence rather than assuming it, so
differences in the remaining columns can be attributed to specific rows.

```
rows that changed group_path: 1,183
rows that gained field_name : 1,183
rows that gained a value    : 0
rows that LOST a value      : 0
rows whose value CHANGED    : 0
```

The eight moves, all on actor blocks (`object_net_guid` null on every one),
none of them to a `_ClassNetCache` group:

```
   945  AresWorldSettings              -> /Script/ShooterGame.AresWorldSettings          exact leaf
   196  Switch_BlackMarket_2           -> /Game/Interactable/...Switch_BlackMarket_2_C   "_C" arm
    14  MinimapSiteA                   -> /Game/UI/...MinimapSiteA_C                     "_C" arm
     8  SoundBarrier                   -> /Game/Blueprint/SoundBarrier.SoundBarrier_C    "_C" arm
     6  MinimapSiteB                   -> /Game/UI/...MinimapSiteB_C                     "_C" arm
     6  MinimapSiteC                   -> /Game/UI/...MinimapSiteC_C                     "_C" arm
     6  AmbientAudio_Jam_OS_Peacock_2  -> /Game/Audio/...Peacock_2_C                     "_C" arm
     2  BombDestination_C              -> /Game/GameModes/Bomb/BombDestination.BombDestination_C
 -----
 1,183
```

This split was PREDICTED before the after-run, from two independent
measurements -- a pre-fix instrumented build counting per-actor-path blocks,
and a per-bare-name row count over the before export -- and reproduced term by
term, not just in total.

### 20-B. Why the binding is right, and where the evidence is weaker

All 1,183 rows gained a `field_name`. `resolve_field_name` returns `Some` only
when the handle indexes a POPULATED declared slot of the bound group, so
1,183/1,183 named means zero out-of-range and zero empty-slot handles. A wrong
group is what produces those.

The names and their wire widths corroborate independently:

```
/Script/ShooterGame.AresWorldSettings   handle 15  32 bits  WorldGravityZ
                                        handle 17  32 bits  TimeDilation
                                        handles 3, 12  3 bits  "216" / "215"
Switch_BlackMarket_2_C                  handle 15  64 bits  LastUsedTime
                                        handle 18  64 bits  GameplayStartTime
                                        handle 16   1 bit   HasPlayed
                                        handle 17   1 bit   IsDisabled
                                        handle 19   1 bit   LeverDown
```

`WorldGravityZ` and `TimeDilation` are exactly the two floats an Unreal
`AWorldSettings` replicates; the booleans are 1 bit and the timestamps 64.
Handles 3 and 12 carry the 3-bit `216`/`215` pair every actor replicates.

WEAKER HALF, stated as such: 947 of the 1,183 rows bind through the exact-leaf
arm (945 `AresWorldSettings` + 2 `BombDestination_C`). The other 236 bind
through `unique_leaf_match`'s `"_C"` arm, which is beyond C# and rests on the
Unreal convention that a level-placed Blueprint instance takes its class's
name. `Switch_BlackMarket_2` is 196 of those 236; 7-H's CNC-derived route
reached the same actor independently, which corroborates the identification
from a different direction. The remaining 40 rows are five map props and
minimap markers.

### 20-C. The ClassNetCache path is untouched, by construction and by count

`resolve_function_count` runs its own instance-name resolver
(`resolve_cnc_for_instance_name`) and only while `current_group_path` is still
a bare name, so a RepLayout group returned by the new call would silence it and
hand `ReadSerializedInt` the wrong capacity. It cannot: `by_leaf` keys are the
text after the last `.`, and the two suffix arms append `Component` and `_C`,
so `path.ends_with("_ClassNetCache")` can only hold if the actor's own path
ends with `_ClassNetCache`.

Measured, not just argued. The instrumented pre-fix build counted 315
ClassNetCache actor blocks across the 11 replays where the leaf rule would
fire; every one matched a `_C` group, so the guard rejected all 315. Zero rows
with a `_ClassNetCache` target appear in the row diff.

### 20-D. Counters, and the arithmetic closing

Per replay, and in total over the 11:

```
overlay_no_field_name  -1,183
overlay_not_in_table   +1,183
overlay_decoded_ok          0
overlay_decode_errors       0
overlay_raw_skip            0
overlay_rows_offered        0
```

No non-overlay counter moved on any replay: chunks, packets, export_groups,
content_blocks, rep_layout_blocks, class_net_cache_blocks, fields, rpcs,
actor_opens, actor_closes, bunches, malformed_packets, skipped_bits,
movement_rows and net_guid_rows are all identical. That is the shape a
reclassification has and a reparse does not.

The five corpus totals over 215 replays are unchanged: blocks 136,545,822,
fields 98,883,979, rpcs 75,571,092, malformed 0, skipped 1,972,080,670.
Corpus-wide the overlay moves `not in table` 116,447,827 -> 116,469,301
(+21,474) with `decoded OK`, `raw/skip` and `rows offered` all unchanged, so
21,474 rows move across the full corpus against 1,183 on the 11.

### 20-E. Metrics and bundle

All 231 cells of `out/xval_summary.json` (11 replays x 21 sections) carry an
identical verdict before and after, and 16/21 is unchanged. Stronger: every one
of the 147,041 metric leaf values across the 11 `metrics.json` files is
identical. 7-H's "no metric section changes" is now measured, not predicted.

The BUNDLE does change, and only in one way. `movement.ndjson` and
`manifest.json` are identical on all 11 replays. `events.ndjson` has the same
line count on all 11, and 1,077 `export_group_received` lines differ; on each,
exactly two keys change -- `export_group_path` from the bare instance name to
the declared group, and `payload` from `{}` to the named fields. No event
appears, disappears or changes type. `_group_path_to_class` and
`_group_path_to_archetype` are NOT reached: they live in the adapter's legacy
fallback for a missing `actors.parquet`, and `guid_class` is filled from
`actors.parquet`'s spawn `class_path`, which this change does not touch.

The C# reference bundle cannot corroborate this. Its `events.ndjson` for
02d4d478 carries 117,841 `export_group_received` events over just 41 distinct
export group paths -- a curated gameplay whitelist -- with zero bare-name paths
and none of the eight groups above.

### 20-F. The 7-H safety audit does NOT transfer, and this is why

7-H's "87 spoke, 87 correct, 0 mismatches" was route A (handle-set uniqueness)
applied to groups whose class was already known. The actor-path analogue --
for every actor block resolved by a HIGHER-priority branch, does
`unique_leaf_match(actor_path)` stay silent or agree? -- was run on an
instrumented build over all 11 replays and has a population of THREE:

```
4,926,463  actor blocks with no actor path at all (archetype/package resolved)
        3  actor blocks resolved above the fallback that DO have an actor path
           (2 RepLayout, 1 ClassNetCache), all three leaf-none
    1,667  actor blocks that fell through to the raw actor path where the leaf
           rule fires (1,352 RepLayout -> bound; 315 CNC -> rejected by guard)
   13,844  actor blocks that fell through where it does not fire
```

Zero disagreements out of three opportunities is not evidence. The population
is empty because an actor GUID only has a path when it is a static actor, and
static actors are exactly the ones that fall through. So the safety argument
for this path rests on two other legs instead:

1. Uniqueness. `AMBIGUOUS_LEAF` makes the rule bind or stay silent, never
   choose. Pinned by `an_ambiguous_actor_leaf_binds_to_nothing`, which was
   driven to failure against a naive first-match implementation before being
   kept.
2. Handle coverage. 1,183 of 1,183 bound rows landed on populated declared
   slots (17-B).

### 20-G. What this did not check

- Non-Bomb game modes, still input-blocked (section 11-A).
- Checkpoint chunks, still never parsed (7-H's own standing caveat).
- Whether the 20,291 corpus-wide moved rows outside the 11 cross-validated
  replays bind correctly. Only the 11 were diffed at row level; the other 204
  replays were checked at counter level only (five totals unchanged, decode
  errors 0). The exposure is bounded but not zero: `AresWorldSettings` moves
  85-86 rows on every one of the 11, so at ~86 x 215 roughly 18,500 of the
  21,474 corpus-wide moves are the exact-leaf binding on the group with the
  strongest independent evidence, leaving the weaker `"_C"` arm under about
  14%. What the counters cannot see is the failure this would produce: a row
  bound to a wrong group whose handle misses a populated slot keeps
  `field_name = None`, stays counted in `no_field_name`, and the arithmetic
  closes identically. That count is a measured 0 of 1,183 on the 11 and an
  inference on the other 204.
- `README.md`'s type-overlay block was stale here and left alone deliberately
  rather than half-corrected. FIXED SINCE, at 4624fef -- and it turned out to be
  stale in three blocks, not one: the overlay figures, the test counts (header
  190 against 252, five of eight per-crate rows wrong), and a decode-error
  sentence still scoped to 20 sample replays. Both blocks now carry the command
  that produces them.

---

## 21. Array leaf names now come from the replay (2026-08-02)

f5feb82 made the combat-report walker ask the overlay table for leaf TYPES,
keyed on the name the replay declares for each handle. It left the NAMES on
`COMBAT_ROUNDS_SCHEMA`, a hardcoded handle->name map, so the job was half done
in two visible ways: 8,609 recovered leaves shipped as `_h{handle}`, and where
the schema did name a handle it sometimes contradicted the wire -- the schema
calls handle 3 `RoundNumber`, every replay declares it `RoundNum`.

`ce02f1a` closes it. `decode_struct_array` now takes the group's declared names
indexed by handle, and `resolve_leaf_label` orders them declaration -> schema ->
`_h{handle}`. The schema stays as the floor for handles a replay does not
declare, and it still decides the NESTING: container path segments keep their
schema name, because handles 44 and 79 both declare `RegionalDamageInteractions`
and the wire cannot tell those apart where the schema can.

### 21-A. What moved, at row level

All 11 cross-validated replays exported before and after and compared
POSITIONALLY, not by join. Same row count on every replay, and all NINE of
`time_ms`, `packet_id`, `channel_index`, `actor_net_guid`, `object_net_guid`,
`group_path`, `handle`, `bit_count`, `raw_bits` identical at every position.

```
rows that changed field_name : 193,241   (15,560 on 02d4d478)
rows that gained a value     : 0
rows that LOST a value       : 0
rows whose value CHANGED     : 0
groups touched               : 1  (Bomb_CombatReportComponent, all 11 replays)
movement/actors/net_guids    : byte-identical on all 11
```

49 distinct path shapes move, and the set is IDENTICAL on all 11 replays --
union 49, intersection 49, no replay carrying a shape another lacks. That is the
whole mapping, not a sample from one replay.

The split is 35 / 14, counted from the diff rather than by eye:

- **35** are `_hN` placeholders gaining a real name. 32 of those are the
  `HUDConfig` / `StateRemainingTime` / `GameTime` / `GamePhase` quartet at its
  eight nesting positions (handles 6-9, 14-17, 27-30, 31-34, 62-65, 66-69,
  99-102, 105-108); the other three are `DamageType` at 35 and 70, and `_h104`
  -> `DeathLocation`.
- **14** are named leaves the wire corrects, spelling out **12** distinct
  renames -- `IsKill` and `IsWallPen` each occupy two shapes, at handles 49/84
  and 48/83. The twelve: `RoundNumber`->`RoundNum`, `Subject`->
  `ParticipantSubject`, `Team`->`ParticipantTeamName`, `CharacterIcon`->
  `ParticipantCharacterIcon`, `KillerPlayerState`->`ParticipantsKillerState`,
  `DidKill`->`bDidKill`, `Died`->`bDied`, `WasKiller`->`bWasKiller`,
  `IsKill`->`bIsKill`, `IsWallPen`->`bIsWallPen`, and Riot's own typos
  `DamageReceived`->`DamageRecieved` and `HitsReceived`->`HitsRecieved`.

An earlier draft of this section and of ce02f1a's message said 34 / 15. That was
counted by hand and was wrong; 35 + 14 = 49 is what the diff says.

Per-column check, stronger than the row diff: of the 14 columns in
`fields.parquet`, exactly ONE changed its compressed bytes -- `field_name`,
675,934 -> 677,634. The other 13, including all four value columns, are
byte-identical page for page. That is the whole of the single export-baseline
DRIFT line (13,584,643 -> 13,586,343 bytes, row count unchanged).

### 21-B. The bundle deliberately does NOT follow, and that is the load-bearing half

The declared names are NOT unique within one flattened element. UE flattens the
same four-member struct at eight nesting positions, so handles 6, 99 and 105 all
declare `HUDConfig` at the Reports level, and 27/31 and 62/66 pair up one level
down. A bundle payload is a JSON object built by last-wins assignment, so keying
it on the declared name MERGES those.

Measured on the pre-change export, before any of this was written: 3,405 of
20,298 distinct payload paths would collapse. Then measured directly, by
building one bundle with the adapter mapping deliberately disabled: the
combat-report leaf values in `events.ndjson` fall from 26,640 to **22,417**.
4,223 values destroyed, and NOTHING in the sweep would have seen it --
`fields.parquet` stays 1:1 because only `field_name` moves, and
`compute_metrics.py` reads none of these names, so 16/21 and all 231 cells stay
green over the loss. This is another change whose damage is invisible to every
counter, the third recorded this session.

So `to_valplay_bundle.py` keys combat-report leaves on the HANDLE: the C#
reference's member name where it has one, `_h{handle}` -- the label the bundle
already carried -- where it does not. `fields.parquet` keeps the `handle` column
and loses nothing; this projection has no such escape hatch. That is the "no
hardcoded names in the parser" invariant working as intended, with the renaming
table in the adapter where labelling is a presentation concern.
`compare_combat_report.py` imports that same table rather than growing a second
copy that could drift.

Result: all 44 bundle files (11 replays x 4) are BYTE-IDENTICAL before and
after, and all 231 `xval_summary.json` cells carry an identical verdict. 16/21
unchanged.

### 21-C. Guards, each seen failing

- The two behaviour tests in `array.rs` were driven to failure first against a
  `resolve_leaf_label` that ignored the declaration. The other three -- schema
  fallback, handle past the end of the declaration, container segment keeps its
  schema name -- passed from the start, which is correct: they pin what must NOT
  change.
- `test_to_valplay_bundle.py`'s three new cases fail (3 of 9) when the wire name
  is passed through instead of mapped.
- `compare_combat_report.py` goes red on three shapes against a deliberately
  wrong handle-20 mapping and a removed handle 22.
- The `_raw` / container-handle exclusion fails its own test when removed.
- The byte-identity of the bundles is itself falsifiable and was falsified: with
  the adapter mapping disabled, 02d4d478's `events.ndjson` loses 4,223 leaf
  values while keeping all 576,247 lines. A byte-identical bundle is therefore
  a result, not a tautology.

### 21-D. What this did not do, and what it corrects

- The corpus totals were re-measured on an UNMODIFIED HEAD binary because two of
  the five in this document were already stale: `fields` 98,884,839 (doc said
  98,883,979) and `skipped` 1,972,018,965 (doc said 1,972,080,670). db42e6a
  moved both and refreshed every build baseline without updating the prose. The
  before and after runs of this change agree exactly on all five, so the change
  moves none of them -- but the sweep instruction to expect the documented
  numbers would have failed for the wrong reason. QUICK START is corrected;
  the dated per-section records are left as the historical measurements they are.
- 15-C is contradicted and is marked SUPERSEDED in place.
- Only the 11 cross-validated replays were diffed at row level. The other 204
  were checked at counter level only (five totals unchanged, decode errors 0
  across all 215).
- `README.md`'s type-overlay block was stale when this landed; fixed since at
  4624fef (see 20-G).
- The container segments still carry schema names, so the emitted path is a
  hybrid: `DealtInteractions` is ours, `DamageDealt` under it is the wire's.
  Deliberate -- the wire declares `DealtIteractions` (Riot's typo) at handle 26
  and the same `RegionalDamageInteractions` at both 44 and 79, so adopting the
  declaration there would lose the one distinction the schema provides and buy
  nothing.
- **The schema's LEAF names are now unreachable for this group, and the text
  above should not be read as saying otherwise.** "The schema stays as the floor
  for handles a replay does not declare" is true as a rule but currently vacuous
  here: the replay declares all 70 handles, and a sweep of the after-export
  finds ZERO leaves still labelled `_hN` across all 11 replays. So only
  `COMBAT_ROUNDS_SCHEMA`'s `sub_arrays` and its container names still do work;
  its leaf `field_names` entries fire for nothing this corpus contains. They are
  kept because a replay that declares a shorter group would need them, which is
  a claim about robustness, not one about current behaviour.
- Two rows the walker SYNTHESISES are excluded from the adapter's relabelling
  after review: `emit_remaining_raw`'s `_raw` row (handle u32::MAX, which would
  have become `_h4294967295`) and the depth-limit container row (which would
  have become `_h4`). Both are unreachable -- a sweep of all 11 after-exports
  finds zero `_raw` rows -- so this closed an edge rather than fixing a defect,
  and the byte-identical bundles were already evidence neither fired.

---

## 22. Five parallel agents: three new tables of data, one closed door (2026-08-04)

Measured at `5c46851`. Five agents ran concurrently in isolated worktrees --
four implementing, one investigating. None was allowed to touch
`tools/baselines/`, this file, or `README.md`: five `--update` runs against five
different trees would have destroyed the one guard that catches counter moves.
Baselines were re-measured once, here, after integration.

### 22-A. What landed

| Agent | Change | Result on `02d4d478` |
|---|---|---|
| A | Event chunks -> `events.parquet` | 195 rows / 10,201 B |
| B | movement `timestamp` / `movement_state` / `move_type` | 11 -> 14 columns, rows unchanged |
| C | manifest metadata incl. the 219 KB loadout blob | 10 -> 32 keys |
| D | effect containers for 9 RPC families | 53,908 rows gained `value_str` |
| E | checkpoint format investigation (read-only) | spec, no code |

Integration order A -> C -> B -> D. One conflict, an import line in
`roundtrip.rs` (A added `Int32Array`, B added `UInt8Array`); `sink.rs` merged
clean because B and D were confined to disjoint regions by their briefs.

### 22-B. Every pre-existing counter held

This is the load-bearing verification, not the feature list. After all four
merges the export baseline drifted in exactly six places, all of them new
surface:

```
effect_blobs_decoded   None -> 53,908
event_rows             None -> 195
events.parquet         new: 195 rows / 10,201 B
fields.parquet bytes   13,586,343 -> 13,742,379   (+156,036, +1.15%)
movement.parquet bytes 30,695,141 -> 31,835,557
```

Unmoved: content blocks 608,020, fields 429,637, RPCs 342,735, movement rows
1,839,607, NetGUID rows 16,167, skipped bits 17,506,923, all five overlay
counters, and both `actors.parquet` and `net_guids.parquet` byte-for-byte.

Corpus-wide, also unmoved: blocks 136,545,822, fields 98,884,839, rpcs
75,571,092, malformed 0, skipped 1,972,018,965. Decode errors 0 across all 215
replays -- which now also covers the effect decoder, since its failures route
through `overlay.decoded_err`. `validate_metrics_corpus` held at 16/21 with all
four shot-dependent sections still exact.

### 22-C. The Event chunk corroborates the 13-kill claim from outside the parser

The README's headline -- 13 kills the C# parser drops -- rested on our own RPC
extraction. It now has an independent witness in the same file.

```
Event chunk characterDeath                          132
MulticastNotifyKilledEnemy.KillerCharacter          132 over 10 characters
  character 576                                      13
C# reference                                        119 over  9 characters
                                                    132 - 119 = 13
```

The payload's two words are the killer and killed NetGUIDs: 132/132 matched in
that order, 0/132 reversed, with the RPC landing 8-9 ms after the event. The C#
parser never opens these chunks (`ReplayChunkDispatcher.cs:152`, "Skipping event
chunk"), so this is not a comparison of two readings of the same bytes.

Scope: the killer/killed pairing is ONE replay. The chunk framing is all 215
files, 43,397 chunks, consumed with zero bytes left over.

### 22-D. Checkpoints do NOT unlock AbilitiesAndBuffsComponent

Section 7-H named checkpoint chunks as the only unexamined region, and this
session went in expecting them to be the key to the 97.3% unattributed-bits
ceiling. THEY ARE NOT.

```
checkpoints scanned         4,024  (all 215 files)
export-group records        1,955,988
paths containing AbilitiesAndBuffs      0
paths containing Buff                   0
_ClassNetCache groups, 02d4d478   ReplayData 147 / checkpoint 147 / cp-only 0
```

`AbilitiesAndBuffsComponent` does occur in checkpoints -- as a NetGUID object
path, which is a different namespace from `NetFieldExportGroup` and one vrfkit
already has. The server never sends that class's ClassNetCache layout anywhere
in the file. Do not reopen this via checkpoints.

The format itself is fully decoded (zero unexplained bytes, zero violations
over 4,024 samples); the spec is in the session scratchpad, and the
implementation is sized at 4 files changed + 1 new (~150 lines), ~24 s to parse
the corpus. Three of this session's own hypotheses were refuted in the process:
the frame starts at `w0 + 8` not `w0 + 7`, the cache does NOT need pre-seeding
from ReplayData (checkpoint frames contain zero net-field-export records), and
there is no variant encoding (stock `iter_demo_frames` parses all 4,024).

Checkpoint schemas contribute only 15 new named handles out of 3,226, and the
frames are full-state snapshots whose actor set is a subset of ReplayData's on
both spot checks. That made "redundant" the expectation. **It is wrong -- see
22-I, which measured it.**

### 22-E. Three things the agents got right by pushing back

- `mode_flags` is not a fourth column. It is assigned from `movement_state` at
  the single `MovementMove` construction site, so it would have been a
  byte-identical copy over 1.84M rows. Three columns shipped, not four.
- `movement_state` is not the posture byte, on this build. Constant 0 across all
  1,839,607 rows, and `move_type` constant 1. Posture already ships as
  `bCrouchHeld` (2,480 rows, 1,241 true / 1,239 false). The brief asserted
  otherwise and was wrong. Not a decoder bug: a shifted header would have broken
  position and yaw, which match C# to zero error.
- `MulticastStopContinuousEffect` carries no EffectContainer. It was named as a
  target on the strength of its untyped bit count; its parameters are scalars
  (`SourceID`, `EffectID`, `StopMovementTime`) that need overlay entries, not a
  blob decoder. A missing-descriptor problem wearing a blob-shaped hat.

D also caught its own near-miss before reporting: pinned handle constants
produced 53,908 structurally valid, fully-consuming, residual-zero arrays of
null tag/value pairs, and read one function's float payload as a tag index.
Nothing caught it -- not the residual check, not the tests, not clippy. What
caught it was measuring the fraction of elements with a non-null tag and value:
0%. The pair is now derived per blob.

### 22-F. Silent-change holes closed during integration

Two of them, both found by comparing what moved against what should have:

- The effect pass moved no counter. It runs after the overlay buckets are
  decided, so 53,908 newly valued rows left `Decoded OK`, `Not in table` and the
  rest exactly where they were. The only trace was a larger `fields.parquet`,
  and a byte count cannot distinguish values from padding. `Effect blobs:` is
  now reported separately -- not folded into `Decoded OK`, which would
  double-count rows already sitting in `Not in table`.
- `check_export_baseline.py` could not see a fifth Parquet file. It drives
  itself off `PARQUET_FILES`, so `events.parquet` was not measured, not diffed,
  and not asserted against: adding it left the file unguarded rather than
  failing. `events` added, `Event rows` cross-checks against its own row count,
  and the pass message now counts the identity list instead of stating "3" --
  it had already gone stale at four.

Also: `test_dir()` in `roundtrip.rs` put every checkout's fixtures in one
directory under the system temp dir, so two trees running `cargo test` at once
overwrite each other. It surfaced during this session as a column-count
mismatch that reads exactly like a schema bug. Now keyed by
`CARGO_MANIFEST_DIR`. `python_interop.py` was separately already broken at HEAD
-- its expected column list had gone stale against `fields_schema()` -- and
nothing ran it, because Cargo does not run `.py` files.

### 22-G. Stale comments corrected, and one latent bug recorded

- `vrf-container/src/info.rs`: the timestamp is Unreal `FDateTime` ticks
  (100 ns since 0001-01-01), not the Windows FILETIME the doc claimed. As
  FILETIME, `02d4d478` dates to 3626 instead of 2026-07-25.
- `vrfkit/src/oracle.rs`: the module doc still reported corpus pass rate
  100.000000% and 3,671 skipped bits. Those were retracted when the silent-drop
  path was exposed in 5-A/5-C; the comment kept presenting them as current for
  a whole session.
- LATENT, recorded not fixed: `ReplayInfo::network_version` is documented
  "always 19 for supported replays" and nothing validates it. `02d4d478` carries
  480767974. Separately, the info section's changelist (5090349) disagrees with
  the header's (2152573997); `manifest.json` reports the header's.

### 22-H. What this did not do

- No checkpoint implementation. 22-D explains why, and names the one measurement
  that would change the answer.
- The 46-51 checkpoint-only export-group paths and the checkpoint NetGUID tables
  (17.2M entries corpus-wide) are not exported. E flagged the emission decision
  as a product call and deliberately did not make it.
- Event payload words are raw for 5 of 7 groups. `roundStarted` and
  `switchTeams` mirror `metadata`; `characterUltimateUsed`'s single word is
  unidentified.
- `movement_state` and `move_type` are exported but their meaning on builds
  other than 13.01 is untested -- both are constant on the whole 13.01 corpus.

### 22-I. Checkpoints are NOT redundant: 6-11% of their values differ (2026-08-04)

22-D left one question open, and it was the one that decided whether a
checkpoint parser is worth four files: does any property value in a checkpoint
differ from what ReplayData carried at the same timestamp? Measured, on three
replays. **It does, and not marginally.**

The decision rule was written down before the run: differences whose last
ReplayData write is within a packet-time or two are alignment residue and mean
nothing; differences on keys ReplayData last wrote long before are the
checkpoint carrying state the incremental stream did not re-send.

| | 02d4d478 | 03c60af4 | 03f82073 |
|---|---|---|---|
| checkpoints | 18 | 5 | 21 |
| checkpoint RepLayout fields | 77,812 | 17,036 | 74,882 |
| MATCH | 92.376% | 93.901% | 89.075% |
| VALUE_DIFFER (same width) | 2,140 | 248 | 3,604 |
| WIDTH_DIFFER | 2,393 | 412 | 3,792 |
| NEW (key ReplayData never sent) | 1,399 | 379 | 785 |
| median staleness of a differing key | 78,776 ms | 75,649 ms | 77,622 ms |
| differing keys within 1 s of the checkpoint | 4.7% | 5.8% | 3.5% |
| group-key guard violations | 0 | 0 | 0 |

The median differing key was last written by ReplayData **~76-79 seconds**
before the checkpoint, and only 3.5-5.8% are within a second. By the stated
rule this is not alignment residue. NEW is nonzero and stable on all three.
**Implementation is justified.**

Alignment: the comparison is made at the checkpoint's `Time1`, not at its
position in the chunk stream. Those differ enormously -- checkpoint0 declares
t=47 ms but sits after a ReplayData chunk spanning ~90 s, so a file-order
comparison would have scored ~90 s of legitimate updates as differences and
produced a confident wrong answer.

Key choice was guarded, not assumed. The key is
`(actor_net_guid, object_net_guid, handle)`; the class GUID (or archetype, for
actor blocks) was recorded alongside and cross-checked. **Zero mismatches over
all three replays**, so the key identifies the same group in both streams and
the counts mean what they say.

Three outcomes, not two, because this repository already has precedent for one
value arriving at different widths (byte properties inside arrays write only
significant bits). `WIDTH_DIFFER` is broken out so an encoding artifact cannot
be reported as a correction. Both buckets are real differences; only their
cause may differ.

The differences are **spread, not concentrated**: 5,932 observations over 2,666
distinct keys on 02d4d478, and the largest single actor class is 5.1%
(`BombPlayerState_C`), followed by `Ability_Melee_Base_C`, `Equippable_Unarmed_C`,
`WindowShieldA1`, `RespawningWallPlate_2`, `BombGameState_C` at 4-5% each. This
is systemic, which is what a full-state snapshot versus a relevance-filtered
incremental stream should look like.

**What this does NOT establish.** That the checkpoint is *more correct* than our
reconstruction. The measurement shows the two disagree; it does not adjudicate
them. The natural reading is that ReplayData is relevance-filtered while the
checkpoint is the server's full snapshot, but no test here distinguishes that
from the reverse. Both readings make checkpoints non-redundant, which is the
only claim this section makes.

**Scope.** RepLayout property fields only. The probe sink returns
`function_count = 0`, so the 0-10 ClassNetCache blocks per checkpoint frame are
not walked and nothing may be claimed about them. Three replays, all
`release-13.01`.

Method: `valdiff` in the session probe. ReplayData is walked packet by packet in
time order maintaining `(actor, object, handle) -> (bit_count, raw_bits,
last_write_ms)`; each checkpoint is compared against that state the moment the
packet clock passes its `Time1`. Comparison is bit-exact on `raw_bits`.
