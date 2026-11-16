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
