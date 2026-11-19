# Phase C recovery checkpoint

## Reconnection supersedes the interruption status

The original environment reconnected later in the same session. Original
Reader, Gameplay and QA source files were recovered. The interruption
record below remains a historical receipt; its unavailable-source and
blocked-execution statements describe that earlier checkpoint. Current
progress and exact recovered source refs are in [the status ledger](PHASE_C_STATUS.md)
and [Phase C results](PHASE_C_RESULTS.md). The structured interruption receipt
is intentionally preserved as historical evidence, not a live status report.
Functional acceptance is tracked separately; recovery alone does not satisfy it.

## Historical status and accepted source at interruption

**At this interruption checkpoint, Phase C was blocked by the unavailable
execution environment. Functional acceptance was incomplete and no final
Phase C candidate existed.** The statements and track table below describe
that point in time, before original source recovery and resumed implementation.

The accepted integration remains
`588be1effc8dfca2bed8810fd51a006828081c1b`. Main remains
`9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9`. PR #58 stays open, draft and
unmerged. These are the verified Phase B source checkpoints, not newly
accepted Phase C features.

The executor failed with a disconnected transport and recovery timeout.
The selected environment then reported unavailable; a subsequent observed
agent result was `environment_offline`. No environment restart/reconnect
operation is exposed in the current tool set. GitHub connector reads and
checkpoint writes still work. This is an execution-availability blocker,
not an approval request or evidence of denied repository access.

No runtime change was merged, no final Phase C CI was run, no performance
benchmark was executed, and no main/tag/release/registry mutation was made.

## Preserved and unavailable work

| Track | Last observed result | Acceptance and source boundary |
| --- | --- | --- |
| A: Reader checkpoints | Shared snapshot decoder, opt-in callbacks, isolated snapshot state, explicit provenance, error ordering and 12 added Rust tests; local commit `0e92e629cb1062de7348a9ec775fe82062ac22c0` | Local core validation only. Exact committed source is not published remotely and cannot currently be read. Shared CLI/all-features compilation, independent public output equivalence and hosted CI are pending. |
| B: Gameplay identity | Exact SpawnedCharacter observations, lifecycle-aware resolution, and transport-provenance movement association; 15 authored tests | A reconstructed, explicitly unverified backup is published on `phase-c/gameplay-identity-unverified`. It is not a byte-identity attestation of the formatted local worktree. No Rust test completion was observed. |
| C: Decoder correctness | Bounded evidence review, 72 focused Python checks, one meaningful source mutation; no decoder modification | Initial record is preserved on `phase-c/dx`. E later established a narrow Clay FocusProjectiles candidate; see the superseding section in `PHASE_C_DECODER_EVIDENCE.md`. No new route was accepted. |
| D: Nonseekable input | Not started | No agent, implementation, or acceptance result exists for this track. Resume after recovering A's actual API/source. |
| E: Independent QA | A/B source review, one lifecycle finding, historical source/binary attribution, bounded historical array evidence, and unexecuted QA harness preparation | Metadata-only recovery is on `phase-c/qa-recovery`. The four prepared QA source files are not preserved in that branch; original filesystem recovery is required. No fresh Phase C public export or independent functional acceptance completed. |
| F: Integration/DX | Independent verification of remote refs, PR state and recovery-claim boundaries | Recovery review only; final migration/release readiness and final acceptance are pending. |

See `evidence/phase-c-recovery.json` for exact backup refs, source trees and
observed checks. Recovery branches must remain unmerged until their source,
formatting, test inventory and full required acceptance are verified.

## A: original source recovery

The last observed author commit was clean:

```text
local branch: phase-c/reader-checkpoints
local commit: 0e92e629cb1062de7348a9ec775fe82062ac22c0
worktree: /workspace/scratch/060a4b490a9f/phase-c-reader
shared object database: /workspace/scratch/060a4b490a9f/vrfkit/.git
```

Inspect the original object database after reconnecting. The coordinator's
attempted README/USAGE test-count update produced no execution result.
E's attempted cherry-pick of A also produced no observed completion.
Therefore inspect current HEAD, index, worktree and interrupted-operation
state before applying any change. Preserve all existing edits.

The original A commit changes 14 files. New files are
`crates/vrfkit/src/checkpoints.rs`,
`crates/vrfkit/tests/reader_checkpoints.rs`,
`crates/vrfkit/examples/stream_checkpoints.rs` and
`docs/evidence/phase-c-reader-checkpoints.md`.
Modified files are `crates/vrfkit/src/driver/checkpoints.rs`,
`driver/mod.rs`, `error.rs`, `lib.rs`, `pass.rs`, `reader.rs`,
`reader_input.rs`, `sink/mod.rs`, plus
`crates/vrfkit/tests/movement_reader.rs` and `docs/RUST_API.md`.

Do not reconstruct a supposed tested commit from a summary. If the original
source cannot be recovered, a replacement is a new candidate requiring fresh
validation, not the already-tested local source.

### Observed A validation

- Eleven new checkpoint integration tests passed.
- The movement test file passed three cases: two prior cases and one new
  nonempty checkpoint movement/raw/provenance/counter comparison.
- The core-only vrfkit suite passed 180 cases **before** the last movement
  case was added. Do not relabel this as a later whole-suite pass.
- Core-only test type checking, strict all-target Clippy, strict rustdoc,
  rustfmt, ASCII and diff whitespace checks passed.
- Decrementing the global checkpoint field offset made the intended snapshot
  ordering assertion fail with exit 101. Restoring the exact original source
  made the subsequent core suite pass.

