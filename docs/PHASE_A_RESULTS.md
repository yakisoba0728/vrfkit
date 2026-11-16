# Phase A engineering results

Phase A recovers the Native RPC correction and adds bounded file input to the
Rust Reader. The attempted movement inlining optimization was withdrawn after
controlled measurements failed to establish a reliable improvement. **Zero
runtime speed optimizations are accepted from Track B.** The Reader has a
measured memory reduction on two public inputs, with no speed claim.

The [final acceptance receipt](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6909992420)
is the authoritative record of the final integration HEAD, merged PRs,
independent acceptance and actual post-merge CI. It is finalized only after
those gates complete. A successful earlier task run does not establish final
integration acceptance. This report retains the executed source identities and
measurement scope without attempting to embed its own future commit hash.

## A. Recovery, implementation and orchestration

Work began from freshly verified integration
`d44bcd2b22209cf6a0fabf3996ecc514f5cab606`. Protected main remained
`9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9`. PRs #47-#57 and #60-#62 were
already merged and were retained. PR #58 remains the open Draft review surface
to main. No main merge, release or registry publication belongs to this phase.

Six distinct subagents were created: parser correctness, profiling, performance
implementation, Reader input, independent QA, and CI/evidence. Three subagents
were observed running together, four agents including the coordinator. Roles
were resumed as slots became available; resumed tasks are not counted as new
agents. The final receipt records terminal completion/failure counts.

