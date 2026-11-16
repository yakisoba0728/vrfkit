# Phase D independent QA evidence

The accepted production runtime remains `ff8596b546736ea4300e013248609c7c3987e231`.
The metadata/documentation candidate raises all ten production crates to 0.6.0
without changing dependency resolution, runtime source, workflows or build flags.
Final hosted CI and package/source-artifact acceptance is recorded separately
after the final candidate exists; `ci-strategy-review.json` is a source review.

## Executed results

- Primary arithmetic: 168 individual worker records, 12 series and six cases
  independently recomputed. Paired medians, marginal medians, dispersion,
  resource calculations, sample order, provenance and strict fingerprints agree.
  All results remain inconclusive against the declared controls.
- Accepted-runtime exports: six retained fresh exports, three public inputs,
  57 Parquet files, 46 checkpoints and 43,443 typed comparisons; zero decoding
  width failures, typed mismatches or raw RPC loss. All main tables remain byte
  identical when checkpoint output is enabled. The Focus oracle reconstructs
  102 main and 18 checkpoint parent records, 53 ordered children and three
  permitted empty trailers. All 22 deleted mandatory-counter controls are caught.
- Experiment: local build commit `df93e66f48d196ed3a1e4fada5da1ec97e61617e`
  and public mirror `1848dc9910662d0247b3f1cebaedd939b01fc33b` share tree
  `8bd1c22aaec09b6be89939996c6bc5c5fe11a29d`. Independent writer tests pass;
  two compiled semantic mutants each fail both tests; restoration passes and
  reproduces the original test executable SHA256. The separate experiment
  output review repeats the same three inputs and adds no unique-input coverage.
- Experiment disposition: **WITHDRAW / DO_NOT_INTEGRATE**. All 112 worker records
  from eight timed series recompute correctly. Every wall comparison changes
  direction and overlaps same-binary variation. Median RSS increases in all
  four conditions. No runtime optimization is accepted and no extra timing
  series is requested.
- Private and cross-parser summaries contain no private catalog filenames.
  Private metadata access does not imply any private binary download or replay
  validation. The default C# export remains `NOT_COMPARABLE` to the Rust export.

The two missing public field identities in larger inputs and three in the small
input remain explicit in the typed receipts; these are not universal semantics
claims. Existing Python wire decoding is reused, with executions performed
freshly against this phase's retained outputs. D produced no additional CLI
exports for either retained-output audit.

## Reproducing read-only audits

Use Python 3.12+ without `-O`; assertions must remain enabled. Install the pinned
repository `requirements.txt` for PyArrow. Use a source checkout containing the
specified commits and the established `docs/evidence/phase_c/independent_qa`
wire oracle. `check_fresh_outputs.py` and `review_candidate_metadata.py` are exact
copies of the executed source. The three other Python files replace session
paths with required CLI arguments. They were AST parsed, **not executed** after
adaptation; the manifest retains the original executed-source SHA256 and exact
adaptation description. Do not assign their new hashes to the earlier runs.

```sh
python review_candidate_metadata.py --repo "$QA_REPO" --baseline ff8596b546736ea4300e013248609c7c3987e231 --candidate "$FINAL_CANDIDATE" --output metadata-new.json
python review_build_provenance.py --repo "$QA_REPO" --benchmark-dir "$QA_EVIDENCE/benchmark" --output build-new.json
python check_primary_arithmetic.py --evidence-root "$QA_EVIDENCE" --output arithmetic-new.json
python check_optimization_arithmetic.py --repo "$QA_REPO" --evidence-root "$QA_EVIDENCE" --output experiment-new.json
python check_fresh_outputs.py --source "$QA_REPO" --preflight "$QA_EVIDENCE/benchmark/preflight.json" --exports "$QA_EVIDENCE/benchmark/preflight-work" --build-receipt "$QA_EVIDENCE/benchmark/frozen-candidate/build.json" --executable "$QA_EVIDENCE/benchmark/frozen-candidate/vrfkit" --output outputs-new.json
```

The evidence root follows the original `benchmark`, `optimization` and
`optimization-measurement` directory layout. Arithmetic checks require the
per-worker measurement/request records; export checks require freshly reproduced
or locally retained Parquet/manifest/parser logs and frozen binaries. These
large or raw input/output files are deliberately absent from this public bundle.
Reproduce the benchmark using the repository benchmark handoff and each frozen
protocol before invoking those audits. JSON receipts alone do not substitute
for unavailable original bytes.

## Reproducing the semantic mutation controls

The exact executed test source and two exact mutation patches are included.
The original orchestrator is omitted because its local cache/toolchain paths
were specific to the session. Its executed SHA256 is retained in
`candidate-mutation-review.json`. The following commands are a reproduction
recipe, not a claim that these new shell commands were executed in this phase.
Use an isolated checkout at public experiment commit
`1848dc9910662d0247b3f1cebaedd939b01fc33b`, and copy
`raw_payload_contract.rs` to `crates/vrf-export/tests/`. Select an isolated target
directory through `CARGO_TARGET_DIR`. Set `CARGO_BUILD_JOBS=1`,
`CARGO_INCREMENTAL=0`, `CARGO_PROFILE_TEST_DEBUG=0` and
`CARGO_PROFILE_DEV_DEBUG=0`; the repository test optimization is level 2.

```sh
cargo +1.86.0 test -p vrf-export --all-features --locked --offline --test raw_payload_contract -- --test-threads=1
git apply "$QA_BUNDLE/null_to_empty.patch"
cargo +1.86.0 clean -p vrf-export
cargo +1.86.0 test -p vrf-export --all-features --locked --offline --test raw_payload_contract -- --test-threads=1
git apply -R "$QA_BUNDLE/null_to_empty.patch"
git apply "$QA_BUNDLE/truncate_payload.patch"
cargo +1.86.0 clean -p vrf-export
cargo +1.86.0 test -p vrf-export --all-features --locked --offline --test raw_payload_contract -- --test-threads=1
git apply -R "$QA_BUNDLE/truncate_payload.patch"
cargo +1.86.0 clean -p vrf-export
cargo +1.86.0 test -p vrf-export --all-features --locked --offline --test raw_payload_contract -- --test-threads=1
```

Expect the positive and restored runs to pass two tests. Each mutant must compile
successfully and fail both tests with the targeted content assertions, returning
test exit 101. The first mutation changes null raw cells into present empty
cells; the second truncates long payloads to 16 bytes. Remove each patch before
the next stage and verify a clean runtime diff afterward. Record source, Cargo
artifact and executable hashes for new runs; do not reuse earlier execution
counts. The original runner explicitly cleaned vrf-export before every compile,
selected a fresh test compiler artifact, ran that executable directly, and
checked the restored binary hash. This recipe may compile additional cached
dependencies and does not promise the original wall time.

## Evidence limits

No private replay, player identifier, original filename, raw replay/parser log,
Parquet payload, binary or credential is included. Local test/scan setup issues
were corrected in the harness: the first Focus scan needed a dictionary-to-
string cast, and the metadata reviewer accounted for unpublished development
packages with Cargo's default 0.0.0 version. No production correctness defect
was inferred from those harness corrections. Historical checks are not counted
as new executions.
