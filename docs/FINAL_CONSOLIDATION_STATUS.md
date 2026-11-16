# Final consolidation and independent QA

## Current state

**IN_PROGRESS**. This continuation audits and improves the recovered Phase A-D
candidate. It is not final acceptance, permission to merge main, or permission
to publish. The sole review entry point remains
[Draft PR #58](https://github.com/yakisoba0728/vrfkit/pull/58).

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
| A | `final/core-correctness` | Core bounds, raw preservation and movement attribution |
| D | `final/reader-cli` | Reader, gameplay, CLI publication/input preservation |
| E | `final/independent-qa` | Independent regression, adversarial and public-corpus verification |
| F | `final/ci-release` | Bounded CI/package audit complete; final exact-HEAD gates pending |
| B | Not started yet | Architecture and incremental refactoring; queued by worker capacity |
| C | Not started yet | Performance/memory review; bounded by valid measurement evidence |

Maximum available concurrency is three specialists plus the coordinator.
Actual created/completed counts will be recorded at final acceptance rather
than inferred from the six role names.

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
Private binary acquisition and validation are still unexecuted at this
checkpoint. A metadata read is not a replay test.

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
