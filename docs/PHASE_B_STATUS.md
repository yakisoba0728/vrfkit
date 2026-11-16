# Phase B execution ledger

This phase continues the accepted Phase A work. Historical Phase A reports
remain unchanged. Only verified task pull requests may merge into
`integration/vrfkit-overhaul`; PR #58 remains open and draft. Main integration,
releases and registry publication are excluded.

## Recovered GitHub checkpoint

Fresh GitHub API and HTTPS Git reads on 2026-11-16 agreed:

- Integration: `9c133fb423c521d7df25b6ef19cb546fa489de4e`.
- Main: `9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9`.
- PRs #63 and #64 were merged; PR #58 was open, draft and unmerged.
- Historical Phase A integration run `43824541302`: 11 successful jobs.
- Historical Phase A Draft PR run `43824550803`: 10 successful jobs.

Those CI runs are recovery evidence, not Phase B validation.

## Current policy and coordination checkpoint

The user subsequently changed the performance policy: **stop quantitative and
repeated replay benchmarks; the user will run the final performance comparisons
themselves**. Agents continue implementation, focused correctness checks,
synthetic harness validation and the required functional CI. Repeated replay
timing, A/B resource series and performance acceptance claims are deferred to
the user. Prior profiling, invalidated builds and withdrawn experiments retain
their original evidence and limitations; they are not rewritten as fresh
acceptance results.

The [user-run benchmark handoff](PHASE_B_BENCHMARK_HANDOFF.md) fixes the Phase A
baseline, requires the final accepted integration commit, isolates Cargo build
targets and records source/compiler-artifact/binary provenance before any
comparison. It uses the existing resource harness and same-binary controls.

At this later checkpoint, **seven unique subagents** have actually been
created: the six roles A, C, D, F, E and B, plus E's independent read-only
QA-harness reviewer. At most three non-root agents ran simultaneously, plus
the coordinator. A, B, C, D and F completed their handoffs; E's additional
reviewer completed its check, and E is executing final combined functional QA.
Resuming A for independent handoff review and F for the benchmark runbook did
not create new unique agents. Completion of a role is not merge, CI or
performance acceptance. The initial allocation table below remains historical.

Source checkpoints are published on `phase-b/cli-input`, `phase-b/events` and
`phase-b/performance`; the latter branches include their preceding runtime
dependencies for combined validation. All task PRs will target the existing
integration branch. The latest exact heads and acceptance state are recorded
on PR #58. No Phase B task is merged at this preparation checkpoint.

## Assigned work

| Role | Branch | State at initial checkpoint |
| --- | --- | --- |
| A: performance profiling | `phase-b/profiling` | Running; fresh baseline and instrumented diagnostics |
| B: performance implementation | To be selected after profiling | Queued, not yet created |
| C: CLI and Reader architecture | `phase-b/cli-input` | Running; bounded CLI input with checkpoint parity |
| D: gameplay events | `phase-b/events` | Running; optional validated native event projection |
| E: independent QA | To be created | Queued, not yet created |
| F: developer experience | `phase-b/dx` | Queued, not yet created |

At this checkpoint three unique subagents have actually been created and
three were simultaneously active, plus the coordinator. Later completion
counts must be recorded from observed results, not inferred from this plan.

The existing Rust 1.86 toolchain and six public replay fixtures are available.
Fixtures and previous evidence are read-only. Private replay binary access
has not been revalidated and no private corpus execution is claimed.

## Acceptance requirements

- For the user's later performance comparison, record full export wall time,
  total CPU, CPU utilization, peak RSS, throughput and output size with
  alternating ordering and a same-binary control. Agents must not execute
  further quantitative/repeated replay benchmark series under the current
  policy.
- Retain equal output schemas, Native counters, raw preservation and manifest
  contracts. Timing instrumentation is not a performance acceptance result.
- Independently verify meaningful failure controls, replay equivalence and
  optional event interpretation before merging.
- Run the required MSRV, platform, feature, Python, documentation and public
  replay checks; recheck the actual final integration revision.
- Keep at most four core task PRs and record unsuccessful experiments rather
  than adopting uncertain performance changes.

## Resume

Read the newest checkpoint on [PR #58](https://github.com/yakisoba0728/vrfkit/pull/58)
and resolve current remote branches before continuing. Apply the later user
policy above rather than the initial performance plan. This ledger does not
claim final Phase B functional acceptance, performance acceptance or a merge.
