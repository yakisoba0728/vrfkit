# Engineering overhaul results - 2026-11-16

This report records implemented changes and evidence actually obtained during
the overhaul. It separates component verification from final integration
acceptance. The [machine-readable record](evidence/overhaul_2026_11_16.json)
contains sanitized aggregates, provenance, CI links and hashes of the original
local evidence. The [execution ledger](OVERHAUL_STATUS.md) tracks remaining work.

**Runtime acceptance checkpoint:** PRs 49 through 56 are merged into
`integration/vrfkit-overhaul`, through
`ed7dc30978261f27d3999b194344bcf55287860d`. All 11 required jobs passed
at the final runtime PR head. Its merged source matches every one of the 149
runtime-fingerprint files from the combined candidate that passed the full
Rust suite and six-replay audit. The original execution provenance is retained.
This documentation checkpoint and the subsequent Draft PR to main record their
own heads and CI in GitHub; main must remain unmerged. This does not claim
every requested goal or replay build is covered.

## Baseline and protected repositories

The measured parser baseline is the existing integration commit
[`67c0a2d94fbcbeeefa56761f1bc13d9f6c829fb0`](https://github.com/yakisoba0728/vrfkit/commit/67c0a2d94fbcbeeefa56761f1bc13d9f6c829fb0).
It already contains the integration validation foundation and an earlier
benchmark guard fix; those are not counted as new results here. Untouched
`main` remains at
[`9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9`](https://github.com/yakisoba0728/vrfkit/commit/9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9).
No main merge, release, registry publication, private archive mutation or
reference-repository modification was performed.

The baseline Rust 1.86 run passed 743 tests on Linux with no failures or ignored
tests; the platform-qualified Windows inventory was 744. Baseline Python passed
1,089 tests with warnings treated as errors. Binaries were retained unchanged
for output/resource comparisons. A Linux count does not replace Windows CI.

## Delivered changes and review history

Each merged PR passed its required Windows/Linux validation before integration.
These URLs identify successful runs associated with the listed PR heads;
Actions checks out a synthetic merge. The final runtime run's actual checkout
and equality to the integrated tree are recorded below.

| Change | Published source head | Integration merge | Required CI |
| --- | --- | --- | --- |
| [PR 49: bit-reader validation](https://github.com/yakisoba0728/vrfkit/pull/49) | `eb6e85d981816bd66e50948a235066f2ab0c1b2d` | `eb2570f5fa3374abd405723235dbe767cbcb6248` | [43776108241](https://github.com/yakisoba0728/vrfkit/actions/runs/43776108241), 11 successful jobs |
| [PR 50: equivalent-export resource measurements](https://github.com/yakisoba0728/vrfkit/pull/50) | `334c86336b0d52736044350c88fa53ab38349982` | `78a856cc19269416a3537dbd533aac1d4e7e23ef` | [43777023619](https://github.com/yakisoba0728/vrfkit/actions/runs/43777023619), 11 successful jobs |
| [PR 51: schema allocation budgets](https://github.com/yakisoba0728/vrfkit/pull/51) | `e211c2378f0ed0a786f1b2f75b1cc5e8ea2726a6` | `d888ff28d5b340139bdd24d121e3119dc7241c9a` | [43778079065](https://github.com/yakisoba0728/vrfkit/actions/runs/43778079065), 11 successful jobs |
| [PR 52: shared Rust replay reader](https://github.com/yakisoba0728/vrfkit/pull/52) | `2720ca9ce4f8bbb9583a7abe5a635251f4b158c6` | `438f3e1786b4f3b0287468ba0b529bbdaa4c745d` | [43778665193](https://github.com/yakisoba0728/vrfkit/actions/runs/43778665193), 11 successful jobs |
| [PR 53: streaming JSONL conversion](https://github.com/yakisoba0728/vrfkit/pull/53) | `0b747c8522598ff965c97341b138202e1e92f5c1` | `499996379ba6bf6f46c10aeb9a661038e97ba158` | [43779583452](https://github.com/yakisoba0728/vrfkit/actions/runs/43779583452), 11 successful jobs |
| [PR 54: comparison correctness](https://github.com/yakisoba0728/vrfkit/pull/54) | `bbebb258706dc9e5c574e480561d044beca79d3e` | `fce76a9a784a806310cad687c67f654cb8e40ad8` | [43780363565](https://github.com/yakisoba0728/vrfkit/actions/runs/43780363565), 11 successful jobs |
| [PR 55: permanent Linux validation](https://github.com/yakisoba0728/vrfkit/pull/55) | `2ea537a0e4a01354d6a081b9d62876d81a887f13` | `dbb86f886b34dc7426f83b789baac7adc2ce89c3` | [43781227633](https://github.com/yakisoba0728/vrfkit/actions/runs/43781227633), 11 successful jobs |
| [PR 56: repeated-schema reuse](https://github.com/yakisoba0728/vrfkit/pull/56) | `65d98f0b7455cb6b7b2a51c37867d0c025e1eacc` | `ed7dc30978261f27d3999b194344bcf55287860d` | [43782959411](https://github.com/yakisoba0728/vrfkit/actions/runs/43782959411), 11 successful jobs |

### Actual hosted suite totals

The final runtime run's logs were read independently after completion:

| Executed check | Observed result |
| --- | --- |
| Rust 1.86 workspace, Windows | 775 tests passed, as reported by the live documentation/test guard |
| Rust 1.86 all-feature workspace, Linux | 774 tests passed across 35 harness summaries; zero failed, ignored or filtered |
| Core-only Reader, Windows and Linux | 135 tests passed on each platform, counted separately |
| Python 3.12 and 3.13, Windows and Linux | 1,143 tests in each of four jobs; each ended `OK`, with zero skips |
| Full required run | 11 successful jobs, including formatter/Clippy/docs, feature checks, public replay validation, workflow lint and Windows packaging gates |

The [Actions receipt](evidence/pr56-actions-observed-test-counts.json) retains
job URLs, extraction methods and decoded-log hashes. These are observed results,
not just expected counts in a README. CI tested synthetic merge
`22cd027c72bf7290c4fbb0be2b01c7a394432dbd`, associated with PR head
`65d98f0b7455cb6b7b2a51c37867d0c025e1eacc`. GitHub commit metadata confirms
its tree and actual merge `ed7dc309` have the same complete tree
`fd9462cadb51d8cf14cc7e65aad2cb1054fd577f`. The distinct commit identities
are preserved in the receipt.

The subsequent automatic **post-merge** run
[43783444550](https://github.com/yakisoba0728/vrfkit/actions/runs/43783444550)
also passed all 11 jobs. Its checkout log confirms the actual integrated
commit `ed7dc309`, as recorded in the
[post-merge receipt](evidence/postmerge-actions-43783444550.json). This is a
separate executed regression gate after integration; it does not relabel the
earlier job logs from which the detailed counts above were extracted.

### Bit-reader correctness

Oversized public read widths previously panicked in debug builds and could
return truncated values in optimized builds. A zero packed-vector scale could
produce non-finite output. The reader now returns `InvalidReadWidth`,
`InvalidRotatorWidth` or `InvalidVectorScale` before advancing the cursor.
Valid boundary widths, including zero-width operations, retain their behavior.
Overlay diagnostics classify invalid caller configuration as rejected.

Three reproductions failed against the old implementation in both debug and
release. Seven new regressions/boundary tests passed after the fix, together
with decoder, core-only, optimized, Clippy and documentation checks. The
measured `BitError` remains 32 bytes on x86_64; no allocation was added. This is
a demonstrated public API defect, not a reproduced malicious-replay exploit:
current replay callers appeared bounded. Exhaustive downstream error matches
must handle the new variants.

### Bounded schema admission

A safe synthetic probe showed that a 215-byte export table could request
1,048,576 dense field slots: 33,554,432 bytes of logical storage on x86_64.
The old per-group check did not bound repeated declarations with distinct
identities. Live and checkpoint readers now admit declarations before allocating
slot vectors, with configurable defaults of 4,096 groups and 1,048,576 slots
per cache. `SchemaError::ResourceLimit` makes refusal explicit.

The slot budget still permits that measured 32 MiB of logical slot storage;
it bounds aggregate growth rather than eliminating amplification. It does not
cap total RSS, strings, GUID maps, allocator overhead or allocations a trusted
caller makes before passing an already constructed group. Alias/merge behavior,
replacement accounting, old descriptors and checkpoint observer ordering remain.

The failing-before regression safely used a 4,097th empty group. Fifteen new
resource regressions and a doctest cover limits, growth, aliases, replacement,
reset, dirty recount, truncation precedence and observers. Disabling admission
made the limit test fail. Among six public samples, the largest checkpoint
contained 542 groups and 10,851 declared slots; these are observations, not
universal bounds. See the
[published schema-limit guide](https://github.com/yakisoba0728/vrfkit/blob/e211c2378f0ed0a786f1b2f75b1cc5e8ea2726a6/docs/SCHEMA_LIMITS.md)
for configuration and migration.

### Shared Rust reader and architecture

The existing CLI implementation is shared by a library and a thin binary entry
point. `ReplayReader`, borrowed `ReplayEvent`/`ReplayRecords` callbacks,
`ReplaySummary` and `ReadError<E>` expose the main-stream decoder without a
second parsing implementation. Callbacks deliver metadata, raw/typed fields,
movement, actor lifecycle, rejected partials and native Event payloads in file
order. Consumer errors preserve their original type/value and stop delivery;
unfinished partials flush once at end of input. Quality counters remain separate.

Seven API tests and a compiling doctest passed. Removing the final flush made
its regression fail. The combined Rust run after this addition passed 774 tests
on Linux; the Windows inventory is 775. On three public fixtures, callback
reconstruction produced 18 main Parquet tables byte-identical to baseline. The
refactored CLI produced 39 main/checkpoint tables byte-identically; manifests
differed only in elapsed time.

The API streams output. `read_path` still buffers the entire input; checkpoints
are explicitly counted as skipped by this API and remain available through
`export --checkpoints`. Callbacks are provisional until successful completion,
so consumers needing atomic publication must stage their output. Successfully
decoded movement keeps the existing representation, without promising a duplicate
raw RPC. Disabling default features removes Arrow, Parquet and Zstd from the
library dependency path. See the
[published Rust API guide](https://github.com/yakisoba0728/vrfkit/blob/2720ca9ce4f8bbb9583a7abe5a635251f4b158c6/docs/RUST_API.md).

An additional, separate Cargo workspace exercised the documented source API
under Rust 1.86 / edition 2024 with `default-features = false`. On the public
13.00 sample, callbacks and the detached summary agreed on 12,181 fields,
4,991 movement rows, 98 actor rows, zero partial rows and six native events.
Those five row counts also matched previously exported Parquet metadata.
Exactly six checkpoint chunks were reported skipped. The 49-package normal
dependency tree and complete 69-package resolved graph contained no Arrow,
Parquet or Zstd package. Input/source hashes stayed unchanged. The
[external consumer receipt](evidence/reader-external-consumer/verification.json)
retains the measured source revision, lockfile and aggregate results; the
[evidence guide](evidence/README.md) explains how to reproduce it outside this
workspace. This validates a source dependency, not a published registry package.

### JSONL conversion

The independently reviewed implementation was `824caf3`, subsequently published
in PR 53. It converts existing Parquet tables with a schema header and a row
envelope per line. It retains exact integer tokens, binary bytes as hexadecimal,
Unicode, null versus empty binary, column order and finite floats including
signed zero. Unsupported selected types and non-finite values fail explicitly.
Atomic publication occurs only after emitted row counts match metadata.

All 19 focused tests passed, including read/write interruption and input aliases.
Independent review passed eight additional scalar cases. Collecting the entire
batch iterator made the streaming regression fail. Every main/checkpoint table
from three public baseline exports was converted and independently compared:

| Build | Tables | Rows checked | Binary bytes checked |
| --- | ---: | ---: | ---: |
| 12.10 | 13 | 59,180 | 123,895 |
| 12.11 | 13 | 40,710 | 97,616 |
| 13.00 | 13 | 38,837 | 76,622 |
| Total | 39 | 138,727 | 298,133 |

All selected logical values matched and source hashes stayed unchanged. This
checks conversion fidelity, not the game interpretation producing the table.
Batch size bounds rows, not bytes or RSS. The format does not preserve Parquet
compression, physical encodings, custom metadata or dictionary indices. Consumers
need integer-aware JSON parsing beyond their language's exact numeric range.

### Comparison-tool corrections

The final locally reviewed comparator is
`dbe83ffa2d1a44cfa7dfb949b20d46af19d26624`, published and merged in
[PR 54](https://github.com/yakisoba0728/vrfkit/pull/54) at source head
`bbebb258706dc9e5c574e480561d044beca79d3e`. It fixes these measurement defects:

- Compare C# `bunch_count` with Rust bunch count; packets containing bunches
  cannot substitute for a missing bunch counter.
- Include the advertised one-millisecond lower boundary for legacy movement
  matching, with unsigned-time clamping.
- Match by packet ID, character GUID and movement timestamp when available.
  Never select candidates using position or velocity.
- Read strong-identity packets even outside the display-time window. Reserve
  ambiguous strong candidates from weaker legacy matching. Missing strong
  identities never fall back to nearby display-time rows.

The focused suite passed 31 tests; the last follow-up failed five assertions
before its fix. This exact comparator reran on all three existing public export
pairs and produced reports byte-for-byte identical to the preceding correct
identity-join reports. All 45 existing export/report/log files retained their
SHA-256 hashes. Neither parser was rerun for this follow-up. The tool remains
a comparison report: exit zero does not certify every value, and no guessed
numeric tolerance converts a difference into a pass.

### Permanent Linux CI

The CI change was merged in [PR 55](https://github.com/yakisoba0728/vrfkit/pull/55)
after [all 11 required jobs passed](https://github.com/yakisoba0728/vrfkit/actions/runs/43781227633)
at exact head `2ea537a0e4a01354d6a081b9d62876d81a887f13`.
It was independently reviewed and fault-injected at `ddc37f1`;
follow-up `6834a2b` only makes JSON reading explicitly UTF-8. Linux Rust 1.86
runtime tests and public main/checkpoint validation move into reusable `ci.yml`,
required by main, main-targeted PRs, integration and release validation. Failure,
cancellation or skipped Linux validation fail the aggregate gate. Existing
Windows checks remain required.

Ten success/failure injections executed the actual Bash snippets. Build,
download, checksum, baseline, audit, typed-validator and malformed-report errors
all propagated. Checksum failure stopped before parsing. Summary generation
retained the validator's nonzero exit status. The local fixture-script smoke
used the immutable baseline binary and checked three pinned public fixtures,
including 16 specified identities and 582 rows with zero mismatches. It omitted
only the build line and does not replace the new workflow's hosted run.

The Linux artifacts contain tracked public source and aggregate reports.
Replay/Parquet paths and untracked files are excluded from the source archive;
detailed export diagnostics are not uploaded by this job. Synthetic sensitive
labels, values and paths were absent from the typed summary and stdout. No
private archive or secrets input route was introduced.

Independent Linux checks on the combined candidate also passed all 27
metadata-derived feature configurations across seven feature-bearing packages:
seven no-default configurations and 20 individual non-default features.
The core-only Reader run passed 135 tests, with no ignored or filtered tests;
strict all-feature and core-only Rustdoc passed with warnings denied. All 15
library, binary and example crate roots inspected retained
`#![forbid(unsafe_code)]`. The
[matrix receipt](evidence/final-linux-feature-matrix.json) records each command,
result and local log digest. These feature checks are not an exhaustive power
set of every possible feature combination.

## Public replay audit and its limits

The six-input baseline and schema-budget candidate audits exercised one public
sample each from 12.05, 12.08, 12.09, 12.10, 12.11 and 13.00. Both passed the
common validation/export gates, with positive checkpoint work in every build.

| Build | Replays | Checkpoints | Scored / passing main blocks | Independently checked typed values |
| --- | ---: | ---: | ---: | ---: |
| 12.05 | 1 | 19 | 558,801 / 558,801 | 20,772 |
| 12.08 | 1 | 17 | 577,810 / 577,810 | 16,325 |
| 12.09 | 1 | 21 | 792,498 / 792,498 | 22,490 |
| 12.10 | 1 | 7 | 13,680 / 13,680 | 236 |
| 12.11 | 1 | 6 | 6,506 / 6,506 | 165 |
| 13.00 | 1 | 6 | 8,860 / 8,860 | 181 |
| Total | 6 | 76 | 1,958,155 / 1,958,155 | 60,169 |

There were zero replay failures, build errors, typed mismatches and width
failures. All 78 candidate Parquet files matched baseline byte for byte, with
no missing/extra tables. These results belong to the recorded baseline and
schema candidate binaries; their actual parser/source/binary hashes are
preserved in the JSON. A later merge SHA must not replace measured provenance.
The combined-candidate execution and integrated-source correspondence below
provide the final runtime acceptance for this six-input scope.

Full matches come from
[unsupported-replays at `c5ba8419`](https://github.com/Matthias1590/unsupported-replays/tree/c5ba8419b96afa8e88e7afb3e594cf4cce9c75fe/replays);
the small CI fixtures come from
[ValorantReplayParser at `b51d6742`](https://github.com/michel-giehl/ValorantReplayParser/tree/b51d67423b7b4952d59051cf91e55efa1c42da05/tests/Test.Integration/Replays).
Input hashes are recorded in the JSON. Originals remained read-only; local
symlinks allowed `.vrf` discovery to find public `.replay` samples without
renaming the originals.

The block oracle is a defined structural acceptance rule. Independent typed
checks cover only the identities observed by their specification. Neither
establishes complete semantic coverage or makes unknown fields known. Byte
equality demonstrates preservation of existing output on these samples, not
perfect reconstruction of every original replay bit.

### Integrated runtime acceptance and measured provenance

The measured combined runtime candidate
`93e107a10788021b59fb30c5cb145960c3fd21a0` subsequently passed 774 Rust tests
and the same six-input audit: 76 checkpoints, 1,958,155 passing/scored main
blocks and 60,169 independently checked values with zero width failures or
typed mismatches. All 78 Parquet files were byte-identical to baseline and
all six manifests matched except `elapsed_ms`.

This does **not** mean every raw error counter was zero. The candidate recorded
22,561 main RPC-stream failures and exactly 22,561 unresolved RPC payloads
preserved as raw data, with zero lost blocks and zero unaccounted RPC loss.
Unknown RPC interpretation remains a limitation despite successful preservation.

The measured runtime-source hash is
`92328ce1ab9a4236c38ae07c49ed816d259e0107cac4470295e05de48fbff93d`, and the
executable hash is
`ef02b508753e05ff1bffd96792f402c3639d652406333978fd0fe8917af1c8ad`.
After PR 56 merged, the integrated tree at
`ed7dc30978261f27d3999b194344bcf55287860d` was checked against this fingerprint:
all 149 contributing files matched individually. The
[source correspondence receipt](evidence/overhaul-runtime-source.json)
retains every path and hash, and the final runtime PR's 11 required CI jobs
passed for head `65d98f0b7455cb6b7b2a51c37867d0c025e1eacc`. The Actions
synthetic checkout and the actual integration merge have identical complete
Git trees. This establishes which integrated source was validated
without claiming an additional replay execution or replacing the candidate's
original commit/binary provenance.

## Resource measurements and performance decisions

The [resource harness](RESOURCE_BENCHMARKS.md) separates wall time, child
user/system CPU, utilization, peak RSS, input throughput and output bytes. Runs
alternate order after warmups. Matching Parquet hashes, normalized manifests,
required counters, preservation gates and unchanged input/binary/harness hashes
are required before ratios are emitted. Failed, missing-counter, changed-output
and interrupted measurements cannot become accepted comparisons. Nineteen
dedicated tests and real self-comparisons passed.

The initial harness exercise verified 96 complete exports across three public
full matches in main/checkpoint modes. Timings were mixed and noisy. The RPC
optimization experiment was not adopted and is not a shipped improvement.

Schema reuse has a narrow benefit when identical declarations repeat. Across
nine measured pairs, the synthetic add loop median changed from 218.976432 ms
to 0.467511 ms; the declaration loop changed from 224.664727 ms to 0.242902 ms.
These loops deliberately repeat identical declarations. A census found none in
the six sampled main streams or their 76 checkpoints. All 108 full exports in
the subsequent six-sample, confirmation and same-binary-control runs passed
equivalence. The first 12.05 paired wall observation was +8.96%, the confirmation
was -9.78%, and an identical-binary control was +7.86%. These results do not
establish an ordinary-replay speedup or regression. Detailed CPU/RSS results,
dispersion and methods are retained in the
[schema benchmark report](SCHEMA_REDECLARATION_BENCHMARK.md) and its
[source reconstruction guide](benchmarks/schema-redeclaration/README.md).

No ordinary-replay CPU/RSS benefit or competitor speed ratio is claimed.
Allocation counts, allocated bytes and isolated I/O time were not measured.
Unix child peak RSS is not the sum of simultaneous process-tree peaks.
Unsupported Windows resource metrics are null, not fabricated zeroes. Choosing
the fastest run or reducing output work would not resolve this uncertainty.

## Reference parser comparison and build recovery

The comparison used unmodified reference source
[`fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8`](https://github.com/michel-giehl/ValorantReplayParser/commit/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8)
and Rust baseline `67c0a2d`. Installing requested .NET SDK 10.0.301 alone did
not resolve a silent parallel restore/build failure. Serial restore/build with
node reuse disabled recovered it. Release compilation reported zero warnings
and errors; CLI help and three public exports completed. The precise parallel
failure cause remains unknown. No reference tracked file or remote was changed.

| Build | Reference movement rows | Rust movement rows | Unique shared identities | Missing / ambiguous |
| --- | ---: | ---: | ---: | ---: |
| 12.10 | 8,853 | 8,907 | 8,853 | 0 / 0 |
| 12.11 | 2,949 | 3,001 | 2,949 | 0 / 0 |
| 13.00 | 4,946 | 4,991 | 4,946 | 0 / 0 |

Across 16,748 shared identities, maximum absolute position difference was
0.00046875000043655746 in exported units; yaw/pitch were equal, and maximum
velocity difference was 0.000048828125045474735. Packet, bunch and actor-open/
close counts also matched. Extra Rust movement rows, field aliases and other
gameplay meanings remain outside this value comparison.

### Additional full-match comparison: explicit unresolved differences

A later comparison used one public 12.09 full match and the combined Rust
candidate's existing main tables. Both commands completed successfully and
all input, export, binary and source hashes remained unchanged. The
[full-match receipt](evidence/reference-full-1209.json) retains exact commands,
provenance, sample limits and measured differences.

| Metric | C# reference | Rust candidate |
| --- | ---: | ---: |
| Packets | 681,831 | 681,831 |
| Bunches | 681,831 | 681,831 |
| Actor opens | 2,424 | 2,424 |
| Actor closes | 2,176 | 2,176 |
| Movement rows | 2,616,931 | 2,634,029 |

The value comparison sampled the **first 50,000 reference movement rows**,
1.910635% of its movement file. There were 49,980 unique exact matches,
20 missing exact identities and zero ambiguous matches: **99.96%**, despite
the comparator's one-decimal display rounding to `100.0%`. This is not a
random sample or a comparison of all 2.6 million movement values.

Among those exact matches, maximum position difference was
0.00046875000043655746 and velocity difference was
0.000048828125045474735 in exported units; yaw and pitch were equal. No numeric
tolerance was invented to mark these differences as accepted.

A bounded scan of the full Rust identity metadata found none of the 20 exact
packet/character/movement-timestamp identities. Each had a same-character/
timestamp alternative two packets later, but those weaker keys recur 20 or 21
times. No weaker match was accepted. The packet attribution difference and
17,098 additional Rust movement rows remain unresolved. Neither parser is
assumed correct merely because it emitted more or fewer rows.

The reference reported zero diagnostics while also reporting 103,388 undecoded
export groups. The existing Rust audit for this input recorded 9,151 unresolved
RPC payloads, all preserved, with zero unaccounted RPC or block loss. These
counters have different definitions and cannot be compared as one accuracy
score. No timing ratio was measured for this comparison.

### Output contracts differ

The reference
[dispatcher](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/Replay.Unreal/Chunks/ReplayChunkDispatcher.cs)
skips checkpoint/container-event chunks, while its
[export runner](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/CliReader/JsonExport/ReplayExportRunner.cs)
enables enrichment. The
[manifest writer](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/CliReader/JsonExport/ReplayExportManifestWriter.cs)
documents latest-move emission and omitted undecoded/shell groups. Default
enriched NDJSON and raw/typed Parquet therefore represent different work.
Their timing ratio is not an equivalent-scope benchmark, and neither parser
is treated as a ground-truth oracle.

### Feature inventory and deliberate omissions

The reference source inventory contains 27 global branches plus China 13.05
and 13.09; vrfkit contains 27 global branches. China mappings were not ported
without fixture evidence. The existing upstream audit already covers reference
revision `fbf24b6`, so existing crosshair, economy and other analytical features
are not counted as newly implemented by this session. Unresolved legacy damage
handle/version questions remain open instead of receiving guessed types.

## Private archive and historical evidence

The read-only metadata inventory at catalog commit
`b1fff6bc26aa040f1948931fadb9ebf29c961ec9` contains 1,011 unique replay hashes,
50,938,107,071 bytes and 24 builds from 11.06 through 13.10. Catalog entries
matched 1,011 release assets with no missing/extra, size or digest discrepancies.
This is metadata consistency. The private connector did not provide a usable
release-binary download path: **zero private replay binaries were downloaded or
decoded in this session**. Interactive metadata access does not imply binary
validation or transfer access to public Actions tokens.

The historical [common build verification](BUILD_VERIFICATION.md) reports
1,018 unique replays at parser `259ed10c7e0c9d87c852f80fab7535d38d2e187f`.
Its multiple supplied roots form a different population from the current
archive catalog. The existing [upstream audit](UPSTREAM_AUDIT_2026_11_16.md)
also retains its own source revisions and measurements. Those reports were
not rerun against this overhaul or added to its six-input coverage count.

Local source/toolchain setup and public binary acquisition were completed,
enabling actual Rust builds, tests, exports and independent parallel workers.
The earlier ledger's no-Rust/no-worker claims are obsolete. This does not imply
persistent AI execution after the session; commits, PRs, CI and the ledger
provide resumable state.

## Reproduction

Pin code/toolchain, install the existing Python dependencies and run project
gates. Use new private work/output paths separate from read-only originals.
These are reproduction instructions, not assertions about a future head.

```bash
rustup toolchain install 1.86.0 --component clippy,rustfmt
python -m pip install -r requirements.txt
cargo +1.86.0 test --workspace --all-features --locked
cargo +1.86.0 test -p vrfkit --no-default-features --locked
cargo +1.86.0 fmt --check
cargo +1.86.0 clippy --workspace --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo +1.86.0 doc --workspace --all-features --no-deps --locked
python -W error -m unittest discover -b -s tools/tests -p 'test_*.py'
python -W error tools/check_docs.py
```

Full platform/feature/interoperability requirements remain in
[CONTRIBUTING.md](../CONTRIBUTING.md). Verify public or separately authorized
local input SHA-256 values before parsing:

```bash
cargo +1.86.0 build --release -p vrfkit --locked
python -W error tools/verify_build_corpus.py \
  --exe target/release/vrfkit --corpus /private/read-only-corpus \
  --work-dir /private/audit/run-001 --output /private/audit/run-001.json --jobs 2
python -W error tools/validate_type_evidence.py /private/audit/run-001 \
  tools/fixtures/public_fixture_type_evidence.json --compare-typed
cargo +1.86.0 run --release -p vrfkit --no-default-features \
  --example stream_replay -- /private/read-only-corpus/example.vrf
target/release/vrfkit export /private/read-only-corpus/example.vrf --out /private/example-export --checkpoints
python tools/export_jsonl.py --export /private/example-export \
  --table fields --columns bit_count raw_bits --out /private/example-fields.jsonl
```

For reference build recovery, run from the pinned reference checkout with its
requested SDK and a separate artifact directory:

```bash
dotnet restore src/CliReader/CliReader.csproj --artifacts-path /private/reference-build \
  --disable-parallel -p:BuildInParallel=false -m:1 -nr:false
dotnet build src/CliReader/CliReader.csproj -c Release \
  --artifacts-path /private/reference-build --no-restore \
  -p:BuildInParallel=false -m:1 -nr:false
```

Return to the vrfkit checkout before running its comparison tool:

```bash
python tools/compare_with_csharp.py /private/reference-export /private/vrfkit-export
```

Follow [RESOURCE_BENCHMARKS.md](RESOURCE_BENCHMARKS.md) for paired resource
measurements: warmups, repetitions, identical modes, quiet execution and output
equivalence. Keep exports and detailed reports private; publish reviewed
aggregates only. Original replay files must remain unchanged.

## Remaining work and release readiness

The JSON records the accepted runtime head, successful Actions run,
six-input audit, baseline parity and exact source correspondence. The combined
JSONL/comparator Python run passed 1,143 tests. Later documentation-only
publication and the Draft PR record their own actual heads and CI; they do
not replace the immutable measurement provenance. The Draft PR remains a
review surface, with no automatic main merge or release.

- Fresh private-corpus validation, including older builds absent from these
  samples, needs authorized binary access and new aggregate reports.
- Legacy damage-handle/version mappings remain unproven where exact wire
  identity evidence is missing. No speculative gameplay mapping was added.
- The 12.09 full-match comparison leaves 20 sampled packet identities and
  17,098 additional Rust movement rows unexplained. Investigate attribution
  before relaxing identity matching or treating either parser as ground truth.
- `Read`/`Seek` input streaming, checkpoint callbacks and further enrichment
  need separate ownership/error contracts and replay evidence.
- A fair competitor benchmark needs an equivalent work contract. No
  two-times-faster claim or broad CPU/RSS benefit is supported.
- Fixture-backed China 13.05/13.09 support remains separate from the 27 global
  branch inventory.
- Formal version/release decisions, registry packaging and broader platform
  distribution remain separate work. CI-built Windows packages are not a
  published release; no crates.io or Python-registry publication occurred.

Repository About/topics were not modified. The existing losslessness wording
needs review against the explicit raw-preservation limits after main review;
the available connected API did not expose that metadata mutation. Documentation
navigation and accurate examples can improve discoverability without claiming
an unmeasured ranking or superiority.

Continue preserving `#![forbid(unsafe_code)]`, raw windows, original files and
failing checks. Material migration requirements are new exhaustive bit-reader/
schema error variants, explicit lower-level `NetGuidCache` budgets for trusted
larger workloads (the current `ReplayReader` uses defaults), and Reader/JSONL
ownership and publication contracts described above.
