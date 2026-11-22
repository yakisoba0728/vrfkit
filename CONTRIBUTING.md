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
(pyarrow, numpy) -- without it most tools test modules fail to import.

**MSRV is 1.86, and the main Rust CI job pins exactly that.** A newer local
toolchain accepts syntax 1.86 rejects — `let` chains are the one that has already broken a build —
so a green `cargo test` on your machine is not evidence CI will pass. Install
the pinned toolchain once and run the sweep through it:

```bash
rustup toolchain install 1.86.0 --component clippy,rustfmt
cargo +1.86.0 clippy --workspace --all-targets --all-features --locked -- -D warnings
```

## Before you open a PR

Run the full sweep. Every one of these must exit 0:

```bash
cargo +1.86.0 fmt --check
cargo +1.86.0 clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo +1.86.0 test -p vrfkit --no-default-features --locked
cargo +1.86.0 test -p vrf-container --no-default-features --locked
RUSTDOCFLAGS="-D warnings" cargo +1.86.0 doc --workspace --all-features --no-deps --locked
VRFKIT_INTEROP_DIR="<private-root>" cargo +1.86.0 test -p vrf-export --test roundtrip write_interop_files --locked -- --exact
python -W error crates/vrf-export/tests/python_interop.py "<private-root>/interop"
python -W error tools/check_ascii.py --check
python -W error tools/apply_type_corrections.py --check
python -W error tools/extract_checksum_types.py --export tools/fixtures/checksum_export --check
python -W error tools/generate_scoped_types.py --check
python -W error tools/check_baseline_schemas.py
python -W error tools/check_docs.py   # runs cargo test --workspace and the tools suite (-b), and checks their counts
```

`check_docs.py` runs both test suites and fails on either, showing the last
4,000 characters of a failing run; rerun that suite alone
(`cargo +1.86.0 test --workspace --locked`, or
`python -W error -m unittest discover -b -s tools/tests -p "test_*.py"`) for
its full output.

The suite-size claims distinguish `(Windows)` and `(non-Windows)`: one
publication test exercises a Windows-only filesystem refusal. The full guard
still runs both suites and checks the executing platform's counts; an
unqualified claim applies everywhere. A local count is not a successful
Windows runtime check.

For the interop lines, point `VRFKIT_INTEROP_DIR` at a private root; the Rust
test writes its files to that root's `interop` child, which is passed exactly
to Python. The script refuses to search the system temp directory, because
"newest" can be a stale fixture from another checkout.

Also check every crate with no default features and with each feature alone.
The cases come from `cargo metadata`, so a new feature is covered without
editing a list; CI runs the same derivation and fails if it yields no case:

```bash
cargo +1.86.0 metadata --no-deps --format-version 1 --locked | python -c "
import json, subprocess, sys
cases = [(p['name'], f) for p in json.load(sys.stdin)['packages'] if p['features']
         for f in ['', *sorted(set(p['features']) - {'default'})]]
for crate, feature in cases:
    subprocess.run(['cargo', '+1.86.0', 'check', '-p', crate, '--no-default-features', '--locked',
                    *(['--features', feature] if feature else [])], check=True)
print(len(cases), 'cases'); sys.exit(not cases)"
```

