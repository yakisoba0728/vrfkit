# Build support validation, 2026-09-24

The latest common all-build audit is [BUILD_VERIFICATION.md](BUILD_VERIFICATION.md).
The results below retain the date and sample scope of this implementation update.

**Status: all sixteen builds from 11.06 through 12.09 have verified payload
transforms.** All 48 available replays pass ReplayData validation and
checkpoint-enabled export. Together with the eight existing versions, the
parser supports 24 exact branches. Unknown branches still fail closed.

## 11.06--12.00 recovery and replay results

The implementation following `6cbf2bec0645c5278c099839d5db9708e9a45f21`
adds seven per-build transforms without changing shared parser behavior.
Resolve its commit with
`git log -1 --format=%H -- crates/vrf-transform/src/versions/v11_06.rs`.
The original binaries have two layers of encrypted code. Both were recovered
fully offline from each matching EXE and `stub.dll`, using bounded native
cipher fragments and independently checked arithmetic. The reproducible
[recovery tool](../tools/recover_native_binaries.py) pins all seven input
pairs and recovered outputs in its [catalog](../tools/fixtures/native_recovery.json).
The [recovery research](BUILD_RECOVERY_RESEARCH.md) records the method and scope.

Each recovered reader was decompiled separately and emulated independently of
the Rust implementation. Its 79 native cases match, for **553 additional
cases** and **1,264 native cases** across all sixteen recovered builds. The
88 upstream vectors remain unchanged. Changing an 11.06 rotation count made
the native comparison fail; restoring it passed. All referenced substitution
tables match the shared S-boxes byte for byte. No installed game or driver
was started or changed.

| Build | Replays | ReplayData scored blocks | Checkpoint blocks | Main typed overlay |
|---|---:|---:|---:|---:|
| 11.06 | 3 | 1,983,592 | 73,760 | 82.5% |
| 11.07 | 3 | 1,987,726 | 72,553 | 80.8% |
| 11.08 | 3 | 1,969,145 | 76,440 | 79.4% |
| 11.09 | 3 | 2,158,062 | 78,172 | 82.7% |
| 11.10 | 3 | 1,839,369 | 67,573 | 82.3% |
| 11.11 | 3 | 2,205,209 | 83,268 | 83.9% |
| 12.00 | 3 | 2,031,069 | 77,828 | 85.1% |

All 21 replays report **100%** ReplayData oracle success over **14,174,172**
scored blocks, and **529,594** checkpoint blocks were walked. Required main
and checkpoint counters are present and reconcile. Malformed/framing,
transform, field-stream, lost-RPC, typed-overlay, movement, array-leaf and
struct-blob failures are zero. Checkpoint skipped bits are zero.

The main overlay decodes **18,544,499 of 22,502,298 offered rows**, with
**1,517,794** decoded checkpoint rows. These ratios measure typed coverage,
not full semantic understanding. Unresolved RPC schemas and untyped properties
remain raw, as on the existing supported builds. They are not hidden as
successful typed values. Independent Python decoding matches **299,827**
values across all 42 main/checkpoint field tables, with no width failures,
missing specifications or typed mismatches. A separate direct bit walk
matches **6,311 TeamEconomy child values from 899 parent rows**. The existing
controller and declared-handle compatibility fixes also cover these samples.

The eight previously supported reference replays were revalidated and
exported with checkpoints. All **104 Parquet files remain byte-identical**.
This is the complete available three-sample-per-build collection, not a claim
that every possible replay or field on these branches has been tested.

To reproduce recovery and native comparison, use this directory layout for
each build: `<root>/<build>/ShooterGame/Binaries/Win64/` containing the
archived `VALORANT-Win64-Shipping.exe` and matching `stub.dll`. The recovered
root is separate; the capture tool uses it only for the seven protected builds.

