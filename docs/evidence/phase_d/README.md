# Phase D evidence and reproduction

This directory contains aggregate evidence and verification sources, not replay
binaries, Parquet exports, raw parser logs or private catalog rows. The
[results](../../PHASE_D_RESULTS.md) explain the measured scope and decision.
The [fixed final receipt](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6918909298)
records final source, CI, package and publication-boundary checks.

## Evidence sets

| Set | Contents and integrity |
| --- | --- |
| [Primary/profile manifest](benchmark/public-evidence-manifest.json) | All 12 primary series, preflight fingerprints, compiler/source/binary identities, environment, protocol, complete fresh profile and diagnostic patch. The manifest covers 24 immutable files. |
| [Optimization manifest](optimization-measurement/public-evidence-manifest.json) | All eight normal-release experimental series, protocol, preflight, build/source correspondence, execution accounting and withdrawal decision; 17 immutable files. |
| [Independent QA manifest](independent-qa/public-evidence-manifest.json) | Original receipts, exact executed portable sources, explicitly identified reproduction adaptations, mutation contracts and privacy review. Its README distinguishes executed evidence from AST-only checked adaptations. |
| [Author verification](optimization/author-verification.json) | The one experiment's actual build/tests and bounded hypothesis. |
| [Allocator diagnostic](optimization/allocator.json) | Exactly one instrumented before/after pair; adverse heap observations and the report-parser recovery remain visible. |
| [Input origins](input-origins.json) | Public repository/file revisions, LFS or Git-blob correspondence and actual SHA-256/size. No download is implied by metadata reads. |
| [Private metadata](private-corpus-summary.json) | Aggregate 1,011-asset reconciliation; explicitly zero private payload downloads, hashes and executions. |
| [C# contract review](cross-parser-summary.json) | Fixed source/SDK evidence and why default complete exports are not equal-work measurements. |

The initial primary checkpoint preserved in GitHub at 50cd0379 was followed by
813329b7, which only clarified filesystem wording and added explicit conclusions
to the derived summary before adding the profile. No original sample, metric,
source/hash, protocol or parameter changed. The final manifests identify the
frozen versions used for independent acceptance.

## Reproduce the controlled resource comparison

Use the complete [build-freezing runbook](../../PHASE_B_BENCHMARK_HANDOFF.md)
and [resource harness contract](../../RESOURCE_BENCHMARKS.md). Build the exact
C-only baseline 1b9bbf222f003e35c882e640c1b19e0bbcc88472 and accepted runtime
ff8596b546736ea4300e013248609c7c3987e231 with Rust 1.86.0 in distinct worktrees
and initially absent target directories. Keep the original release profile,
default export features, target, native toolchain and lockfile unchanged.
Freeze actual fresh compiler-artifact executables; do not substitute the old
Phase C test-profile binary or another worktree's cached executable.

Select unchanged input bytes from input-origins.json and independently verify
their size and SHA-256. Use a new report and private work directory for every
invocation. The following is a template, not a new run:

```bash
python tools/benchmark_resources.py \
  --baseline-exe /absolute/frozen-baseline/vrfkit \
  --candidate-exe /absolute/frozen-candidate/vrfkit \
  --replay /absolute/unchanged-public-input.replay \
  --work-dir /absolute/new-private-work \
  --output /absolute/new-report.json \
  --warmups 1 --repeats 6 --checkpoints
```

First pass the candidate executable to both executable arguments for A/A.
Then run the baseline/candidate series. Run main-only separately without the
checkpoint flag. Strict output equality is mandatory for every warmup and
measured execution; never remove rows, counters or preservation checks to make
a comparison pass. Follow the recorded protocol and budget, retain all pairs,
and distinguish marginal medians from median paired ratios. The published
host noise means reruns are not guaranteed to reproduce the same ratios.

The accepted production source excludes the experiment. To reproduce only its
negative result, the remote experiment 1848dc9910662d0247b3f1cebaedd939b01fc33b
has the same complete tree as the actual local build df93e66f48d196ed3a1e4fada5da1ec97e61617e.
Use the four-large-case optimization protocol and retain the small-input
functional preflight. No new candidate or confirmation series was performed.

## Reproduce diagnostic and correctness checks

Apply the recorded stage-profile patch only to an isolated ff8596b worktree,
then build/freeze it separately. Its timers and thread-local counters change
execution; profile outputs must retain parity, and overlapping stage wall
values must not be summed into end-to-end time. The patch/build/report hashes
and instrumentation boundaries are in the profile receipts.

The [independent QA guide](independent-qa/README.md) gives retained-output audit
arguments and the exact two mutation substitutions. Detailed rows remain
local. The public-writer oracle exercises null/empty/spilled/large bytes and
main/checkpoint encoding. A deliberately broken implementation must compile,
then fail its intended content assertion; restored source must pass again.

The glibc diagnostic counts the interposed malloc/calloc/realloc/free subset.
Its heap totals are neither all Rust allocations nor process RSS and are not
substitutes for the uninstrumented release comparison. Per-core utilization,
physical disk latency and separately isolated output I/O remain unmeasured.
