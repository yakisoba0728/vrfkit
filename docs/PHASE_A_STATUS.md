# Phase A recovery and execution record

This is the bounded continuation of the earlier overhaul, not a new audit of
all project features. Historical receipts and measurements remain unchanged.
Phase B, main integration, releases and registry publication are excluded.
The [results record](PHASE_A_RESULTS.md) retains completed measurements,
accepted changes and negative results. The
[final acceptance receipt](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6909992420)
is authoritative for the final integration HEAD, final PRs, independent
acceptance and actual post-merge CI. Earlier task CI is not final acceptance.

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
Six distinct subagents have now actually been created. Three subagents were
observed active together. Completed roles may be resumed; another task on the
same agent does not count as a newly created agent. Only the coordinator may
merge. Final completed/failed totals remain to be recorded when work ends.

| Track / role | Branch | Current state |
| --- | --- | --- |
| A / parser correctness | `fix/phase-a-native-rpc` | Recovery, fresh six-public audit and independent review complete; PR #63 merged into integration |
| B / profiling | `perf/phase-a-profiling` | Completed measured bottleneck investigation; evidence folded into performance branch |
| C / performance implementation | `experiment/phase-a-movement-inline` | Candidate measured across 60 equivalent exports, then withdrawn; zero runtime optimizations accepted |
| D / Reader input | `feat/phase-a-reader-input` | Corrected bounded input measured on two real inputs; six-input equality and rejected first design retained |
| E / independent QA | `test/phase-a-independent-qa` | Native and combined Reader QA passed; independent memory proof and measurement review retained |
| F / CI and integration evidence | `docs/phase-a-integration-evidence` | Results and observed CI receipts prepared; exact final HEAD/CI published in the final receipt |

## Native recovery

The previously unpublished `97e6c59` candidate contains a signed 32-bit native
byte-array envelope correction and preservation of unsupported A3/A4 bodies.
The starting GitHub integration loses the complete raw RPC in a generated
unsupported-A3 case while retaining its movement prefix. The fresh regression
fails on that baseline and passes with the recovered correction.

The recovered implementation is committed as
`603aa714f0d677b133b7193b5be6b69c894c55cd`. Its runtime source files match the
historically measured native build. That correspondence is not a new replay
execution. Fresh tests and remaining checks are distinguished in
[the recovery receipt](evidence/phase-a-native-recovery.json) and
[the framing and migration guide](NATIVE_MOVEMENT_COMPONENTS.md).

Fresh public validation passed six inputs, 76 checkpoints, 1,958,155 scored
blocks and 60,169 typed/raw comparisons. All 78 Parquet files are byte-identical
to the accepted baseline; manifest differences are limited to the documented
counter migration and elapsed time. Independent QA passed 31 generated
scenarios, two runtime-failing mutation controls, a separate direct read of all
78 files, 24 manifest negative controls and 151 source hashes. The receipts
retain the original frozen executable and baseline-output attribution.

Published native checkpoint `cdbda315cf78b86ba61a2eb55252d5bbc797c2c0`
has the same tree as local `bad13721fff8eff692e9a1c1bc08f9d72d31109a`.
[Run 43820770749](https://github.com/yakisoba0728/vrfkit/actions/runs/43820770749)
passed all 11 jobs; the [CI receipt](evidence/phase-a-native-ci-checkpoint.json)
records observed counts and synthetic-checkout/tree correspondence.
The later head `41d3dcabfdceabc37dea66c8091524fc2854a638` adds independent
QA evidence. Its own
[run 43821570049](https://github.com/yakisoba0728/vrfkit/actions/runs/43821570049)
passed all 11 jobs before PR #63 merged at
`3e61f331f5e41b3ab34c8e5ba45d5bc04e021999`. The actual synthetic checkout
`c50f8ce1a14cb9779e90ce8ca652c48f20a611b7` and that integration merge share
tree `5f72cae0dc55b00779901155c1dffbff6a544382`, as verified in the
[accepted Native CI receipt](evidence/phase-a-native-pr-ci.json).
This correspondence does not relabel a PR run as final post-merge execution.

## Measurement policy and outstanding work

Performance comparisons will use identical native contracts, pinned executables,
alternating order, warmups, repeated measurements and an identical-executable
control. Reverse-order reports must be interpreted using executable hashes,
not their baseline/candidate labels alone. No existing synthetic or noisy
experiment is promoted into an ordinary-replay speed claim.
The narrow candidate added only `#[inline(always)]` to `parse_single_move`.
Sixty complete exports passed strict equality, but same-binary and reverse
controls did not establish a reliable performance benefit. The annotation was
withdrawn. The exact experimental source is preserved at GitHub commit
`8b5e5502f8fd5ab182b84bee8a8db2cdca7a545c`; the integrated proposal has no
runtime change from this experiment. See [the decision](PHASE_A_PERFORMANCE.md).

Reader input work must preserve borrowed callback ordering, error provenance,
early consumer cancellation and provisional partial output. The CLI's existing
output is verified separately. An incomplete prototype will remain outside the
stable default path.
Reader revision `d78f30a` substantially reduced RSS but increased paired wall
time by about 27%; the negative result is retained. Corrected `6a9ea4e` uses
independent read-only file handles for header traversal and decompression,
with one prepared decompressed chunk of lookahead. Its paired peak RSS fell
53.68% on 12.09 and 63.53% on 12.05. Timing changes overlap the identical-binary
control variation, so no speed claim follows. The measurement, correctness and
resource limits are retained in [the Reader report](READER_INPUT_STREAMING.md).
Combined-source QA and final integration CI are recorded separately in the
final receipt.
The [combined QA receipt](evidence/phase-a-reader-independent-review.json)
records six independent helper tests, the 26 existing Reader/preamble tests,
and a 2 GiB sparse-file read under a 512 MiB address-space limit. The same
executable's whole-file buffering failed explicitly with `OutOfMemory`.
This synthetic skip proof is not a large-decoded-chunk memory guarantee.
The coordinator also freshly built the combined default-feature CLI and
repeated the six-public main/checkpoint audit. All 78 Parquet files match the
fresh Native-accepted exports; same-contract manifests differ only in elapsed
time. [The combined receipt](evidence/phase-a-final-cli-validation.json)
records 76 checkpoints and 60,169 typed/raw comparisons with zero mismatches.

Original replay files, the external reference repository and the private
archive are read-only. No private replay has been downloaded for this phase.
Detailed replay values and player identities remain outside public evidence.

## Resume commands and stopping rule

Fetch remote integration and existing branches first. Compare the remote HEAD
to the exact SHA in the final acceptance receipt, then inspect PRs and source
differences. Do not resume from a withdrawn experiment or an earlier local
measurement checkpoint. The live record is retained on
[PR #58](https://github.com/yakisoba0728/vrfkit/pull/58).

Run the full sweep from `CONTRIBUTING.md`, then the affected public replay and
independent QA checks. Merge only verified task PRs into integration and verify
the resulting integration revision itself. Stop after the three Phase A tracks;
do not start Phase B automatically.
