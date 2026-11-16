# Phase C benchmark handoff

The filename is retained for existing links. This revision supersedes the
Phase B candidate/baseline instructions with the Phase C output contract.

**Performance comparisons are now user-run.** At the user's request, the
agents stopped quantitative and repeated replay benchmarking. This runbook
prepares a reproducible comparison for the user; its commands are not a
record of measurements already executed. Functional tests, replay output
checks and CI remain separate acceptance work. No Phase C speed, CPU
or peak-RSS improvement is claimed here. Earlier profiling and rejected
experiments keep their original scope and history.

The existing [resource harness](RESOURCE_BENCHMARKS.md),
`tools/benchmark_resources.py`, already provides alternating pairs, resource
measurements and strict output gates. Use it for every series. The small
[build-freezing helper](examples/freeze_benchmark_binary.py) below only checks
Cargo artifacts and copies a binary; it never executes a replay or measures
performance.

## 1. Choose the comparison before running it

| Input | Required choice |
| --- | --- |
| Baseline | Immutable C-only PR #72 source `1b9bbf222f003e35c882e640c1b19e0bbcc88472`: Phase B integration plus the exact FocusProjectiles route/counter/schema change. This is a proposed output-compatible baseline; every selected input must still pass all harness gates. |
| Candidate | The **full final integration commit ID after functional acceptance**, supplied in the final Phase C receipt on PR #58 and linked from [Phase C results](PHASE_C_RESULTS.md). A branch name, an intermediate agent commit or a local QA snapshot is not a substitute. |
| Compiler and target | Rust/Cargo 1.86.0, the same target triple, host, linker and native compiler for both builds |
| Features and profile | Default `export` features, `--release`, `--locked`, repository release profile: optimization 3, fat LTO, one codegen unit, debug level 1 |
| Dependency selection | Identical `Cargo.lock` bytes; stop and explain a dependency change before comparing |
| Replay and mode | The same unchanged input bytes for both executables; decide main-only or `--checkpoints` before the series. Checkpoint mode requires actual checkpoint decoding work. |
| Build flags | Identical Rust/native compiler flags and Cargo configuration; record any deliberate override |
| Measurements | Full export wall time, CPU, utilization, peak RSS, input throughput and output size; do not reduce output or decoding to improve a number |
| Controls | An identical-executable comparison on the same replay and mode, with the same repetitions and machine conditions |

### Why the historical baseline changed

Phase C adds `route_children_focus_projectiles` and
`focus_projectiles_empty_trailers` as required main/checkpoint sink keys. The
evidence-qualified 12.05 fixture also gains 53 main child rows while retaining
the original raw parents. Unmodified Phase A and Phase B binaries cannot pass
the current harness: missing required keys fail even on inputs with zero
FocusProjectiles rows. Ignoring absent keys, removing child rows or weakening
manifest equality is not a valid correction.

