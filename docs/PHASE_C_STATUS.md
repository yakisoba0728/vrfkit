# Phase C execution ledger

## Recovered source and scope

Fresh GitHub API and HTTPS Git reads agree on the starting checkpoint:

- Integration: `588be1effc8dfca2bed8810fd51a006828081c1b`.
- Main: `9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9`.
- PR #58 is open, draft, and unmerged; it is the only open pull request.
- Historical Phase B push run `43839207666` and PR run `43839217064`
  succeeded. They are recovery evidence, not Phase C validation.
- No Phase C branch or ledger existed remotely at recovery.

Phase C implements bounded, evidence-supported non-performance work.
Quantitative performance measurement and repeated A/B benchmarks are excluded.
Only reviewed task pull requests may merge into `integration/vrfkit-overhaul`.
Main, tags, releases, package registries, the reference parser, and the private
archive remain outside the mutation scope. Private replay binaries and player
identifiers must not enter public logs, commits, or artifacts.

## Initial allocation

| Role | Branch | Initial state |
| --- | --- | --- |
| A: Reader checkpoints | `phase-c/reader-checkpoints` | Implementing optional callbacks and shared checkpoint decoding |
| B: Gameplay identity | `phase-c/gameplay-identity` | Checking lifecycle-aware identity evidence and exact movement provenance |
| C: Decoder correctness | `phase-c/decoder-correctness` | Bounded current-source reproduction; unproven types remain raw |
| D: Nonseekable input | Pending A's Reader contract | Queued; not yet created |
| E: Independent QA | Pending implementation | Queued; not yet created |
| F: Integration and DX | `phase-c/dx` | Coordinator establishes ledger; agent not yet created |

At this checkpoint three unique subagents were actually created. The observed
maximum concurrency is three non-root agents plus the coordinator. No agent
has completed yet. Later counts must use observed lifecycle outcomes.

## Acceptance and stopping rules

- Preserve raw payloads, existing default Reader behavior, CLI output, and
  native counter semantics. Document any Rust source migration explicitly.
- Exercise checkpoint isolation, ordering, corruption, callback errors, empty
  snapshots, multiple snapshots, and reused buffers with meaningful controls.
- Gameplay joins must preserve lifecycle/time and expose missing, null, and
  ambiguous evidence. Never retroactively resolve future GUID observations.
- Decode changes require exact identities, bit widths, complete consumption,
  independent values, and malformed negative controls. Otherwise record
  `blocked_by_evidence` and the missing input, without guessing.
- Run Rust 1.86, strict Clippy/rustdoc, feature cases, Python and interoperability
  checks, Windows/Linux CI, public replay/checkpoint/output comparisons, and
  independent QA on the actual accepted source.
- Keep at most five core task PRs; stop after this bounded phase. PR #58 stays
  draft and unmerged. Final performance acceptance belongs to the user.

No Phase C implementation, replay validation, final CI, or performance result
is claimed by this initial checkpoint. Final receipts will identify accepted
commits, exact runs and source correspondence, exclusions, and the user's
benchmark candidate.

## Execution interruption checkpoint

**Current result: `BLOCKED_BY_EXECUTION_ENVIRONMENT`. Functional acceptance is
not complete.** The allocation and zero-completion counts above describe the
initial checkpoint only.

Five unique subagents were created: A, B, C, E and F. All five returned a
checkpoint or review; that is not five completed feature tracks. Maximum
observed concurrency was three non-root agents plus the coordinator. D was
not started. No worker crash was observed; compiler SIGKILLs and executor
disconnection are recorded separately.

The Reader reached last-observed local commit
`0e92e629cb1062de7348a9ec775fe82062ac22c0` and partial core validation.
Its exact source is not published remotely. B preserved an explicitly
unverified gameplay reconstruction on `phase-c/gameplay-identity-unverified`
at `0e40d8c86623b4d1f0a78be9395fa213e54cc244`. E preserved metadata only on
`phase-c/qa-recovery` at `ad64e9ee92697a3cf0f288641f154997878ef788`;
its prepared QA sources still require original filesystem recovery.

The decoder record has been corrected with E's later narrow historical
Clay FocusProjectiles evidence. No decoder route has been implemented.
Ten public inputs are prepared for a later functional sweep; no fresh
Phase C sweep completed.

Main and accepted integration remain the exact starting SHAs above. PR #58
remains open, draft and unmerged. No final Phase C candidate, feature CI,
performance comparison or publication is claimed.

The [recovery checkpoint](PHASE_C_RECOVERY.md) and
[structured receipt](evidence/phase-c-recovery.json) record exact source
boundaries, observed checks, unobserved operations and restart order.

## Resumed after environment reconnection

The interruption checkpoint above is historical. The original environment
reconnected during the same session. Fresh filesystem inspection recovered
A's exact local commit, B's original staged source and all four E QA sources.
The original A source is now published on `phase-c/reader-checkpoints` at
`9c4e698b829c1ea4c056512b2fa01060fdd7a516`, with tree
`970f489ec0bcdd6cd6c36523a7705a7ea1746e76` exactly matching the local candidate.
The recovered QA source draft is published on `phase-c/independent-qa` at
`57f7f9da19bd267917378526e96cec67e5b7ea66`, tree
`623192b3844b33a7d34e80962a7470e3fe91b213`.

Runtime implementation and required acceptance have resumed. B confirmed all
15 new tests passed after reconnection; E successfully reran the expanded
historical FocusProjectiles evidence receipt. These are intermediate results,
not final acceptance. C is now implementing the narrow evidence-qualified
array route. Historical unverified recovery branches remain unchanged.
Main/integration and PR #58 retain their accepted starting states until the
task PR gates pass. The performance prohibition remains in force.
