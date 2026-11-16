# Phase B group-path dictionary runs

This is an implementation checkpoint, pending independent functional
acceptance. It records a bounded change to Arrow array construction and the
tests added for it. **No wall-time, CPU, RSS, allocation, or speedup claim is
made.** Quantitative performance work stopped at the user's direction; the
existing Phase B profiling only informed the choice of column.

## Scope and behavior

`FieldRecord::columns` builds `group_path` for both the main `fields` table and
`checkpoint_fields`. Its new `dict_runs` declaration has exactly the existing
`Dictionary<Int32, Utf8>` Arrow type. The private helper walks the input in its
original order and uses Arrow's `append_values` operation once per consecutive
equal-value run. Every row still receives one dictionary key.

The equality check first recognizes a shared `Arc<str>`, then compares string
content so independently allocated equal values behave identically. Each run
is appended at its original position; a value's first occurrence therefore
retains its dictionary index. The helper carries only the previous borrowed
value and a run count. It retains the existing builder capacities and the
existing dictionary-overflow behavior.

Only this non-null column selects the run builder. Nullable dictionary columns,
checkpoint identity columns, schemas, compression, statistics, page boundaries,
Arrow batch sizes, and Parquet row-group policy retain their existing paths.
The extra adjacent-value comparisons also have a cost; their net runtime
effect is unmeasured.

## Added correctness checks

The array test compares the new helper with an independent element-at-a-time
`StringDictionaryBuilder`. It checks dictionary keys, ordered dictionary
values, and validity for empty input, an empty string, a long shared-Arc run,
equal values in separate allocations, a value recurring after other runs,
embedded zero bytes, an escaped Unicode value, and more distinct values than
the initial dictionary capacity.

The Parquet test replaces the candidate's `group_path` array with an
independently constructed element-at-a-time reference array. It compares whole
Parquet byte streams for records with repeated, separately allocated, empty,
and singleton group paths. The records cross the normal Arrow batch boundary;
the test exercises a small row group, a row group crossing an Arrow batch, and
the production default row-group size. Other record columns retain varying
values and nulls in the fixture.

These tests were added in implementation commit
`ae682ea3034ddebe1cf0a973364349ed76b3809d`. Their execution and the deliberate
run-count mutation check are **pending independent QA at this checkpoint**.
The planned mutation changes the helper's run increment from one to two;
both new tests must fail, the original source hash must be restored, and both
must then pass.

## Local build record and acceptance boundary

All twelve workspace release packages were invalidated before using the shared
target directory from this checkout; external dependency artifacts were kept.
The build used Rust 1.86.0, the existing lockfile and release profile, one build
job, and no extra compiler flags. The compiler emitted a fresh `vrf-export`
library artifact from this checkout during the first CLI build.

Both CLI release build attempts then exited 101 after the CLI compiler process
received SIGKILL. A focused release test command also exited 101, during
Parquet dependency compilation, before any test executed. No executable was
accepted from these attempts, and no replay export or performance measurement
was run for this candidate. Formatter, ASCII, and diff-whitespace checks passed.

The independent integration pass owns the executable tests, mutation proof,
complete public replay output comparisons, and remaining pinned correctness
gates. Hosted optimized release checks must validate the production artifact;
the source review and partial compilation recorded here do not replace them.
The existing Phase A withdrawn experiment and negative evidence are unchanged.

The [sanitized checkpoint record](evidence/phase-b-dictionary-runs-checkpoint.json)
contains source hashes, exact build commands, observed failures, and the
remaining acceptance checks. Detailed compiler logs stay local.
