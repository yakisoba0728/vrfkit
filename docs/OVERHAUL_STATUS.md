# Overhaul execution ledger

## Scope and boundaries

The integration target is `integration/vrfkit-overhaul`. Work is reviewed in
separate pull requests. Do not push or merge to `main`, publish releases,
change repository secrets, or modify the private replay archive as part of
this effort. Private replays, decoded player information and credentials must
not appear in public logs, Git history or artifacts.

Correctness and raw preservation take priority over speed. Breaking API or
schema changes need explicit versioning, migration instructions and replay
regressions. No performance ratio is claimed until measured on equivalent
workloads. CPU time, elapsed time and peak memory are separate measurements.

## Starting point

- Parser baseline: `9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9`.
- Existing main CI: run `43740143674`, concluded `success`.
- Read `CONTRIBUTING.md` and `CLAUDE.md` before changing implementation.
- Existing CI only selects `main` for branch/PR triggers. The integration
  workflow calls the same reusable CI without weakening any existing checks,
  and adds Linux/MSRV runtime coverage.
- The editing environment has Python 3.13 but no Rust toolchain or outbound
  network. GitHub is accessed through its connected API; Rust builds and
  integration verification run on GitHub Actions. No separate AI worker
  execution capability is available in this environment.
- Public pinned fixtures are available to Actions. Access to the private
  archive through an interactive connection does not grant that access to a
  workflow's repository-scoped token. No private corpus run is claimed until
  an authorized execution path and an actual report exist.

## Work queues

| Track | Acceptance evidence | State |
| --- | --- | --- |
| Integration validation | Existing CI plus Linux/MSRV pass | In progress |
| Correctness audit | Failing-before regression and actual affected-path check | Pending |
| Resource/performance tooling | Per-process metrics, repeated paired runs, honest output scope | Pending |
| Rust API and CLI usability | Compiling examples and unchanged legacy behavior | Pending |
| Analysis tools and output | Bounded memory, stable identity and explicit missing values | Pending |
| Upstream feature comparison | Source-pinned inventory, replay evidence before typing | Pending |
| Developer documentation | Runnable quick start, released-vs-main distinction | Pending |
| Final integration | Exact-head CI, evidence ledger, draft PR to main only | Pending |

## Reporting rules

Distinguish implemented, verified, measured, blocked and deferred work.
A green unit suite is not a complete replay audit. A public-fixture audit is
not a private-corpus audit. A Rust-versus-C# comparison is not fair unless the
input, output scope, build configuration and validation work are described.
Do not declare the entire overhaul complete because a subset of checks passed.
