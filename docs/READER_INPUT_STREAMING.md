# Reader file input: design and measurement

`ReplayReader::read_path` and `read_path_with_schema_limits` now read
container chunks from read-only file handles instead of retaining the whole
input file. `read(&bytes, ...)` still borrows the caller's complete buffer.
Both use the same decode pass, schema policy, records, source identities and
consumer-error handling. The CLI's file loading and Parquet writers are
unchanged by this change.

This report is an **isolated Reader comparison on the parser contract
before the native movement component change**, based on commit `d44bcd2`. It
does not substitute for validation of the combined 0.6.0 source.

## Input and ownership design

Metadata uses the existing container parsers. A prefix bounded by the existing
info-format limits and the declared Header payload are buffered and released;
the prefix probe can reread a small amount of data after seeking. The new
`vrf_container::read_preamble(Read + Seek)` helper keeps I/O errors distinct
from existing structural errors and leaves the cursor after the Header.

The main pass and decompression helper open independent file handles. They
scan 8-byte chunk headers independently, but assign each body to one reader:
the main cursor reads Event payloads and the helper reads ReplayData payloads.
Checkpoint, repeated Header and unknown payloads are range-checked before
seeking over them. No complete chunk index or complete input copy is retained.
The helper retains a reusable encoded buffer and, as in the slice path,
at most one additional decompressed chunk ahead of the main callback.

Memory therefore depends on the largest encoded, Event and Header chunks,
current and prefetched decoded chunks, parser state and consumer retention.
An individual large chunk is still buffered in full; this is not a fixed
total-memory bound. Existing decompression and schema limits still apply.

Files must remain unchanged until the read returns. Independent opens do not
provide an immutable snapshot. Detected length changes fail, but concurrent
same-length rewrites or replacement are not guaranteed to be detected. Call
`read` on an owned snapshot when that isolation is needed. High-level input
from arbitrary non-seekable `Read` sources and checkpoint callbacks remain
outside this change.

Callbacks remain provisional until success. A callback error returns the
original consumer error and stops further callbacks, including after later
malformed data. Cancellation does not promise immediate cessation of all
work: the helper can finish decompression or scan non-ReplayData headers to
its next send before noticing the closed receiver. This follows the existing
slice prefetch pattern and is a remaining cancellation-work limitation.

## Real-input measurements

The same optimized Rust 1.86 executable has `buffered` and `file` modes. Each
series uses one warmup pair and five measured pairs, alternating execution
order. Each child runs in the existing fresh resource-measurement worker.
Runs were scheduled in quiet windows with no other builds or tests on the
host. The host still exhibited temporal variation; quiet scheduling does not prove
an otherwise stationary machine.

The consumer counts every emitted row and raw field byte in both modes. Full
callback fingerprints were compared separately so diagnostic formatting does
not dominate normal-use timings. All measured and warmup count outputs were
identical; input and executable SHA-256 hashes were unchanged after each
series. CPU time includes user plus system time; utilization is CPU/wall and
can exceed 100% because decompression runs on another thread.

The following are the **individual variant medians**, not paired ratios:

| Input / mode | Wall seconds | Total CPU seconds | CPU utilization | Peak RSS bytes | Input MiB/s |
| --- | ---: | ---: | ---: | ---: | ---: |
| 12.09 buffered | 1.63684 | 1.98021 | 121.08% | 107,819,008 | 35.55 |
| 12.09 file | 1.51649 | 1.81504 | 121.42% | 49,946,624 | 38.37 |
| 12.05 buffered | 1.16250 | 1.49575 | 125.05% | 86,511,616 | 44.88 |
| 12.05 file | 1.15345 | 1.42746 | 123.76% | 31,563,776 | 45.23 |

Input sizes were 61,015,648 bytes (12.09) and 54,702,346 bytes (12.05).
For a paired comparison, each file-mode measurement is divided by its
same-round buffered measurement before taking the median:

| Series | Paired wall change | Paired total CPU change | Paired peak RSS change |
| --- | ---: | ---: | ---: |
| Corrected 12.09 | -2.59% | +2.15% | -53.68% |
| Corrected 12.05 | +1.70% | -0.61% | -63.53% |
| Identical file/file control, 12.09 | -8.35% | -5.31% | -0.025% |

The identical-command control shows that the timing differences do **not**
establish a speedup or universal performance neutrality. The stable finding
is lower peak RSS on these two real inputs. No broad replay-speed benefit,
allocation-count reduction, isolated I/O-time improvement or private-corpus
result is claimed. Resource I/O block counters are retained in the JSON, but
are not independent I/O timings. The Reader creates no Parquet files; emitted
row/raw-byte equality is separate from the unchanged CLI's export validation.

### Rejected initial scheduling

The first implementation put all chunk notifications on one rendezvous
channel. On 12.09 it reduced RSS by 74.69% but increased paired wall time by
26.95%, with paired total CPU change -0.73%. Its variant wall medians were
1.68978 -> 1.96849 seconds and utilization 122.24% -> 99.29%.

Header inspection found 22 ReplayData chunks, 21 checkpoints and 254 Events
in that input, with 8-21 non-data chunks between successive data chunks.
12.05 had gaps as large as 24. The rendezvous blocked the helper at each Event
while the main thread was still decoding the preceding chunk. A queue of one
or two notifications would not bridge these observed gaps. The corrected
independent header scans restore the original slice pipeline's lookahead
without increasing the number of prefetched decoded chunks. The initial
negative measurements remain in the evidence and are not acceptance results.

## Correctness verification

Eleven new Rust tests cover all truncated prefixes of generated preambles and
mixed chunk streams, every first-chunk kind and malformed size, the maximum
permitted info fields, oversized skipped chunks, a sparse 64 MiB skipped
payload fixture, I/O categories, callback ordering and original consumer-error
identity. Existing custom schema-limit and provisional-record tests also pass.
All 26 focused preamble/Reader tests passed on the corrected implementation,
as did core Clippy through Rust 1.86. The initial implementation's combined
core container/Reader suite passed 218 tests; that older execution is not
relabeled as an additional corrected-source full-suite run.

The corrected executable was then run in both modes on all six public inputs:
12.05, 12.08, 12.09, 12.10, 12.11 and 13.00. Every callback's Debug projection,
explicit IEEE float bits, movement provenance and detached summary had the
same diagnostic length and FNV-1a-64 fingerprint. This is a same-decoder input
equivalence check, not an independent reinterpretation of the wire format or
a cryptographic reconstruction proof. Every input replay SHA-256 stayed unchanged.

The largest input comparison included 4,295,629 emitted rows: 1,657,000 fields,
2,634,029 movement samples and 4,600 actor rows, plus 254 native Events and
21 explicitly skipped checkpoint chunks. Both modes counted 17,491,180 raw
field bytes, zero lost content blocks and zero decode errors. The earlier
independent C# movement comparison is preserved as earlier work and was not
rerun by this input test.

## Reproduce

Build the aggregate-only probe from the source revision under review:

```bash
cargo +1.86.0 build --release -p vrfkit --no-default-features --example reader_input_probe --locked
target/release/examples/reader_input_probe buffered /path/to/match.vrf --fingerprint
target/release/examples/reader_input_probe file /path/to/match.vrf --fingerprint
```

The two fingerprint outputs must match exactly. The probe does not print
player names or payload values. Default-feature builds and CI on the
combined source are separate checks.
