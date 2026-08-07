# Contributing to vrfkit

Thanks for your interest. vrfkit is a reverse-engineered parser for a format
that changes every game build, so it has a few unusual rules. Please read these
before opening a PR.

## Build

```bash
cargo build --release -p vrfkit                          # inspect / validate / export
cargo build --release -p vrfkit --no-default-features    # inspect / validate only
```

Edition 2024. `#![forbid(unsafe_code)]` is in every crate — do not add `unsafe`.

Python tooling under `tools/` needs `pip install -r requirements.txt`
(pyarrow, numpy) -- without it several checks below fail to import instead of
running.

**MSRV is 1.86, and CI pins exactly that.** A newer local toolchain accepts
syntax 1.86 rejects — `let` chains are the one that has already broken a build —
so a green `cargo test` on your machine is not evidence CI will pass. Install
the pinned toolchain once and run the sweep through it:

```bash
rustup toolchain install 1.86.0 --component clippy,rustfmt
cargo +1.86.0 clippy --all-targets -- -D warnings
```

## Before you open a PR

Run the full sweep. Every one of these must be green:

```bash
cargo +1.86.0 fmt --check
cargo +1.86.0 clippy --all-targets -- -D warnings
cargo +1.86.0 test --workspace
RUSTFLAGS=-D warnings cargo +1.86.0 build -p vrfkit --no-default-features
python tools/check_ascii.py --check
python tools/apply_type_corrections.py --check
python tools/check_effect_decoder.py --check
python tools/check_docs.py            # not --fast: that skips the count check
python -m unittest discover -s tools/tests -p "test_*.py"
```

`check_docs.py` without `--fast` runs the suites so it can compare the numbers
the docs quote against the real ones. CI runs `--fast` and cannot do otherwise —
the full mode shells out to `cargo test`, and the Python job is Ubuntu-only
because the Rust job needs Windows for the Oodle FFI. **That check is yours to
run, not CI's**, and it is the only thing that catches a stale count in prose.

If your change affects exported output, also run the regression guards in
[`docs/USAGE.md`](docs/USAGE.md) §6 (`check_export_baseline.py`,
`check_decode_errors_corpus.py`, `validate_corpus.py`) against a replay, and
update baselines with `--update` only after explaining each changed line. Those
need a corpus — see [Environment](#environment) below.

## Environment

The corpus guards read their inputs from environment variables rather than
hardcoded paths, so nothing in the tree points at one person's disk. None of
them are needed for the sweep above; all of them are needed for §6.

| Variable | What it points at | Read by |
|---|---|---|
| `VRFKIT_CORPUS_DIR` | Directory of `.vrf` replays; a bare filename in a baseline resolves against it | `check_export_baseline.py`, `check_corpus_baseline.py` |
| `VRFKIT_CSHARP_DIR` | Checkout root of the C# reference parser | `analyze_coverage.py`, `compare_*.py` |
| `VRFKIT_VALPLAY_DIR` | valplay checkout root | `check_metrics_baseline.py` |
| `VRFKIT_JOBS` | Worker count for the corpus sweeps; default is cores - 2, capped at 16 | `validate_corpus.py` |
| `VRFKIT_REQUIRE_CORPUS` | Set to anything to turn "corpus absent, skipping" into a failure | `crates/vrf-container/tests/corpus.rs` |

**`tests/corpus.rs` is a container-level smoke test, not a decode sweep.** It
parses each replay's header and decompresses its Oodle chunks; it never reaches
a field. A green `cargo test` with the corpus present therefore says nothing
about decoding. The sweeps that do are `validate_corpus.py` (RepLayout framing
on every content block) and `check_decode_errors_corpus.py` (the overlay), and
neither runs under `cargo test` or in CI. The names do not distinguish them, so
the distinction is written here.

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
  (counted in `RPC stream failed`), never guessed.
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
| `crates/vrf-decode/src/table.rs` | `tools/extract_descriptors.py` then `tools/apply_type_corrections.py` |
| `crates/vrf-transform/src/sbox.rs` | `tools/extract_sboxes.py` |
| `crates/vrf-transform/tests/data/golden_vectors.rs` | `tools/extract_golden.py` |
| `tools/equippable_table.py` | `tools/extract_equippables.py` |

Ordering for the overlay table is load-bearing:
`extract_descriptors.py` → `apply_type_corrections.py` → `cargo fmt` →
`extract_checksum_types.py` (against a **fresh** export).

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
