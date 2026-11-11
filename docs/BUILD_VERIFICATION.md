# Common build verification

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
| 13.08 | 2 | 1 | 1,539,679 | 56,791 | 19,975 |
| 13.09 | 4 | 4 | 1,883,501 | 64,842 | 50,808 |
| 13.10 | 1 | 1 | 1,065,451 | 39,835 | 28,182 |

Preserved unresolved RPC payloads total
**5,589,152 main / 1,666 checkpoint**.
Their associated skipped-bit counters are
**11,054,801,711 main / 377,598 checkpoint**.
The separately reported RPC suffix counters are
**24,926,105 main / 0 checkpoint bits**.
These nonzero populations remain outside a claim of complete semantic decoding.

## 13.08

The 13.08 row comes from a separate run of the same tool on 2 replays, with
parser commit `f609c3a52fecd918f83951c467a3517eef2f3f39`. Its numbers are not
part of the 1,018-replay totals above. In the
[machine-readable report](../tools/fixtures/build_verification.json), the
top-level `provenance`, `executable_changed`, `build_errors` and `failures`
describe the 1,018-replay run. Each entry of `supplements` is a later run's
complete result for the builds it names, and those builds' rows are copied into
`builds`.

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

The 13.09 row comes from a separate run of the same tool on 4 replays, with
parser commit `7213fc296167bbe79f8502d63a2c2c0c94ba0d87`, recorded as the second
entry of `supplements`. Its numbers are not part of the 1,018-replay totals
above.

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

The 13.10 row comes from a separate run on one replay (SHA-256
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
properties remain explicit in the report. A clean audit does not imply full
semantic coverage, complete understanding of every field, or verification
against what the game displayed.

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

The shared replay audit supplements the transform tests. The eighteen builds
11.06--12.09, 13.08 and 13.09 have 79 independently captured native-machine-code
cases each; the eight other builds have eleven golden cases each. These remain
different sources of arithmetic evidence, not substitutes for the replay
checks; [LEGACY_BUILD_SUPPORT.md](LEGACY_BUILD_SUPPORT.md) records the 48
samples used while adding 11.06--12.09.
