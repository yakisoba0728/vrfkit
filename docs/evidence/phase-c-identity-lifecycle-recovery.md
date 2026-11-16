# Phase C identity recovery: bounded lifecycle regression

This is a recovery candidate, not a Phase C completion or merge approval.
No benchmark or quantitative performance comparison was executed.

## Provenance

- Accepted integration: `588be1effc8dfca2bed8810fd51a006828081c1b`.
- Preserved original backup: `0e40d8c86623b4d1f0a78be9395fa213e54cc244`.
- Recovery branch: `phase-c/gameplay-recovery-validation`, Draft PR #70.
- The former Reader worktree and commit `0e92e629cb1062de7348a9ec775fe82062ac22c0`
  were not accessible in this session. They have not been recreated or certified.
- No subagents were created in this recovery session. Tests below are separately
  authored public-API regressions, not a claim of an independent agent review.

## Executed tests before the fix

Run [43852181714](https://github.com/yakisoba0728/vrfkit/actions/runs/43852181714)
checked out `1ffe7c3cb23179e9376f470347ace7937b12acdc` on Linux/Rust 1.86.0.
Its diagnostic workflow formatted tracked public Rust source in the runner only.
All 15 existing identity unit tests passed before and after mutation restoration.
Two compiled mutants were killed by their specific assertions (exit 101):

1. Removing exact packed-GUID payload consumption allowed a suffix.
2. Removing the active repeated-open check accepted a contradictory lifecycle.

Artifact `14219265176` SHA-256:
`ddc42f6c20553fb3debba29099467301996651ea673af2e9b426825b31d75156`.
Formatted-source tar SHA-256:
`ec4cb3aae1f7d8412df8012e7943e45501ce8c27ed9a5f8ec87f5708a2dcf8cb`.
This artifact contains public source and synthetic test evidence only.

The separately authored `tests/gameplay_lifecycle.rs` was then committed as
`57f992575989e03d94c33ad4c5a89772fcd08494` before changing runtime behavior.
Run [43852941649](https://github.com/yakisoba0728/vrfkit/actions/runs/43852941649),
Linux job `133249985574`, checked out PR merge
`ad21dd673367c03dcf8af014396f583687031309` and compiled the all-feature workspace.
Both retired-lifetime regressions failed their assertions with an unexpected
`Ambiguous` result; the same-packet negative control passed. This was a runtime
assertion failure, not a compile failure. Later steps skipped after that failure
are not reported as passed.

## Narrow fix and preserved contracts

When a current lifetime has an observed previous end, a declaration strictly
before that end cannot describe the new lifetime. It remains stored for earlier
queries but no longer competes with a fresh, independently observed relation.
This is checked for both the PlayerState and character lifetime before counting
ambiguous candidates. A sole retired relation remains explicitly unresolved as
`PreviousLifetime` rather than being silently resolved.

Equality is deliberately not discarded: a declaration in the same packet as an
end has unknown ordering relative to the other record vector. The negative
control must continue to return `Ambiguous` when another candidate exists.
Historical queries are asserted in each public-API regression.

The raw decoder, Reader, CLI, schema tables, manifest and export writers are not
changed. Identity remains explicitly opt-in. No account/name/kill-credit join,
final-GUID backfill, proximity match or new round inference is introduced.

## Outstanding acceptance gates at this commit

The fix has been authored, but its fresh full CI result is not yet known at this
checkpoint. Required remaining checks include formatting, strict Clippy,
Windows/Linux and feature tests, rustdoc, measured documentation counts,
public-replay smoke/evidence comparison and source/checkout attribution.
Public corpus and private-corpus acceptance must not be inferred from the
synthetic tests above. PR #70 stays Draft/unmerged until its relevant gates pass;
PR #58 stays Draft/Open/Unmerged. Main, tags, release and package registries remain
untouched. No new final Phase C benchmark candidate has been certified.
