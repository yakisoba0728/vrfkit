# Core correctness audit

Baseline: `8581642283bd5a571967129d73897db8ee5408ba`.
Toolchain: Rust/Cargo 1.86.0, Linux x86_64. This is new local evidence;
historical Phase A-D runs are not counted as current executions.

## Fixed: malformed repeated character GUID retains stale attribution

Severity: **P2** (incorrect typed movement attribution on malformed input).

`crates/vrf-movement/src/rpc.rs::decode_single_update` used to retain a
previous valid `shooter_guid` when a later occurrence of that field had a
width other than 32 bits. A following component could therefore emit moves
under the previous character's identity. The existing error counter moved,
but the emitted typed row still claimed the stale character.

The fix clears the current GUID on a malformed-width occurrence. Already
emitted movement prefixes remain valid observations; a later valid GUID
restores attribution for subsequent components. The malformed field and its
orphaned component are counted separately, retaining the existing raw-RPC
fallback contract. No new wire interpretation or public API is introduced.

The new regression constructs the following wire order in one update:

1. Valid GUID 11 and a valid component.
2. A repeated GUID field of width 0, 31, 33 or 64 bits and a component.
3. Valid GUID 22 and another valid component.

It requires exactly two rows, attributed to 11 and 22, with component arrival
ordinals 0 and 2, two component attempts/completions and two errors. This
checks rejection, preceding-prefix preservation, recovery and source identity.

### Executed checks

| Check | Result | Evidence |
| --- | --- | --- |
| New regression against baseline implementation | Failed as intended: 3 rows instead of 2 | `stale_guid_baseline_failure.log` |
| Complete movement crate suite after fix | 23 passed, 0 failed | `movement_fixed_tests.log` |
| Same suite with the repository release profile | 23 passed, 0 failed | `movement_fixed_release_tests.log` |
| Movement Clippy, all targets, warnings denied | See exit status in `receipt.json` | `movement_clippy.log` |
| Movement rustfmt check / Git whitespace check | See `receipt.json` | Commands recorded below |

```sh
cargo +1.86.0 test -p vrf-movement --locked tests::a_malformed_repeated_guid_invalidates_only_later_component_attribution -- --exact
cargo +1.86.0 test -p vrf-movement --locked
cargo +1.86.0 test -p vrf-movement --release --locked
cargo +1.86.0 clippy -p vrf-movement --all-targets --locked -- -D warnings
cargo +1.86.0 fmt -p vrf-movement --check
git diff --check
```

The first command was run before the implementation fix. All build commands
used the same Cargo.lock, `CARGO_BUILD_JOBS=2` and a task-local target directory.
The initial interrupted dependency/build preparation is not a successful
verification run. The complete final workspace, replay preservation and
Windows validation are coordinator/independent-QA gates, not these local
movement-only results.

## Audit scope and handoff

Reviewed the bounds and state transitions in bitio read/copy/window/string
operations; transform dispatch and size admission; container chunk/preamble,
Event and Checkpoint framing and Oodle size admission; DemoFrame section and
packet framing; schema admission/cache/path resolution/live exports/checkpoint
tables; net packet/bunch reassembly/content blocks/channel/spawn/field framing;
decode primitive/geometry, dynamic-array/FastArray/FText/effect/struct framing;
and native movement RPC/component handling. Generated version/type tables were
not independently re-derived from game binaries, and this audit makes no new
wire claims for evidence-blocked features. It is a targeted manual audit, not
an exhaustive proof of every line or every possible input.

Additional concrete candidates were handed to the coordinator and independent
QA rather than silently excluded:

- **Checkpoint path-reference allocation amplification:** one long literal
  path is cloned into a separate String for every indexed GUID reference.
  `checkpoint.rs` permits up to 1,000,000 GUID entries and 1 MiB per literal;
  field-slot limits do not cover these allocations. The architecture role
  received the exact code paths and a bounded synthetic reproduction plan.
  No schema change is included in this movement patch.
- **Direct partial-accumulator API precondition:** `add_fragment` takes a
  separate bit count and byte slice without documenting or checking that the
  slice is long enough; `append_bytes` slices to `ceil(bit_count/8)` and can
  panic for an empty slice with eight declared bits. Normal pipeline staging
  supplies a matching slice. A safe public rejection needs an explicit error
  and preservation contract; independent QA received a minimal reproducer.
- **Direct packet API extreme size:** `compute_bit_size` casts the byte
  length to i32 before multiplying by eight. A caller-supplied packet at
  268,435,456 bytes can overflow in checked builds. The replay DemoFrame
  reader rejects packet sizes above 2,048 bytes before either packet API is
  called. No large allocation was made merely to exercise this boundary.
- **Seekable preamble resource scope:** the standalone `read_preamble`
  bounds its info prefix but allocates the full declared Header after checking
  file length. This is documented behavior, not evidence of an input-limit
  bypass in the high-level Reader. No speculative Header format cap was added.
- **Effect handle scan scope:** `scan_element_handles` derives a pair without
  enforcing every final terminator/residual condition. The production JSON
  decode subsequently enforces those conditions. No production data-loss
  failure was established, so no redundant parser rewrite was introduced.

The packet-size and preamble observations are scoped API limitations, not
claims that a small replay bypasses the normal framing limits. Runtime memory
and cross-platform end-to-end coverage are reported separately by the roles
executing them.
