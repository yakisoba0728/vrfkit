# Performance, memory and I/O review

## Decision and scope

**Bounded review complete; fresh performance measurement NOT_EXECUTED.**
No new runtime optimization is proposed or accepted by this review. There is
no new throughput, process-RSS, allocation-rate or performance-nonregression
claim. The Phase D reserve-capacity experiment stays withdrawn.

Role C reviewed the full attached final-consolidation prompt, `CLAUDE.md`,
`CONTRIBUTING.md`, the Phase D report, protocols, raw comparison/control records,
stage profile and withdrawal evidence. The source checkout is
`6e54158f0747ce55334f7138e7d5ce7ee8068bb1`; its product baseline is
`8581642283bd5a571967129d73897db8ee5408ba`. Separate immutable reviews cover
the coordinator's net changes through `aa00e6a480d874da543dcb0eefab0590be4166fa`
and B's checkpoint resource fix `f849f62ed10518905dce845743e07daa2d55a8b3`.
This receipt does not label those changes as executed in the baseline checkout.

The coordinator reserved heavy-work slots while rebuilding and running final
functional QA. No isolated control window or new demonstrated optimization
mechanism justified another benchmark experiment. Source review and existing
evidence were enough to reject speculative tuning. Final product/runtime,
Windows and hosted CI acceptance belong to the separate integration receipts.

## Retained measurements independently recomputed

[`review_retained.py`](review_retained.py) uses Python's standard library,
without importing the benchmark harness. Its new execution checked all 20
retained series: **280 historical records, comprising 40 warmups and 240
measured records**, and 2,640 per-metric summary-statistic cells. It verifies
series hashes against their aggregate report, complete AB/BA order, six pairs
per variant, same-executable A/A identity, before/after provenance, exit/status
and validity fields, recorded output fingerprints, the nine native movement
and two Focus counter keys per applicable scope, CPU/throughput arithmetic,
medians, minima/maxima, sample standard deviations, median absolute deviations
and paired ratios. The result is
[`retained-arithmetic.json`](retained-arithmetic.json).

All six primary and four withdrawn-experiment wall comparisons still have
pairs on both sides of 1 and overlap their contemporary A/A ranges. Their
historical classification remains **INCONCLUSIVE**. Range overlap is
descriptive; it is not a confidence interval, an equivalence test or a proof
that no regression exists. Paired medians are medians of individual ratios,
not ratios of independently summarized medians.

Unfavorable observations are retained:

- The primary 12.05 main comparison has a **+9.5463%** paired wall median.
- The withdrawn candidate's 12.09 checkpoint RSS increases in **all six pairs**,
  from +0.6723% to +3.1098%, with a +1.5823% paired median.
- Its allocator-interposed diagnostic removes 1,571 realloc calls but
  increases reported heap total by **27,201,974 bytes** and reported peak by
  **3,464,100 bytes**. Those are one historical diagnostic pair's allocator
  observations, not process RSS or ordinary-release efficiency.
- Other adverse samples remain in the raw records and recomputed condition
  list, including six positive primary 13.00 checkpoint RSS ratios. None is
  omitted because it complicates an aggregate conclusion.

Four in-memory negative controls changed a sample's successful exit code,
fingerprint, summary median, or mandatory counter. Each was rejected, and the
restored checker passed; see
[`arithmetic-controls.json`](arithmetic-controls.json). These repeated metadata
reads do not add replay coverage or measurement samples. Historical Parquet
files and replay bytes were not reopened by this arithmetic review.

## Source review and bounded decisions

| Path | Observed implementation and retained evidence | Decision |
| --- | --- | --- |
| Parsing and schema lookup | Packet/bunch processing is interleaved; pass buffers are reused. Existing name interning and adjacent group-path dictionary runs avoid repeated work. Decode order carries channel/cache state. Phase D attributed 1.781 s scheduled CPU to the main parser on its diagnostic input. | No parser parallelization, forced-inline change or cache rewrite without a newly isolated mechanism and preservation evidence. |
| File input and decompression | `reader_input.rs` range-checks chunk extents, reuses encoded buffers, skips unrequested checkpoint bodies, and uses independent file cursors. A rendezvous permits one decompressed ReplayData chunk ahead. Reader documentation makes the largest chunk and parser/consumer state explicit. | Retain current bounded-by-chunk design. It is not a global process-memory cap or evidence of physical-storage throughput. |
| Arrow construction and raw payload copies | `FieldRecord` uses interned names and inline `SmallVec` payload storage. Schema macros preserve nullable binary semantics and field order. The raw-bits reserve candidate is absent from the accepted nullable-byte builder. | Do not resurrect the withdrawn capacity experiment or replace it with a speculative arena. Raw absence, empty bytes, spilled payloads and first-occurrence dictionary order remain contracts. |
| Parquet encoding | `TableWriter` converts at 8,192 rows, independently of row groups; the 1,024 write batch alignment, column dictionaries and ZSTD policy are pinned. Rows are cleared before encoding. Byte-budgeted tables close groups on the documented retained-byte budget. | Do not tune row/page/group thresholds or dictionary/compression settings without exact output parity and an isolated resource benefit. |
| Writer scheduling | Fields/movement writers use 16,384-row messages, queue depth 4 and one recycled buffer. Errors are joined and propagated; independent checkpoint work joins before publication. Queue bounds count messages, not a universal heap-byte limit. | Preserve backpressure and failure semantics. More threads or deeper queues have no demonstrated benefit here. |
| Checkpoint GUID path expansion | B's fix charges cumulative decoded UTF-8 bytes per declaration before indexed-path cloning, callbacks and cache insertion. `net_guid_rows` later creates owned path strings, so sharing only cache strings would leave detached-copy amplification. | Accept the bounded resource-admission reasoning, subject to independent runtime verification. No throughput claim. |

