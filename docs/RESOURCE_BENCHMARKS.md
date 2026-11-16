# Reproducible export resource comparisons

`tools/benchmark_resources.py` compares two existing vrfkit executables on the
same replay. It measures complete exports and checks that every run produces
equivalent output before publishing comparison ratios. It requires Python
3.11 or newer and the standard library; it does not require PyArrow or an
external timing executable.

This complements `tools/bench_export.py`, the existing wall-time regression
guard. It does not update that guard's committed baseline, publish artifacts,
download a corpus, or run a competitor with a different work contract.

For the same-output Phase C baseline and exact build provenance, see the
[benchmark handoff](PHASE_B_BENCHMARK_HANDOFF.md). The user explicitly delegated
quantitative measurements to the agents in Phase D; the earlier user-only
restriction is historical. The [Phase D ledger](PHASE_D_STATUS.md) identifies
current work. Functional validation and measured performance remain separate
acceptance gates.

## Run a paired comparison

Build both revisions in separate worktrees, with the same Rust toolchain,
features and release profile. Use **separate, initially absent Cargo target
directories** too: a shared `CARGO_TARGET_DIR` can retain another worktree's
artifacts. Select each executable from a successful Cargo `compiler-artifact`
record and retain source/lockfile/compiler/binary hashes; see the handoff's
[build procedure](PHASE_B_BENCHMARK_HANDOFF.md#2-isolate-and-record-both-builds).
Keep both executables unchanged for the entire
comparison. Use absolute executable paths and new work/report paths:

```bash
python tools/benchmark_resources.py \
  --baseline-exe /path/to/baseline/vrfkit \
  --candidate-exe /path/to/candidate/vrfkit \
  --replay /private/corpus/example.vrf \
  --work-dir /private/benchmarks/run-001 \
  --output /private/benchmarks/run-001-report.json \
  --warmups 1 --repeats 5 --checkpoints
```

The example paths are placeholders. Windows accepts native executable and
directory paths, including an `.exe` suffix. Omit `--checkpoints` to compare
main-pass exports; both executables always receive the same mode. Both modes
should be measured when the changed code affects checkpoint work.

Each round runs both executables sequentially. Order alternates baseline /
candidate, then candidate / baseline, to reduce systematic order effects.
Warmup pairs use the same alternation and undergo the same output checks, but
their measurements are excluded from statistics. Five measured pairs are the
default; more repetitions are useful when dispersion is large.

Run the comparison without other builds, tests or parser jobs on the same
machine. Filesystem cache state is not reset: this is a warmed comparison, not
a cold-storage benchmark. Hashing outputs between runs also warms the
filesystem cache. Alternation reduces ordering bias but cannot remove machine
noise or make runs on different hosts comparable.

## Measurements and units

Each export is launched by a fresh measurement worker. Its only child is the
selected executable, so earlier exports cannot contaminate the next export's
child high-water memory measurement. Wall time begins before process launch
and ends when the child exits. Worker interpreter startup, output hashing,
manifest checks, cleanup and report writing are outside that interval. A
watchdog permits a blocking process wait rather than timeout polling, which
would add avoidable delay to short runs.

| Field | Meaning |
| --- | --- |
| `wall_seconds` | Process launch through completion, including all export I/O |
| `cpu_user_seconds`, `cpu_system_seconds` | Child user and kernel CPU time |
| `cpu_total_seconds` | Sum of child user and kernel CPU time |
| `cpu_utilization_percent` | `100 * cpu_total_seconds / wall_seconds`; 100% means one logical CPU fully occupied, and a threaded parser can exceed 100% |
| `peak_rss_bytes` | Child peak resident memory, normalized to bytes |
| `input_mib_per_second` | Input bytes divided by wall seconds and 1,048,576; compressed input throughput, not decoded-byte throughput |
| `io_input_blocks`, `io_output_blocks` | OS-reported block I/O operation counts; these are neither bytes nor I/O time |
| `parquet_bytes` | Total bytes in the expected Parquet files |
| `output_bytes` | Total Parquet plus manifest bytes; the manifest's elapsed-time text can vary in length |

On Unix, resource measurements use `getrusage(RUSAGE_CHILDREN)`. CPU time may
include descendants waited for by the measured command. On Linux, peak RSS is
reported by the OS in KiB and multiplied by 1,024; on macOS it is already in
bytes. Other platforms with unknown RSS units report `null` for RSS. The
standard-library implementation reports `null` for CPU, RSS and block I/O on
Windows, where the `resource` module is unavailable. It does not substitute a
machine-wide measurement or fabricate zero usage.

Child peak RSS is the largest reported process peak, **not the sum of
simultaneously resident processes**. The harness is intended for the vrfkit
process and its threads. It does not manage detached descendants or terminate
an arbitrary process tree on timeout. Do not pass a launcher that daemonizes
its actual work.

Allocation counts, allocated bytes, separate I/O time and independent semantic
correctness are explicitly listed as unmeasured. A faster wall time can cost
more CPU or memory; compare those measurements separately. A low utilization
percentage alone is not evidence of efficiency.

## Equivalence and validity gates

Before accepting a sample, the harness requires all of the following:

1. Successful process exit before the timeout.
2. Exactly the expected main tables and, when requested, checkpoint tables.
   Entries must be regular files and have a Parquet header/footer envelope.
3. A strict JSON manifest, with no duplicate keys or nonfinite constants.
4. Required nonnegative integer quality counters, the existing zero-error
   decoder/preservation gates, matching parser/sink event tallies, no unmatched
   RPC-loss count, and positive main decoding work. Requested checkpoints must
   have observed checkpoint chunks and decoded content.
5. Every required CLI summary counter, zero error counters, and reconciled
   overlay bucket totals.
6. Identical Parquet SHA-256 hashes, identical summary counters, and identical
   normalized manifests across **every warmup and measured run**.
7. Unchanged input, executable and harness hashes after the complete run.

Only `source_file` and `elapsed_ms` are excluded from manifest equality. Other
metadata, including player identities, still participates through a canonical
hash; none of those private values is copied into the report.

The comparison is deliberately strict. Different Parquet bytes invalidate it
even when an independent decoder could establish equal rows after an encoding
change. In that case, retain exports with `--keep-exports`, perform the
independent value/raw-preservation comparison separately, and report this
harness's result as invalid until its equivalence requirement is satisfied.
Do not remove output to obtain a favorable timing.

Byte equality proves equality to the baseline for this input and mode. The
lightweight Parquet envelope check is not a full footer or row decoder, and
the counter gates do not independently prove game semantics or reconstruction
of all original bytes. Continue to run `verify_build_corpus.py`, raw/typed
evidence checks and targeted mutation tests for parser changes.

## Reports, privacy and interpreting results

The work directory must be new and is created with owner-only permissions on
platforms honoring POSIX modes. It contains command requests, raw parser logs
and diagnostic details, which can identify private inputs. Treat it as private
on every platform. Exports are removed after fingerprinting unless
`--keep-exports` is specified; this limits disk use to one export at a time.

The JSON report contains executable/input/harness SHA-256 provenance, platform
information without a hostname, numbered samples, fixed counter names,
output digests and statistics. It omits input basenames, absolute paths,
command lines, player identities and decoded values. Content hashes still
identify inputs to someone who already has the same files; review report
sharing under the corpus owner's policy.

The report is checkpointed after each sample. A partial report has
`status: "running"`, `equivalent: false` and `comparison: null`, so an
interrupted run cannot look like a completed comparison. Existing report/work
paths are refused, and the report must live outside the work directory so it
cannot overwrite per-run diagnostics. Use new paths for a rerun. No baseline
is silently replaced.

For each variant, measured samples provide the median, minimum, maximum,
sample standard deviation and median absolute deviation. A single sample has
no estimated standard deviation (`null`). Paired ratios are candidate /
baseline, with one ratio per measured round and their median. For example,
`0.80` wall time means the candidate took 80% of baseline time in the paired
median; the corresponding reciprocal speed ratio is 1.25. These are measured
ratios, not confidence intervals or a universal speed guarantee. Missing
metrics, or a zero denominator caused by CPU timer resolution, produce a
`null` ratio.

Any failed export, missing/invalid counter, output mismatch or changed input
invalidates equivalence and suppresses all comparison ratios. The exit status
is 0 for completed equivalent work, 1 for a failed comparison, and 2 for
invalid arguments. A failure stops the run instead of repeatedly timing a
known-broken export. Valid samples preceding the failure remain in the report
for diagnosis; they are not an accepted comparison.

## Measurement references

- [Python `resource.getrusage` and resource fields](https://docs.python.org/3/library/resource.html#resource.getrusage)
- [Linux `getrusage(2)` units and child-process accounting](https://man7.org/linux/man-pages/man2/getrusage.2.html)
- [Python subprocess timeout and process-wait behavior](https://docs.python.org/3/library/subprocess.html#subprocess.Popen.wait)

The dedicated test suite executes synthetic child processes and checks
failure, timeout, peak-RSS isolation, output mutation, missing counters and
input modification. Run it with:

```bash
python -W error -m unittest discover -b -s tools/tests -p 'test_benchmark_resources.py'
```