The [fixed C-only commit](https://github.com/yakisoba0728/vrfkit/commit/1b9bbf222f003e35c882e640c1b19e0bbcc88472)
is exactly one commit ahead of Phase B integration
`588be1effc8dfca2bed8810fd51a006828081c1b`. Its
[source comparison](https://github.com/yakisoba0728/vrfkit/compare/588be1effc8dfca2bed8810fd51a006828081c1b...1b9bbf222f003e35c882e640c1b19e0bbcc88472)
contains the same semantic/schema extension on top
of Phase B, before the Reader, identity and bounded-input integrations. Use
that exact immutable commit even if PR #72's branch later moves to resolve
integration dependencies. Comparing it with the final candidate can measure
the aggregate runtime difference **after applying the same decoder/output
contract to both sides**. It cannot measure the whole Phase A/B-to-Phase C
change, the cost of the FocusProjectiles feature itself, or an individual
change's contribution. A strict mismatch stops that input's comparison; an
expected historical difference is not permission to normalize it away.

First run the final candidate against the same frozen candidate executable
as a control. If a future comparison needs another baseline, derive and
review a separately pinned source with the same schema and output semantics;
retain the identical gates and explain which changes that comparison can
attribute. No compatibility baseline or performance result is synthesized by
this runbook.

### Pinning the accepted candidate

The final acceptance comment on [PR #58](https://github.com/yakisoba0728/vrfkit/pull/58)
must supply the full final integration SHA and CI checkout identity. A commit
cannot embed its own SHA. If [the in-tree results](PHASE_C_RESULTS.md) pin the
last runtime candidate, and the acceptance commit changes documentation only,
the receipt must name both and verify unchanged runtime, harness, lockfile
and build configuration. Copy the **final accepted integration SHA** into the
command below; do not benchmark the earlier runtime pin merely because it is
already printed in a document. This checkpoint does not yet claim final
integration acceptance.

Select representative inputs and the repetition budget before inspecting
results. The commands below show one replay, one warmup pair and six measured
pairs; six gives three pairs in each first-run order. They are an example
budget for the user to choose, not an agent instruction to execute it. Repeat
the chosen procedure separately for additional inputs or modes and keep every
result, including failed or noisy series.

## 2. Isolate and record both builds

The commands use Bash and Python 3.11+. Linux reports all of the harness's
resource metrics. macOS has documented RSS unit handling; native Windows
can run the harness but reports CPU, RSS and block I/O as `null`. Use the
platform actually intended for the comparison and keep its results separate.

Run from a repository checkout containing this handoff. Edit the three
explicit values below. The candidate must be available locally after fetching
the accepted integration branch. The benchmark root must be a new directory
outside the repository and outside the replay directory.

```bash
set -euo pipefail

VRFKIT_BENCH_REPO=$(git rev-parse --show-toplevel)
VRFKIT_BENCH_BASE=1b9bbf222f003e35c882e640c1b19e0bbcc88472
VRFKIT_BENCH_FINAL=PASTE_FULL_ACCEPTED_INTEGRATION_COMMIT
VRFKIT_BENCH_ROOT=/absolute/path/to/new-private-benchmark
VRFKIT_BENCH_REPLAY=/absolute/path/to/unchanged-match.vrf

[[ "$VRFKIT_BENCH_FINAL" =~ ^[0-9a-f]{40}$ ]]
git -C "$VRFKIT_BENCH_REPO" cat-file -e "$VRFKIT_BENCH_BASE^{commit}"
git -C "$VRFKIT_BENCH_REPO" cat-file -e "$VRFKIT_BENCH_FINAL^{commit}"
test -f "$VRFKIT_BENCH_REPLAY"
test ! -e "$VRFKIT_BENCH_ROOT"
test ! -L "$VRFKIT_BENCH_ROOT"
mkdir -m 700 "$VRFKIT_BENCH_ROOT"

git -C "$VRFKIT_BENCH_REPO" worktree add --detach "$VRFKIT_BENCH_ROOT/source-baseline" "$VRFKIT_BENCH_BASE"
git -C "$VRFKIT_BENCH_REPO" worktree add --detach "$VRFKIT_BENCH_ROOT/source-candidate" "$VRFKIT_BENCH_FINAL"
cmp "$VRFKIT_BENCH_ROOT/source-baseline/Cargo.lock" "$VRFKIT_BENCH_ROOT/source-candidate/Cargo.lock"

rustup toolchain install 1.86.0 --profile minimal
rustc +1.86.0 -vV > "$VRFKIT_BENCH_ROOT/rustc.txt"
cargo +1.86.0 --version > "$VRFKIT_BENCH_ROOT/cargo.txt"
VRFKIT_BENCH_TARGET=$(sed -n 's/^host: //p' "$VRFKIT_BENCH_ROOT/rustc.txt")
test -n "$VRFKIT_BENCH_TARGET"
```

**Separate worktrees alone are insufficient.** During Phase B, a shared
`CARGO_TARGET_DIR` retained another worktree's dependencies and an older
instrumented CLI while Cargo reported it as fresh. Those initial records were
invalidated; see [CLI build identity](CLI_INPUT_STREAMING.md#build-identity-and-bounded-input-proof).
Use a distinct, initially absent target directory for each build, as below.
Neither `cargo test` succeeding nor a file already present at
`target/release/vrfkit` proves that the intended source produced that binary.

If storage forces target reuse, clean **all workspace packages** from that
release target before switching source, not only the `vrfkit` binary crate.
Retain the cleaning command, new compiler artifacts and source fingerprints.
A new isolated target is easier to audit. Never clean another worker's active
target directory.

Run both builds sequentially. Keep the worktrees unchanged from build start
through freezing. Use the same Cargo configuration, linker and native build
environment; review machine-level Cargo configuration before starting. The
following environment selects no extra Rust flags or compiler wrappers. It
also removes environment profile/target Rust-flag overrides. It does not
change Cargo configuration files or native compiler flags.

```bash
export RUSTFLAGS="" CARGO_ENCODED_RUSTFLAGS=""
unset RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER RUSTC
while IFS= read -r VRFKIT_BENCH_ENV_NAME; do
  case "$VRFKIT_BENCH_ENV_NAME" in
    CARGO_PROFILE_*|CARGO_TARGET_*_RUSTFLAGS) unset "$VRFKIT_BENCH_ENV_NAME" ;;
  esac
done < <(compgen -e)
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0

vrfkit_build_and_freeze() {
  local label="$1" revision="$2"
  local source="$VRFKIT_BENCH_ROOT/source-$label"
  local target="$VRFKIT_BENCH_ROOT/target-$label"
  test ! -e "$target"
  test ! -L "$target"
  test -z "$(git -C "$source" status --porcelain --untracked-files=all)"
  local command=(cargo +1.86.0 build --release -p vrfkit --bin vrfkit --locked
    --target "$VRFKIT_BENCH_TARGET" --target-dir "$target"
    --message-format=json-render-diagnostics)
  printf '%q ' "${command[@]}" > "$VRFKIT_BENCH_ROOT/$label.command.txt"
  printf '\n' >> "$VRFKIT_BENCH_ROOT/$label.command.txt"
  (
    cd "$source"
    "${command[@]}" > "$VRFKIT_BENCH_ROOT/$label.cargo.jsonl" \
      2> "$VRFKIT_BENCH_ROOT/$label.build.log"
  )
  python "$VRFKIT_BENCH_REPO/docs/examples/freeze_benchmark_binary.py" \
    --source "$source" --revision "$revision" --target-dir "$target" \
    --cargo-messages "$VRFKIT_BENCH_ROOT/$label.cargo.jsonl" \
    --rustc-info "$VRFKIT_BENCH_ROOT/rustc.txt" \
    --cargo-info "$VRFKIT_BENCH_ROOT/cargo.txt" \
    --build-command "$VRFKIT_BENCH_ROOT/$label.command.txt" \
    --output-dir "$VRFKIT_BENCH_ROOT/frozen-$label"
}

vrfkit_build_and_freeze baseline "$VRFKIT_BENCH_BASE"
vrfkit_build_and_freeze candidate "$VRFKIT_BENCH_FINAL"
```

`set -e` stops the sequence if compilation fails. The helper also requires a
successful `build-finished` record and freshly compiled non-test artifacts
from every current runtime crate in the selected checkout. It selects the
`vrfkit` executable from its `compiler-artifact` record, checks the release
profile/default export features and expected target directory, and freezes
that exact file. It records the source commit/tree, tracked runtime-file
hashes, lockfile hash, compiler identity, selected build environment, artifact
metadata and executable SHA-256 in each `frozen-*/build.json`.

Keep the complete command/log/receipt set. An output directory without
`build.json` is not an accepted frozen build. The receipt documents this local
build procedure; it is not an independent binary reproducibility attestation.
Review configuration-file overrides and preserve their hashes alongside the
receipts when using them. These build records can include local paths and
should stay private unless reviewed for sharing.

Compare both receipts before timing: toolchain, target, feature set, release
profile, flags and `source.files["Cargo.lock"]` must agree. Commit, source tree,
runtime-file hashes and executable hashes identify the intended source
differences; they are not measurements of performance.

## 3. Run the selected series yourself

Use the **candidate checkout's harness for both labels**, so the output gates
and counter schema are identical. Do not modify either frozen executable,
input, harness or helper modules during the series. Stop concurrent builds,
tests, exports and other known heavy work; record the host/OS/CPU, power mode,
storage location and any unavoidable activity in a private experiment note.
There is no exclusive-host or cold-cache guarantee.

The following runs the final candidate against itself as the control, then
the C-only baseline against the final candidate, both with checkpoints.
Each work directory and JSON report must be new:

```bash
VRFKIT_BENCH_HARNESS="$VRFKIT_BENCH_ROOT/source-candidate/tools/benchmark_resources.py"
VRFKIT_BENCH_BASE_EXE="$VRFKIT_BENCH_ROOT/frozen-baseline/vrfkit"
VRFKIT_BENCH_FINAL_EXE="$VRFKIT_BENCH_ROOT/frozen-candidate/vrfkit"
VRFKIT_BENCH_WARMUPS=1
VRFKIT_BENCH_REPEATS=6

python "$VRFKIT_BENCH_HARNESS" \
  --baseline-exe "$VRFKIT_BENCH_FINAL_EXE" --candidate-exe "$VRFKIT_BENCH_FINAL_EXE" \
  --replay "$VRFKIT_BENCH_REPLAY" \
  --work-dir "$VRFKIT_BENCH_ROOT/control-checkpoints-work" \
  --output "$VRFKIT_BENCH_ROOT/control-checkpoints.json" \
  --warmups "$VRFKIT_BENCH_WARMUPS" --repeats "$VRFKIT_BENCH_REPEATS" --checkpoints

python "$VRFKIT_BENCH_HARNESS" \
  --baseline-exe "$VRFKIT_BENCH_BASE_EXE" --candidate-exe "$VRFKIT_BENCH_FINAL_EXE" \
  --replay "$VRFKIT_BENCH_REPLAY" \
  --work-dir "$VRFKIT_BENCH_ROOT/comparison-checkpoints-work" \
  --output "$VRFKIT_BENCH_ROOT/comparison-checkpoints.json" \
  --warmups "$VRFKIT_BENCH_WARMUPS" --repeats "$VRFKIT_BENCH_REPEATS" --checkpoints
```

For main-only output, use another new control/comparison pair of paths and
omit `--checkpoints` in **both** commands. On Windows, use the `.exe` names
recorded by the freezing helper. For another replay, use another new set of
paths. A control after the comparison, or a preplanned comparison with swapped
labels, can expose drift; report its label direction explicitly. Do not rerun
only until a favorable result appears or discard inconvenient samples.

The harness alternates baseline/candidate and candidate/baseline inside each
phase. Warmups also pass the output gates but are excluded from statistics.
The input is hashed before timing, and outputs are hashed between samples,
so this is a warmed-cache export comparison. Keep build, hash and report time
separate from the measured export interval. `--keep-exports` retains every
private export for investigation and requires substantially more disk space;
normal runs remove each export after checking it.

## 4. Accept evidence before interpreting a ratio

A usable report has exit status 0, `status: "completed"`, `equivalent: true`,
an empty failure list and identical before/after provenance. The provenance
hashes for baseline and candidate must match their frozen build receipts.
The same-binary control must use the same executable hash in both roles.

Every warmup and measured export must have the expected tables, required
quality counters, positive decode work, byte-identical Parquet, matching CLI
counters and matching normalized manifests. Only `source_file` and
`elapsed_ms` are excluded from manifest equality. A nonzero exit, changed
input, missing counter or output mismatch invalidates the series and
suppresses ratios. A running/partial or invalid report is not a performance
result. Never relax those gates or drop columns/rows to manufacture a match.

Read wall time, total CPU, utilization and RSS separately. Utilization counts
100% as one logical CPU, so a threaded export can exceed 100%. Throughput is
compressed input MiB per wall second; output size reports the actual Parquet
and manifest bytes. Block-I/O counters are neither byte throughput nor I/O
time. Allocation counts, allocated bytes, isolated I/O time and independent
game semantics remain unmeasured by this harness.

The paired ratio is **candidate / baseline**, computed inside each measured
pair before taking the median. Keep its per-pair values, dispersion and
same-binary control together. A smaller marginal median alone is insufficient
when the effect overlaps control noise, reverses across inputs or trades CPU
against wall time or memory. The harness does not calculate confidence
intervals or establish a universal speed guarantee.

This compares two vrfkit revisions doing the same work. The reference parser's
default enriched NDJSON output has a different contract; these reports cannot
support a competitor-performance claim. See the
[pinned comparison](REFERENCE_COMPARISON.md#integration-contracts-at-the-pinned-revision).
This C-only-to-final comparison also cannot isolate the contribution of any
one Phase C change, and it excludes the semantic cost of adding the exact
FocusProjectiles output contract to Phase B.

## 5. Keep functional acceptance separate

Rust/MSRV, feature, Python, documentation, public replay and platform CI checks
remain required by [CONTRIBUTING.md](../CONTRIBUTING.md#before-you-open-a-pr).
The harness's equality gate establishes equality to a baseline on selected
inputs and modes; it does not replace independent typed/raw checks, gameplay
validation or mutation controls. CI runtime also is not a controlled benchmark.

Before handing results back, retain the full commit IDs, build receipts,
unchanged executable/input/harness hashes, chosen replay/mode matrix,
configuration, same-binary controls and all series reports. Work directories
contain private parser logs and paths; report JSON is designed to omit those
values, but content hashes can still identify known inputs. Share only the
reviewed evidence permitted for the selected replay.

No command in this runbook publishes a release, changes `main`, uploads a
corpus or updates the committed timing baseline.