If your change affects exported output, also run the regression guards in
[`docs/USAGE.md`](docs/USAGE.md) §6 (`check_export_baseline.py`,
`check_decode_errors_corpus.py`, `validate_corpus.py`) against a replay, and
update baselines with `--update` only after explaining each changed line. Those
need a corpus — see [Environment](#environment) below.

### What CI runs

- **`rust`** (Windows, Rust 1.86): fmt, clippy, the derived feature matrix,
  strict rustdoc, the core-only `vrfkit` and `vrf-container` tests, the
  interop test, and `check_docs.py` in full -- both suites with Python
  warnings as errors; a failed process, a missing or zero count, or a skipped
  Python test fails it. It then runs `check_corpus_baseline.py`,
  `verify_build_corpus.py` (validation, checkpoint export, reconciled
  counters, independent raw/typed comparisons, positive checkpoint decoding
  per build) and `validate_type_evidence.py --compare-typed` on the 12.10,
  12.11 and 13.00 fixtures, public test replays fetched from a pinned commit
  and SHA-256 checked.
  Every identity in `tools/fixtures/public_fixture_type_evidence.json` must
  be observed and every independently decoded value must match; the fixtures
  cover only the fields they contain. The report and logs are kept 14 days as
  an artifact, failures included; replays and Parquet are not uploaded.
- **`linux-rust`** (Ubuntu 24.04, Rust 1.86): all-feature workspace tests,
  core-only CLI tests, and the same three pinned, SHA-256-checked public
  fixtures as Windows. It runs the corpus baselines, main and checkpoint
  export audit, and the independent typed/raw evidence comparison. The
  aggregate `linux-public-replay-verification` reports are retained 14 days,
  including failures; replay files, Parquet and detailed per-replay logs are
  not uploaded by this job. The `vrfkit-source` artifact retains a tracked
  source snapshot, its commit ID and SHA-256 for 3 days; replay and Parquet
  paths are explicitly excluded from that snapshot.
- **`python-checks`** (Python 3.12 and 3.13, Windows and Ubuntu): the ASCII,
  generator and baseline-schema checks, the tools suite and
  `check_docs.py --fast`.
- **`rust-stable`** (Windows): all-feature workspace tests and core-only CLI
  tests.
- **`workflow-lint`**: checksum-pinned actionlint. Actions are pinned to
  commit IDs, permissions are read-only, superseded runs are cancelled and
  every job has a time limit.
- **`windows-package`**: see [Tagged Windows releases](#tagged-windows-releases).

`CI complete` succeeds only when every job above succeeds; a failure,
cancellation or skip cannot pass it. The 13.02, 13.04, 13.05 and 13.06
baselines have no public fixture and are still yours to run.

These checks live in the reusable `ci.yml`, so Linux runtime and replay
validation are required on main, main-targeted PRs, manual runs and tagged
release validation (`release.yml` calls `ci.yml` for a `v*` tag).

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

Pushing a tag such as `v0.6.0` runs the same `ci.yml` as the branch/PR checks
through `workflow_call`. The tag must be SemVer with a leading `v`; optional
prerelease and build suffixes are allowed. It must also name the
`[workspace.package] version` in `Cargo.toml`: without the leading `v` and
ignoring any `+build` metadata on either side, tag and version must be equal,
so a prerelease tag needs the same prerelease workspace version.
`tools/package_release.py` refuses a mismatch when creating and when verifying
the package; `python tools/package_release.py version` prints the version to
tag. Untagged CI runs package as `v<version>+ci.<commit-sha>`, so the check
runs on every PR too. The release job runs only after all
checks succeed for that exact tagged commit. Only that publishing job receives
`contents: write`; tests and builds retain read-only repository permissions.

Version numbers follow the CLI, which is what a release ships. A release bumps
the patch version unless it breaks how `vrfkit` is invoked or what it writes;
such a break bumps the minor version. New manifest keys and new summary lines
do not count as breaks. The workspace crates are not yet published to crates.io. The `vrfkit` crate
provides a public source API; consumers should pin an exact source revision.
Source API compatibility is not currently promised across revisions, so
document Rust API changes and migration steps even when CLI arguments and
output schemas are unchanged. Every crate carries the workspace version.
Define the registry package compatibility policy before the first crates.io
release.

The Windows package job is also required on every PR and manual CI run. It
builds and tests the optimized CLI for `x86_64-pc-windows-msvc` (and
`vrf-movement`, whose width assertion must hold without debug assertions),
creates a ZIP
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

To validate a release commit before tagging or merging, run the **CI** workflow
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
| `VRFKIT_REQUIRE_CORPUS` | Set to anything to turn "corpus absent, skipping" into a failure | `crates/vrf-container/tests/corpus.rs`, `check_export_baseline.py`, `check_corpus_baseline.py`, `bench_export.py` |

`compare_combat_report.py` and `compare_rpc_params.py` take `--reference` and
`--ours` (a machine-local C# export, produced as
[docs/USAGE.md](docs/USAGE.md#regression-guards----after-non-trivial-changes)
describes), and `compare_with_csharp.py` takes both directories as positional
arguments; none reads an environment variable. Nothing checks this table:
verify a row with `grep -rn "VRFKIT_" tools/*.py | grep environ`.

**`tests/corpus.rs` is a container-level smoke test, not a decode sweep.** It
parses each replay's preamble, walks every chunk, checks Event payloads against
the measured layouts and decompresses the first ReplayData chunk; it never
reaches a field. A green `cargo test` with the corpus present therefore says nothing
about decoding. The sweeps that do are `validate_corpus.py` (RepLayout framing
on every content block), `check_decode_errors_corpus.py` (the overlay), and
`verify_build_corpus.py` (the common main/checkpoint audit). These are separate
from `cargo test`. CI runs the common audit and the corpus baselines on the
three public fixtures only ([What CI runs](#what-ci-runs)); the full private
corpus audit remains a local check.

```bash
export VRFKIT_CORPUS_DIR=/path/to/replays
python tools/check_decode_errors_corpus.py ./target/release/vrfkit "$VRFKIT_CORPUS_DIR"
```

**Without `VRFKIT_CORPUS_DIR` the guards skip and exit 0**, printing `SKIP:
replay not present`. That is deliberate — the corpus lives outside the repo, so
a contributor without one is not blocked — but it means an unset variable reads
as a pass at a glance. If you meant to run them, read the output and check it
says how many replays it walked.

`check_export_baseline.py` rejects an obsolete movement-counter baseline (a
tail/trailer key from before the [native component migration](docs/NATIVE_MOVEMENT_COMPONENTS.md#counter-and-source-api-migration),
main or checkpoint) before that missing-input skip, and
`check_baseline_schemas.py` refuses one in the tree. The committed
export/checkpoint baselines were remeasured with the 0.6.0 decoder; an obsolete
baseline is replaced the same way, by a genuine export with `--update`, never
by converting its old numbers into the new counters. The old summary/manifest
fields were removed in 0.6.0, a minor version bump.

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
| `crates/vrf-decode/src/checksum_table.rs` | `tools/extract_checksum_types.py` against one or more fresh exports |
| `crates/vrf-decode/src/scoped_types.rs` | `tools/generate_scoped_types.py` from reviewed exact group/name/checksum evidence |
| `crates/vrf-transform/tests/data/native_vectors.rs` | `tools/capture_native_transforms.py` against pinned original executable readers |

The overlay table `crates/vrf-decode/src/table.rs` and
`tools/equippable_table.py` are not generated. A type change to the table goes
into `tools/apply_type_corrections.py` with its evidence; run it
(`python tools/apply_type_corrections.py`), rebuild, re-export, and only then
run `extract_checksum_types.py` against that **fresh** export: it learns from
what the table declares, so an earlier run leaves the new entries' siblings
raw. `generate_scoped_types.py` also owns
`tools/fixtures/scoped_type_evidence.json`'s layout: write mode rewrites the
fixture into it, and `--check` fails on any other layout.

## Type corrections are conservative

`tools/apply_type_corrections.py` pins a type only on complete wire evidence
-- one width on every row plus a value distribution a wrong type cannot
produce (e.g. `Money` = 800 at pistol-round start across all actors) -- or on
a checksum or sibling that fixes the type. Do not widen the `ADDITIONS` list
"by eye"; read the bar stated above it in the script first.

## Commit style

Conventional commits (`feat:`, `fix:`, `docs:`, `test:`, `perf:`, `refactor:`,
`chore:`), lowercase, present tense. Keep the subject short; put the "why" in
the body.
