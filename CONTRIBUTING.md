# Contributing to vrfkit

Thanks for your interest. vrfkit is a reverse-engineered parser for a format
that changes every game build, so it has a few unusual rules. Please read these
before opening a PR.

## Maintenance and response times

I am a student maintaining vrfkit in my spare time. Contributions and bug
reports are welcome, but replies and PR reviews may take some time while I
balance the project with my studies. There is no guaranteed response time.
Thank you for your patience and for helping improve the project.

## Build

```bash
cargo +1.86.0 build --release -p vrfkit --locked                       # inspect / validate / export
cargo +1.86.0 build --release -p vrfkit --no-default-features --locked # inspect / validate only
```

Edition 2024. `#![forbid(unsafe_code)]` is in every crate — do not add `unsafe`.

Python tooling under `tools/` needs `pip install -r requirements.txt`
(pyarrow, numpy) -- without it several checks below fail to import instead of
running.

**MSRV is 1.86, and the main Rust CI job pins exactly that.** A newer local
toolchain accepts syntax 1.86 rejects — `let` chains are the one that has already broken a build —
so a green `cargo test` on your machine is not evidence CI will pass. Install
the pinned toolchain once and run the sweep through it:

```bash
rustup toolchain install 1.86.0 --component clippy,rustfmt
cargo +1.86.0 clippy --workspace --all-targets --all-features --locked -- -D warnings
```

## Before you open a PR

Run the full sweep. Every one of these must be green:

```bash
cargo +1.86.0 fmt --check
cargo +1.86.0 clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo +1.86.0 test --workspace --locked
cargo +1.86.0 check --workspace --all-targets --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo +1.86.0 doc --workspace --all-features --no-deps --locked
cargo +1.86.0 check --manifest-path tools/probe_offset/Cargo.toml --locked
VRFKIT_INTEROP_DIR="<private-root>" cargo +1.86.0 test -p vrf-export --test roundtrip write_interop_files --locked -- --exact
python -W error crates/vrf-export/tests/python_interop.py "<private-root>/interop"
python -W error tools/check_ascii.py --check
python -W error tools/apply_type_corrections.py --check
python -W error tools/check_effect_decoder.py --check
python -W error tools/extract_checksum_types.py --export tools/fixtures/checksum_export --check
python -W error tools/generate_scoped_types.py --check
python -W error tools/extract_equippables.py --check
python -W error tools/extract_descriptors.py third_party/vrp/Replay.Valorant crates/vrf-decode/src/table.rs
python -W error tools/apply_type_corrections.py
cargo +1.86.0 fmt -p vrf-decode
git diff --exit-code -- crates/vrf-decode/src/table.rs   # regenerated table == committed table
python -W error tools/check_baseline_schemas.py
python tools/check_docs.py            # not --fast: that skips the count check
python -W error -m unittest discover -s tools/tests -p "test_*.py"
```

For the interop lines, point `VRFKIT_INTEROP_DIR` at a private root; the Rust
test writes its files to that root's `interop` child, which is passed exactly
to Python. The script refuses to search the system temp directory, because
“newest” can be a stale fixture from another checkout.

Run every advertised core-only and singleton feature, not just the default
workspace. These are the commands CI executes; each singleton intentionally
starts from `--no-default-features`:

