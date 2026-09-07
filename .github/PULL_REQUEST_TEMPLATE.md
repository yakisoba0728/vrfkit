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
- [ ] `python -W error tools/check_baseline_schemas.py`
- [ ] `python -W error tools/check_docs.py` (not `--fast`: that skips the count check)
- [ ] `python -W error -m unittest discover -s tools/tests -p "test_*.py"`

## Replay validation (parser changes)

<!-- Required for parsing, payload transforms, decoding, export behavior, and
new-build support. For unrelated changes, write "Not applicable".
If no replay is available, write "Not run", explain what input is needed,
and leave validation for the maintainer to complete before merging.
CI or a skipped corpus check does not replace a real replay run.
See CONTRIBUTING.md, "Replay evidence for parser changes". -->

- Game build/branch and replay count per build:
- Tested commit and exact commands (include checkpoint coverage if affected):
- Results: successful/failed replays, transform/framing/decode failures, and oracle pass rates where available:
- Before/after comparison on the same replays (include an older supported build for shared parser changes):
- Output differences, remaining failures, and scope or limitations of validation:

## Invariant checklist (skip none that apply)

- [ ] No field's `raw_bits` is dropped because its type is unknown (no skip path).
- [ ] Output is **byte-identical by committed SHA-256** on valid replays (or the measured baseline change is explained line by line).
- [ ] No `unsafe` added.
- [ ] No non-ASCII in Rust code or comments.
- [ ] No generated file (`table.rs`, `checksum_table.rs`, `sbox.rs`, `golden_vectors.rs`, `equippable_table.py`) hand-edited.
- [ ] No new hardcoded display names in a Rust crate.
