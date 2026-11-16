# Phase D execution ledger

## Engineering scope complete; final source gates use the fixed receipt

The scoped verdict is **FUNCTIONALLY_READY_PERFORMANCE_INCONCLUSIVE**.
The [results](PHASE_D_RESULTS.md) retain the complete resource comparison,
controls, fresh profiles, rejected optimization, public QA and private-access
limits. The [fixed final receipt](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6918909298)
records actual final task/integration/PR CI, checkout and artifact identities.
An incomplete receipt is not final acceptance. This documentation checkpoint
precedes its own merge, so the final SHA is deliberately recorded outside it.

## Source and publication boundaries

- Starting accepted integration/runtime: `ff8596b546736ea4300e013248609c7c3987e231`.
- Unchanged main: `9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9`.
- C-only same-output baseline: `1b9bbf222f003e35c882e640c1b19e0bbcc88472`.
- The user authorized actual quantitative measurements and bounded optimization
  for Phase D; earlier Phase B/C user-only measurement policies are historical.
- [Task PR #74](https://github.com/yakisoba0728/vrfkit/pull/74) carries unpublished
  0.6.0 metadata, current-contract corrections and the evidence. Runtime,
  dependency resolutions, build flags and workflows remain unchanged.
- [The one experiment](https://github.com/yakisoba0728/vrfkit/commit/1848dc9910662d0247b3f1cebaedd939b01fc33b)
  is **WITHDRAWN_DO_NOT_INTEGRATE**. Zero Phase D runtime optimizations are accepted.
- PR #58 remains Open/Draft/Unmerged with auto-merge disabled. No main push/merge,
  Draft removal, tag, public Release, registry upload or private-source mutation.

## Completed bounded work

| Track | Outcome |
| --- | --- |
| Recovery/build | Recovered about 6.7 GiB by removing verified old Cargo caches; retained source, originals and evidence. Fresh isolated Rust 1.86 default release binaries passed compiler-artifact and hash checks. |
| Primary performance | Six input/mode cases; 12 series, 168 exports plus 12 untimed preflight. Every output/provenance gate passed; all six performance conclusions INCONCLUSIVE. |
| Diagnosis | Three normal and three instrumented 12.09 checkpoint exports; measured parser, writer, Arrow, encoding, decompression and cached-read costs with explicit overlap/overhead limits. |
| Optimization | One bounded raw-byte reservation candidate; author and independent mutation checks passed. Eight controlled series/112 exports plus 12 preflight passed output gates, but no robust gain and higher observed RSS led to withdrawal. |
| Independent QA | Recomputed all primary and experimental samples. Fresh accepted outputs: 57 Parquet files, 46 checkpoints, 43,443 typed/raw values, exact 53 Focus children/three trailers, zero loss/mismatch, 22 counter-deletion controls detected. Two compiled experiment mutants detected and exact restoration passed. |
| Private corpus | Metadata-only: 1,011 assets, 24 builds, 50,938,107,071 bytes reconciled; zero private downloads, payload hashes or executions. |
| C# comparison | Current source rechecked, .NET 10 runnable; default exports NOT_COMPARABLE because their work/preservation/output contracts differ. No speed ratio or new C# replay execution. |
| Release/DX | Ten production package versions use unpublished 0.6.0. Migration, packaging and registry limits documented. Final actual Windows package and source verification belongs to the fixed receipt. |

A executed 310 full CLI exports and B's allocator diagnostic added two, for
312 local full exports. There were zero export failures and no removed samples.
Repeated runs and rejected-source rescans add no unique corpus coverage.
The private corpus and old Phase A-C counts are not relabeled as fresh runs.
Two evidence-parser issues were repaired and retained as harness recoveries.

## Worker accounting

Six unique specialists were created: A benchmark/profiling, B optimization,
C private corpus, D independent QA, E release/DX and F cross-parser/runtime
review. Maximum observed concurrency was three non-root workers, four including
the coordinator. One capacity-refused spawn created no worker. No worker crash
or task resumption was recorded. The fixed receipt records the final completion
count after D's final-source/CI/artifact check.

Only the coordinator integrates reviewed changes. Required CI on the final
source must pass before task merge, followed by actual integration-push and
PR #58 CI/source readback. Stop after this qualification. Do not begin another
phase, main merge or publication without a new user instruction.
