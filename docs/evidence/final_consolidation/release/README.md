# CI and release audit at the recovered source

This bounded audit reviews source
`8581642283bd5a571967129d73897db8ee5408ba`, tree
`03fb832d96714343e85c30ea9468993c1af63d97`. It does not qualify the later
final integration HEAD. The machine-readable [receipt](audit.json) records
the reviewed file hashes, executed commands, process results and limitations.

## Findings

No reproducible blocking defect was identified in the reviewed workflow,
package-identity and documentation-guard paths. No runtime, workflow or
packaging refactor was made without a demonstrated defect.

The local Python 3.12 run passed the 19 package tests, 65 documentation-guard
tests and fast documentation checks. Package tests deliberately exercise
incorrect identities, changed bytes, unexpected ZIP entries and failed or
timed-out smoke commands. Their smoke commands are mocked: these unit tests
are not actual Windows executable acceptance.

## Final integration gates

The existing integration push workflow calls the reusable CI workflow. It
requires 11 jobs including its completion wrapper. PR #58 targets main and
requires the 10-job CI workflow. Each completion job rejects failures,
cancellations and skips in its required dependencies.

The Windows package job already builds and tests the optimized MSRV CLI and
movement library, constructs the deterministic ZIP, checks source identity
and executable hash, runs an extracted executable, and audits the public
12.10 replay with checkpoints and independent typed/raw comparisons.
Artifacts are `windows-release` and `release-replay-verification`; the
published artifact and execution report must still be reconciled against
the actual final checkout. The PR checkout can be a synthetic merge commit,
so its tree must be compared with the intended integration tree.

No tag is needed for those gates. Branch/PR test packages use
`v0.0.0-ci.<checkout-sha>`. The release workflow publishes only for a tag ref,
and remains outside this task's authorization. Workspace version 0.6.0 is
an unpublished source candidate; crates.io and PyPI packaging are not
prepared by the Windows CLI job.

After other roles add tests, update the live suite-count claims in README
and USAGE using measured final counts. Do not rewrite historical Phase A-D
counts, source revisions or performance conclusions.

## Recovered remote state

The fresh recovery read confirmed unchanged main at
`9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9`, only PR #58 open, Draft/unmerged
with auto-merge disabled, ten existing tags through v0.5.0, and latest public
release v0.5.0. The synthetic PR merge
`7360230343071cc5f357c49b0fe765602f11f75e` has the recovered integration tree
and main/integration parents. These facts describe recovery only and require
a final readback after integration.
