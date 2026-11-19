# Phase A performance investigation

This is a diagnostic checkpoint, not acceptance of an optimization. The
historical measurements below are retained as earlier work. The fresh profile
was executed during Phase A on the recovered native-framing candidate, whose
151 runtime-source hashes equal the existing `2e3526f` build receipt. The
unmodified executable SHA-256 is
`e6b855ed2ea154260a30bca8606eefa15454e1b76cecca09a0e886e07d096a69`.
Native correctness review and integration are separate gates.

## Fresh observed bottlenecks

The public 12.09 input is 61,015,648 bytes. Three uninstrumented and three
instrumented full exports ran sequentially in a coordinated quiet window,
including checkpoints. Every export passed the existing strict resource
harness: all 13 Parquet files, required summary counters, and nonvolatile
manifest values matched. Each export wrote 43,984,122 Parquet bytes. No
validation, decoding, table, or row was disabled for this profile.

The diagnostic build records Linux per-thread scheduled CPU time at thread
entry/exit, wall time around each Arrow batch build and Parquet write, and
wall time around one of every 1,024 movement calls. It is not a statistical
CPU stack sampler. The patch changes code generation and incurs measurement
overhead; stage wall times can overlap and must not be summed into process
wall time. The sampled movement times are diagnostic samples, not a precise
estimate of all movement CPU cost.

| Observed stage | Median across three diagnostic runs | Implication |
| --- | ---: | --- |
| Main parsing thread CPU | 1.747 s | Largest separately measured CPU participant and nearly busy for its whole pass |
| Main fields writer CPU | 1.074 s | Significant concurrent CPU work |
| Main movement writer CPU | 0.768 s | Significant concurrent CPU work |
| Checkpoint fields writer CPU | 0.261 s | Smaller than the main writers |
| Main fields Arrow batch construction | 241.678 ms | Worth future allocation/column-construction profiling |
| Main fields Parquet encoding | 788.258 ms | Larger than its Arrow construction cost |
| Main movement Arrow batch construction | 39.233 ms | A movement batch-copy change has limited measured opportunity here |
| Main movement Parquet encoding | 747.932 ms | Encoding dominates this writer's measured work |
| Whole-file input read | 39.133 ms | Input buffering matters primarily to memory in this warm-cache sample |

The movement decoder handled exactly 2,634,029 `parse_single_move` calls per
run. Each run sampled 2,573 calls. Per-run sample medians were 70, 70, and
80 ns; the 95th-percentile samples were 110, 120, and 111 ns. An earlier
assembly inspection showed an out-of-line call at both movement-section
call sites; explicitly inlining the function removed both calls. This
supports a small, isolated code-generation experiment. It does not establish
an end-to-end speedup by itself.

The uninstrumented diagnostic controls had the following medians. These
three observations characterize this profiling window; they are not a paired
optimization benchmark or a universal baseline:

| Metric | Native candidate, uninstrumented |
| --- | ---: |
| Wall-clock | 2.096439 s |
| Total CPU time | 4.948344 s |
| CPU utilization, one core = 100% | 251.776% |
| Peak RSS | 198,193,152 bytes |
| Input throughput | 27.756 MiB/s |
| Parquet bytes | 43,984,122 |
| Total output bytes | 44,676,171 |

The OS reported zero input block reads and a median 87,336 output blocks for
these controls. This is consistent with cached input; it is not a measurement
of physical disk throughput. Allocation counts, allocated bytes, and isolated
I/O time were not measured. Other participants, including decompression and
checkpoint decoding, are not separately attributed in this limited profile.

## Historical results retained without relabeling

All rows below are earlier measurements, not Phase A reruns. Each resource
comparison used one warmup pair and five measured alternating pairs. The
reports' hashes and raw paired ratios are retained in the accompanying
historical summary.

| Earlier experiment | Paired wall ratio | Paired CPU ratio | Interpretation |
| --- | ---: | ---: | --- |
| Provenance correction, older native contract | 1.162312 | 1.179217 | Observed slower in every primary pair; environment drift prevents treating it as an established general regression |
| Identical corrected executable control | 0.970472 | 1.007032 | Pair-level variation was substantial, even with one binary |
| Native contract, inline / unannotated | 0.900649 | 0.831262 | Promising full-export result, pending fresh confirmation |
| Identical inline executable control | 0.966104 | 0.998380 | Same-binary drift remains visible |
| Native contract, **unannotated / inline** | 1.074046 | 1.084482 | Labels deliberately reversed; this agrees with inline being faster |
| Native contract, inline / unannotated, validate only | 0.957363 | 0.979861 | Different work contract; cannot stand in for complete export |

The reversed series must be interpreted by executable hashes, not by the
generic names `baseline` and `candidate`. Its positive ratios do not mean the
inline candidate regressed. Conversely, its direction does not eliminate
measurement uncertainty or replace the new acceptance run.

## Ranked experiments for the implementation stage

1. **Explicitly inline `parse_single_move`.** Change only the function's inline
   annotation on the reviewed native source. Verify Rust 1.86 code generation,
   correctness, and repeated complete exports against an unannotated build
   with the same native contract. Include an identical-executable control,
   more than one actual replay, CPU/RSS/throughput, and strict output equality.
   Withdraw it if the new evidence does not establish a useful benefit.
2. **Investigate fields column construction or Parquet encoding only if
   needed.** The measured opportunity is real, but row-group/page boundaries,
   dictionaries, and deterministic bytes are part of the existing contract.
   A speculative batch-size or compression-policy change is not justified by
   this profile. No such implementation is proposed for automatic adoption.

Reader streaming is a separate memory experiment. In this sample, eliminating
the input allocation may reduce peak memory substantially while the direct
read time is small; actual Reader measurements must decide the tradeoff.

## Evidence and reproduction

- [Fresh sanitized measurements](evidence/phase-a-fresh-profile.json)
- [Historical report provenance and ratios](evidence/phase-a-historical-performance.json)
- [Diagnostic source patch](evidence/phase-a-stage-profile.patch)
- [Standalone diagnostic runner](evidence/phase-a-profile-runner.py)

Use an isolated checkout of the reviewed native source. Keep the original
release executable unchanged, apply the diagnostic patch only in the isolated
checkout, and build with `cargo +1.86.0 build --release -p vrfkit --locked`.
The patch is an experiment artifact and is not part of normal runtime source.
The original diagnostic binary hash is
`3fdba9c2bf4cda7f5d324a3d32eb0f83205b4e24e0687e7099d07f05cf0892cd`.
Different checkout paths or toolchains can change executable hashes; record
the new ones instead of claiming byte identity.

```bash
python docs/evidence/phase-a-profile-runner.py \
  --baseline /path/to/uninstrumented-vrfkit \
  --profile /path/to/instrumented-vrfkit \
  --replay /path/to/public-12.09.replay \
  --harness-dir /path/to/native-checkout/tools \
  --work-dir /path/to/new-private-work \
  --report /path/to/new-sanitized-report.json
```

Use the native contract's `benchmark_resources.py` and `summary_counters.py`
together. The report includes aggregate values and hashes only. Parser logs,
requests, input names, and manifests remain in the private work directory;
the runner deletes only the export directories it created after checking
their fingerprints. It refuses existing report and work paths.
