# Phase B baseline profiling

This is the fresh profiling checkpoint for Phase B, not acceptance of a
runtime optimization. It starts from integration commit
`9c133fb423c521d7df25b6ef19cb546fa489de4e`. The Phase A records and withdrawn
movement-inlining experiment remain unchanged. No performance improvement or
competitor advantage is inferred from these diagnostic runs.

## Build and work contract

Both executables were freshly built sequentially in the same checkout with
Rust 1.86.0, the existing lockfile, default features, and the repository release
profile (optimization 3, fat LTO, one codegen unit, debug level 1). Each was
copied to a frozen executable before the next build. No extra Rust flags were
set. The diagnostic edits are retained only as a patch; normal runtime source
in this profiling commit is unchanged.

| Identity | SHA-256 |
| --- | --- |
| Uninstrumented executable | `33f4a2e286055e487ac677a9713e626f7b4a340932630a15679a1d84f2d2af87` |
| Public 12.09 input, 61,015,648 bytes | `925fe2d1dfce6dff071df724bff03faa7220ffd209bdf795b76888b65157efff` |

Six complete exports, in baseline/profile/profile/baseline/baseline/profile
order, included checkpoints. The other Phase B agents held builds, tests, and
exports during the timing window; this was not an exclusive-host guarantee.
Input and output hashing warm the cache. The same existing preservation,
counter, normalized-manifest and exact-Parquet-byte gates were applied to
every run. Three observations per executable are diagnostic controls, not an
optimization acceptance sample.

The uninstrumented median measurements were:

| Metric | Observed value |
| --- | ---: |
| Wall-clock | 1.704392 s |
| Total CPU time | 4.530802 s |
| CPU utilization, one core = 100% | 250.283% |
| Peak RSS | 195,883,008 bytes |
| Compressed input throughput | 34.1407 MiB/s |
| Parquet bytes | 43,984,122 |
| Total output bytes | 44,676,171 |
| Input block operations | 0 |
| Output block operations | 87,336 |

Each metric is its own median; dividing the median CPU by median wall need not
equal the median utilization. Baseline wall times ranged from 1.568021 to
1.810272 s and CPU times from 3.834884 to 4.597708 s. This substantial variation
persists despite local coordination. The diagnostic binary's lower medians do
not demonstrate that instrumentation is an optimization.

## Stage and column measurements

The instrumented build reads Linux per-thread scheduled CPU time and times
Arrow array construction and Parquet calls. It also counts the consecutive
equal values in the dictionaries after constructing each array. These counters
and clocks change code generation and add work. In particular, the whole
Arrow-stage wall measurements include the dictionary census, while the
individual column clocks end before that census. Concurrent stages overlap;
do not add their wall times to reconstruct process time.

| Stage | Median across three diagnostic exports |
| --- | ---: |
| Main parsing thread CPU | 1.568585 s |
| Main fields writer CPU | 0.852515 s |
| Main movement writer CPU | 0.606187 s |
| Checkpoint fields writer CPU | 0.256897 s |
| Main fields Arrow construction, including census | 184.310 ms |
| Main fields Parquet calls | 608.757 ms |
| Main movement Arrow construction | 27.250 ms |
| Main movement Parquet calls | 546.163 ms |
| Checkpoint fields Arrow construction, including census | 62.839 ms |
| Checkpoint fields Parquet calls | 168.711 ms |

The main fields table contains 1,657,000 rows and checkpoint fields contain
439,892 rows. The measured individual fields-column builders were:

| Column | Main build ms | Checkpoint build ms |
| --- | ---: | ---: |
| `field_name` | 40.498 | 19.753 |
| `group_path` | 37.660 | 10.727 |
| `raw_bits` | 28.486 | 7.255 |
| `value_str` | 22.913 | 4.326 |

`group_path` has 542,085 consecutive equal-value runs in the main pass and
16,641 in checkpoints, compared with 1,657,000 and 439,892 values respectively.
The current Arrow dictionary builder performs one lookup for every value. A
run-aware builder could reduce these lookups by 67.3% and 96.2%, preserving
first-occurrence dictionary order. The sum of string lengths processed by the
current main group-path lookup is 102,421,139 bytes, versus 32,982,186 bytes
at run starts; checkpoint figures are 26,703,544 and 845,321 bytes. These are
string-length census figures, not measured physical copies or cache traffic.

This opportunity is narrow: about 48 ms of measured group-path construction
against several seconds of whole-process CPU. Other columns have much less
repetition: main `field_name` has 1,521,740 runs for 1,654,734 non-null values.
Applying run detection indiscriminately could cost more than it saves.

## Statistical CPU instruction samples

The standard `perf` executable was unavailable, but a normal, unprivileged
`perf_event_open` call for this task's child succeeded. The retained standalone
Python diagnostic uses Linux software CPU-clock sampling at a requested
1,999 Hz, one inherited child event per allowed CPU, and excludes kernel/hypervisor
execution. It neither changes kernel policy nor attaches to unrelated tasks.

