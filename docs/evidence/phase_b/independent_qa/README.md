# Independent Phase B functional acceptance

Agent E independently reviewed and verified the combined CLI input, gameplay
projection and dictionary batching changes. There are no blocking findings for
the contracts and fixture scope recorded here. This is functional acceptance;
no performance improvement is asserted. The user reserved final quantitative
performance measurements for their own execution.

The accepted build source is
`2fbead4f716f12f7c5d41ff067ff4ee63a55dc90`. The full local sweep ran at
`9800679c8747fb1f2bcda6b2736bb0053c04206c`; only documentation was added between
those checkpoints. All 133 recorded runtime source/manifests/lockfile hashes
match the sweep, frozen build, post-mutation restoration and merged integration
commit `e294fcf13c2d66ff47d45067341eb6f6e32eb27f`.

## Results and boundaries

| Check | Observed result | Boundary |
| --- | --- | --- |
| Mandatory local gates | All 41 commands exited 0; 826 all-feature Rust tests, 1,178 Python tests, 27 feature cases | Linux with Rust 1.86; includes strict clippy/rustdoc, core-only suites and exact-root Rust/Python interop |
| Public validation and checkpoint export | Six of six public replays passed; 76 checkpoint chunks | Builds 12.05, 12.08, 12.09, 12.10, 12.11 and 13.00; positive checkpoint decoding per build |
| Output preservation | All 78 Parquet files byte-identical; all required summary counters and nonvolatile manifest values equal | Fresh combined-source exports compared to accepted baseline fingerprints; only manifest `source_file` and `elapsed_ms` are normalized |
| Independent raw/typed check | 60,169 values across all 16 specified identities; zero missing identities or mismatches | Only observed fields in the pinned public fixture specification |
| Gameplay projection | 560 native and 302 phase observations match exactly | Values, order and multiplicity within each source stream; no global inter-stream ordering claim |
| Movement cross-check | 16,748 matched wire updates representing 16,899 Rust move rows; zero missing, extra or mismatched updates | Fresh Reader output on 12.10, 12.11 and 13.00 against frozen C# output; latest-move values at f32 precision |
| Implementation mutation controls | Eight of eight deliberately broken implementations failed their selected test assertions | Cargo exit 101 plus the exact test failure; compile failures or zero-test passes are rejected |
| Clean restoration | 22 reader/CLI/gameplay tests and 24 export tests passed; all source and frozen executable hashes unchanged | Compiled target artifacts are not accepted merely because source bytes were restored |
| Adoption example | Documented aggregate PyArrow example passed on main and checkpoint fields | Fresh public 12.10 output; aggregate counts only |

The six replays preserve 22,561 unresolved main-stream RPC payloads. This is
explicitly retained uncertainty, not evidence that every RPC is semantically
decoded. All bytes, native event records, counter categories and table hashes
remain covered by the export comparison. No private replay was executed.

The movement oracle joins by packet, actor, object, channel and update index;
numeric values are never join keys. Earlier moves are counted and indexed, but
the C# reference emits values only for the latest move in an update. The C#
outputs were not regenerated: their input hashes, completion status, raw-output
hashes, historical accepted report hashes and manifest parser-version suffix
were checked. Each manifest identifies parser commit
`fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8`. Old Rust movement dumps were not
misattributed as C# references.

## Source review decisions

The CLI uses file-backed traversal for main input, prefetch and checkpoints,
with separate file cursors and bounded prefetch ownership. The input must stay
unchanged while being read; this does not promise detection of every concurrent
same-size rewrite. A skipped chunk must still fit within the captured file
extent. Late main/checkpoint failures do not replace a previously published
export. Cancellation is cooperative, and the main failure path can still wait
for the checkpoint worker. The existing publication contract handles in-process
errors; it does not promise power-loss or forced-process-termination atomicity.
The earlier author sparse-input proof is historical evidence, not a new E
resource measurement or a claim about total memory for arbitrary decoded data.

The optional gameplay API projects exact recognized native layouts and exact
phase schema/raw/typed matches. Unknown phase values and unresolved observations
remain explicit. It does not infer player identity, kill credit, a complete
match state or checkpoint gameplay. Known-looking but inconsistent native
declarations, timestamps and phase raw windows fail instead of producing a
plausible guessed observation.

The dictionary helper groups adjacent equal `group_path` values by pointer
identity or string equality. The independent elementwise reference and exact
Parquet comparison cover first-occurrence dictionary order, key multiplicity,
different allocations with equal text, transitions and batch boundaries. A
wrong-value mutation that preserves row count is rejected as well as a wrong
repeat-count mutation. Correctness is accepted; its performance effect remains
unmeasured here.

## Artifact provenance

[build.json](build.json) records the exact compiler artifacts, per-file source
hashes and dependency-file source paths. All accepted workspace dependencies
belong to the QA checkout. The target directory was created for QA and was not
shared with an older worktree. The binaries were frozen before replay checks
and before mutations; their hashes were checked again afterward.

