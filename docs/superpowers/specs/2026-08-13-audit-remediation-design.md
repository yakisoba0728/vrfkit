# vrfkit Audit Remediation Design

## Scope

This change fixes the confirmed defects from the 2026-08-13 whole-repository
audit. Speculative protocol differences are not changed without a reproducible
fixture. The work is split by ownership so two agents can proceed without
editing the same files:

- Rust agent: crates, CLI/driver, feature/build/package/rustdoc contracts.
- Python/integration agent: `tools/`, baselines, CI and human documentation;
  the same agent migrates the sibling `valplay` repository to `vrfkit`.

## Behavioral design

Untrusted wire counts and bit lengths are checked before allocation or reader
construction. Framed records that abandon data are observable failures, not
successful empty values. Every loss/quality counter reaches the run totals,
summary and manifest. Schema exports and packets are applied in wire order so a
packet never observes a later schema revision. Validation closes all partial
state and checks ReplayData trailing bytes before deciding its verdict.

Exports are published transactionally: write a complete run in a sibling
temporary directory, close every writer, then replace the destination. A
failed run must leave the previously completed destination intact and must not
leave detached writer threads. Raw payloads remain available whenever an
additive movement/RPC/array/struct decoder fails or returns partial output.

Public APIs reject invalid sizes with typed errors instead of panics. Advertised
feature combinations and the standalone `probe_offset` tool build on Rust
1.86.0. Strict rustdoc is warning-clean, and publishable packages include the
repository license/notice material.

Python tools validate path containment before deletion, reject empty generator
inputs, write generated/baseline output atomically, require all advertised
counters and oracle cardinalities, and key corpus results by relative path.
Baseline claims use content hashes for byte identity. Documentation and CI run
the same gates they advertise.

## Verification

Every behavior change follows red-green TDD. Focused regression tests must be
observed failing before implementation. Final verification uses Rust 1.86.0:
format, workspace Clippy with warnings denied, workspace tests, all-features
all-target checks, a no-default feature matrix, strict rustdoc, Python tests
with warnings as errors, documentation/generator guards, and at least one real
replay through validate/export/adaptation. Corpus-wide checks are run when their
local inputs are available and never silently skipped in the reported result.