The successful local runs used Rust 1.86.0, one build job, no incremental
compilation or debug information, and explicit local dev/test dependency
optimization level zero. Cargo.toml and hosted CI profiles were unchanged.
Three earlier external-dependency compilations ended with SIGKILL and are
failed attempts, not test results.

The live documentation count would increase by 12 for A alone, but the full
workspace and Windows counts must be confirmed by their actual runs before
acceptance. Existing default Reader methods continue skipping checkpoints;
the new options opt into snapshot callbacks. ReplayEvent and the summaries
were already non-exhaustive. The additional typed checkpoint schema error
still needs its documented source-migration review.

## B and E verification boundaries

B's pre-outage formatting, ASCII, fast documentation and whitespace checks
passed on the local staged candidate. Compilation then started, with only
an external dependency compile line observed. The later transport failure
did not return a successful test result. Its small final comment clarification
did not apply: the pending patch failed with `environment_offline`.

E found an active actor reopening on either the same or another channel
could leave earlier identity evidence eligible. B changed the candidate and
authored a regression case. The test has not been observed executing.

E verified the historical frozen Phase B CLI SHA-256
`29fa495b19b7dbce2bfe7bbedc1dc9be2492a4c6a5aca2a9d1468d3d74c657d4`
and all 133 recorded runtime source hashes against the starting baseline.
Those accepted historical exports are useful comparison inputs. They are
not fresh Phase C output.

The four unavailable prepared QA files were
`checkpoint_probe.rs`, `public_qa.py`, `candidate_inventory.py` and
`focus_array_evidence.py`, under
`/workspace/scratch/060a4b490a9f/phase-c-qa/docs/evidence/phase_c/independent_qa/`.
The callback adapter was formatted, the Python harness passed syntax checking,
and the historical candidate inventory executed. None establishes a compiled
or frozen final Phase C QA harness.

## Public and private input scope

The six existing public inputs for 12.05, 12.08, 12.09, 12.10, 12.11 and
13.00 were SHA-256 checked against the tracked Phase B receipt; all matched.
Four additional small public fixtures were downloaded from pinned reference
commit `b51d67423b7b4952d59051cf91e55efa1c42da05`. Their Git blob identities
were verified and SHA-256 values recorded. The accepted Phase B CLI inspected
their metadata: one additional 12.08 and three 13.01 inputs.

That prepares a bounded ten-input/seven-build final QA scope. It does not
claim ten successful Phase C decodes. No fresh Phase C output comparison
completed. Preserve the distinction between six historical baselines and
four added inputs requiring attributable before/after output.

The private archive check was metadata-only: repository access and one
release page with 27 assets. It was not a full archive inventory. No private
binary was downloaded or decoded, no private repository was modified, and
no private replay bytes or player account identifiers were published.

## Resume order and limits

1. Reconnect the original execution environment and inspect all worktrees and
   interrupted processes before changing them. Recover A's exact local commit
   and E's uncommitted QA sources if available.
2. Compare B's remote unverified backup with its original staged source.
   Review any differences; run formatting and the 15 authored tests. Recover
   or independently rebuild QA sources with explicit provenance.
3. Publish A on a task branch only after checking source identity and updating
   actual measured documentation counts. Run shared CLI/all-feature checks
   and Windows/Linux CI. Independently compare the callback tables, schemas,
   raw bytes, ordering, all counters and manifests.
4. Complete B's independent temporal/lifecycle and movement-provenance
   acceptance. Keep missing, ambiguous, null and unresolved states explicit.
5. Revisit only the narrow evidenced Clay array candidate. Any implementation
   must retain raw parents, qualify both checksums and explain the observed
   53 additive child rows. Other evidence-gated candidates remain bounded
   follow-up work.
6. Start D only against A's recovered stable API. Require a bounded explicit
   input/storage contract and cleanup/error tests; do not add automatic
   unbounded buffering or disk spooling.
7. Finish the ten-input functional comparison, meaningful mutation controls,
   required full suites/platform/feature/interop checks, DX/migration and
   release checklist for a future version at least 0.6.0. Do not publish.
8. Merge only accepted task PRs into integration, with at most five core task
   PRs. Validate the actual final integration HEAD. Keep PR #58 draft/open.
9. Record a new final candidate SHA for the user's later benchmark handoff
   only after functional acceptance. Do not run benchmark commands.

The five unique subagents were A, B, C, E and F; D was not created. The maximum
observed simultaneous execution was three non-root agents plus the
coordinator. Returned checkpoints and reviews are not equivalent to accepted
features. No further runtime work is started while this execution blocker
remains.

## Remote backup references

- [Gameplay candidate and reconstruction receipt](https://github.com/yakisoba0728/vrfkit/tree/0e40d8c86623b4d1f0a78be9395fa213e54cc244):
  commit `0e40d8c86623b4d1f0a78be9395fa213e54cc244`, tree
  `a44a3f2685c2ae0a1eddfbc27f0906d5c7f0b523`.
- [Independent QA metadata-only receipt](https://github.com/yakisoba0728/vrfkit/blob/ad64e9ee92697a3cf0f288641f154997878ef788/docs/evidence/phase_c/independent_qa/RECOVERY_UNVERIFIED.md):
  commit `ad64e9ee92697a3cf0f288641f154997878ef788`, tree
  `0878868ec49dab4c77517ebe6ab9593863b3c3d8`.

Both refs, parent source and changed-file scopes were re-read through GitHub.
This verifies the stored backup identities, not runtime correctness.