```bash
cargo +1.86.0 check -p vrfkit --no-default-features --locked
cargo +1.86.0 check -p vrfkit --no-default-features --features export --locked
cargo +1.86.0 check -p vrf-bitio --no-default-features --locked
cargo +1.86.0 check -p vrf-bitio --no-default-features --features alloc --locked
cargo +1.86.0 check -p vrf-container --no-default-features --locked
cargo +1.86.0 check -p vrf-container --no-default-features --features oodle --locked
cargo +1.86.0 check -p vrf-container --no-default-features --features event --locked
cargo +1.86.0 check -p vrf-container --no-default-features --features checkpoint --locked
cargo +1.86.0 check -p vrf-decode --no-default-features --locked
cargo +1.86.0 check -p vrf-decode --no-default-features --features array --locked
cargo +1.86.0 check -p vrf-decode --no-default-features --features effect --locked
cargo +1.86.0 check -p vrf-decode --no-default-features --features overlay --locked
cargo +1.86.0 check -p vrf-decode --no-default-features --features structs --locked
cargo +1.86.0 check -p vrf-export --no-default-features --locked
cargo +1.86.0 check -p vrf-export --no-default-features --features parquet --locked
cargo +1.86.0 check -p vrf-export --no-default-features --features fields --locked
cargo +1.86.0 check -p vrf-export --no-default-features --features movement --locked
cargo +1.86.0 check -p vrf-export --no-default-features --features actors --locked
cargo +1.86.0 check -p vrf-export --no-default-features --features net-guids --locked
cargo +1.86.0 check -p vrf-export --no-default-features --features events --locked
cargo +1.86.0 check -p vrf-export --no-default-features --features partials --locked
cargo +1.86.0 check -p vrf-export --no-default-features --features checkpoint-context --locked
cargo +1.86.0 check -p vrf-export --no-default-features --features snappy --locked
cargo +1.86.0 check -p vrf-net --no-default-features --locked
cargo +1.86.0 check -p vrf-net --no-default-features --features diagnostics --locked
cargo +1.86.0 check -p vrf-schema --no-default-features --locked
cargo +1.86.0 check -p vrf-schema --no-default-features --features checkpoint --locked
```

`check_docs.py` without `--fast` runs the suites and compares the documented
counts with the measured results. The Windows MSRV job runs this full check,
including Python with warnings treated as errors. Failed processes, missing
or zero test counts, and skipped Python tests fail the measurement.

The Python checks also run on Windows and Ubuntu with Python 3.12 and 3.13;
those jobs use `check_docs.py --fast` for source/document consistency. A
separate Windows Rust stable job runs all-feature workspace tests and
core-only CLI tests. The pinned MSRV feature matrix above remains required.

CI validates the workflow with checksum-pinned actionlint, pins Actions to
commit IDs, grants read-only repository permissions, cancels superseded runs,
and limits job durations. The final `CI complete` job succeeds only when every
required job succeeds, including Windows release packaging; failures,
cancellations and skipped jobs cannot pass it.

