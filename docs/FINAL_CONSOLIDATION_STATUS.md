# Final consolidation and independent QA

## Current state

**IN_PROGRESS**. This continuation audits and improves the recovered Phase A-D
candidate. It is not final acceptance, permission to merge main, or permission
to publish. The sole review entry point remains
[Draft PR #58](https://github.com/yakisoba0728/vrfkit/pull/58).
The [fixed acceptance receipt](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6932599335)
will record the final source and actual CI/artifact identities after all gates.

## Recovered immutable baseline

| Item | Observed identity / state |
| --- | --- |
| Main | `9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9` |
| Integration | `8581642283bd5a571967129d73897db8ee5408ba` |
| Integration tree | `03fb832d96714343e85c30ea9468993c1af63d97` |
| PR synthetic merge | `7360230343071cc5f357c49b0fe765602f11f75e`; tree equals integration |
| Cargo.lock SHA-256 | `ef4c199b359baeca53be4c5d634a288ab4c7e00e1590c8d0948b186e5c5c316a` |
| PR state | Only open PR; Draft, unmerged, automatic merge disabled |
| Source version | Ten production packages at unpublished `0.6.0` |
| Publication baseline | Ten tags through `v0.5.0`; latest public release `v0.5.0` |
| Baseline CI | Push `43886907894` and PR `43886919331` succeeded historically; not this continuation's final gates |

Main still contains none of the integration-only changes. No rollback is
needed. Main, the private archive, reference repositories, original inputs,
tags and releases are outside the mutation scope. No extra PR will be opened.
Local worktrees isolate changes; only independently reviewed changes reach
`integration/vrfkit-overhaul` with fast-forward publication.

## Recovered phase contracts

| Phase | Accepted work retained | Evidence / limits |
| --- | --- | --- |
| A | Signed native RPC framing/raw fallback and bounded seekable Reader input | [Results](PHASE_A_RESULTS.md); rejected timing experiment remains rejected |
| B | Reader/CLI streaming integration, gameplay projections and export improvements | [Results](PHASE_B_RESULTS.md); prior executions retain their original scope |
| C | Opt-in independent checkpoint callbacks, explicit bounded nonseekable input, ordered gameplay identity, exact 12.05 FocusProjectiles route | [Results](PHASE_C_RESULTS.md), [migration](PHASE_C_MIGRATION.md); no speculative wire semantics |
| D | Controlled qualification and unpublished 0.6.0 metadata | [Results](PHASE_D_RESULTS.md); six primary performance conditions inconclusive and zero accepted runtime optimizations |

All nine native counters, both FocusProjectiles counters, typed/raw preservation,
Reader error identity, checkpoint separation, identity ambiguity/lifetimes and
atomic output contracts remain required. Historical evidence is retained, not
relabelled as new execution.

## Execution ownership

| Role | Worktree / branch | Current scope |
| --- | --- | --- |
| A | `final/core-correctness` | Completed: malformed movement identity fix and author regression |
| D | `final/reader-fixes` | Export source protection and diagnostic replacement; tests in progress |
| E | `final/independent-qa` | Independent core/Reader checks complete; frozen candidate corpus and mutation checks in progress |
| F | `final/ci-release` | Baseline audit complete; resumed for final exact-HEAD CI and artifact gates |
| B | `final/architecture` | Completed: checkpoint path admission fix and architecture review |
| C | `final/performance-review` | Completed: bounded source/harness review; no fresh timing claims |

Six unique specialists have been created; the observed maximum is three
specialists concurrently, plus the coordinator. A, B and C are complete. F
completed its first audit and has resumed for final gates; D and E are active.
No agent failure has been observed. Final completion counts will be recorded
in the acceptance receipt.

## Implemented correction checkpoint

The isolated, clean runtime source is
`1d2d8f6fe68f3ad1f196bca5adac24b0d3fbee6c` (tree
`833dae7add06f21589ffdae9680d5df12a5f89fe`). This is the local build source;
the published checkpoint is reconciled by complete tree and file fingerprints.
All six corrections are implemented. Final regular-suite and hosted checks
remain pending in this checkpoint.

| Severity | Correction | Completed evidence |
| --- | --- | --- |
| P1 | Refuse an export directory containing its replay source, including output-owned filenames and canonical aliases | Author baseline failure; fixed package suite 259 pass; independent six containment refusals and safe sibling/hardlink cases pass |
| P2 | Keep the prior diagnostic report if Windows rename fails | Delete-before-rename removed; author regression passes; compiled old-algorithm Linux negative control and restoration in progress; native Windows gate pending |
| P2 | Clear stale shooter identity after a malformed repeated GUID | Author before/after and independent Reader/partial/two-checkpoint checks; compiled mutation detected and restored executable hash equal |
| P2 | Bound resolved checkpoint GUID paths before amplification | Seven new author tests; independent public 256 MiB exact-boundary/excess probes fail before and pass after |
| P2 | Reject direct partial-buffer bit claims beyond available bytes | Two author tests and three independent public-API tests preserve prior/neighbor state and honest byte accounting |
| P2 | Avoid signed packet-length overflow | Allocation-free old-arithmetic boundary fails before; checked-wide regression and full network package debug/release suites pass |

The [migration](FINAL_CONSOLIDATION_MIGRATION.md) covers the new public enum
cases and fixed checkpoint policy. No dependency, lockfile, generated wire
mapping, workflow, production version or required counter is changed.

The frozen default-release candidate is
`c3a3010083b81332e03d90b888d38dfe069dfb659f3c866637cfed0051d6d417`.
All workspace release artifacts were cleared before recompilation; Cargo
reported current source paths and `fresh: false` for all 11 local artifacts.
The earlier baseline SIGKILL remains a failed attempt; its separate serialized
retry succeeded with unchanged release flags.

Fresh independent candidate results already completed: five public inputs,
59 checkpoints, 65 byte-equal Parquet tables, 43,844 typed/raw comparisons,
all 16 evidence specifications observed, 30 main-only tables unchanged,
39 JSONL tables/138,727 rows, 22 detected missing-counter controls and 25 CLI
adversity cases. The complete stable manifests and 153 CLI counter values per
input match baseline. All 43 selected integrated contracts pass. Real-input GameplayIdentity and
final-source mutation receipt integration are still running. These observations add no private-corpus coverage.

## Evidence and acceptance gates

The [evidence index](evidence/final_consolidation/README.md) links new receipts.
Implementation findings need a failing-before regression, a fix, independent
review and a passing-after result. The final gates include all Rust/Python,
feature, documentation and public-replay CI jobs on both the actual integration
checkout and PR synthetic merge, plus Windows package provenance and execution.

Local Rust 1.86 and Python 3.12 execution has been prepared. GitHub provider
access reads private archive metadata; the local Git client has no private
archive or write credential. The connected GitHub mutation API is used for
authorized integration publication; no credential extraction is needed.
Private metadata contains 1,011 assets across 24 releases; actual binary
downloads, hash checks and replay executions remain **NOT_EXECUTED (0)** because
no available authenticated binary transport serves private Release assets.
A metadata read is not a replay test; no credentials were extracted.

## Recovery procedure

1. Read PR #58 and resolve its current head; do not assume this document embeds
   its own future commit hash.
2. Fetch `integration/vrfkit-overhaul`, verify the complete tree and inspect
   local worktrees before continuing.
3. Resume only incomplete entries in this ledger and verify each receipt's
   source revision. Do not recreate completed Phase A-D work.
4. Run final actual-HEAD CI after the last source/documentation integration;
   retain the independent receipt, synthetic checkout and artifact identities
   in PR #58 without another documentation-only validation loop.
5. Leave PR #58 Draft/open/unmerged and main/releases unchanged, then report
   `READY_FOR_MAINTAINER_REVIEW` or `BLOCKED_WITH_EVIDENCE` honestly.