The executables use Cargo's test build profile: optimization level 2, no debug
information, debug assertions and overflow checks enabled, and ordinary
executable targets (`test: false`). They are not default release-profile
performance artifacts. The accepted CLI SHA-256 is
`29fa495b19b7dbce2bfe7bbedc1dc9be2492a4c6a5aca2a9d1468d3d74c657d4`.
The author's interrupted release builds produced no accepted executable.

[ci-review.json](ci-review.json) records E's review of F's sanitized CI receipt
and local verification of each PR head tree and changed source/test hashes.
PRs 65, 66 and 67 each have 11 successful jobs and 128 successful steps, with
zero failed, cancelled or skipped jobs/steps. Their Linux/Windows counts are
812/813, 824/825 and 826/827 respectively. The latest stacked PR also matches
all 133 accepted runtime hashes. The Windows package job succeeded for each
head. Synthetic checkout/tree correspondence comes from F's API/log receipt;
those synthetic objects were not available in E's local Git database. E did
not independently download the hosted logs or execute Windows locally. Final
exact integration CI remains the root coordinator's gate.

## Negative controls and rejected work

[mutations.json](mutations.json) records native timestamp, phase raw-window,
skipped-extent, checkpoint omission, checkpoint error propagation, dictionary
repeat-count and Parquet value/order corruptions. Each has original, mutant
and restored source hashes plus the exact failing test and log hash.
[restoration.json](restoration.json) records the clean follow-up suites.

A separate read-only child reviewer completed a harness review without builds,
tests, replay runs, measurements, edits or external-service operations. Its
three findings were addressed: freeze clean binaries before mutations; refuse
Python's assertion-disabled mode; and derive/check the C# commit from each
manifest. The two oracle/mutation scripts were actually invoked with `-O` and
refused with exit 1. See [harness-review.json](harness-review.json).

[rejected-attempts.json](rejected-attempts.json) preserves the rejected scopes:
an interrupted partial sweep, an initial scanner run that found only three
`.vrf` fixtures, stale shared-target author evidence and interrupted author
release builds. The accepted six-fixture run first pins all six hashes and
creates local read-only `.vrf` aliases for the three `.replay` inputs.

The resource-measurement session was interrupted with exit 130 after the user
changed policy. All partial resource reports were invalidated, including any
already completed comparisons. There are zero accepted resource reports.
Cancellation was observed from the execution session; another session's
process namespace was not treated as a global process inventory. No discarded
resource numbers are included here.

## Reproduction

The archived scripts preserve the exact accepted harness source. They require
ordinary Python with assertions enabled, the repository's Python dependencies,
Rust 1.86, a source-specific Cargo target and new output directories. Use the
same checkout throughout. The source/input/reference pins in the reports make
later reruns attributable; a new source revision requires its own receipt.
Detailed local logs and probe rows must remain outside the public repository.

First run the complete [CONTRIBUTING sweep](../../../../CONTRIBUTING.md), plus
`cargo +1.86.0 test --workspace --all-features --locked`, using the environment
and exact commands in [local-gates.json](local-gates.json). Then freeze and run
the functional oracles before any mutations:

```bash
python "$QA_SOURCE/docs/evidence/phase_b/independent_qa/qa_freeze_build.py" \
  --source "$QA_SOURCE" --target "$QA_TARGET" \
  --probe-source "$QA_SOURCE/docs/evidence/phase_b/independent_qa/gameplay_probe.rs" \
  --output "$QA_OUTPUT/frozen"
python "$QA_SOURCE/docs/evidence/phase_b/independent_qa/qa_fresh_oracles.py" \
  --source "$QA_SOURCE" --exe "$QA_OUTPUT/frozen/vrfkit" \
  --gameplay-probe "$QA_OUTPUT/frozen/qa_gameplay_probe" \
  --movement-probe "$QA_OUTPUT/frozen/movement_provenance" \
  --corpus "$PUBLIC_CORPUS" --reference-root "$REFERENCE_EVIDENCE" \
  --output "$QA_OUTPUT/oracles"
python "$QA_SOURCE/docs/evidence/phase_b/independent_qa/qa_mutations.py" \
  --source "$QA_SOURCE" --target "$QA_TARGET" --output "$QA_OUTPUT/mutations"
```

`REFERENCE_EVIDENCE` contains
`reference-comparisons/<build>/reference/{manifest.json,movement.ndjson}` for
12.10, 12.11 and 13.00. The public corpus must contain exactly the six pinned
inputs in the accepted CLI baseline receipt. The oracle script invokes the
common corpus audit, all-identity raw/typed check, native/phase comparison,
wire-identity movement comparison and aggregate adoption example.

After mutations, rerun the commands in
[restoration.json](restoration.json), compare every runtime hash in the frozen
build receipt, check `git diff HEAD`, and recheck the four frozen executable
hashes. Do not reuse a compiled target executable merely because a mutation
script restored the source. No benchmark invocation is part of this recipe.

The top-level [receipt.json](receipt.json) indexes the aggregate receipts and
harnesses by hash. Replay files, Parquet, raw probe rows, GUID values, private
paths and full logs are deliberately absent from this commit.