```powershell
python tools/recover_native_binaries.py --binaries '<archive-root>' --output '<recovered-root>'
python tools/capture_native_transforms.py --binaries '<archive-root>' --recovered-binaries '<recovered-root>' --check
cargo +1.86.0 test -p vrf-transform --locked
cargo +1.86.0 build --release -p vrfkit --locked
vrfkit validate '<replay-root>/11.06/sample-1.vrf' --diagnostics
vrfkit export '<replay-root>/11.06/sample-1.vrf' --out '<exports>/11.06/sample-1' --checkpoints
python tools/validate_type_evidence.py '<exports>' tools/fixtures/public_fixture_type_evidence.json --compare-typed
```

Repeat validate/export for samples 1--3 of all seven builds. Required-counter,
nonzero-work and reconciliation checks use `check_decode_errors_corpus.py`.
Recovery needs optional `pefile`, `unicorn` and `numpy`; ordinary Rust tests
need no game binaries or emulator.

## 12.01--12.09 recovery and replay results

The implementation following base commit `0d7a798` adds nine transforms and
two compatibility fixes. The containing implementation commit can be resolved
with `git log -1 --format=%H -- crates/vrf-transform/src/versions/v12_01.rs`.

Each executable was imported with Ghidra 12.1.2, using PE runtime-function
records and chained unwind entries to identify complete function boundaries.
The PRNG multiplier, reader object layout and caller argument shape located
the candidate readers. The replay reader was checked independently by
emulating its original x86-64 instructions with Unicorn, including its native
bit-copy helper. No game process was launched. The catalog records the exact
[executable SHA-256 and reader RVA](../tools/fixtures/native_transform_readers.json).
All three 256-byte S-box tables referenced by the applicable readers match
the existing tables byte for byte.

There are 79 native cases per build: the eleven upstream staging boundaries,
36 cases covering six edge seeds at six lengths, and 32 deterministic random
cases up to 512 bits. All **711 expected-byte cases** match Rust, in addition
to the unchanged 88 upstream golden vectors. Reversing one 12.09 rotation's
count makes the native test fail; restoring it passes. The capture tool
verifies executable hashes before emulation and rejects failure to return,
instruction/time exhaustion, or an incorrect reader position.

Two real-corpus failures required fixes beyond the arithmetic:

- 12.01--12.06 call the replay controller `BaseJanusController`. Recognizing
  that exact class consumes its net-player-index byte before framing. Without
  this, the first bunch and checkpoint controller bunches lose data. The new
  pipeline regression fails before the name fix and passes afterward.
- 12.01--12.05 declare `TeamEconomy` members at handles 53--55, rather than
  56--58. Their names and checksums match the later members. The exporter now
  selects by the replay's declaration, including hardcoded FName `241` for
  the IntPacked replication ID. Unknown declarations still fail. The original
  fixed-handle API remains available for compatibility.

| Build | Replays | ReplayData scored blocks | Checkpoint blocks | Main typed overlay |
|---|---:|---:|---:|---:|
| 12.01 | 3 | 1,905,287 | 72,795 | 84.6% |
| 12.02 | 3 | 2,027,973 | 77,588 | 84.2% |
| 12.03 | 3 | 1,944,438 | 74,386 | 84.2% |
| 12.04 | 3 | 2,425,410 | 86,588 | 84.7% |
| 12.05 | 3 | 1,886,622 | 69,916 | 85.0% |
| 12.06 | 3 | 1,642,411 | 58,280 | 84.9% |
| 12.07 | 3 | 1,934,113 | 77,802 | 84.8% |
| 12.08 | 3 | 1,977,496 | 69,130 | 81.2% |
| 12.09 | 3 | 2,170,389 | 74,575 | 81.4% |

Every replay reports **100%** ReplayData oracle success: 17,914,139 scored
blocks in total. All 661,060 checkpoint blocks were walked. Main and checkpoint
passes report zero malformed/framing, transform, field-stream, lost-RPC,
partial-reassembly, typed-overlay and struct-blob failures. Checkpoint
`skipped_bits` is 906 in 12.05 sample-2: six unresolved RPC blocks are retained
whole as raw payloads, rather than lost. All other checkpoint skipped-bit
counters are zero.

