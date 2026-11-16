# Phase C engineering results

**The bounded Phase C implementation and independent functional QA are complete.**
All four runtime task PRs are merged into `integration/vrfkit-overhaul`.
This documentation checkpoint precedes DX #69's own merge and the final
actual integration-HEAD CI. Their outcomes and full final SHA are tracked in
the [fixed acceptance receipt](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6915000647).
The earlier environment interruption is historical; unproven semantics
remain explicitly outside the implemented scope below.
No performance benchmark was run for Phase C, and no speed, CPU or memory
improvement is claimed.

## Source and acceptance receipt

| Item | Recorded state |
| --- | --- |
| Starting integration | `588be1effc8dfca2bed8810fd51a006828081c1b` |
| Main, outside the mutation scope | `9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9` |
| Accepted final runtime candidate | `adec3a40b10c655af83bbbb0d4059dc48339c59e`, tree `e7e20b544c91312a9fefbe2d96baed6f52c87aa3`; includes A, D, C and B. This is the actual integration merge, with the same tree as combined task head `200d70d026fa9347bb967340f3cbefa14694ada0` and its successful CI checkout. |
| Final post-DX integration SHA and CI | The [fixed PR #58 acceptance receipt](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6915000647) supplies the full immutable SHA, final DX merge and exact checkout/run correspondence. Check its completed status before selecting the user benchmark candidate. |
| Main-facing PR | [#58](https://github.com/yakisoba0728/vrfkit/pull/58) remains draft, open and unmerged |
| Publication | No version bump, tag, GitHub Release, crates.io or PyPI publication is part of this phase |

The [fixed PR #58 acceptance comment](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6915000647)
identifies the exact final integration
commit. A commit cannot contain its own hash. If this page pins the last
runtime commit and a later commit changes documentation only, the final
receipt must name both commits and establish runtime/tool-source equality;
the benchmark candidate is the final accepted integration commit, not an
unqualified branch name or an intermediate CI head.

The coordinator compared the exact 136-file runtime key set and SHA-256 map
with E's actual frozen build source `8c2c6216fcbf1675b2ba050dd2470cb1466faf21`,
tree `10b32307fe786e8d686ae934dc716b0e6b4866ae`, and found them equal. The
canonical sorted compact JSON map hash is
`93924a24ac070fc40118999b289eebdd8c67fd7f2e21ce85ab86a7c6b25e13ea`;
E's frozen CLI SHA-256 is
`2b7387ca90f7a9ba00f5a6990577b9f8805ee03284543319a1ef712d0be2be55`.
This ties that binary's functional observations to the published runtime.
Some invocation receipts retain `d413265aa12138c07a569c87d7951afc29db19f6`
as their source-context field; the frozen build receipt identifies the
executable's actual build commit above. Runtime-file equality, not equality
of those documentation/harness commit IDs, establishes their correspondence.
The QA binary uses Cargo's test profile with optimization level 2 and debug
assertions; it is a functional test build, not a release-performance build.
The [independent QA evidence](evidence/phase_c/independent_qa/README.md)
and [final aggregate receipt](evidence/phase_c/independent_qa/final-acceptance.json)
contain the frozen-build, corpus, callback, identity and mutation results.
Only that evidence subtree was imported from published QA checkpoint
`58f76c298d9c64834e40c26295ccb6f63e7a835e`; its receipt/source bytes are
preserved exactly, without importing the QA branch's older top-level docs.
The completed public runner used commit
`05b30c038d9a468c84217ed8c4074e010db45f23`, tree
`49fb9cc557707d0678d405ade0dd01c9ec723cb7`; its runtime hashes matched the
frozen build. Supporting source and receipt hashes are retained in the final
aggregate, while earlier partial receipts remain explicitly historical.
The external acceptance comment records the later documentation/evidence
commit and actual final-HEAD CI without introducing a self-hash loop.

## Implemented scope

| Track | Delivered contract | Boundary |
| --- | --- | --- |
| A: Reader checkpoints | Optional checkpoint callbacks through `ReadOptions`; shared Reader/CLI snapshot decoder; snapshot metadata, declarations, records and separate summaries | Existing `read`/`read_path` callers skip checkpoints by default. Snapshot parser state and counters stay separate from main-stream state. |
| B: Gameplay identity | Exact SpawnedCharacter observations, `MainIdentityTimeline`, lifecycle-aware body resolution and movement association | Main-stream observations only. Resolution exposes null, missing and ambiguous evidence; movement uses the complete transport provenance window. It does not establish account identity, kill credit or native-event timestamp joins. |
| C: Decoder correctness | Exact 12.05 Clay `FocusProjectiles` child decoding, raw-parent preservation and explicit route/trailer counters | Exact build, group, handles and checksums are required. Other identities remain raw. This deliberately adds typed rows and required counter keys. |
| D: Nonseekable input | `ReplayReader::read_bounded` for `Read` input with an explicit encoded-byte cap; aggregate-only stdin example | Buffers the finite input through EOF before parsing. No automatic spooling, unlimited default or incremental network-callback contract. |
| F: DX and release handoff | Migration, adoption examples, release checklist and an output-compatible benchmark procedure | Release preparation only; functional acceptance and user-run performance measurement are separate. |

See [Rust API](RUST_API.md), [Gameplay API](GAMEPLAY_API.md),
[adoption](ADOPTION.md), and [Phase C migration](PHASE_C_MIGRATION.md) for the
caller-visible contracts and examples.

### FocusProjectiles evidence and intended output difference

The route is limited to build 12.05,
`/Game/Characters/Clay/Clay_PC.Clay_PC_C`, parent handle 45/checksum
`957945936` and member handle 46/checksum `831657485`. The measured public
fixture contains 102 main parents and 18 checkpoint parents: 53 main parents
are populated, 49 main parents and all 18 checkpoint parents are empty. The
53 member GUIDs have independent actor-open corroboration. The exact source
receipt and negative-control scope are in
[the checked-in evidence](evidence/phase_c/focus_projectiles_1205.json).

The intended export difference is 53 additional main child rows on that
fixture, with existing rows and raw parents retained. The new required sink
counters are `route_children_focus_projectiles` and
`focus_projectiles_empty_trailers`, including zero values in main and
checkpoint output. This is a documented output extension, not an assertion
of byte equality to unmodified Phase B. Generic array grammar is not relaxed.
Fresh independent corpus comparison confirmed this exact difference, as
recorded below.

## Validation ledger

The permanent authored suite delta is 45 Rust tests: A adds 12, B adds 18,
C adds 4 and D adds 11. C adds 3 Python tests. The resulting inventory is
871 Rust tests on non-Windows, 872 on Windows, and 1181 Python tests. On the
exact accepted runtime tree, the combined CI's Linux workspace block ran
43 test processes totaling 871 passed, zero failed and zero ignored. Its
Windows MSRV documentation guard explicitly measured 872 Rust tests and
1181 Python tests. The separate final actual-HEAD readback belongs to the
fixed acceptance receipt.
Temporary independent contracts do not inflate the checked-in suite count.

| Gate | Observed result at this checkpoint |
| --- | --- |
| Reader task CI | [Run 43852188720](https://github.com/yakisoba0728/vrfkit/actions/runs/43852188720): all 11 jobs succeeded for PR #71 head `9c4e698b829c1ea4c056512b2fa01060fdd7a516`; the synthetic merge tree equals source tree `970f489ec0bcdd6cd6c36523a7705a7ea1746e76` |
| Intermediate DX CI | [Run 43852794091](https://github.com/yakisoba0728/vrfkit/actions/runs/43852794091): 11 jobs succeeded; this predates the final combined Phase C source |
| Gameplay/combined task CI | [Run 43855117296](https://github.com/yakisoba0728/vrfkit/actions/runs/43855117296): all 11 jobs succeeded for PR #70 head `200d70d026fa9347bb967340f3cbefa14694ada0`, containing A, D, C and B. Checkout `fe5debbc9e99711f9aab6e23fbb454d940035711`, that task head and actual integration merge `adec3a40b10c655af83bbbb0d4059dc48339c59e` share tree `e7e20b544c91312a9fefbe2d96baed6f52c87aa3`. The earlier run at `5ce480b...` is not the accepted source gate. |
| Decoder task CI | [Run 43853641134](https://github.com/yakisoba0728/vrfkit/actions/runs/43853641134) succeeded on immutable C-only head `1b9bbf222f003e35c882e640c1b19e0bbcc88472`. Updated-stack [run 43854953389](https://github.com/yakisoba0728/vrfkit/actions/runs/43854953389) also succeeded in all 11 jobs; checkout `8323635c9f2a50313ffb8e8f73e89f76df301c8c` and actual merge `39c440658013a8c43b89e6f83245d35aaeb5b5bc` share tree `5f4a065b377aea1e8abc813fdfe0882b414c30c6`. |
| Stream task CI | [Run 43854479576](https://github.com/yakisoba0728/vrfkit/actions/runs/43854479576): all 11 jobs succeeded for PR #73; checkout `603017e345abea802942e7a1be3888395ee423b9` has the same tree as head `f0ed266ee69b74f9e8a8de40b44cb9274aaf2b86` and accepted merge `6ef705ef227f6c0e8dda87d7dfa29ef8bfbc2c8d`. |
| Combined runtime coverage | Passed Rust 1.86, Windows/Linux, rustfmt, strict Clippy/rustdoc, feature matrix, Python, documentation, Rust/PyArrow and CLI/replay CI gates. DX's own merge and final actual-HEAD run follow this documentation checkpoint; see the fixed acceptance comment for their results and exact final SHA. |
| Accepted runtime integration CI | Actual integration push [run 43857608643](https://github.com/yakisoba0728/vrfkit/actions/runs/43857608643) and main-facing PR #58 [run 43857617144](https://github.com/yakisoba0728/vrfkit/actions/runs/43857617144) both completed successfully with head `adec3a40b10c655af83bbbb0d4059dc48339c59e`. These validate the runtime pin; the later post-DX actual-HEAD run is still a separate required gate. |
| Independent public sweep | Ten inputs across seven builds, 99 decoded checkpoints and 61,002 independently checked typed/raw values; no unexpected production differences |
| Independent checkpoint API | All 70 callback Parquet tables byte-equal to CLI output; every input's full 46-key network and 54-key sink maps agree for main and checkpoint streams, plus 25 checkpoint scalars and four first-error option fields |
| Independent identity/gameplay | All 6,310,086 movement rows compared for values, provenance and identity proof/status; 39 raw SpawnedCharacter declarations checked. Existing projection oracle matched 583 native and 386 phase observations (969 total). |
| Existing movement reference | Fresh Rust probes matched pinned historical C# latest-move output for 12.10, 12.11 and 13.00. The reference was not regenerated; its limited output is separate from the full ten-input identity comparison. |
| Independent focused contracts | 35 passed: 9 independently authored, 3 external-consumer, 11 Reader-checkpoint and 11 bounded-input tests, plus one separate nonempty native-checkpoint positive control. The build receipt's 34-test subtotal excludes that separately recorded positive control. |
| Compiled-source mutations | All four compiled successfully and were detected: three caused the targeted assertion to fail; the FocusProjectiles mutant still exported successfully but failed the independent 53-child oracle. Restored source compiled successfully, all 9 independent and 3 external-consumer tests passed again, all 136 runtime hashes matched, and frozen binaries were unchanged. |
| Private archive | Metadata access covered one page with 27 assets. No actual private binary download was attempted and no private binary was tested. This is neither a complete private inventory nor an access-denial finding; historical large-corpus results retain their original revisions and scope. |
| Performance | Not executed; wall time, CPU, utilization, peak RSS, allocation counts and throughput improvements remain unmeasured |

### Output preservation and coverage

Fresh coverage consists of one 12.05 input, two 12.08 inputs, one each for
12.09, 12.10, 12.11 and 13.00, and three 13.01 inputs. This is ten public
inputs across seven build labels; it does not establish a fresh audit of
every supported build or the historical 1,018-replay corpus.

Of 130 main/checkpoint Parquet tables compared with the old-code baseline,
129 were byte-identical. The sole intended difference is the 12.05 main
fields table: all 1,240,315 prior rows remain unchanged, with exactly 53
additive FocusProjectiles child rows and three recognized empty trailers.
The explicit comparison independently predicts the new rows from raw parent
bytes, verifies their order and values, and allows only the documented
counter/manifest increments. No unexplained table, summary or manifest
difference was observed. All nine native movement-component counters remain
part of the complete equality checks.

Each input also produced seven callback-written checkpoint tables, giving
70 byte-identical callback/CLI comparisons. The four first-error option
fields remained `None` in this corpus; corruption and consumer failures are
covered separately by focused controls. Informational diagnostics for raw,
unresolved RPCs remain visible and are not mistaken for structural errors.

The identity comparison checked every movement row: 5,669,526 resolved body
relationships and 640,560 `Unresolved(NoReference)` outcomes. It also compared
6,269,101 movement component groups and 16,663 fragmented movement rows.
Null, ambiguity, retired lives, dormancy, concurrent changes and malformed
evidence are exercised by synthetic tests, not claimed as observed cases in
this corpus. The independent oracle uses raw SpawnedCharacter values and
packet/lifecycle intervals, not account data, timestamp proximity or a future
GUID-cache backfill.

The public checkpoint counter comparisons are supplemented by the separate
nonempty native/movement checkpoint positive control. It exercises four
checkpoint moves, their exact provenance, preserved raw RPCs and all nine
native counters. Its authored origin and single-test invocation are retained
in [the positive-control receipt](evidence/phase_c/independent_qa/checkpoint-native-positive.json).

## PR and agent accounting

Five core task PRs are the limit: DX [#69](https://github.com/yakisoba0728/vrfkit/pull/69),
Gameplay [#70](https://github.com/yakisoba0728/vrfkit/pull/70), Reader
[#71](https://github.com/yakisoba0728/vrfkit/pull/71), Decoder
[#72](https://github.com/yakisoba0728/vrfkit/pull/72), and Streams
[#73](https://github.com/yakisoba0728/vrfkit/pull/73). All five slots are used.

| Task | Observed integration state |
| --- | --- |
| Reader #71 | Merged as `187ee934ea2484b8036ea4e6c659b5a262065b46` |
| Streams #73 | Merged as `6ef705ef227f6c0e8dda87d7dfa29ef8bfbc2c8d` |
| Decoder #72 | Merged as `39c440658013a8c43b89e6f83245d35aaeb5b5bc` |
| Gameplay #70 | Merged as `adec3a40b10c655af83bbbb0d4059dc48339c59e` after independent source controls and restoration passed |
| DX #69 | Documentation and final evidence prepared. Its own CI/merge and final actual-HEAD CI are pending when this in-tree checkpoint is written; subsequent outcomes are recorded in the [fixed acceptance receipt](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6915000647). |

Six unique agents were actually created: A, B, C, D, E and F. Maximum observed
concurrency was three non-root agents plus the coordinator. The first D spawn
was refused because capacity was full; a later retry created D. The temporary
execution disconnect and compiler SIGKILLs are distinct from worker crashes
or feature-test failures. All six roles completed their bounded implementation,
review or handoff work; no agent feature-task crash was observed. Initial
checkpoint returns are not counted as extra agents or completed tracks.

The [execution ledger](PHASE_C_STATUS.md) and
[historical recovery checkpoint](PHASE_C_RECOVERY.md) preserve the interruption
and exact source recovery. Reconstructed or metadata-only recovery branches
are not substitutes for the subsequently recovered original sources.

## Remaining boundaries and handoff

Unproven field types, full match reconstruction, native-death-to-player joins,
kill credit and account identity remain outside the new gameplay API. The
finite `Read` API does not promise incremental callbacks, async cancellation
or a total process-memory bound. Inputs outside the measured decoder identity
remain raw until exact evidence exists.

Use the [migration guide](PHASE_C_MIGRATION.md) and the preserved
[release and compatibility checklist](PHASE_C_RELEASE_CHECKLIST.md) before a
separately authorized release of **0.6.0 or later**. Keep main, tags and
registries untouched in this phase. Once the stated gates and five task PRs
are resolved, stop this phase without starting a broader audit.

The [user-run benchmark handoff](PHASE_B_BENCHMARK_HANDOFF.md) retains strict
output gates. It proposes the immutable C-only source as a schema-compatible
baseline and requires a final-candidate identical-binary control. Unmodified
Phase A/B binaries lack the new required keys even on zero-Focus inputs;
they are not valid baselines for the current harness.
