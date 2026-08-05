# Contributing to vrfkit

Thanks for your interest. vrfkit is a reverse-engineered parser for a format
that changes every game build, so it has a few unusual rules. Please read these
before opening a PR.

## Build

```bash
cargo build --release -p vrfkit                     # inspect / validate
cargo build --release -p vrfkit --features export   # + Parquet export
```

Rust 1.85+, edition 2024. `#![forbid(unsafe_code)]` is in every crate — do not
add `unsafe`.

## Before you open a PR

Run the full sweep. Every one of these must be green:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
python tools/check_ascii.py --check
python tools/apply_type_corrections.py --check
python tools/check_effect_decoder.py --check
python tools/check_docs.py --fast
python -m unittest discover -s tools/tests -p "test_*.py"
```

If your change affects exported output, also run the regression guards in
[`docs/USAGE.md`](docs/USAGE.md) §6 (`check_export_baseline.py`,
`check_decode_errors_corpus.py`, `validate_corpus.py`) against a replay, and
update baselines with `--update` only after explaining each changed line.

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
`extract_descriptors.py` → `apply_type_corrections.py` → `cargo fmt`.

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