The main overlay decoded 23,902,871 offered rows, and the checkpoint overlay
decoded 1,919,133. The percentages above are the main overlay's decoded/offered
ratio, not semantic completeness. Unknown properties, unresolved RPC schema,
and unnamed fields remain explicitly raw, as on previously supported builds.
Independent Python decoding of the existing public-fixture evidence types
matches **410,350 values**, with no missing specification, width failure or
typed mismatch, across all 54 main/checkpoint field tables. A separate direct
bit walk of TeamEconomy matches **7,797 child values from 1,110 parent rows**,
including the 12.01--12.05 nonzero loadout values. Captured regression fixtures pin
the first two 12.01 updates and reject unknown/missing declarations.

Eight previously supported builds were revalidated and re-exported, one replay
each with checkpoints. Their **104 Parquet files are byte-identical** to the
pre-change outputs. This is the full available 27-file sample for 12.01--12.09, not a
claim about every replay ever recorded on these builds.

The committed 13.01 export baseline was stale from the earlier scoped smoke
field additions. Both the pre-change `18a0c60` binary and this implementation
produce the same four differences: 116 more decoded rows, 116 fewer
not-in-table rows, `fields.parquet` growing by 133 bytes to 16,455,178, and its
SHA-256 changing to
`b186a9b50b4f73f1c3a7d8482431732af5413420ba2ca9992c314bc61cb41b93`.
The 116 values are 20 `CreatedByCharacter` GUIDs and 96 `bInPersistentData`
booleans on the previously accepted Smonk/Wushu scopes. Independent Python
IntPacked/Bool decoding matches all of them. No rows or other main tables change.
The baseline and its quoted documentation figures are refreshed for those
already-present values; the 12.01--12.09 support changes introduce no 13.01 drift.
The paired checkpoint baseline additionally gains 48 GUID and 48 Bool values
from those same scopes, all independently matched to raw bits. Its field file
shrinks by 320 bytes to 1,218,992, with SHA-256
`21ce023b9a7c3b67c0d892cc519f50b097a7e97c88d6da93d98ad78cae00806e`.
The archived pre-upstream binary reproduces both original baseline hashes;
column comparison confirms only these previously-null typed cells change.
All rows, raw bytes, names, identities and other columns are identical.

To reproduce, use the acquired binaries and the three pinned source samples
for each build. Native capture needs optional `pefile` and `unicorn` Python
packages; ordinary Rust tests use the committed vectors and need neither.

```powershell
python tools/capture_native_transforms.py --binaries '<binary-root>' --recovered-binaries '<recovered-root>' --check
cargo +1.86.0 test -p vrf-transform --locked
cargo +1.86.0 build --release -p vrfkit --locked
vrfkit validate '<replay-root>/12.01/sample-1.vrf' --diagnostics
vrfkit export '<replay-root>/12.01/sample-1.vrf' --out '<exports>/12.01/sample-1' --checkpoints
python tools/validate_type_evidence.py '<exports>' tools/fixtures/public_fixture_type_evidence.json --compare-typed
```

Repeat validate/export for samples 1--3 of all nine builds. The same exports
were checked with the required-counter and reconciliation routines from
`check_decode_errors_corpus.py`, alongside the framing/loss counters.

## Replay evidence

