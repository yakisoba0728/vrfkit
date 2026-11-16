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

- Measure full export wall time, total CPU, CPU utilization, peak RSS,
  throughput and output size with balanced ordering and a same-binary control.
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
and resolve current remote branches before continuing. This initial ledger
does not claim any Phase B implementation has passed acceptance or merged.