| Track | Result | Evidence and publication |
| --- | --- | --- |
| Native RPC correctness | Implemented, independently verified and merged | [PR #63](https://github.com/yakisoba0728/vrfkit/pull/63), integration merge `3e61f331f5e41b3ab34c8e5ba45d5bc04e021999` |
| Real performance profiling | Completed | [Profiling report](PHASE_A_PROFILING.md), fresh stage measurements |
| Runtime optimization | Candidate implemented and measured, then withdrawn | [Negative-result report](PHASE_A_PERFORMANCE.md), zero accepted runtime changes |
| Reader input memory | Implemented; corrected scheduling and measured lower RSS | [Reader report](READER_INPUT_STREAMING.md), final independent/CI acceptance in the receipt |
| Final integration | Coordinator publishes only after required gates | [Final receipt](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6909992420) |

Ordinary HTTPS git push lacked authentication. The existing connected GitHub
API published source trees instead; exact local/GitHub tree equality was
checked and commit-identity differences were retained. This did not require
new credentials, permissions, or repository changes outside the task branches.

## B. Native correctness and compatibility

The old movement decoder used an unproven u16/tail interpretation of the
native signed32 byte-array envelope. A supported movement prefix followed by
an unsupported A3 body could appear successful, preventing raw-RPC fallback.
A generated Reader regression compiled and then failed against the actual
starting integration because the expected complete raw payload was absent.

The recovered correction validates the signed count, exact outer length,
unused-bit count, explicit movement terminator, bounded A3 window and optional
A4 header. An unknown body or unsupported header retains the whole original
RPC and every already decoded movement prefix with its source provenance.
Following independently framed components remain decodable. Fully represented
movement may still deliberately omit a duplicate raw RPC; this is distinct
from silently losing an unsupported body. An unchecked count never drives an
allocation.

| Fresh verification | Observed result |
| --- | --- |
| Failing-before A3 regression | Compiled, then failed at runtime with missing raw payload |
| Fixed movement/core tests | 171 passed |
| Python tools tests | 1,178 passed |
| Public builds | 12.05, 12.08, 12.09, 12.10, 12.11, 13.00 |
| Checkpoints exercised | 76 |
| Main structural blocks | 1,958,155 passed / 1,958,155 scored |
| Independent typed/raw comparisons | 60,169, zero mismatches or width failures |
| Unresolved main RPC payloads | 22,561, all preserved |
| Parquet comparison | 78/78 byte-identical files |
| Unexplained manifest differences | Zero |

The fresh six-input execution reused the frozen native executable built at
`2e3526f`, after all 151 recorded runtime source hashes matched the recovered
implementation. It is a fresh replay execution using an existing matching
binary, not a new binary build. The older accepted exports retain their
original baseline attribution. The public samples contain header-only A3/A4
forms; unsupported bodies were verified with generated inputs, not claimed as
an observed corruption in these six files.

Independent QA authored three Reader tests covering 31 generated scenarios:
24 unknown-body combinations, two supported optional-A4/padding forms, and
five malformed envelopes. Tests traverse both fragmented and ordinary RPCs,
assert exact raw bytes and movement/provenance, and check subsequent-component
recovery without duplicates. Removing the A3 completion guard and relaxing
exact envelope length each compiled and then failed at runtime. Restored
source passed again.

A separate Python implementation directly reread and footer-parsed all 78
Parquet files, compared JSON types as well as values, rejected 24 manifest
mutations, and independently checked the 151 source hashes. It did not import
the author's comparison implementation or treat a previous report as a fresh
read. These three manual verification tests are retained separately; they do
not inflate the ordinary Cargo suite inventory.

### Counter migration

Eight obsolete tail/trailer counters are replaced by nine native component
counters in Rust results, human-readable summaries and manifest quality data.
Rust error/result consumers need the new variants; summary/manifest consumers
must update the removed key names. There is no valid numeric conversion from
the old tail counters to the new component counters.

The [migration guide](NATIVE_MOVEMENT_COMPONENTS.md#counter-and-source-api-migration)
explains every key and error change. Historical private baselines are unchanged;
current-executable comparisons refuse obsolete contracts before a missing
input can turn the check into a skip. A release containing this output change
requires a minor version bump from 0.5.x to at least 0.6.0. No version or
release was published during Phase A.

## C. Performance: measured bottlenecks and withdrawn optimization

Fresh source instrumentation on the public 12.09 checkpoint export attributed
approximately 1.747 CPU seconds to the main parsing thread, 1.074 to the fields
writer, 0.768 to the movement writer and 0.261 to checkpoint fields. Fields
Arrow construction measured about 242 ms and Parquet encoding about 788 ms.
Movement decoding invoked `parse_single_move` 2,634,029 times. These diagnostic
measurements overlap across threads and change code generation; they identify
work, not exact uninstrumented percentages or an accepted speedup.

The implemented experiment added only `#[inline(always)]` to that function.
Both executables were built from the same directory with Rust 1.86, the same
lockfile/features and release profile. Frozen disassembly confirmed two
out-of-line calls before and zero after; executable size fell by 592 bytes.
Neither result proves a complete-export performance benefit.

The following are **individual variant medians**, not ratios of paired runs:

| Input / variant | Wall s | Total CPU s | CPU use | Peak RSS bytes | Input MiB/s | Parquet bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 12.09 checkpoints / unannotated | 2.028978 | 4.939242 | 236.368% | 196,775,936 | 28.679 | 43,984,122 |
| 12.09 checkpoints / inline | 1.942531 | 4.883212 | 249.369% | 197,349,376 | 29.955 | 43,984,122 |
| 12.05 main / unannotated | 1.485688 | 3.072077 | 204.154% | 133,054,464 | 35.114 | 31,815,958 |
| 12.05 main / inline | 1.358216 | 2.904734 | 204.140% | 132,976,640 | 38.409 | 31,815,958 |

For each measured round, the harness divided candidate by baseline before
computing the median. These paired statistics need not equal ratios of the
marginal medians above:

| Series | Direction | Pairs | Paired wall ratio | Paired CPU ratio | Paired RSS ratio |
| --- | --- | ---: | ---: | ---: | ---: |
| 12.09 checkpoints | Inline / unannotated | 7 | 0.879335 | 0.983599 | 0.997995 |
| 12.05 main | Inline / unannotated | 7 | 0.972847 | 0.949747 | 1.015324 |
| Same inline executable control | Same / same | 5 | 0.974209 | 1.039398 | 0.997303 |
| Reverse confirmation, 12.09 | **Unannotated / inline** | 7 | 1.043695 | 0.983101 | 0.998694 |

The identical-binary control itself appeared 2.58% faster in wall time and
3.94% higher in CPU. The second input's small paired wall effect overlaps that
variation. The reverse confirmation's CPU direction changes: expressed as
inline/unannotated, its ratio is approximately 1.0172, or 1.72% more CPU.
No outlier was removed. The measurements do not establish that inlining is
harmful or universally neutral; they fail to establish a reliable benefit.
The annotation was therefore removed from the integrated proposal.

All 60 complete exports passed process, counter, preservation and strict
output-equivalence gates: 52 measured executions and eight warmups. They
produced 668 Parquet-file fingerprints across repeated runs of two inputs;
this is not 668 distinct fixtures or independent files. Total output medians
were 44,676,171 bytes for 12.09 and 32,463,843 for 12.05, identical between
variants. All input/executable/harness hashes remained fixed within each
series. Focused optimized movement tests passed 22/22.

The experiment remains on
[`experiment/phase-a-movement-inline`](https://github.com/yakisoba0728/vrfkit/tree/8b5e5502f8fd5ab182b84bee8a8db2cdca7a545c),
GitHub commit `8b5e5502f8fd5ab182b84bee8a8db2cdca7a545c`, with the exact
candidate tree `078e1007c6c77d9c29a1657bdce87636f1b30982`. It is not merged as
an optimization. Full raw statistics, build hashes, candidate patch, controls
and the withdrawal decision are in [the experiment report](PHASE_A_PERFORMANCE.md).

Allocation counts, allocated bytes and isolated I/O times were not measured.
All resource comparisons were warm-cache runs with coordinated pauses in the
other agents' builds/tests; this is not an exclusive-host guarantee. The older
+16.23% wall / +17.92% CPU observation and prior favorable experiments remain
historical evidence. No ordinary-replay speedup or competitor ratio is claimed.

## D. Reader input: lower memory, bounded buffering

`read_path` and `read_path_with_schema_limits` now use file/chunk input instead
of retaining the complete input. `read(&bytes, ...)` still borrows the caller's
buffer. The same decoder, schema limits, ordering, callbacks, source identities
and consumer-error handling remain shared. The CLI's loading path, command
arguments and Parquet writers are unchanged by this implementation.

The main pass and decompression helper open independent read-only file
handles. Both scan eight-byte headers, while each Event or ReplayData body is
assigned to one reader. Skipped payloads are bounds-checked before seeking.
A reusable encoded buffer, the current decoded chunk and at most one prepared
decoded chunk replace the whole-file buffer. The preamble probe is bounded by
existing info-format limits and may reread a small prefix after seeking; it
is not a complete second file read or a retained index.

The same optimized executable measured `buffered` and `file` modes with one
warmup pair and five measured alternating pairs for each series. Individual
variant medians were:

| Input / mode | Wall s | Total CPU s | CPU use | Peak RSS bytes | Input MiB/s |
| --- | ---: | ---: | ---: | ---: | ---: |
| 12.09 buffered | 1.63684 | 1.98021 | 121.08% | 107,819,008 | 35.55 |
| 12.09 file | 1.51649 | 1.81504 | 121.42% | 49,946,624 | 38.37 |
| 12.05 buffered | 1.16250 | 1.49575 | 125.05% | 86,511,616 | 44.88 |
| 12.05 file | 1.15345 | 1.42746 | 123.76% | 31,563,776 | 45.23 |

| Paired series | Wall change | Total CPU change | Peak RSS change |
| --- | ---: | ---: | ---: |
| Corrected 12.09 | -2.59% | +2.15% | **-53.68%** |
| Corrected 12.05 | +1.70% | -0.61% | **-63.53%** |
| Identical file/file control, 12.09 | -8.35% | -5.31% | -0.025% |

The large and consistent result is reduced peak RSS on these two inputs.
Timing differences cannot be separated from the control variation, so there
is no speedup or universal neutrality claim. These are isolated Reader
measurements at local corrected revision `6a9ea4e` on the pre-Native parser
contract; final combined-source validation is separate and is not relabeled
as another performance measurement.

### Rejected scheduling and correctness

The initial `d78f30a` prototype reduced RSS by 74.69% but increased paired
wall time by 26.95%; its wall medians were 1.68978 -> 1.96849 seconds. One
rendezvous channel blocked decompression at intervening Event/checkpoint
notifications. Header measurements found 8-21 non-data chunks between data
chunks on 12.09 and up to 24 on 12.05. The corrected independent scans restore
lookahead without an enlarged queue of decoded chunks. The failed design and
its measurements remain in the [Reader report](READER_INPUT_STREAMING.md).

Eleven new tests cover generated preamble/chunk truncations, first-chunk and
length errors, maximal allowed info, bounds checks before skipping, a sparse
64 MiB payload, I/O categories, callback order and consumer-error identity.
All 26 focused Reader/preamble tests passed on corrected source, with Clippy.
The earlier combined core run of 218 tests belongs to the initial checkpoint,
not a fresh full-suite execution of the corrected revision.

The corrected executable's six public file/buffer comparisons had equal
callback projections, explicit IEEE float bits, movement provenance and final
summaries, with equal diagnostic lengths and FNV-1a-64 fingerprints. Input
SHA-256 values stayed unchanged. The full 12.09 comparison covered 2,634,029
movement samples, 1,657,000 fields, 4,600 actors, 254 native Events and 21
explicitly skipped checkpoint chunks. Both modes counted 17,491,180 raw field
bytes. This checks equivalent input paths through one decoder, not independent
wire interpretation or cryptographic whole-file reconstruction.

### API and resource limits

High-level methods keep their signatures. The additive container helper
`read_preamble(Read + Seek)` preserves separate I/O/structural error causes.
Callbacks remain borrowed, synchronous and provisional until success; returning
a consumer error stops later callbacks and retains the original error.
The helper may finish an in-progress decompression or header scan before
noticing cancellation, so immediate cessation of all work is not guaranteed.

The largest encoded, Header and Event payloads still need local buffers.
Parser state, decompressed chunks and consumer-retained output consume memory;
there is no fixed total-RSS promise. Files must remain unchanged throughout
the read. Length changes are detected, but same-length concurrent changes or
replacement are not guaranteed to be detected. Use an owned snapshot with
`read` when snapshot isolation is required. Arbitrary non-seekable input and
checkpoint callbacks remain deferred.

## E. Independent verification and CI

The [independent combined-source review](evidence/phase-a-reader-independent-review.json)
passed on Native plus corrected Reader source, with the inlining experiment
absent. Six separately authored helper tests passed: the 31 Native scenarios
through file input, consumer-error identity/precedence, typed late truncation
and provisional callbacks, plus the sparse fixture builder. The existing
15 Reader API, six input and five preamble tests also passed on that combined
source. Retained helpers do not increase the ordinary suite inventory.

A separate executable read a synthetic sparse file of 2,147,483,988 logical
bytes under a 512 MiB address-space limit. File input succeeded with two
callbacks and peak RSS 7,630,848 bytes. Whole-file buffering in the same
executable failed explicitly with `OutOfMemory`. The fixture occupied only
12,288 filesystem bytes and mostly contains skipped payloads; this proves the
path does not require a complete input buffer, not that large decoded chunks
fit within a fixed global cap. The
[memory receipt](evidence/phase-a-reader-memory-independent.json) records
the source/script hashes, actual resource limit and both outcomes.

The same independent reviewer recomputed recorded pair ratios, medians, CPU
totals/utilization and executable identities, including reversed labels. The
review checked six Reader equality records, 60 export receipts and 668 recorded
Parquet fingerprints and supported withdrawing the optimization. It was a
review of retained measurements, not a new benchmark or another read of every
exported Parquet file. See [measurement review](evidence/phase-a-measurements-independent.json).

### Fresh combined CLI export verification

The coordinator built the default-feature release CLI from combined Native
and Reader source `16345d94c2a35e50aeb1c1a98012e19cbd38b448`, then froze
executable SHA-256
`ffee1414bda3716083348c99ee909f7d1a04b02d13fedaa68f977ae6e1d8b0d9`.
This is a new combined-source build and execution, distinct from the earlier
Native audit's reused binary and the isolated Reader timing probe.

All six public inputs passed the fresh main/checkpoint audit: 76 checkpoints,
1,958,155 passed/scored structural blocks and 60,169 typed comparisons, with
zero typed mismatches or width failures. All 22,561 unresolved main RPCs remain
preserved, with zero main or checkpoint RPC loss. A separate comparison read
all 78 resulting Parquet files against the fresh Native-accepted exports;
every file was byte-identical. These two builds share the current counter
contract, so their manifests differ **only in elapsed time**.

The [combined validation receipt](evidence/phase-a-final-cli-validation.json),
[build fingerprint](evidence/phase-a-final-cli-build.json),
[audit](evidence/phase-a-final-cli-audit.json) and
[parity receipt](evidence/phase-a-final-cli-parity.json) retain exact commands,
source hashes and output identities. Runtime files match the frozen build;
later report/QA changes do not claim another CLI execution.

### Hosted checks and final publication

Native PR #63 was merged only after latest
[run 43821570049](https://github.com/yakisoba0728/vrfkit/actions/runs/43821570049)
passed 11/11 jobs. Its actual synthetic checkout was
`c50f8ce1a14cb9779e90ce8ca652c48f20a611b7`; its complete tree
`5f72cae0dc55b00779901155c1dffbff6a544382` exactly matches integration merge
`3e61f331f5e41b3ab34c8e5ba45d5bc04e021999`.
[The receipt](evidence/phase-a-native-pr-ci.json) retains both identities.
The actual Windows log reported 798 Rust / 1,178 Python tests, and the Linux
all-feature step passed 797 tests across 36 harness summaries with zero
failures, ignored or filtered tests. The Windows feature matrix contained 27
cases. The earlier checkpoint receipt remains unchanged.

Reader adds exactly eleven unconditional tests to that source inventory,
so the expected combined suite is 809 on Windows and 808 elsewhere, with
1,178 Python tests. A fast documentation guard checks consistency, not those
runtime counts. Final hosted logs must confirm the actual counts. The manual
independent QA helper tests are not added to that regular inventory.

The required final gate includes Rust 1.86 on Windows/Linux, Rust stable,
rustfmt, Clippy, feature combinations, strict Rustdoc, Python 3.12/3.13 on
both platforms, documentation/generator/baseline checks, Rust/PyArrow
interoperability, pinned public main/checkpoint validation and Windows package
smoke validation. Successful package validation is not a release publication.

The [final acceptance receipt](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6909992420)
records the independent combined-source verdict, final Reader PR head, its CI,
actual integration HEAD and post-merge run. Until that receipt states those
gates have completed, earlier PR success is not final Phase A acceptance.
No private replay or C# full-movement rerun is included in this phase's scope.

## F. Remaining work and exact resumption

After final validation and the Phase A report, stop. These items remain for a
separately authorized Phase B:

- Stable measurement and further optimization research; the inlining candidate
  is preserved as an unresolved experiment, not a delivered speed improvement.
- Broader private-corpus validation if a usable authorized binary path exists.
- Event enrichment, additional gameplay semantics and other unproven field/build
  mappings, with exact wire evidence and independent checks.
- Checkpoint callbacks and arbitrary stream inputs, if their ownership and
  cancellation contracts are designed and verified.
- China branch support and an equivalent-scope competitor benchmark.
- Native migration version decisions, release packages and registry publication.

Resume from the exact integration SHA recorded in the final acceptance receipt,
not from an earlier local measurement or withdrawn experiment. Fetch
`integration/vrfkit-overhaul`, compare its remote HEAD to that receipt, inspect
existing PRs and source differences, and choose only newly authorized work.
PR #58 stays draft and unmerged; main, private archive, replay originals and
reference sources remain unchanged by Phase A.