The 48 samples are from
[`Matthias1590/unsupported-replays`](https://github.com/Matthias1590/unsupported-replays/tree/c5ba8419b96afa8e88e7afb3e594cf4cce9c75fe):
three per build, 11.06--11.11 and 12.00--12.09. The downloaded `.replay` files
were renamed `.vrf` without changing their bytes. The repository's SHA-256
manifest identifies the source payloads.

Before the fix, `inspect` failed on all 36 samples through 12.05. The first
four builds ran past the end of the header, while larger headers from 11.10
onward reached an invalid level-name count. All twelve samples from
12.06--12.09 already passed inspection but were rejected for missing payload
transforms.

The legacy header places UE4Version directly after the replay branch string:
the next three little-endian u32 values are 522, 1009 and 77. The current
reader instead interpreted 522 as the length of a VALORANT extension. The
extension's length and bytes first appear in the 12.06 samples. Reading it
only outside the twelve measured legacy branches restores the correct
package-version, level-name, game-specific-data and recording-parameter
boundaries. Malformed modern headers are not retried with the legacy layout.

After the fix, all 48 samples pass inspection and the container corpus smoke
test. That smoke test also decompresses the first ReplayData chunk and checks
known Event layouts; it does not decode replication payloads. Synthetic
regressions cover every legacy branch, the 12.06 boundary, zero/nonzero
extensions and rejection of missing modern extensions. The legacy regression
was run against the original reader and failed on the same misplaced length.

## Earlier identity-transform probe

A temporary identity-transform probe was run on `sample-1` from every build.
All sixteen validations failed, with framing oracle rates of only
0.711762%--3.804203%. The experimental registrations were removed. Neither
plaintext passthrough nor merely accepting the branch is a decoding fix.

The upstream parser at
[`2b66c65`](https://github.com/michel-giehl/ValorantReplayParser/tree/2b66c65a7b116154e18ebb84d9f6795f2b080233/src/Replay.Encoding/PayloadEncryption/VersionedTransforms)
has transforms only for the eight previously supported builds.
The additional nine transforms above were recovered from the acquired
game executables listed below. The upstream maintainer describes
locating the transformed reader through `UActorChannel::ReadContentBlockHeader`
in [issue #2](https://github.com/michel-giehl/ValorantReplayParser/issues/2).

Each new transform must come with independent expected-byte vectors, followed
by validation and checkpoint-enabled export of all three available samples
for its build. Frame success alone is not evidence that typed values agree
with the wire.

## Binary analysis feasibility check

The analysis path was exercised on the installed 13.06 executable, without
launching the game or modifying its files. Its SHA-256 is
`8f033b34913a2e16fb6630fe67ac758baf3612e08315b8b74241ed7f5eb537a2`.
Ghidra 12.1.2 headless import and targeted decompilation located a seeded bit
reader at RVA `0x04534320` (VA `0x144534320` at image base `0x140000000`).
Its seed addend `0xe974593c`, initialization offset `0x3c`, PRNG multiplier
`0x2545f4914f6cdd1d` and transform operations match the 13.06 implementation.

For an independent check, Unicorn emulated that original x86-64 function
and its native bit-copy helper directly from the PE image. The Windows x64
arguments were a reader pointer, output pointer and bit count. In this
executable the reader's source pointer, total bits, current bit position and
seed are at offsets `0x98`, `0xa8`, `0xb0` and `0xb8`. Each case reset the
reader and input bytes, used bit position zero, and set the seed to
`bit_count ^ actor_net_guid`. Emulation required return to the caller within
the instruction/time limit before comparing output bytes.

All eleven 13.06 cases from
[`golden_vectors.rs`](../crates/vrf-transform/tests/data/golden_vectors.rs)
matched: 0, 1, 7, 8, 31, 32, 63, 64, 65, 287 and 288 bits. This verifies a
practical way to obtain an independent native-code oracle; this initial feasibility check did not itself add
build support. Addresses, reader layouts and algorithms must
be recovered and checked for each executable, not assumed to carry over.

The manifest-link archive
[`Morilli/riot-manifests` at `573d6e7`](https://github.com/Morilli/riot-manifests/tree/573d6e78edc51395a03513800230eab3dbadbf92/VALORANT/na)
contains 29 patch entries covering all sixteen missing builds. These are
links to Riot manifests, not archived game executables. Acquisition was
initially blocked in the measured environment. A browser check of the CDN
hostname's HTTP root displayed an SK Broadband school-network notice stating
that firewall policy blocks the page. DNS resolves several Riot hostnames to
that warning server, whose expired, mismatched certificate caused the HTTPS
failures. The earlier TLS error is therefore evidence of the local network
block, not evidence that Riot's own CDN certificate is invalid or that the
archived files have disappeared.

After the user requested a retry, DNS resolved to Riot's CloudFront endpoint
and certificate-verified HTTPS downloads succeeded. All sixteen executables
below were acquired from the latest recorded patch for each replay branch.
Only `ShooterGame/Binaries/Win64/VALORANT-Win64-Shipping.exe` was selected;
the installed game was not replaced or launched. Every downloaded file:

- Passed `ManifestDownloader --verify-only` against its RMAN chunk hashes.
- Contained its expected `++Ares-Core+release-<build>` branch label.
- Had a valid Windows Authenticode signature from `Riot Games, Inc.`.
- Was recorded with its SHA-256, manifest ID/hash, source URL and archive
  commit in the local acquisition catalog.

| Replay build | Acquired patch | Executable bytes |
|---|---|---:|
| 11.06 | 11.06.00.3836880 | 201,181,352 |
| 11.07 | 11.07.00.3855133 | 201,408,608 |
| 11.08 | 11.08.00.3918089 | 202,602,752 |
| 11.09 | 11.09.00.3920876 | 203,230,824 |
| 11.10 | 11.10.00.4002057 | 202,949,848 |
| 11.11 | 11.11.00.4091853 | 208,825,552 |
| 12.00 | 12.00.00.4183428 | 208,267,728 |
| 12.01 | 12.01.00.4211771 | 208,325,576 |
| 12.02 | 12.02.00.4226954 | 210,556,856 |
| 12.03 | 12.03.00.4322591 | 210,990,720 |
| 12.04 | 12.04.00.4354757 | 212,279,904 |
| 12.05 | 12.05.00.4440267 | 213,755,024 |
| 12.06 | 12.06.00.4440219 | 214,659,936 |
| 12.07 | 12.07.00.4488404 | 214,767,368 |
| 12.08 | 12.08.00.4578383 | 214,750,840 |
| 12.09 | 12.09.00.4704114 | 184,074,872 |

The selected manifest file sizes sum to **3,312,627,760 bytes** (3.313 GB,
3.085 GiB). The sixteen RMAN files add 148,679,917 bytes. Summing the compressed
chunks referenced by those executables gives 1,688,334,507 bytes, or
**1,837,014,424 bytes** including manifests for the nominal download payload;
HTTP/TLS overhead and retries are not measured by that figure. Generated
metadata and future Ghidra databases need additional space.

All sixteen executables and the seven matching protected-build stubs were
acquired and verified. The 11.06--12.00 executables have encrypted `.text`, a
NOP entry point and an imported `stub.dll!packman`; their stubs themselves
require section decompression. This initially blocked native reader capture.
The offline recovery above resolves that dependency and pins each recovered
image by SHA-256. The installation and Vanguard configuration are unchanged.

The first-hand [Packman analysis](https://hypercall.net/posts/Packman/) helped
identify the runtime loader architecture. Public in-process unpackers were
examined as references but were not used to run or inject into the game. The
committed tool reconstructs the code directly from the archived local bytes.

## Regression scope and commands

Eight previously supported builds were checked before and after this change:
12.10, 12.11, 13.00, 13.01, 13.02, 13.04, 13.05 and 13.06, one replay each.
All validations and checkpoint-enabled exports passed. All 104 Parquet files
(13 per replay) were byte-identical between the two binaries.

The baseline binary was built from `18a0c607ff85cbd7cc785210dbc9add1d6e6e166`.
To repeat the legacy container checks, set `VRFKIT_CORPUS_DIR` to one build's
directory and require that it exists:

```powershell
$env:VRFKIT_CORPUS_DIR = '<replay-root>/11.06'
$env:VRFKIT_REQUIRE_CORPUS = '1'
cargo +1.86.0 test -p vrf-container --test corpus parse_all_vrf_files --locked -- --exact --nocapture
vrfkit inspect '<replay-root>/11.06/sample-1.vrf' --redact-identifiers
vrfkit validate '<replay-root>/11.06/sample-1.vrf' --diagnostics
```

Repeat for every affected directory. For the supported-build comparison, run
both binaries on the same replay with `validate` and
`export <replay> --out <separate-directory> --checkpoints`, then compare the
SHA-256 of each output Parquet file. Replay bytes and local export bundles are
not committed.