If your change affects exported output, also run the regression guards in
[`docs/USAGE.md`](docs/USAGE.md) §6 (`check_export_baseline.py`,
`check_decode_errors_corpus.py`, `validate_corpus.py`) against a replay, and
update baselines with `--update` only after explaining each changed line. Those
need a corpus — see [Environment](#environment) below.

### Replay evidence for parser changes

For changes to parsing, payload transforms, decoding, or exported output,
including support for a new game build, include actual replay validation
results in the PR. Passing CI alone does not establish that real replays
decode correctly. Before merging, the contributor or maintainer should run
the affected validation and export paths on real replays and record:

- The tested game build/branch and replay count for each build.
- The code commit and exact commands used, including whether checkpoint
  decoding was exercised when the change affects it.
- Successful and failed replay counts, relevant transform/framing/decode
  failure counts, and oracle pass rates where available.
- A before/after comparison on the same replays, including a previously
  supported build when shared parser code changes, with any output differences
  or remaining failures explained.

State the scope of the check: a few samples or a full available corpus. If no
replay is available, you can still open a PR; say that replay validation was
not run and what input is needed so a maintainer can complete it before
merging. A skipped corpus check is not replay validation. You do not need to
upload replay files publicly to contribute; recorded commands and results
can document a local run.

## Tagged Windows releases

Pushing a tag such as `v0.1.0` runs the same `ci.yml` as the branch/PR checks
through `workflow_call`. The tag must be SemVer with a leading `v`; optional
prerelease and build suffixes are allowed. The release job runs only after all
checks succeed for that exact tagged commit. Only that publishing job receives
`contents: write`; tests and builds retain read-only repository permissions.

The Windows package job is also required on every PR and manual CI run. It
builds and tests the optimized CLI for `x86_64-pc-windows-msvc`, creates a ZIP
with `tools/package_release.py`, runs the extracted executable, and audits the
pinned public 12.10 replay with the packaged binary, including checkpoints and
independent typed/raw comparisons. The ZIP includes `vrfkit.exe`, `LICENSE`,
`NOTICE.md` and `build-info.json` (tag, source commit, target and binary hash).
It and its SHA-256 sidecar are retained for 14 days as `windows-release`.
The corresponding replay report and diagnostic logs use a separate artifact.

On tag pushes, the publishing job downloads those verified assets, checks the
ZIP checksum and embedded provenance again, and publishes them without a
second build. Prerelease tags produce GitHub prereleases. Release notes are
generated automatically. Runs for the same tag are serialized; an existing
release is not overwritten. Packaging rejects existing output directories.

To validate a candidate before tagging or merging, run the **CI** workflow
manually on its branch and inspect the `windows-release` and
`release-replay-verification` artifacts. Branch, PR and manual CI runs do not
publish releases. No release is made merely by merging the workflow.

## Environment

The corpus guards read their inputs from environment variables rather than
hardcoded paths, so nothing in the tree points at one person's disk. None of
them are needed for the sweep above; all of them are needed for §6.

| Variable | What it points at | Read by |
|---|---|---|
| `VRFKIT_CORPUS_DIR` | Directory of `.vrf` replays; a bare filename in a baseline resolves against it | `check_export_baseline.py`, `check_corpus_baseline.py`, `check_metrics_baseline.py` |
| `VRFKIT_VALPLAY_DIR` | valplay checkout root | `check_metrics_baseline.py`, `validate_metrics_corpus.py` |
| `VRFKIT_JOBS` | Worker count for the corpus sweeps; default is cores - 2, capped at 16 | `validate_corpus.py` |
| `VRFKIT_REQUIRE_CORPUS` | Set to anything to turn "corpus absent, skipping" into a failure | `crates/vrf-container/tests/corpus.rs`, `check_export_baseline.py`, `check_corpus_baseline.py` |

`VRFKIT_CSHARP_DIR` is gone. `analyze_coverage.py` and
`extract_equippables.py` read the C# descriptors vendored under
[`third_party/vrp/`](third_party/vrp/README.md) by default; nobody had set the
variable, so both ran without their input (`extract_equippables.py` stopped at
"resolver not found"). Pass `--csharp-dir` / `--csharp-root` to use another
checkout.

The `compare_*.py` scripts were listed against `VRFKIT_CSHARP_DIR` here, which
none of them read. Two of them (`compare_combat_report.py`,
`compare_rpc_params.py`) then read `VRFKIT_VALPLAY_DIR` for a C# bundle under
valplay's `pipeline/exports` that no longer exists; they now take `--reference`
and `--ours` and default to a machine-local C# export, produced as described in
[docs/USAGE.md](docs/USAGE.md#regression-guards----after-non-trivial-changes).
The third, `compare_with_csharp.py`, reads **no environment variable at all**
-- it takes the C# bundle directory and the vrfkit output directory as its two
positional arguments. Nothing checks this table, so verify a row by grepping
for the variable rather than by reading the name:

```bash
grep -rn "VRFKIT_" tools/*.py | grep environ
```

**`tests/corpus.rs` is a container-level smoke test, not a decode sweep.** It
parses each replay's header and decompresses its Oodle chunks; it never reaches
a field. A green `cargo test` with the corpus present therefore says nothing
about decoding. The sweeps that do are `validate_corpus.py` (RepLayout framing
on every content block), `check_decode_errors_corpus.py` (the overlay), and
`verify_build_corpus.py` (the common main/checkpoint audit). These are separate
from `cargo test`. CI runs the common audit on three public replays; the full
private corpus audit remains a local check.

The pinned baseline guard also runs in CI: `check_corpus_baseline.py`, on the
12.10, 12.11 and 13.00 fixtures only. Those three are byte-identical to the
upstream parser's public test replays, so the Windows job fetches them from a
pinned commit and checks their SHA-256 first. The 13.02, 13.04, 13.05 and 13.06
baselines have no public fixture and are still yours to run.

The same job runs `verify_build_corpus.py` on all three fixtures, requiring
validation, checkpoint-enabled exports, reconciled counters, independent
raw/typed value comparisons, and positive checkpoint decoding per build.
It then runs `validate_type_evidence.py --compare-typed` against
`tools/fixtures/public_fixture_type_evidence.json` across the combined exports:
every listed field identity must be observed and every independently decoded
value must match. These public fixtures cover only the fields they contain;
they do not replace the full available corpus audit.

The public-fixture report, per-replay results and diagnostic logs are retained
as a GitHub Actions artifact for 14 days, including on failure. Replay files
and Parquet exports are not uploaded by the workflow.

```bash
export VRFKIT_CORPUS_DIR=/path/to/replays
python tools/check_decode_errors_corpus.py ./target/release/vrfkit "$VRFKIT_CORPUS_DIR"
```

**Without `VRFKIT_CORPUS_DIR` the guards skip and exit 0**, printing `SKIP:
replay not present`. That is deliberate — the corpus lives outside the repo, so
a contributor without one is not blocked — but it means an unset variable reads
as a pass at a glance. If you meant to run them, read the output and check it
says how many replays it walked.

## The load-bearing invariants (do not break)

These corrupt downstream consumers silently — no test fails when they break.

- **No skip path.** Every walkable field emits `raw_bits` even when its type is
  unknown or decoding fails. Typed `value_*` columns are an *additive* overlay;
  a decode failure leaves them null with the raw bits intact.
- **No silent success.** A block whose group cannot be resolved fails loudly
  (counted in `validate`'s `RPC payload lost` line), never guessed.
- **Byte-identical output.** Exported Parquet is reproducible run to run. If you
  change row buffering, batch sizes, or iteration order, verify the output is
  byte-identical (the baselines pin this).
- **ASCII only** in Rust code and comments — the Windows cp949 console truncates
  output at the first non-ASCII byte in a format string.
- **No hardcoded names** in the parser. Display names live in the Python adapter
  (`tools/equippable_table.py`), never in a Rust crate.

## Generated files — never hand-edit

| File | Generator |
|---|---|
| `crates/vrf-decode/src/table.rs` | `tools/extract_descriptors.py` on `third_party/vrp/Replay.Valorant`, then `tools/apply_type_corrections.py` |
| `crates/vrf-decode/src/checksum_table.rs` | `tools/extract_checksum_types.py` against one or more fresh exports |
| `crates/vrf-decode/src/scoped_types.rs` | `tools/generate_scoped_types.py` from reviewed exact group/name/checksum evidence |
| `crates/vrf-transform/src/sbox.rs` | `tools/extract_sboxes.py` |
| `crates/vrf-transform/tests/data/golden_vectors.rs` | `tools/extract_golden.py` |
| `crates/vrf-transform/tests/data/native_vectors.rs` | `tools/capture_native_transforms.py` against pinned original executable readers |
| `tools/equippable_table.py` | `tools/extract_equippables.py` from the vendored `third_party/vrp/Replay.Valorant/Combat/ValorantEquippableResolver.cs` |

Ordering for the overlay table is load-bearing:
`extract_descriptors.py` → `apply_type_corrections.py` → `cargo fmt` →
`extract_checksum_types.py` (against a **fresh** export).

The C# descriptor input is vendored under
[`third_party/vrp/`](third_party/vrp/README.md),
copied verbatim from the commit that README names. A descriptor change is an
edit there, committed together with the regenerated `table.rs`. CI runs the
first three steps against that directory and fails if `table.rs` changes:

```bash
python tools/extract_descriptors.py third_party/vrp/Replay.Valorant \
    crates/vrf-decode/src/table.rs
python tools/apply_type_corrections.py
cargo +1.86.0 fmt -p vrf-decode
git diff --exit-code -- crates/vrf-decode/src/table.rs
```

The checksum step is last because it learns from what the overlay table
declares. Run it before the additions land and the new entries are not donors
yet -- the symptom is a field typed on the group you declared and still raw on
its siblings, which is easy to read as the propagation not working. Re-export
after rebuilding, then regenerate.

## Type corrections are conservative

`tools/apply_type_corrections.py` carries two kinds of entry:

- **Corrections** — the C# descriptor declares a type and the wire disagrees.
  Each has cited wire evidence.
- **ADDITIONS** — the C# descriptor is silent. These rest on unusually complete
  wire evidence (e.g. `Money` = 800 at pistol-round start across all actors). Do
  not widen the ADDITIONS list "by eye" — that undoes the reason it is allowed.
  Read the rationale block at the top of the script first.

## Commit style

Conventional commits (`feat:`, `fix:`, `docs:`, `test:`, `perf:`, `refactor:`,
`chore:`), lowercase, present tense. Keep the subject short; put the "why" in
the body.
