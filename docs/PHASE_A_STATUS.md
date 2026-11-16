# Phase A recovery and execution record

This is the bounded continuation of the earlier overhaul, not a new audit of
all project features. Historical receipts and measurements remain unchanged.
Phase B, main integration, releases and registry publication are excluded.

## Verified recovery checkpoint

- Remote integration: `d44bcd2b22209cf6a0fabf3996ecc514f5cab606`.
- Remote main: `9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9`.
- PR #58 is open, draft and unmerged; integration is 29 commits ahead.
- PRs #47-#57 and #60-#62 are merged. Their implementations are retained.
- [CI 182](https://github.com/yakisoba0728/vrfkit/actions/runs/43817290464)
  passed 10 jobs. [Integration validation 29](https://github.com/yakisoba0728/vrfkit/actions/runs/43817282538)
  passed 11 jobs. Both reference the integration revision above.
- The six previously obtained public replay inputs and Rust 1.86 caches were
  found locally. Existing uncommitted work was not assumed: a clean checkout
  was obtained from GitHub before inspecting old candidates.

## Work allocation

The observed concurrency limit is four active agents including the coordinator.
The initial three subagents were actually created and observed running together.
Further roles are queued as slots become free. Only the coordinator may merge.

| Track / role | Branch | Current state |
| --- | --- | --- |
| A / parser correctness | `fix/phase-a-native-rpc` | Recovered local-only candidate `97e6c59`; fresh failing-before regression and fixed tests pass; real-input acceptance pending |
| B / profiling | `perf/phase-a-profiling` | Real export stage profiling in progress; no new performance improvement accepted |
| C / performance implementation | Not yet assigned | Wait for measured bottleneck evidence |
| D / Reader input | `feat/phase-a-reader-input` | Bounded file/chunk input implementation and contract tests in progress |
| E / independent QA | Queued | Must independently assess native preservation, optimization equivalence and Reader behavior |
| F / CI and integration evidence | Queued | Final exact revision CI and acceptance record |

## Native recovery

The previously unpublished `97e6c59` candidate contains a signed 32-bit native
byte-array envelope correction and preservation of unsupported A3/A4 bodies.
The current GitHub integration still loses the complete raw RPC in a generated
unsupported-A3 case while retaining its movement prefix. The fresh regression
fails on that baseline and passes with the recovered correction.

The recovered implementation is committed as
`603aa714f0d677b133b7193b5be6b69c894c55cd`. Its runtime source files match the
historically measured native build. That correspondence is not a new replay
execution. Fresh tests and remaining checks are distinguished in
[the recovery receipt](evidence/phase-a-native-recovery.json) and
[the framing and migration guide](NATIVE_MOVEMENT_COMPONENTS.md).

Before merge: fresh public main/checkpoint exports, table/hash and manifest
comparison, independent review, and all required hosted jobs must pass.

## Measurement policy and outstanding work

Performance comparisons will use identical native contracts, pinned executables,
alternating order, warmups, repeated measurements and an identical-executable
control. Reverse-order reports must be interpreted using executable hashes,
not their baseline/candidate labels alone. No existing synthetic or noisy
experiment is promoted into an ordinary-replay speed claim.

Reader input work must preserve borrowed callback ordering, error provenance,
early consumer cancellation and provisional partial output. The CLI's existing
output is verified separately. An incomplete prototype will remain outside the
stable default path.

Original replay files, the external reference repository and the private
archive are read-only. No private replay has been downloaded for this phase.
Detailed replay values and player identities remain outside public evidence.

## Resume commands and stopping rule

Fetch the remote integration and working branches first. Inspect open PRs,
latest commits and exact revision CI before resuming; this file is a checkpoint,
not a claim that later work has already finished. The live progress record is
also retained on [PR #58](https://github.com/yakisoba0728/vrfkit/pull/58).

Run the full sweep from `CONTRIBUTING.md`, then the affected public replay and
independent QA checks. Merge only verified task PRs into integration and verify
the resulting integration revision itself. Stop after the three Phase A tracks;
do not start Phase B automatically.