One full baseline export produced **8,471 samples across six threads, zero lost
records**. Every Parquet file and normalized manifest still matched. Addresses
inside the executable were assigned to bounded outer symbol ranges from
`nm -S -n -C`; these are instruction samples, not call stacks. Inlined code is
included in its containing outer symbol. This is a diagnostic ranking from one
run, not an exact decomposition of all CPU time.

| Outer symbol | Samples | Fraction of all samples |
| --- | ---: | ---: |
| Parquet `GenericColumnWriter::write_mini_batch` | 714 | 8.429% |
| Zstd `ZSTD_compressBlock_fast` | 433 | 5.112% |
| `TransformVersion::apply` | 346 | 4.085% |
| Parquet `RleEncoder::put` | 300 | 3.541% |
| Arrow dictionary `append_option` | 229 | 2.703% |
| `BitReader::read_int_packed` | 219 | 2.585% |
| Name interner `pooled` | 216 | 2.550% |
| Export sink `on_rpc` | 214 | 2.526% |

The executable accounted for 6,550 samples, libc for 1,917, and libm for four.
The installed libc is stripped: nearest exported-symbol names from
`addr2line` misleadingly labeled optimized memory/string routines, so the
published report keeps only the libc image aggregate. No caller attribution
for those 1,917 samples is claimed. CPU user call stacks and hardware cache
miss counters were not collected.

## Allocation and I/O limits

A separate complete baseline export loaded the system glibc `libmemusage`
interposer. Its output was again byte/counter/manifest equivalent. It reported:

| Intercepted function | Calls | Library-reported bytes |
| --- | ---: | ---: |
| `malloc` | 2,559,372 | 2,408,438,208 |
| `realloc` | 932,103 | 504,305,843 |
| `calloc` | 730,498 | 247,155,207 |
| `free` | 3,289,879 | 3,027,485,158 |

The library reported a heap total of 3,159,899,258 bytes and heap peak of
184,122,012 bytes under its own accounting. **These are a diagnostic subset,
not complete Rust allocated bytes, allocation call stacks, or process RSS.**
The interposer exports only malloc/calloc/realloc/free; aligned allocation
paths can bypass it. Reallocation byte accounting is the interposer's metric,
not an independently established sum of full requested sizes. Timing from
this interposed run is not used as normal performance evidence.

`strace` could not start the requested I/O diagnostic:
`PTRACE_TRACEME: Operation not permitted`. No traced replay ran and no policy
change or alternate-privilege attempt was made. Isolated syscall/disk time
therefore remains unmeasured. Zero input block operations in warm-cache
controls does not prove zero reads or establish cold disk throughput.

## Implementation priorities and acceptance boundary

1. The largest measured writer costs remain Parquet encoding and compression.
   Existing dictionary selection and row-group/page boundaries have prior
   evidence and are part of the current output contract. Do not switch codecs,
   omit statistics, or narrow output to create a speed claim. A larger encoding
   optimization needs its own exact-work validation.
2. Test a small run-aware dictionary builder for highly repetitive group-path
   or checkpoint-identity columns. Arrow provides bulk append methods that can
   preserve value order and nulls. Accept it only after equivalent repeated
   full exports, a second replay, and an identical-executable control separate
   its benefit from observed noise.
3. The name interner and RPC path are additional sampled opportunities. Any
   caching proposal must retain resource limits, schema-generation changes,
   unresolved references, and raw payload preservation. Their sampled cost
   alone does not justify another speculative cache.

The ordinary parser, movement decoder, schemas, and compression options are
unchanged by this profiling commit. No Phase B optimization has passed an
acceptance gate in this document; later implementation reports must provide
their own results. Allocation/CPU/RSS measurements cover this single public
12.09 input and do not establish private-corpus or competitor performance.

## Evidence and reproduction

- [Fresh stage samples and exact output gates](evidence/phase-b-profile-stages.json)
- [Build identity and per-pass column census](evidence/phase-b-profile-build.json)
- [CPU sample ranking](evidence/phase-b-profile-cpu.json)
- [Allocator subset and blocked I/O diagnostic](evidence/phase-b-profile-extra.json)
- [Instrumentation-only source patch](evidence/phase-b-stage-profile.patch)
- [Standalone Linux x86-64 CPU sampler](evidence/phase-b-cpu-sample.py)

Apply the diagnostic patch only to an isolated baseline checkout. Freeze a
normal release executable first, then apply the patch and build with the same
command. Run the existing [stage runner](evidence/phase-a-profile-runner.py)
using fresh work/report paths, the two frozen executables, this revision's
`tools` directory, and the pinned public input. That runner verifies exact
output equality before accepting each of its six measurements.

The CPU sampler takes explicit executable arguments and local output paths:

```bash
python docs/evidence/phase-b-cpu-sample.py \
  --report /private/new-samples.json --log /private/new-parser.log -- \
  /private/frozen-vrfkit export /public/sample.replay \
  --out /private/new-export --checkpoints
```

Its raw address/mapping report and parser log remain private. Resolve each
sample against the corresponding recorded executable mapping, then use
`nm -S -n -C` to assign only addresses lying inside a symbol's declared size.
Keep unknown/shared-library addresses separate; do not substitute the nearest
stripped export name. Independently fingerprint the sampled output using the
same resource harness before accepting a report. The published evidence here
contains aggregates and hashes only.
