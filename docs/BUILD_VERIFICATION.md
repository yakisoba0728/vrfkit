# Common build verification

This document and its committed report retain measurements at the source
revisions named in each section. The 0.6.0 decoder and counter contract were
measured separately on the archived private corpus; see
[0.6.0 revalidation](#060-revalidation). The 13.01 export and checkpoint
baselines under `tools/baselines` (`*_02d4d478.json`) were remeasured with the
0.6.0 decoder. The `build_*.json` corpus baselines were rechecked with the
0.7.0 decoder: 12.10, 12.11, 13.00, 13.04, 13.05 and 13.06 match unchanged,
and 11.06-12.09, 13.01 and 13.08-13.10 were pinned new. `build_1302.json` was
not rechecked: its replay is not in the archived private corpus.
`metrics_builds.json` was not remeasured with either decoder.

All **1,018 unique available replays**, spanning **24 supported builds**, were
checked with parser commit `259ed10c7e0c9d87c852f80fab7535d38d2e187f`. The two
supplied roots held 1,074 paths; SHA-256 deduplication removed 56 copies.

All 1,018 pass ReplayData validation and checkpoint-enabled export. The oracle
passes **657,565,897/657,565,897** scored main blocks;
**24,015,541** checkpoint blocks were walked. Independent
Python comparisons match **13,387,751** observed values, with no
width failures or typed mismatches. All 24 builds contain positive main and
checkpoint decoding work. No run is counted as passing because it was skipped.

The all-counter acceptance rule gives **1,018/1,018 clean**. The audit command
exits **0**; its failure and `build_errors` lists are empty, and the executable
was unchanged at completion.

The [machine-readable report](../tools/fixtures/build_verification.json)
contains build aggregates, input hashes and the complete finding list.

| Build | Checked | Strictly clean | Scored main blocks | Checkpoint blocks | Values compared |
|---|---:|---:|---:|---:|---:|
| 11.06 | 3 | 3 | 1,983,592 | 73,760 | 38,742 |
| 11.07 | 3 | 3 | 1,987,726 | 72,553 | 40,654 |
| 11.08 | 3 | 3 | 1,969,145 | 76,440 | 45,996 |
| 11.09 | 3 | 3 | 2,158,062 | 78,172 | 41,271 |
| 11.10 | 3 | 3 | 1,839,369 | 67,573 | 47,289 |
| 11.11 | 3 | 3 | 2,205,209 | 83,268 | 48,123 |
| 12.00 | 3 | 3 | 2,031,069 | 77,828 | 37,752 |
| 12.01 | 3 | 3 | 1,905,287 | 72,795 | 38,374 |
| 12.02 | 3 | 3 | 2,027,973 | 77,588 | 59,482 |
| 12.03 | 3 | 3 | 1,944,438 | 74,386 | 40,751 |
| 12.04 | 3 | 3 | 2,425,410 | 86,588 | 58,200 |
| 12.05 | 3 | 3 | 1,886,622 | 69,916 | 51,263 |
| 12.06 | 3 | 3 | 1,642,411 | 58,280 | 45,476 |
| 12.07 | 3 | 3 | 1,934,113 | 77,802 | 38,249 |
| 12.08 | 3 | 3 | 1,977,496 | 69,130 | 38,100 |
| 12.09 | 3 | 3 | 2,170,389 | 74,575 | 40,455 |
| 12.10 | 1 | 1 | 13,680 | 1,501 | 175 |
| 12.11 | 1 | 1 | 6,506 | 1,363 | 112 |
| 13.00 | 1 | 1 | 8,860 | 1,343 | 123 |
| 13.01 | 215 | 215 | 136,874,459 | 4,999,774 | 2,763,537 |
| 13.02 | 205 | 205 | 141,928,372 | 5,117,150 | 2,923,122 |
| 13.04 | 108 | 108 | 67,440,898 | 2,474,063 | 1,357,848 |
| 13.05 | 401 | 401 | 253,658,021 | 9,310,150 | 5,085,060 |
| 13.06 | 38 | 38 | 25,546,790 | 919,543 | 547,597 |
| 13.08 | 3 | 2 | 2,113,274 | 80,615 | 38,141 |
| 13.09 | 5 | 5 | 2,046,446 | 69,917 | 56,720 |
| 13.10 | 28 | 28 | 13,908,176 | 513,867 | 421,297 |

In the historical 1,018-replay audit, preserved unresolved RPC payloads total
**5,589,152 main / 1,666 checkpoint**.
Their associated skipped-bit counters are
**11,054,801,711 main / 377,598 checkpoint**.
The separately reported RPC suffix counters are
**24,926,105 main / 0 checkpoint bits**.
These nonzero populations remain outside a claim of complete semantic decoding.

## 13.08

The first 13.08 supplement comes from a separate run on 2 replays, with
parser commit `f609c3a52fecd918f83951c467a3517eef2f3f39`. Its numbers are not
part of the 1,018-replay totals above. In the
[machine-readable report](../tools/fixtures/build_verification.json), the
top-level `provenance`, `executable_changed`, `build_errors` and `failures`
describe the 1,018-replay run. Each entry of `supplements` is a later run's
complete result for the builds it names. The top-level `builds` rows aggregate
the runs after confirming their input hashes do not overlap; the November 16
follow-up below contributes one additional 13.08 replay.

Both replays pass validation (oracle **1,539,679/1,539,679**) and
checkpoint-enabled export, with no transform, framing, field-stream, array or
typed-overlay failures, main or checkpoint. One replay fails the checkpoint
GUID cross-check, and the audit stops there for that replay: its table
measurements and the independent comparison are not in the report, so the
**19,975** values compared are the other replay's. Run separately on the
failing replay's export, `validate_type_evidence.validate(...,
compare_typed=True)` compares 25,238 values with no width failure or typed
mismatch.

The cross-check finds 2 of 25,861 indexed entries whose path differs from the
main stream's, and only in case. Both are net GUID 9901,
`/Engine/BasicShapes/Sphere`. The checkpoint writes its leaf name as an index
to the literal `sphere`, first written for `/Game/VFX/Radianite/Meshes/sphere`
(net GUID 6787) earlier in the same checkpoint. So the checkpoint shares one
literal between names that differ only in case, which fits Unreal's
case-insensitive `FName` comparison; the writer itself was not inspected. The
reader follows the index rule; the cross-check compares case-sensitively. No
case-only mismatch was observed in the 1,018 exports above.

## 13.09

The first 13.09 supplement comes from a separate run on 4 replays, with
parser commit `7213fc296167bbe79f8502d63a2c2c0c94ba0d87`, recorded as the second
entry of `supplements`. Its numbers are not part of the 1,018-replay totals
above. The November 16 follow-up adds one independently checked replay.

All four replays pass validation (oracle **1,883,501/1,883,501**) and
checkpoint-enabled export, with no transform, framing, field-stream, array,
struct-blob or typed-overlay failures, main or checkpoint. The checkpoint GUID
cross-check finds no differing path, and the independent comparison matches
**50,808** values with no width failure or typed mismatch. The run's failure
and `build_errors` lists are empty.

13.09 adds `WinningTeamMode` and the nested `TeamResults` array to
`RoundResults`; before the decoder named them, every 13.09 `RoundResults` blob
was a struct-blob failure (6--21 per replay, main and checkpoint).

## 13.10

The first 13.10 supplement comes from a run on one replay (SHA-256
`6cb43f11c74c483b84fc64d70ec6bc0642af3ed5ec877a90d1e3353defd4ba97`).
Its branch is `++Ares-Core+release-13.10`; the matching game executable has
SHA-256 `8e6f270fd509b3996685ef986423866796b7b4db12f41eb51c769e50a99a1372`.
The native reader at RVA `0x432e600` supplies seed addend `0x2c249eb3`,
positive initial offset `0x33`, and the word/byte operation sequence in the
transform registry. It is an integer reader, not the arbitrary-length reader
used by `capture_native_transforms.py`. For the native cross-check, emulate it
with the source pointer at reader offset `0x98`, available bit length at
`0xa8`, position zero at `0xb0`, and seed `bit_count ^ 2` at `0xb8`; pass the
destination in `RDX` and the maximum integer in `R8`. Maxima `2`, `127`,
`255`, `0x7fffffff`, and `0xffffffff` make it consume respectively 1, 7, 8,
31 and 32 bits. Mask the destination's unused high bits before comparing it
with the five corresponding fixed boundary vectors. The longer fixed vectors
regress the disassembled step list, but have no independent native capture.

ReplayData validation passes all **1,065,451/1,065,451** scored blocks, with
zero transform, framing and field-stream failures. Export walks all 30
checkpoints with zero transform or framing failures, and independently compares
28,182 observed typed values without a mismatch. The strict audit is **1/1
clean**, with zero main and checkpoint array errors, truncations or leaf decode
errors. The complete supplement in the machine-readable report records the
clean counters and provenance.

The 13.10 replay declares `RegionalDamageInteractions` nested arrays at
CombatReport handles 45 and 81, and `ReceivedInteractions` at 62. Its damage
RPC `LifeChangeEvents` shifts `ChangedComponent`, `LifeResult`, `DeltaLife`
and `bAliveAfterChange` to handles 11--14. All 3,334 observed handle-11
values decode as packed NetGUIDs and resolve to damage section objects in the
replay's GUID table. A pinned 13.10 payload test
checks the nested walk and shifted life-change fields; the full replay export
types the observed regional `Region`, `Hits`, `Damage`, wall penetration, kill
and destroyed armor members.

## November 16 follow-up

A new run on 29 unique local replays checks 27 of 13.10, one of 13.09 and
one of 13.08. None of their SHA-256 identities overlaps an earlier supplement.
All **29/29** pass the same strict gate: **13,579,265/13,579,265** scored main
blocks, **502,931** checkpoint blocks across **396** checkpoint chunks, and
**417,193** observed values compared independently. Main and checkpoint array
errors, truncations and leaf decode errors are zero. The executable is
unchanged at completion. The fourth supplement records the complete run;
the table above aggregates it with the earlier supplements.

This run uses the Rust sources of `6194bde`; the follow-up code changes only
Python bundle naming and comparison. Three pinned public replays (12.10,
12.11 and 13.00) also pass the strict gate. On the additional 13.09 replay,
the bundle's events, movement and manifest are byte-identical before and
after the Python correction. Regression tests additionally cover shifted
regional fields, shifted nonregional fields, unknown declarations, legacy
output and the comparison tool's use of the replay's declarations.

The strict gate's default evidence list does not contain the new regional
damage or life-change fields. An additional Python checker independently
walks their raw parent arrays using the replay's declarations, then compares
the emitted child paths, bit lengths and raw bytes: **3,372,841 leaves**
agree across the 27 new 13.10 exports, main and checkpoint. It also decodes
**804,482 typed values** directly from those raw windows, with zero mismatch:
`ChangedComponent`, `LifeResult`, `DeltaLife`, `bAliveAfterChange`, and the
regional `Region`, `Hits`, `Damage`, `bIsWallPen`, `bIsKill`, `DestroyedArmor`.
Each life-change member has 40,863 main observations. Checkpoint observations
contain the six regional members (91,909 each), but no damage life-change
RPCs; absence is not a successful comparison.

Three negative controls change a typed value, omit a child and change a
child's raw bytes. The checker detects all three. Its source, reports and
mutation runner are retained with the private corpus; the checker SHA-256 is
`ab3bca35a29ac602d3390a9b6b658b4223b883f0d86b84b79a15dd205b7df919`.
The supplement includes the aggregate counts and negative-control results.
These comparisons establish byte/value agreement, not complete gameplay
semantics: among 40,863 `ChangedComponent` observations, one is the zero
sentinel and three nonzero references lack a final GUID path. No reference
or player attribution is invented for them.

## 0.6.0 revalidation

The 0.6.0 source (`8118350f0d632c1169e0f90c4da8887389e4828a`, release build,
Rust 1.86) was run against the archived private corpus: **1,011 unique
replays** of **24 builds** (11.06-12.09, 13.01, 13.02, 13.04-13.06, 13.08-13.10;
every file matched its catalogued size and SHA-256 before the run). This is a
new measurement of the native movement component decoder and the 0.6.0
counter contract; the tables above remain measurements of their own
revisions.

| Build | Checked | Strictly clean |
|---|---:|---:|
| 11.06-12.09 (16 builds) | 3 each | 3 each |
| 13.01 | 215 | 215 |
| 13.02 | 204 | 204 |
| 13.04 | 108 | 108 |
| 13.05 | 401 | 401 |
| 13.06 | 6 | 6 |
| 13.08 | 1 | 1 |
| 13.09 | 1 | 1 |
| 13.10 | 27 | 27 |
| **Total** | **1,011** | **1,011** |

- `tools/verify_build_corpus.py` exits 0 with empty failure and
  `build_errors` lists; `tools/check_decode_errors_corpus.py --checkpoints`
  and `tools/validate_corpus.py` exit 0 on each build folder and on the whole
  set (`replays read: 1011/1011`, `OK: 1011/1011`). All were run with
  `VRFKIT_REQUIRE_CORPUS=1`.
- Main-stream movement components: **1,940,388,746** attempted, the same
  number complete, 0 errors, 0 empty, on all 24 builds. No checkpoint reaches
  the movement decoder (0 attempted), so the checkpoint movement gate is not
  exercised by this corpus.
- `focus_projectiles_declaration_refusals` and `partial_payload_overrun` are 0
  in every main and checkpoint manifest. `FocusProjectiles` emits 53 children
  and 3 empty trailers on one 12.05 replay and 0 children on every other
  build. No export hit a schema resource limit; the largest main-stream group
  count is 602 (limit 4,096).
- Against the released v0.5.1 executable, on 95 replays (all 83 of the builds
  with at most 27 replays, and the first three by file name of 13.01, 13.02,
  13.04 and 13.05): all 13 Parquet tables are byte-identical except
  `fields.parquet` of the one 12.05 replay, which differs only by the 53 new
  child rows; the manifests differ only by the documented counter keys.
- The comparison was checked to fail: changing one `f64` by 1 ULP, one `i64`
  by 1, or deleting one movement row is reported with a nonzero exit.

Not covered: the 13.08 replay with the case-only path difference described
above is not in this archive, so that finding was not re-observed; 12.10,
12.11 and 13.00 have no replay in it; the main-stream field-slot total is not
exposed by the executable.

## Method

`tools/verify_build_corpus.py` recursively discovers each supplied corpus root,
deduplicates by SHA-256, and applies the following checks to every unique file:

1. `vrfkit validate`: a successful exit, an identified branch, positive scored
   block count, and an exact passed/scored match. The rounded percentage alone
   is insufficient.
2. `vrfkit export --checkpoints`: a successful exit and all thirteen Parquet
   tables. Required CLI counters must be present, overlay categories must
   reconcile, and exported row counts and checkpoint GUID-path counts must
   agree with their independent table/manifest measurements.
3. Main and checkpoint quality: no malformed packets, rejected partials,
   unfinished partials, framing/transform/field-stream loss, lost RPCs,
   resource-limit failures, typed-overlay failures, struct failures, movement
   failures, array errors, array truncations or array-leaf errors. Missing or
   invalid counters fail. Positive checkpoint work is checked per build.
4. Independent value comparison: Python decodes the observed fields specified
   by `public_fixture_type_evidence.json` directly from raw payload bits and
   compares them with the Rust-exported typed values. Each replay must contain
   observed evidence. Absent field identities are recorded separately; they
   are not treated as successful comparisons.

**Clean/checked** means the number of replays meeting all four conditions,
divided by every unique replay checked. A successful export alone is not a
strict clean result. Registered build support and a completely clean field
audit are separate measurements.

Unknown whole RPCs preserved as raw payloads are counted separately from RPC
loss. Skipped bits, RPC suffix bits, refused handle conflicts and untyped
properties remain explicit in the report; RPC suffix bits fail the audit only
in the checkpoint pass, where none has been observed. A clean audit does not
imply full semantic coverage, complete understanding of every field, or
verification against what the game displayed.

The audit command also reports `build_errors` and exits unsuccessfully if any
build has no observed checkpoint decoding. Documentation checks require
positive checkpoint block and decoded-value counts, not just an evidence
label. The recorded report carries an empty `build_errors` list, and all 24
builds meet these conditions.

## Reproduce

Build the parser from the revision recorded in the report. Use fresh output
paths and supply all desired replay roots; filenames and folder names do not
determine the build, which is read from validation output and cross-checked
against the export manifest.

```powershell
cargo +1.86.0 build --release -p vrfkit --locked
python tools/verify_build_corpus.py --exe target/release/vrfkit.exe --corpus '<archive-root>' --corpus '<preserved-fixture-root>' --work-dir '<new-private-work-dir>' --output '<new-report.json>' --jobs 4
```

The recorded run used these two roots and `--jobs 12`; the worker count does
not affect aggregation. The report pins the parser commit, Rust source digest,
executable digest, runner digest, evidence specification and input content
hashes; inputs are hashed again after processing. It contains no source paths
or player identities, and is written even when the strict gate exits nonzero.
The source, runner and evidence digests hash working-tree bytes, so they depend
on line endings: compare sources with Git, not by digest alone.

## Arithmetic evidence

The shared replay audit supplements the transform tests, which cover all 27
registered builds. The eighteen builds 11.06--12.09, 13.08 and 13.09 have 79
independently captured native-machine-code cases each; the eight builds
12.10--13.06 have eleven golden cases each; 13.10 has eleven fixed boundary
vectors, of which the 1-, 7-, 8-, 31- and 32-bit cases were checked against its
native reader (`crates/vrf-transform/tests/data/recovered_1310_vectors.rs`).
These remain
different sources of arithmetic evidence, not substitutes for the replay
checks; [LEGACY_BUILD_SUPPORT.md](LEGACY_BUILD_SUPPORT.md) records the 48
samples used while adding 11.06--12.09.
