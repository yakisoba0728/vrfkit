# Phase D execution ledger

## Active: recovery and qualification

The user explicitly authorized quantitative performance measurement and bounded
optimization in Phase D. Earlier Phase B/C user-only performance restrictions
remain historical. Main merge, PR #58 draft removal, tags, releases and registry
publication are not authorized.

## Verified starting state

- Integration: `ff8596b546736ea4300e013248609c7c3987e231`.
- Main: `9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9`.
- PR #58 is open, draft and unmerged.
- Latest public release is `v0.5.0`; workspace version is `0.5.0`.
- Prior Phase C integration CI 43861110187 has 11 successful jobs and PR CI
  43861119792 has 10. These are historical results, not Phase D reruns.
- Proposed same-output baseline: `1b9bbf222f003e35c882e640c1b19e0bbcc88472`.

## Environment recovery and current work

The initial executor had only about 119 MiB free. Four reproducible Cargo target
caches from completed vrfkit work were removed after verifying their Cargo cache
tags; source, frozen binaries, replay originals and evidence were preserved.
Available space recovered to about 6.7 GiB. Rust 1.86 and offline dependencies
were found. Fresh isolated release builds are in progress; the old test-profile
QA executable is not eligible for official release timing.

Three unique workers were successfully created: A benchmark/profiling, C private
corpus/evidence, E release/DX. Maximum observed concurrency is three workers plus
the coordinator. None has completed at this initial checkpoint. Additional
roles will use freed slots; role names alone are not creation counts.

## Gates

- Strict output contract equality before and during every timed sample.
- Same-session A/A controls, then balanced C-only/candidate comparisons on public
  12.05, 12.09 and 13.00 samples, separately main-only and checkpoints.
- Minimal evidence-backed optimization, independent QA, meaningful negative
  controls and exact-head required CI before integration.
- Private metadata and binary access reported separately; no private bytes,
  names, player identifiers or raw logs in public evidence.
- Release candidate version/migration/packaging and final source/CI checks.

No Phase D performance result, private binary validation or final acceptance is
claimed at this checkpoint. Final results will be recorded in PHASE_D_RESULTS.md
and PR #58. Only the coordinator may merge reviewed task PRs into integration.

