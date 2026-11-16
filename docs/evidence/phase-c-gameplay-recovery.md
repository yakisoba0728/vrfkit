# Phase C Gameplay recovery checkpoint - UNVERIFIED

This branch is an implementation backup, not an accepted feature candidate.
Do not merge it without restoring the normal Rust, replay, independent QA and
Windows/Linux CI gates.

## Why this branch exists

The shared execution environment became unavailable during the first Track B
Rust build. The last observed compiler output was only:

```text
Compiling proc-macro2 v1.0.107
```

No Rust compilation or test completion, gameplay example execution, mutation
run, fresh public replay comparison, or Phase C CI pass was observed for B.
There is no performance measurement or performance-improvement claim.

Parent/source base: `588be1effc8dfca2bed8810fd51a006828081c1b`.
The original local branch was `phase-c/gameplay-identity` and had no new local
commit when the environment disconnected.

## Reconstruction provenance

The five candidate files in this commit were reconstructed from the complete
recorded inputs of successful apply_patch calls and the two pinned base files,
using the connected GitHub Git-data API. Existing gameplay.rs and GAMEPLAY_API.md
were fetched at the base commit and transformed by their exact known edits.
The three new Rust files reproduce the successful authored patch bodies.
The confirmed repeated-active-open correction and its synthetic test are
included.

This is **not a byte-identical recovery claim for the formatted local worktree**.
The local rustfmt transformation was observed to succeed but its full formatted
file bytes and hashes were not captured before disconnection. Recovered new
source therefore uses the original authored patch formatting and must run
rustfmt again. No local source/tree hash is invented.

A final two-line code/doc clarification was attempted during the disconnect.
Its result was initially unobserved; after this backup was created, draining the
pending call returned a verification failure before reading identity.rs:
`409 Conflict, environment_offline: Environment is not connected`.
The patch therefore did not apply and the subsequent Cargo poll in that call
was never reached. The clarification is not applied here. Its intended wording
was that a movement association describes the observed transport window and is
not backdated to the move's server tick or a native Event timestamp. Review and
add that clarification after recovery.

## Observed checks and their limits

The following checks passed on the **formatted pre-disconnect local worktree**,
not on this reconstructed remote commit:

- Rust 1.86 fmt --all --check.
- ASCII guard: 168 tracked Rust files, including the three new files.
- tools/check_docs.py --fast.
- git diff --cached --check.

Fifteen new synthetic tests were authored but none is reported as executed.
The attempted first command was the no-default-features vrfkit library test,
filtered to gameplay::identity::tests, using Rust 1.86, one job, debug information
disabled and local opt-level 0. The dependency compilation did not return an
observed result before environment loss.

## Candidate scope

- Exact SpawnedCharacter group/checksum/raw-packed-GUID projection.
- Optional main-only as-of-packet PlayerState/body relation, with Null,
  Unresolved, Ambiguous and Resolved results.
- Retained source evidence, null/reconnect history, close/reuse boundaries,
  dormancy preservation, and invalidation after bad observations.
- Exact same-index movement provenance, using the initial/completion packet
  window and leaving indeterminate concurrent changes unresolved.
- An aggregate-only example and API contract documentation.
- Existing native/phase gameplay API and ordinary Reader/CLI behavior unchanged
  by design; no runtime equivalence claim has yet been validated.

An independent source review identified repeated actor opens while a channel
remained active. The authored implementation now marks that lifecycle
conflicting rather than assuming continuity, and the authored test covers
the same and a different channel. Independent runtime verification is pending.

Historical public six-replay exports provided a bounded wire-evidence lookup:
33 SpawnedCharacter rows, exact checksum 1413303948, the Bomb PlayerState group,
and 8/16-bit packed GUID payload widths. These were existing exports, not fresh
runs of this candidate. Synthetic values in tests are not private replay data.

## Resume requirements

1. Restore the checkout and run rustfmt before making a reviewable commit.
2. Compile and execute all 15 new tests and the existing gameplay tests.
3. Run strict Clippy, rustdoc, MSRV/feature and full repository gates.
4. Complete the independent raw/export oracle on fresh public fixtures,
   including exact identity/movement value, multiplicity, ordering and
   malformed/negative controls.
5. Confirm byte-preserved ordinary exports and previous gameplay observations.
6. Integrate with the separately implemented checkpoint Reader API while
   keeping this timeline main-only.
7. Update actual suite-size documentation after the new tests execute.
8. Create a normal implementation PR only after reviewable validation exists.

This backup has no implementation PR and is not merged into integration or
main. No release, tag, package publication, private repository change or
benchmark was performed.
