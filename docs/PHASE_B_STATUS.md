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

**Seven unique subagents** were actually created: the six roles A, C, D, F,
E and B, plus E's independent read-only QA-harness reviewer. At most three
non-root agents ran simultaneously, plus the coordinator. All seven produced
implementation or review evidence; F continues final-head CI monitoring.
Resuming A or F did not create another unique agent. No worker failure was
observed. The initial allocation table below remains historical.

## Accepted runtime checkpoint

The coordinator merged PRs #65, #66 and #67 after independent combined-source
QA and 11/11 successful CI jobs on each task head. Integration is now
`e294fcf13c2d66ff47d45067341eb6f6e32eb27f`, tree
`c76a2f1fc5dcbf4285538c6531fe6e5e4bc300ae`. Its actual post-merge push run
`43836533296` passed 11/11 jobs and Draft PR #58 run `43836541263` passed
10/10. Main remains `9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9`.

| Role | Branch | Accepted work |
| --- | --- | --- |
| A | `phase-b/profiling` | Earlier profiling preserved; independent review of user-run benchmark handoff completed without real measurements |
| B | `phase-b/performance` | PR #67 merged; group-path dictionary operation reduction, independently tested |
| C | `phase-b/cli-input` | PR #65 merged; bounded validate/export input and checkpoint parity |
| D | `phase-b/events` | PR #66 merged; optional validated gameplay observations |
| E | `phase-b/qa` | Combined functional acceptance, fresh public replay oracles and eight mutation controls completed |
| F | `phase-b/dx` | Adoption/examples and benchmark runbook prepared for the fourth task PR; runtime CI receipts complete |
| E's reviewer | Read-only | Independent QA-harness controls reviewed and addressed |

The [results](PHASE_B_RESULTS.md) separate the six fresh public replay checks,
78 byte-identical Parquet files, 862 gameplay observations, typed/raw and
movement checks, full suites and historical comparison data. All 133 runtime
source/config hashes matched between the independent frozen QA source and
actual runtime integration. Quantitative speed/CPU/RSS acceptance is deferred
to the user and is not claimed.

This ledger is included in the fourth, documentation-only task PR. The
terminal [PR #58 receipt](https://github.com/yakisoba0728/vrfkit/pull/58)
identifies that PR, its accepted head, the final documentation merge SHA and
the separately executed final integration/Draft PR CI. Use that full final
SHA for later benchmarking, not the intermediate runtime SHA above.

## Historical initial allocation

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

## Resume and remaining work

Read the newest checkpoint on [PR #58](https://github.com/yakisoba0728/vrfkit/pull/58)
and resolve current remote branches before continuing. Apply the later user
policy above rather than the initial performance plan. Runtime functionality
is accepted; final documentation publication and final-head CI are recorded
in the terminal PR receipt so this report does not claim that an earlier run
tested its own later documentation commit.

Further quantitative evaluation is user-run. Reader checkpoint callbacks,
nonseekable input, richer player/round/movement joins and a fresh private-corpus
audit remain separate work. Main merge, formal releases and registry
publication remain excluded. Once the final-head gates are observed and the
terminal receipt is posted, Phase B stops at that recorded integration SHA.
