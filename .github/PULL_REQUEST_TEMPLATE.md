<!-- Thanks! Please confirm the invariant checklist below. -->

## Summary

<!-- What does this change do, and why? -->

## Verification

- [ ] `cargo +1.86.0 fmt --check`
- [ ] `cargo +1.86.0 clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo +1.86.0 test --workspace --locked`
- [ ] Core-only/singleton feature matrix and strict rustdoc from `CONTRIBUTING.md`
- [ ] Rust `write_interop_files` fixture verified by `crates/vrf-export/tests/python_interop.py` using its exact private directory
- [ ] `python -W error tools/check_ascii.py --check`
- [ ] `python -W error tools/apply_type_corrections.py --check`
- [ ] `python -W error tools/extract_checksum_types.py --export tools/fixtures/checksum_export --check`
- [ ] `python -W error tools/check_baseline_schemas.py --allow-missing-hashes`
- [ ] `python -W error tools/check_docs.py --fast`
- [ ] `python -W error -m unittest discover -s tools/tests -p "test_*.py"`

## Invariant checklist (skip none that apply)

- [ ] No field's `raw_bits` is dropped because its type is unknown (no skip path).
- [ ] Output is **byte-identical by committed SHA-256** on valid replays (or the measured baseline change is explained line by line).
- [ ] No `unsafe` added.
- [ ] No non-ASCII in Rust code or comments.
- [ ] No generated file (`table.rs`, `checksum_table.rs`, `sbox.rs`, `golden_vectors.rs`, `equippable_table.py`) hand-edited.
- [ ] No new hardcoded display names in a Rust crate.
