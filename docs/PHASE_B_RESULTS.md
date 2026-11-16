# Phase B implementation and acceptance record

Phase B continues integration `9c133fb423c521d7df25b6ef19cb546fa489de4e`.
The main branch remains outside this work. PR #58 stays open and draft;
releases and registry publication are not part of this phase.

This document is prepared before final acceptance. The latest checkpoint on
[PR #58](https://github.com/yakisoba0728/vrfkit/pull/58) is authoritative for
accepted task heads, actual integration commits and their CI results. Pending
checks below must not be read as successful executions.

## Changed performance policy

During this phase the user stopped quantitative performance measurements and
repeated A/B runs, assigning final performance comparison to the user. Existing
profiling remains available in [the profiling record](PHASE_B_PROFILING.md).
The interrupted QA resource session is not an accepted performance result.

The implementation can remove duplicate operations or whole-file input
retention after correctness review; that does not establish a wall-time, CPU,
RSS, throughput or competitor advantage. These metrics remain unmeasured for
the final combined implementation. Use the
[benchmark handoff](PHASE_B_BENCHMARK_HANDOFF.md) to compare the accepted final
revision with the Phase A baseline under matching conditions.

## Implemented changes

### Bounded CLI input

`validate` and `export` reuse the file-backed input source introduced for the
Reader in Phase A. Main-stream parsing keeps its independent prefetch cursor.
The optional checkpoint pass uses another read-only cursor and buffers one
encoded checkpoint at a time, retaining parallel checkpoint decoding and
independent state. The full input file is no longer retained for these CLI
commands.

The existing output publication, schema, native-counter and manifest contracts
are preserved. Input files must remain unchanged while being read. This is
not a total-memory bound: decoded chunk sizes, parser state and writer buffers
still matter. `inspect`, `diag`, nonseekable input and Reader checkpoint
callbacks are outside this change. See [CLI input](CLI_INPUT_STREAMING.md).

The correctly attributed author-side build passed six public inputs across
12.05, 12.08, 12.09, 12.10, 12.11 and 13.00, with 76 checkpoints and 78
byte-identical Parquet files. Nonvolatile manifests and required counters also
matched. A sparse skipped-payload test distinguishes bounded input from
whole-file allocation under an explicit address-space limit. These are
correctness/resource-bound checks, not a measured real-replay RSS improvement.

### Optional gameplay observations

The public `vrfkit::gameplay` module projects validated native round starts,
character deaths, spike plants, defuses and explosions, plus strictly
identified main-stream round-phase observations. Callers opt in from Reader
callbacks; the default parser and CLI do not invoke these helpers.

Native and derived sources remain distinct. Raw source data is borrowed and
retained, unknown phases keep their byte value, and actor references are
explicitly null or unresolved. No player identity, kill credit, complete-round
result or movement join is guessed. Observations remain provisional until the
read completes successfully. Reader checkpoints remain skipped, not projected
as gameplay observations.

The author-side Rust 1.86 focused suite passed 12 tests. Six fresh Reader
executions matched 862 observations against independently decoded raw values
from explicitly identified historical exports: 76 round starts, 424 deaths,
37 plants, 18 defuses, five explosions and 302 phase observations. That receipt
does not relabel the historical exports as freshly generated. Final QA uses
its own fresh-output provenance. See [gameplay API](GAMEPLAY_API.md).

### Dictionary operation reduction

The fields/checkpoint-fields `group_path` dictionary builder groups adjacent
equal values and uses Arrow's repeated-value insertion. First-seen dictionary
order and one key per row are retained. The helper handles shared allocations
and separately allocated equal strings without changing compression, batch,
row-group, schema or null policies.

Two independent-reference tests compare dictionary arrays and complete
Parquet bytes with an element-at-a-time builder. The additional adjacent-value
comparisons themselves cost work, so net runtime benefit is unmeasured. See
[the implementation record](PHASE_B_DICTIONARY_RUNS.md).

### Adoption and reproduction

The front page now identifies the integration checkout/source pin needed for
these APIs. Rust Reader/gameplay examples, a projected PyArrow batch example,
JSONL recipes and source/package readiness are documented in
[the adoption guide](ADOPTION.md). The reference comparison is pinned to
`fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8` and distinguishes native observations,
derived analytical models and movement-emission contracts.

The existing resource harness remains unchanged. A new provenance helper and
runbook require separate initially absent Cargo target directories, actual
compiler-artifact records and source/compiler/binary receipts. Synthetic
verification does not count as executing the user-run benchmark procedure.

## Rejected evidence and build limitations

An initial shared Cargo target reused the profiler executable from another
worktree while reporting fresh artifacts. All affected CLI tests, output
comparisons and resource runs were excluded. The corrected CLI build cleaned
all workspace release packages, retained external dependencies, recompiled the
runtime crates, verified source hashes, and reran its input proof and six-input
comparison with the correct executable. The rejected evidence remains labeled
and is not silently replaced by a success claim.

Several local optimized compilation attempts ended with SIGKILL. Failed
compilation did not produce an accepted executable or test result. Final local
correctness checks identify their actual dev/test profile; required hosted
checks independently validate the optimized release package. No alternate
profile is presented as a repository-default release measurement.

## Independent acceptance and publication

Independent combined-source QA, meaningful implementation mutations, the full
local sweep, each task PR's required CI and actual final integration CI are
pending at this preparation checkpoint. Their exact source heads, outcomes and
limitations will be recorded before this phase is marked complete.

The six primary roles were actually created, and the independent QA role
created one additional read-only harness reviewer. Resuming a role does not
create another unique agent. The final receipt records observed completion
counts and concurrency rather than the planned allocation.

## Remaining scope

- Quantitative final performance evaluation belongs to the user; no speedup,
  CPU reduction, real-replay RSS percentage or competitor ratio is claimed.
- Reader checkpoint callbacks, nonseekable input and richer Rust identity,
  movement and round joins remain separate work.
- Native/phase observations do not implement every game or ability semantic.
- Private archive metadata was readable, but no private replay binary was
  obtained or validated during this phase. Historical private-corpus results
  are not a fresh Phase B audit.
- Main integration, a formal release and package-registry publication require
  a separate decision and are not performed here.
