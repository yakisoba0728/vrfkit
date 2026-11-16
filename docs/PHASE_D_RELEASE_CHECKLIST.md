# Phase D 0.6.0 candidate and release gates

This prepares an unpublished **0.6.0** source candidate on
`integration/vrfkit-overhaul`. It does not authorize a main merge, removal of
PR #58's Draft status, tag creation, GitHub Release or registry upload.
The [Phase D result](PHASE_D_RESULTS.md) and
[fixed PR #58 final receipt](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6918909298)
determine acceptance; this checklist alone is not a passing test report.
The measured runtime remains the accepted Phase C source: the one new
optimization experiment was withdrawn after controlled evaluation.

The Phase D user instruction permits actual performance qualification and a
candidate version change. The earlier Phase B/C restrictions on running
benchmarks are historical for those phases. Their recorded measurements,
source identities and limitations remain unchanged.

## Package identity

| Item | Candidate contract |
| --- | --- |
| Production packages | Ten crates inherit workspace version `0.6.0`; their ten local lockfile entries match. |
| Development packages | `vrf-testkit` and `extract-component-classes` retain `0.0.0` and `publish = false`. |
| Rust toolchain | Edition 2024, MSRV 1.86.0; the existing release profile and dependency resolutions are retained. |
| Windows target | `x86_64-pc-windows-msvc`; the existing CI builds, packages, extracts and runs the actual optimized CLI. |
| License | Apache-2.0; the ZIP includes the existing `LICENSE` and `NOTICE.md`, including retained MIT notices. |
| Rust consumption | Pin an exact source revision. Internal dependencies are path-only; registry dependency versions and a publication policy are not prepared. |
| Python consumption | Source tools plus `requirements.txt`; there is no Python distribution manifest or PyPI candidate. |

The version metadata change is separate from runtime optimization. Record the
benchmark source, binary hash, Cargo.lock, compiler and build flags before
any later version/documentation change. If a final candidate changes only
metadata/documentation, establish exact runtime-file and release-profile
equality and disclose the different source/lockfile identities. A version
string is not evidence that an executable came from the requested commit.

## Required migrations

| Surface | Consumer action |
| --- | --- |
| Native movement counters | Replace the eight retired tail/trailer fields with all nine native component counters in main and checkpoint quality checks. Do not invent a numeric conversion from historical counters. |
| FocusProjectiles counters | Require `route_children_focus_projectiles` and `focus_projectiles_empty_trailers`, including zero values. Missing keys must remain an error. |
| FocusProjectiles rows | Accept the evidence-qualified 12.05 additive child rows while retaining the unchanged raw parents. Do not generalize this route to other builds or identities. |
| Reader checkpoint events | Existing reads skip snapshots by default. Opt-in consumers must retain independent snapshot identity/state and preserve their configured schema limits. |
| Reader errors and callbacks | Add the `ReplayError::CheckpointSchema` case to exhaustive matches and handle `ReadError::InputLimit`. Callbacks are provisional until the whole read succeeds; preserve original consumer errors. |
| Finite nonseekable input | Supply an explicit encoded-byte cap. `read_bounded` buffers through EOF; it is neither incremental network decoding nor a process-memory limit. |
| Gameplay identity | Feed each main-stream batch exactly once and in callback order. Preserve null, unresolved and ambiguous results; do not feed checkpoint snapshots into the main timeline. |

The complete rules and examples are in
[Phase C migration](PHASE_C_MIGRATION.md),
[native counter migration](NATIVE_MOVEMENT_COMPONENTS.md#counter-and-source-api-migration),
[Rust API](RUST_API.md), [gameplay API](GAMEPLAY_API.md) and
[CLI input streaming](CLI_INPUT_STREAMING.md).

## Exact-source qualification

Complete these gates against the actual final integration source. An earlier
passing run, historical corpus report or authored test count does not complete
a current gate. Record executed commands, input hashes and actual results in
the final Phase D receipt.

- Record the starting and final main/integration SHAs, final tree and PR #58
  head/base/synthetic-merge relationship. Confirm main is unchanged and PR #58
  is open, draft and unmerged with automatic merge disabled.
- Require final integration push CI and final PR #58 CI, including Windows
  and Linux Rust 1.86, Windows stable, strict formatting/Clippy/rustdoc, the
  derived feature matrix, Python 3.12/3.13 on both systems, Parquet/PyArrow
  interoperability and the actual Windows package job. Inspect execution logs.
- Retain independent raw/typed and main/checkpoint validation of the changed
  runtime. Compare the full required counter/manifest contracts and all
  unchanged Parquet bytes; admit only documented, independently checked deltas.
- Record fair A/A and A/B experiments, output parity per run, failures and
  noise. Report unsupported performance comparisons as such. A performance
  claim requires its own immutable source/binary/input receipt.
- State private metadata access separately from actual binary downloads,
  SHA-256 checks and executed replay validation. Publish only safe aggregate
  receipts. No private corpus access is implied by historical README counts.
- Retain unresolved semantic scopes as `blocked_by_evidence`: unsupported
  movement layouts, InputEventData, unproven GAS/Ability meanings, RPC tails,
  owner/healing lifecycle and complete account/kill/round semantics. Do not
  guess values to complete a release checklist.

## Windows package evidence

The existing `Windows release package` job is the package gate. It:

1. Builds the optimized MSRV CLI for Windows x64 and runs release-mode CLI
   and movement tests.
2. Creates a deterministic ZIP containing exactly `vrfkit.exe`, `LICENSE`,
   `NOTICE.md` and `build-info.json`, plus a SHA-256 sidecar.
3. Checks the PE architecture, ZIP membership, checksum and embedded source,
   target and executable hash, then runs the extracted executable's `--help`.
4. Uses that extracted binary to audit the pinned public 12.10 input,
   including checkpoint export and independent typed/raw comparisons.

Retain the `windows-release` and `release-replay-verification` artifacts and
their job/run URLs for the exact accepted checkout. Branch and PR packages
use a `v0.0.0-ci.<checkout-sha>` test label; that label creates no Git tag and
is separate from the source workspace version. PR packages can identify a
synthetic merge commit, so compare its tree with the intended integration
source instead of relabeling it as the integration commit.

The existing [release procedure](../CONTRIBUTING.md#tagged-windows-releases)
publishes the already verified ZIP without rebuilding only after a separately
authorized tag and successful tagged-commit CI. No publication action is
part of this candidate preparation. Registry publication also requires work
on package metadata and compatibility policy beyond this Windows CLI gate.
