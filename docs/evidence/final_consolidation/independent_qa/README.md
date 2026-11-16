# Independent QA for final consolidation

Role E owns the probes and verification here. A, B, D and the coordinator own
the product fixes. Historical Phase C/D oracles are reused only where named;
their historical execution counts are not credited to this run.

## Frozen candidate: public corpus and contracts passed

Runtime source: `1d2d8f6fe68f3ad1f196bca5adac24b0d3fbee6c`.
Default-release executable SHA-256:
`c3a3010083b81332e03d90b888d38dfe069dfb659f3c866637cfed0051d6d417`.
`candidate-public-review.json` binds the source, frozen build receipt,
toolchain, inputs, runner/dependency hashes, commands and reports.

| Fresh final-candidate gate | Result |
| --- | --- |
| Five public builds 12.05, 12.09, 12.10, 12.11, 13.00 | All validate and checkpoint exports pass; 59 checkpoints |
| Baseline/candidate preservation | All 65 Parquet files byte-identical; five stable full manifests and 765 counter values equal |
| Independent typed/raw evidence | 43,844 values; all 16 specifications observed across the five inputs |
| Main/checkpoint mode preservation | Five additional main exports match all 30 main tables; zero raw RPC loss in either scope |
| Focus 12.05 raw-parent oracle | Main: 102 parents, 53 children, three empty trailers; checkpoints: 18 parents, no children/trailers |
| Mandatory counter deletion controls | All 22 omissions rejected |
| JSONL logical-value round trip | 39 tables, 138,727 rows, 298,133 binary bytes compared exactly |
| CLI inspect | Five matching branch/size/checkpoint checks |
| CLI adversity | 25/25 pass: containment, legal sibling/hardlink output, malformed-input rollback and diagnostic replacement |
| Exact-source selected contracts | 43/43 pass: 28 maintained tests, nine retained independent contracts, six new independent regressions |

The five input files remain read-only and hash-identical. Repeated commands
and malformed derivatives do not increase the distinct input count. The 28
maintained contracts overlap the coordinator's regular test suite. The frozen
release source checkout remains clean; temporary QA adapters are installed only
in a separate checkout at the same commit. Fresh public GameplayIdentity joins
and final-source compiled mutation/restoration are still running at this
checkpoint. Hosted CI and Windows package gates belong to the coordinator.

## Checkpoint: baseline and core fixes

`checkpoint-review.json` records commands, source revisions, source/test/log
hashes and results. It is an intermediate checkpoint, not final acceptance.

| Fresh execution | Result |
| --- | --- |
| Five pinned public inputs, clean baseline default release | 5/5 validate and 5/5 checkpoint exports pass |
| Baseline coverage | 59 checkpoint chunks, 1,380,345 oracle blocks, 43,844 typed/raw comparisons |
| Independent net public API probe before fix | 0/3 pass: two slice panics and an incorrect discarded-bit count |
| Same net probe after fix | 3/3 pass |
| Independent schema public 256 MiB boundary before fix | 1/2 pass: 90,019 wire bytes admit 268,500,992 resolved path bytes |
| Same schema probe after fix | 2/2 pass; accepted prefix retained, no rejected-entry callback/cache insertion |
| Fresh identity, callback, cancellation and reader contracts | 38/38 pass on A+B+net integration |
| Baseline CLI source-containment cases | Six expected refusals fail; five destroy/replace the disposable replay |

The sixth CLI case replaces an output-directory symlink while leaving the
original target replay bytes intact. It is a failed conservative refusal
expectation, not a sixth demonstrated input-loss case.

The path resource measurement replays `checkpoint_guid_entries.parquet`
literal/index declarations, including every repeated GUID declaration. Across
196,727 declarations in 59 checkpoints, the largest per-checkpoint resolved
UTF-8 path total is 252,908 bytes; the longest single path is 130 bytes. These
observations establish a margin for the five public inputs only.

## Evidence and reproduction

- `input-acquisition.json`: pinned URLs, hashes, sizes and actual acquisition.
  The original files are read-only and stored outside the repository.
- `private-corpus-capability.json`: sanitized metadata/capability inspection.
  Private binary downloads, hash checks and executions are all zero.
- `guid-independent-review.json`, `reader_malformed_guid.rs` and the two GUID
  logs: newly authored independent Reader-level malformed-identity regression.
  The tests cover ordinary/partial RPCs, valid prefix and recovery, five invalid
  widths, main plus two checkpoints, raw parent preservation and provenance.
- `net_payload_bounds.rs`, `schema_public_budget.rs`: independently authored
  public API probes. Copy them respectively to
  `crates/vrf-net/tests/qa_final_payload_bounds.rs` and
  `crates/vrf-schema/tests/qa_final_path_budget.rs` in an isolated worktree.
- `checkpoint-review.json` and the named logs: baseline failures, fixed core
  checks and freshly rerun Reader contracts. The retained nine independent
  tests come from `docs/evidence/phase_c/independent_qa/contracts.rs`, installed
  temporarily as `crates/vrfkit/tests/qa_phase_c_contracts.rs`.
- `baseline-corpus-report.json`, `baseline-path-resources.json`: fresh baseline
  CLI/PyArrow evidence, aggregate counts and resource measurements.
- `adversarial_cli.py`, `baseline-cli-adversity.json`: disposable public-input
  source-preservation baseline. Candidate mode additionally checks positive
  sibling/hardlink destinations, malformed-input rollback and diagnostic
  replacement.
- `compare_public.py`: candidate comparison runner. Its path-resource helper
  has been run for the baseline measurement. Full final-candidate execution is
  recorded in `final-public-comparison.json` and the matching log.
- `mutate_movement.py`, `movement-mutation.json` and the three `movement-*.log`
  files: compiled stale-identity negative control on the A+B+net integration.
  Removing only the reset compiles and fails with 12 rows instead of 8. Restoring
  the exact source restores the exact test-executable hash and passes. The
  mutation never touches the frozen release executable or public input bytes.
- `compare_identity.py`: final-public-corpus runner for the retained Rust
  GameplayIdentity adapter and independent Python interval oracle. Prepared;
  final-source execution remains pending.
- `run_final_contracts.py`: installs isolated adapters, runs the 43 selected
  contracts and builds the core-only identity adapter. It records every tracked
  Rust/Cargo source hash and verifies that the tracked checkout remains
  unchanged. `final-contracts.json` and the contract/build logs record the exact
  final-source pass and the frozen identity adapter hash.

Published test logs preserve all test outcomes. Workspace prefixes are replaced
with `$QA_ROOT`; raw and published log hashes are separately recorded. Detailed
public replay rows and CLI logs remain outside the repository. No private
replay rows, filenames, player identifiers, or credentials are published.

## Scope limits

Linux results do not establish Windows runtime behavior; the
coordinator owns hosted CI and Windows package gates. No process-wide memory
cap, hostile filesystem race, crash durability, quantitative speedup, or full
private-corpus coverage is claimed. Existing unresolved wire semantics remain
unresolved.
