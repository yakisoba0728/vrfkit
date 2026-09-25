# Upstream parity update, 2026-09-23

The latest common all-build audit is [BUILD_VERIFICATION.md](BUILD_VERIFICATION.md).
The results below retain the date and sample scope of this implementation update.

This update reviews ValorantReplayParser at
[`d23c13e12262fb1da9fc005d1cd0ef9f8d0d36fd`](https://github.com/michel-giehl/ValorantReplayParser/tree/d23c13e12262fb1da9fc005d1cd0ef9f8d0d36fd).
There are 14 upstream commits since the `b51d674` revision recorded in
[REFERENCE_REPOSITORIES.md](REFERENCE_REPOSITORIES.md). The local starting
commit is `06807e718e052d0c820f6044439d66ccf233b5ba`.

Implementation commit: `4533ff77ad331a0899cfe3c8661bbc631e019a7a`.

The historical 714-replay totals were not remeasured. This update uses twelve
preserved replays, an existing 13.01 export for raw evidence only, upstream
golden vectors and independent Python decoders. It does not inspect or modify
the valplay repository.

## Disposition of the upstream changes

| Upstream change | vrfkit disposition |
|---|---|
| `7b4ef8a`: 13.06 transform | Added `V13_06`, dispatch and eleven vectors. All 88 vectors across eight builds pass. The three S-box tables remain byte-identical. |
| `cf58163`: Phoenix flash path | Added exact `MulticastSetPath.NetworkedProjectilePath` array children: elapsed seconds, location and velocity. |
| `c0f2a30`: flashes | Added declaration-qualified `BlindManagerComponent.ActiveBlinds` children, including IDs, references, flags and durations. |
| `13818c6`: missing smoke descriptors | Added nine exact group/name/checksum primitive identities with observed wire evidence. |
| `c04a904`: further ability descriptors | Surveyed smoke, wall and nearsight fields. Existing ownership fields are already typed; remaining new wall fields have no local payload evidence. |
| `aebea81`: Viper movement rotation | Already corrected to `ByteComponents` by the local type-correction generator. No duplicate correction. |
| `2611920`: property exports shadow RPC tables | vrfkit already keeps the two groups separate. Added a regression for both insertion orders and late property re-declaration; a deliberate aliasing mutation makes it fail. |
| `ce4f62a`, `914039a`: player RoundInfo / PR #5 | Existing named-member RoundInfos decoder covers this capability. Raw parent retention and real replay struct counts were rechecked. |
| `d3ec140`, `65f35e8`: release-aware descriptors / PR #7 | vrfkit resolves these struct members through replay-provided names. Generator reports version-selected custom C# decoders as opaque review candidates rather than claiming to port them. |
| `75df2eb`: parser refactor, limits and diagnostics | Reviewed partial limits, EOF handling, close/dormancy, raw retention and cache behavior against existing implementations/tests. No speculative parser rewrite. |
| `d23c13e`: Breach ExitResults wall exit | Deferred: no Breach group or ExitResults payload in 16,165,103 surveyed main/checkpoint field rows. A matching real payload is needed for exact-consumption validation. |
| `0e296c2`: README | Reference status and supported-build documentation updated locally. |

Upstream semantic flash/nearsight events include temporal or spatial
association as well as direct wire evidence. This update exports measured wire
values; it does not promote those associations to guaranteed cast/hit events.
Upstream's chunk dispatcher still skips container Event and Checkpoint chunks;
vrfkit's existing handling of those chunks remains in use.

## Descriptor comparison

The generator now resolves inherited virtual movement quantization, static
constant path expressions and four reviewed ClassNetCache factory definitions
with fifteen concrete calls. Unsupported declaration forms fail visibly.
The comparison report has schema version 3 and explicitly lists the one
version-selected custom decoder left as `Raw`.

Vendored input: 1,185 named entries and 84 handle entries. Upstream input:
929 named entries and 260 handle entries. The delta contains 188 additions,
444 removals, eleven type changes and 176 handle additions. These are review
candidates, not 188 automatically validated types. The tool identifies 593
local entries at risk from wholesale replacement, so the vendored input is
preserved. Re-extraction, local corrections and formatting reproduce the
existing `table.rs` byte for byte.

```powershell
python -W error tools/compare_descriptor_sources.py `
  --baseline third_party/vrp `
  --candidate "$env:LOCALAPPDATA/vrfkit/upstream-vrp::d23c13e12262fb1da9fc005d1cd0ef9f8d0d36fd" `
  --downstream-table crates/vrf-decode/src/table.rs --output descriptor-audit.json
```

The C# comparison helper also now counts unnamed rows separately. Real 13.06
exports exposed a crash when it sorted a null field name beside a string;
unknown identities now remain outside named-field coverage.

## Replay validation

The six preserved older fixtures cover 12.10, 12.11, 13.00, 13.02, 13.04 and
13.05, one replay each. All six were validated and exported both normally and
with `--checkpoints`. The six available 13.06 replays were validated and
exported with `--checkpoints`. Every invocation exited successfully.

The 13.06 framing baseline is committed in
[`tools/baselines/build_1306.json`](../tools/baselines/build_1306.json):
3,369,208 content blocks, 2,476,146 property fields, 1,869,797 RPCs,
zero malformed blocks, and a 100.000000% framing oracle rate on every replay.
The baseline also retains the nonzero skipped-bit counters; framing success
does not mean that every payload has a known meaning.

Across all twelve replays, the checkpoint-enabled decode guard reports:

| Measurement | Result |
|---|---:|
| Main overlay values decoded | 7,481,715 |
| Main overlay errors | 0 |
| Main struct blobs decoded / failed | 1,709 / 0 |
| Checkpoint overlay values decoded / errors | 590,387 / 0 |
| Checkpoint struct blobs decoded / failed | 1,701 / 0 |
| Checkpoint array / truncated RPC / movement failures | 0 / 0 / 0 |

There are 75 existing opaque empty reward variants in the main stream; they
remain explicit. Non-null values and preservation were checked separately
from these counters.

Before/after comparison preserved all 14,516,354 prior main/checkpoint field
rows, including identical raw columns and every one of the 10,557,328 prior
non-null typed values. It adds 716,450 array-child rows and types 1,893
previously null scalar rows. Of 132 non-field Parquet files, 126 are
byte-identical. The six changed files are the 13.06 `checkpoint_blocks`
tables: only `field_row_start` and `field_row_count` change to describe added
children. All other columns and block counts are identical, ranges remain
contiguous, and their totals equal the new checkpoint field-row counts.
The six older normal exports also preserve every prior row/value and every
non-field Parquet byte; they add 3,681 array children and type 295 existing
rows. These are the same six replays, so they are not added to the twelve-replay
totals above.

The older baseline binary is `06807e7`. Because that version cannot decode
13.06, the new-build baseline is an isolated checkout of the same commit with
only the 13.06 transform added. This separates transform support from added
typing and measured-array routes. The candidate enables the previously
validated array routes on 13.06 as well as the two new routes. Their additional
13.06 rows must not all be attributed to the two new abilities.

## Independent typed-value evidence

The two ability routes require exact parent identities and observed build
scope. ActiveBlinds additionally checks each member declaration's handle,
name and checksum; path members use the pinned PathPoint descriptor, not
unrelated sibling parameter declarations. Unexpected widths, unknown
handles, malformed or missing terminators and unconsumed bits prevent
promotion. Parent raw payloads remain available on success and refusal.
Regression tests cover zero-width unknown members and counted declaration
refusal with raw-parent retention, plus missing path members and non-finite
elapsed time through the actual RPC sink.

Across six 13.06 candidate exports, the independent Python decoder matched:

| Route | Parents | Elements | Typed children matched |
|---|---:|---:|---:|
| ActiveBlinds | 335 | 169 | 1,521 |
| NetworkedProjectilePath | 50 | 800 | 2,400 |

The first 13.06 replay contains neither route; the validator reports that
absence. On the 13.05 fixture it also matched 630 blind children and 960 path
children; on 13.02 it matched 603 blind children and 1,488 path children.
Comparisons cover child path/context, raw bytes and every typed
value, including FName strings, packed references, floating values and vectors.
The tool checks main fields; it does not claim checkpoint samples of an
ability merely because checkpoint decoding succeeded.

Nine smoke scalar identities add five `CreatedByCharacter: ObjectNetGuid`
and four `bInPersistentData: Bool` entries. Their explicit upstream declarations
are in `Smokes/Descriptors/SmokeDescriptors.cs`; concrete paths are bound in
the agent descriptors. All 1,893 matching rows in the twelve candidate
replays changed from null to a typed value equal to an independent exact
decode, with no missing identity, decode failure or mismatch. The broader
raw audit found 2,105 matching rows including 212 in the existing 13.01 export.
Those 13.01 rows were not freshly re-exported: no permitted source replay was
available for this run. That existing export also contains neither of the two
new ability-array routes. Detailed per-identity evidence is in
[`scoped_type_evidence.json`](../tools/fixtures/scoped_type_evidence.json).

## Reproduction and checks

Replay files remain private. The preserved inputs live below
`$env:LOCALAPPDATA/vrfkit/baseline-corpora`; validation artifacts live below
`$env:LOCALAPPDATA/vrfkit/upstream-implementation-20260923`. Pass equivalent
local paths when reproducing elsewhere.

```powershell
cargo +1.86.0 build --release -p vrfkit --locked
$exe = "<candidate-vrfkit.exe>"
$corpus = "<preserved-corpus-directory>"
$env:VRFKIT_JOBS = "2"
python -W error tools/validate_corpus.py $exe $corpus --recursive
python -W error tools/check_decode_errors_corpus.py $exe $corpus --recursive --jobs 2 --checkpoints
python -W error tools/check_corpus_baseline.py --baseline tools/baselines/build_1306.json --exe $exe --corpus "$corpus/build_1306" --require-input
& $exe validate "<replay.vrf>"
& $exe export "<replay.vrf>" --out "<export-directory>" --checkpoints
python -W error tools/validate_ability_array_evidence.py "<export-directory>" --compare-typed --require-routes
python -W error tools/validate_type_evidence.py "<export-root>" "<nine-new-scalar-specifications.json>" --compare-typed
```

The scalar specification is the nine new entries selected from the scoped
evidence fixture, passed as a JSON list. Export-root validation must select
only one export per replay to avoid counting normal/checkpoint duplicates.
Run the ability validator over several directories together if a single
replay does not contain both routes.

A private checkpoint export baseline for 13.06 sample
`a4b7406f-e3f2-4831-9111-1456e62871e1` was created with
`check_export_baseline.py --update`, then checked again without `--update`.
All thirteen Parquet hashes and fourteen counter/file identities matched.

The upstream C# CLI at the pinned revision built locally without warnings
and exported the first 13.06 replay. The comparison report joined all of its
first 50,000 movement rows, but does not prove value identity: maximum
position-axis difference was 5.14 and maximum velocity-axis difference 15.8.
These differences remain recorded rather than being called parity; the
before/after Rust comparison checks that this update did not change the
existing movement output.

The MSRV 1.86 development sweep passed: formatting, workspace clippy with
warnings denied, all-target/all-feature checks, rustdoc with warnings denied,
the offset probe, Rust/Python Parquet interoperability, 27 advertised feature
configurations, ASCII and generated-file checks, baseline schema checks and
the full documentation guard. The final suites contain 701 passing Rust tests
and 836 passing Python tests. Eleven 13.06 golden vectors are included in the
88-vector transform test. After the last failure-accounting fix, the final
binary again passed the twelve-replay checkpoint decode guard and the private
13.06 export hash baseline.
