<!-- Thanks! Please confirm the invariant checklist below. -->

## Summary

<!-- What does this change do, and why? -->

## Verification

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `python tools/check_ascii.py --check`
- [ ] `python tools/apply_type_corrections.py --check`
- [ ] `python tools/check_docs.py --fast`

## Invariant checklist (skip none that apply)

- [ ] No field's `raw_bits` is dropped because its type is unknown (no skip path).
- [ ] Output is **byte-identical** on valid replays (or the baseline change is explained line by line).
- [ ] No `unsafe` added.
- [ ] No non-ASCII in Rust code or comments.
- [ ] No generated file (`table.rs`, `sbox.rs`, `golden_vectors.rs`, `equippable_table.py`) hand-edited.
- [ ] No new hardcoded display names in a Rust crate.
