# Phase B implementation and acceptance record

Phase B continues integration `9c133fb423c521d7df25b6ef19cb546fa489de4e`.
The main branch remains outside this work. PR #58 stays open and draft;
releases and registry publication are not part of this phase.

The runtime changes passed independent acceptance and were merged through
PRs #65, #66 and #67. Their combined integration commit is
`e294fcf13c2d66ff47d45067341eb6f6e32eb27f`, tree
`c76a2f1fc5dcbf4285538c6531fe6e5e4bc300ae`. Its post-merge integration CI passed
11/11 jobs and Draft PR #58 CI passed 10/10 jobs.

This tracked report accompanies the fourth, documentation-only task PR. The
terminal receipt on [PR #58](https://github.com/yakisoba0728/vrfkit/pull/58)
records that PR, the final documentation merge commit and CI executed on the
actual final integration revision. The runtime checkpoint above must not be
substituted for that later final revision in a user-run comparison.

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

## Independent acceptance

The [independent QA record](evidence/phase_b/independent_qa/README.md)
accepts the combined runtime source without a blocking finding. The local full sweep used source
`9800679c8747fb1f2bcda6b2736bb0053c04206c`; fresh replay checks used
`2fbead4f716f12f7c5d41ff067ff4ee63a55dc90`, which adds only the dictionary
implementation record. Every one of the 133 tracked runtime source/config
hashes matched the actual merged runtime integration commit. The
[source correspondence receipt](evidence/phase-b-runtime-source-correspondence.json)
separates local source verification from hosted synthetic-merge checks.

| Check | Observed result and scope |
| --- | --- |
| Rust 1.86 local workspace | 826 tests passed; no failed or ignored tests |
| Core-only runtime | 169 `vrfkit` tests and 39 container tests passed |
| Python | 1,178 tests passed |
| Feature matrix | 27 core-only/singleton cases passed |
| Static/documentation checks | rustfmt, all-target/all-feature Clippy with denied warnings, strict Rustdoc, measured documentation, ASCII and schema/type generators passed |
| Rust/Python interoperability | Exported Rust Parquet was accepted by the PyArrow verifier |
| Public replay scope | Fresh executions on 12.05, 12.08, 12.09, 12.10, 12.11 and 13.00; 76 decoded checkpoints |
| Export equivalence | All 78 Parquet files byte-identical to accepted Phase A outputs; nonvolatile manifests and summary/native counters equal |
| Independent typed/raw values | 60,169 values across 16 specified identities; no missing identity, raw-width error or value mismatch |
| Gameplay | 862 fresh observations: 560 native and 302 phase observations; zero value, multiplicity or source-order differences |
| Movement reference | Three public small fixtures; 16,748 unique wire updates representing 16,899 moves; no missing reference update, extra Rust component or latest-move mismatch after f32 quantization |
| Meaningful failure controls | All eight implementation mutations failed their intended assertions; clean restoration passed 22 Reader/CLI/gameplay and 24 export tests |

Fresh gameplay observations were checked against independently decoded raw
values from fresh combined-source exports. The movement comparison instead
uses frozen C# outputs with checked content hashes and reference manifest
revision `fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8`; C# was not rerun. It covers
main-stream variant-1 movement components and the reference's latest move per
update. Earlier moves are counted, but their values are not emitted by that
reference. Decimal reference differences before f32 quantization remain in
the receipts; this is not an exact-byte equality claim between JSON and
Parquet representations.

The frozen local CLI and probes were explicitly built with the Cargo test
profile, optimization level 2, no debug information or incremental builds,
with debug assertions and overflow checks enabled. They are not the default
release binaries. Sources were hashed and clean executables frozen before
mutation builds; source restoration alone is not binary restoration. All
frozen executable hashes remained unchanged after QA.

The separate QA-harness reviewer identified three controls that were applied:
freeze the clean executables before mutants, reject Python optimized mode
that would disable assertions, and verify the reference revision from its
manifest. A discovery pass that found only three `.vrf` files was rejected as
six-input evidence. The accepted rerun used read-only, SHA-pinned aliases for
the three `.replay` inputs and verified all six. No private replay was run.

## Pull requests and hosted CI

Only the coordinator merged task PRs, all into the existing integration
branch. Each runtime PR passed 11/11 required jobs on its exact head; the
tested synthetic tree was checked against the merged tree. When preceding
PRs changed the base, the regenerated synthetic merge tree was checked again.

| Task PR | Accepted head | Integration merge |
| --- | --- | --- |
| [#65: bounded CLI input](https://github.com/yakisoba0728/vrfkit/pull/65) | `384d0c043c4a00b0087c45b3ef8655b1c22a04c7` | `a6eb925586f182867d4230996d45c45282495405` |
| [#66: gameplay observations](https://github.com/yakisoba0728/vrfkit/pull/66) | `45f787c460b8d7a54876572d3c7e6a1aeb01dafc` | `9b71c9d61ef1c89a415a2161536c74c4da75ce6e` |
| [#67: dictionary operation reduction](https://github.com/yakisoba0728/vrfkit/pull/67) | `d56daa16f4c67052cc0eff1556599ace4046b60f` | `e294fcf13c2d66ff47d45067341eb6f6e32eb27f` |

The [task CI receipt](evidence/phase-b-runtime-pr-ci.json) retains run, job,
checkout and log evidence. The final runtime PR observed 826 Linux and 827
Windows Rust tests; each of its four Python jobs passed 1,178 tests. Rust
1.86.0 and stable 1.100.0, Windows/Linux, the feature matrix, strict
documentation, public replay checks and the Windows release package passed.
Earlier runtime PRs have the smaller test inventories appropriate to their
source. The [integration receipt](evidence/phase-b-runtime-integration.json)
records merge gates, commits and current-branch checks.

The combined runtime push run
[43836533296](https://github.com/yakisoba0728/vrfkit/actions/runs/43836533296)
passed 11/11 jobs; Draft PR #58 run
[43836541263](https://github.com/yakisoba0728/vrfkit/actions/runs/43836541263)
passed 10/10. These runs validate the merged runtime tree. The fourth task PR
and actual final documentation merge receive their own CI before the terminal
receipt is published; earlier successful runs are not labeled as final-head
executions. The [post-merge CI receipt](evidence/phase-b-runtime-postmerge-ci.json)
records all 21 job outcomes, checkout identities and selected log evidence.

## Actual agent execution

Seven unique non-root agents were created: A profiling, B implementation,
C CLI/Reader, D gameplay, E independent QA, F adoption/CI, and E's additional
read-only QA-harness reviewer. The maximum observed concurrency was three
non-root agents plus the coordinator, four total. Resuming A or F did not
create another unique agent.

All seven produced their assigned implementation or review evidence. F's
final-head CI monitoring continues through the publication gate; the terminal
receipt records terminal completion counts. No worker failure was observed.
Local compiler SIGKILLs, rejected stale artifacts and the interrupted resource
session are separately recorded execution outcomes, not successful tests or
failed-agent counts.

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
