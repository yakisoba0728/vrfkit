# Direct network API boundary fixes

The coordinator reproduced two defects in the recovered integration baseline.
These author tests are separate from E's independent public-API probes and C's
source review, linked by the final evidence index.

## Findings and behavior

1. **P2: short partial buffers.** `PartialBunchAccumulator::add_fragment` accepts
   a slice and a separate bit count. Empty and short slices caused indexing
   panics; a `usize::MAX` claim could instead report nonexistent discarded bits.
   The guard now runs before admission, retires and returns any prior assembly,
   sets `PayloadOverrun`, and counts only supplied bits plus retained prior bits.
   It neither completes a fragment nor contaminates later valid reassembly.
2. **P2: wide packet lengths.** Narrowing byte length to `i32` before multiplying
   by eight overflows at 268,435,456 bytes in debug builds. The private arithmetic
   helper was first extracted with the same expression so the failing boundary
   could be exercised without allocating a huge packet. Checked `u64`
   multiplication/subtraction now preserves all ordinary sentinel positions and
   rejects unrepresentable sizes. Normal frame parsing already caps packets at
   2,048 bytes; this finding concerns direct packet-reader use.

## Executed checks

The test-only commit `8db7d967cf03655c5109fd9d12d8d0def927eee3` failed both short-buffer
regressions and the length regression. Fixed commit
`aa00e6a480d874da543dcb0eefab0590be4166fa` passed all 119 package tests in debug
and the unchanged workspace release profile. Clippy with warnings denied and
format checks passed. Commands, platform, hashes and actual exit codes are in
[receipt.json](receipt.json), with complete logs alongside it.

The three new regular test functions contain additional boundary/recovery
cases; case count is not represented as a larger test count. The CLI string
mapping was extended for the new raw-preservation reason. Its integrated
compilation and final cross-platform execution belong to the final workspace
and CI receipts, not to this package-only receipt.

## Compatibility and limits

`PartialSequenceKind` and `PartialPayloadReason` each add `PayloadOverrun`;
downstream exhaustive matches must add a case. No existing case or payload is
removed, no wire type is inferred, and no manifest counter is added. The
ordinary pipeline stages complete byte windows and therefore does not produce
the new case for previously valid replays.

No throughput, latency, RSS, Windows execution or private-corpus claim is made
here. The byte-length tests establish arithmetic and refusal behavior, not a
large-file benchmark.
