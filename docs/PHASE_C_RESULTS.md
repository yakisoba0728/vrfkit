# Phase C engineering results

**Acceptance is in progress.** The original execution environment recovered;
the earlier interruption is a historical checkpoint, not the current blocker.
This page records implemented contracts separately from completed acceptance.
No performance benchmark was run for Phase C, and no speed, CPU or memory
improvement is claimed.

## Source and acceptance receipt

| Item | Recorded state |
| --- | --- |
| Starting integration | `588be1effc8dfca2bed8810fd51a006828081c1b` |
| Main, outside the mutation scope | `9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9` |
| Published combined runtime candidate | `200d70d026fa9347bb967340f3cbefa14694ada0`, tree `e7e20b544c91312a9fefbe2d96baed6f52c87aa3`; includes A, D, C and B, with final integration acceptance still pending |
| Final integration acceptance SHA and CI | Pending; the coordinator must record the full immutable SHA and exact checkout/run correspondence |
| Main-facing PR | [#58](https://github.com/yakisoba0728/vrfkit/pull/58) remains draft, open and unmerged |
| Publication | No version bump, tag, GitHub Release, crates.io or PyPI publication is part of this phase |

The final PR #58 acceptance comment must identify the exact final integration
commit. A commit cannot contain its own hash. If this page pins the last
runtime commit and a later commit changes documentation only, the final
receipt must name both commits and establish runtime/tool-source equality;
the benchmark candidate is the final accepted integration commit, not an
unqualified branch name or an intermediate CI head.

The coordinator compared the exact 136-file runtime key set and SHA-256 map
with E's frozen source `d413265aa12138c07a569c87d7951afc29db19f6` and found
them equal. The canonical sorted compact JSON map hash is
`93924a24ac070fc40118999b289eebdd8c67fd7f2e21ce85ab86a7c6b25e13ea`;
E's frozen CLI SHA-256 is
`2b7387ca90f7a9ba00f5a6990577b9f8805ee03284543319a1ef712d0be2be55`.
This ties that binary's functional observations to the published runtime;
the final independent receipt and accepted integration CI remain required.

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
Fresh independent corpus comparison and its exact row/counter accounting
remain acceptance gates below.

## Validation ledger

The permanent authored suite delta is 45 Rust tests: A adds 12, B adds 18,
C adds 4 and D adds 11. C adds 3 Python tests. The resulting inventory is
871 Rust tests on non-Windows, 872 on Windows, and 1181 Python tests; these
are expected counts until the final combined source completes the suites.
Temporary independent contracts do not inflate the checked-in suite count.

| Gate | Observed result at this checkpoint |
| --- | --- |
| Reader task CI | [Run 43852188720](https://github.com/yakisoba0728/vrfkit/actions/runs/43852188720): all 11 jobs succeeded for PR #71 head `9c4e698b829c1ea4c056512b2fa01060fdd7a516`; the synthetic merge tree equals source tree `970f489ec0bcdd6cd6c36523a7705a7ea1746e76` |
| Intermediate DX CI | [Run 43852794091](https://github.com/yakisoba0728/vrfkit/actions/runs/43852794091): 11 jobs succeeded; this predates the final combined Phase C source |
| Gameplay/combined task CI | [Run 43855117296](https://github.com/yakisoba0728/vrfkit/actions/runs/43855117296) for current PR #70 head `200d70d026fa9347bb967340f3cbefa14694ada0`: pending final result. The earlier run at `5ce480b...` is not the current source gate. |
| Decoder task CI | [Run 43853641134](https://github.com/yakisoba0728/vrfkit/actions/runs/43853641134): all jobs succeeded on the immutable C-only head `1b9bbf222f003e35c882e640c1b19e0bbcc88472`. The updated integration stack has [run 43854953389](https://github.com/yakisoba0728/vrfkit/actions/runs/43854953389), still pending here. |
| Stream task CI | [Run 43854479576](https://github.com/yakisoba0728/vrfkit/actions/runs/43854479576) for PR #73 head `f0ed266ee69b74f9e8a8de40b44cb9274aaf2b86`: pending final result |
| Final combined acceptance | Pending: actual accepted integration SHA, Rust 1.86, Windows/Linux, rustfmt, strict Clippy/rustdoc, feature matrix, Python, documentation, Rust/PyArrow and CLI/replay gates |
| Independent corpus sweep | The ten-input CLI comparison and typed/raw audit exited 0 across seven public builds with positive checkpoint work. Final exact table/counter totals, callback sweep and identity results remain pending the independent receipt. |
| Independent contracts and mutations | All 34 focused contracts passed (9 independent, 3 external-consumer, 11 Reader-checkpoint, 11 bounded-input). Final corpus probe and mutation/source-restoration receipts remain pending; these focused passes alone are not final acceptance. |
| Private archive | Metadata access covered one page with 27 assets. No actual private binary download was attempted and no private binary was tested. This is neither a complete private inventory nor an access-denial finding; historical large-corpus results retain their original revisions and scope. |
| Performance | Not executed; wall time, CPU, utilization, peak RSS, allocation counts and throughput improvements remain unmeasured |

The independent comparison must retain every existing main/checkpoint value,
raw RPC and parent window, all nine native movement-component counters and
manifest semantics, while explaining the exact FocusProjectiles delta.
No historical 1,018-replay audit is counted as a fresh Phase C sweep.

## PR and agent accounting

Five core task PRs are the limit: DX [#69](https://github.com/yakisoba0728/vrfkit/pull/69),
Gameplay [#70](https://github.com/yakisoba0728/vrfkit/pull/70), Reader
[#71](https://github.com/yakisoba0728/vrfkit/pull/71), Decoder
[#72](https://github.com/yakisoba0728/vrfkit/pull/72), and Streams
[#73](https://github.com/yakisoba0728/vrfkit/pull/73). All five slots are used.
Merge status is pending the
coordinator's final readback; task CI success alone does not mean merged.

Six unique agents were actually created: A, B, C, D, E and F. Maximum observed
concurrency was three non-root agents plus the coordinator. The first D spawn
was refused because capacity was full; a later retry created D. The temporary
execution disconnect and compiler SIGKILLs are distinct from worker crashes
or feature-test failures. Final completed/failed track counts are pending
the acceptance receipt.

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

Use the [migration and release checklist](PHASE_C_MIGRATION.md) before a
separately authorized release of **0.6.0 or later**. Keep main, tags and
registries untouched in this phase. Once the stated gates and five task PRs
are resolved, stop this phase without starting a broader audit.

The [user-run benchmark handoff](PHASE_B_BENCHMARK_HANDOFF.md) retains strict
output gates. It proposes the immutable C-only source as a schema-compatible
baseline and requires a final-candidate identical-binary control. Unmodified
Phase A/B binaries lack the new required keys even on zero-Focus inputs;
they are not valid baselines for the current harness.