The historical stage profile also attributes main fields Arrow/Parquet work
to 194.174/693.390 ms and movement Arrow/Parquet work to 32.015/606.079 ms.
These are instrumented, overlapping thread intervals. They cannot be summed
into end-to-end wall time or converted into a promised speedup. Cached payload
reads (43.6 ms in that profile) do not measure physical disk latency; Parquet
encoding includes output I/O. The previously measured raw-bits construction
opportunity was only 35.601 ms main plus 7.710 ms checkpoint, and its bounded
candidate already failed the adoption bar.

## Independent net and resource-change review

The cumulative net patch first refuses a byte slice shorter than its declared
bit count. Using ceiling division avoids multiplying the untrusted count; it
accepts byte-padded valid windows. Refusal occurs before sequence admission,
marks `PayloadOverrun`, returns the retired prior assembly, and counts supplied
bits without inventing missing bytes. It does not falsely mark resource
admission, fragment or completion success. Normal pipeline staging supplies a
complete bit window, so this branch protects direct accumulator consumers.

Packet sentinel arithmetic now converts the byte length to `u64`, checks the
multiplication and subtraction, and returns a malformed packet on arithmetic
failure. A nonzero last byte is checked by the caller. Ordinary sentinel
positions are unchanged; the former signed 32-bit boundary no longer wraps or
panics. The reviewer found no static blocker. Actual positive/negative runtime
checks remain in the coordinator and independent-QA evidence, not this receipt.
The added public `PartialSequenceKind::PayloadOverrun` and
`PartialPayloadReason::PayloadOverrun` require downstream exhaustive matches
to handle a new variant; this source-API migration must stay documented.

B's fixed 256 MiB policy counts **cumulative resolved path bytes per checkpoint**,
including repeated GUID declarations, literal paths and legacy decimal paths.
It is not an RSS cap, a live-cache-only bound, or a new configurable
`SchemaLimits` field. `Cow` postpones indexed clones until admission. Existing
index/truncation checks and an earlier sink error retain precedence; earlier
callbacks remain provisional on a later failure. The reviewer found no static
blocker, with the public-limit probe and real-input margins delegated to QA.

## Python benchmark harness

The focused `test_benchmark_resources.py` suite was executed once under Python
warnings-as-errors: **20 tests passed, exit 0**, no failures or skips. The full
log is [`harness-tests.log`](harness-tests.log). The command was:

```sh
PYTHONPATH=tools:tools/tests python -W error -m unittest discover -v -s tools/tests -p 'test_benchmark_resources.py'
```

Coverage includes process failure and launch failure, timeout/reaping, fresh
worker RSS isolation, supported/missing resource units, AB/BA planning and
warmup exclusion, paired statistics, invalid measurements, table-set/magic
checks, exact table hashes, nonvolatile manifest hashes, required and duplicate
summary lines, native movement contract checks, positive checkpoint evidence,
counter reconciliation, malformed JSON, input mutation, and invalid arguments.
Mutated raw output with unchanged counters and a failed export both force an
invalid report with no comparison. Synthetic worker timings in these tests are
test mechanics and are not parser performance results.

The harness deliberately checks equal work and preservation metadata; its
Parquet envelope check is not independent decoding or a footer/value oracle.
Typed/raw semantic QA therefore remains necessary. Unsupported resource metrics
stay null. RSS is the largest reaped process peak, not summed process-tree
memory. Full workdir logs remain separate from shareable hashes and counters.
The caller supplies the protocol: the generic default is five measured pairs;
the retained Phase D protocol explicitly supplied six balanced pairs and
separate A/A controls. No default invocation should be relabeled as that
protocol.

The older `bench_export.py` is a wall-clock smoke guard: it checks real exit
status and finite positive samples, uses a wide tolerance, and rejects both
large slowdowns and stale faster baselines. It does not provide paired same-host
controls, exact output equivalence, resource accounting or an optimization
acceptance experiment. Missing-input skips must remain visibly unexecuted.
It was source-reviewed, not executed here.

## Fresh reference check

[`reference-review.json`](reference-review.json) records new read-only GitHub
reads. Default HEAD remains `fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8`
(2026-11-11), and the sole published release is prerelease
`v0.1.0-alpha.2`. The stable-latest endpoint returned 404; the release collection
resolved that endpoint distinction. No source or release delta from Phase D
was observed.

Fresh reads of five pinned source files confirm that default reference export
skips container Events and checkpoints, writes enriched NDJSON, omits undecoded
groups/empty shells, and emits a latest movement sample per update. vrfkit's
typed/raw Parquet product performs different work. Default complete exports
remain **NOT_COMPARABLE** for performance; no C# build, replay execution,
matched-work benchmark or speed ratio was performed.

## Reproduction and remaining boundary

```sh
python -W error docs/evidence/final_consolidation/performance/review_retained.py
```

This command rereads only tracked historical JSON. Source/evidence hashes and
execution accounting are in [`review.json`](review.json). This review made no
product edit, remote-reference mutation, release or public deployment.
It adds no unique real replay, private-corpus or Windows runtime coverage and
does not replace final integration CI.
