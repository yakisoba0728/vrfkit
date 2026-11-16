# Phase C decoder evidence disposition

## Scope and result

This is the bounded Track C review of integration commit
`588be1effc8dfca2bed8810fd51a006828081c1b`. It does not repeat the historical
1,018-replay audit. No decoder or type-table change is justified by a field
name, a plausible value, or a bit walk that happens to consume its input.

The candidate list comes from [FOLLOWUP.md](FOLLOWUP.md). The dispositions
below distinguish an unresolved type from a demonstrated regression. The
historical counts identify the missing evidence; they are not new Phase C
measurements. Existing raw payloads and typed values remain unchanged.

| Candidate | Current evidence and behavior | Disposition and required next evidence |
| --- | --- | --- |
| Four rotation-free `ReplicatedMovement` classes | `GameObject_Iris_E_Smoke_C`, `GameObject_Cashew_E_AirStrikeMortar_C`, `Zone_Gumshoe_4_Cage_C`, and `Projectile_NetToss_Underhand_C` have a historical whole-unit location observation but no observed rotation component. Byte and short rotator hypotheses are indistinguishable on those windows. | `blocked_by_evidence`: obtain native-class quantization metadata or a nonzero rotation window for each exact group/checksum, then compare all components with an independent reader, malformed controls, and unchanged raw bytes. Do not copy another class's width. |
| Cypher cage projectile and its successor | `Projectile_Gumshoe_4_CageTrap_C.ReplicatedMovement` remains `Skip`; the 13.01 successor `Projectile_Gumshoe_Q_CageTrap_C` remains raw. The historical entry-survival record establishes a paired rename, not a fresh payload comparison on both sides. | `blocked_by_evidence`: validate both exact class identities, the rotator and location levels, and the entry-survival relation on accessible old/new replay inputs before changing either side. |
| `Clay_PC_C.FocusProjectiles` | No sink route, scoped identity, or checked-in checksum/handle declaration identifies this parent. FOLLOWUP records 24,409 exact main arrays and 788 empty zero-trailer arrays, with 12,837 resolved elements, from a prior audit. | `blocked_by_evidence`: capture exact parent and member identities and build/stream coverage, reproduce populated and empty shapes, independently compare every child, and preserve the parent. The generic object-array walker already accepts exactly one trailing zero byte; loosening the generic grammar is unnecessary. |
| `InputEventData` | The documented seven-tag/53,605-row sample has numeric structure but no established tag meaning, and is not a verification of the larger historical population. | `blocked_by_evidence`: establish tag meanings independently, revalidate widths and full consumption by exact route/build, and keep unknown tags and all original bytes. A second anonymous payload representation alone does not justify a new semantic type. |
| Abilities/GAS FastArray and `StopEffectType` | FastArray replication keys, item IDs and field windows have an existing explicit-checksum-mode structural decoder. The item/property schema remains unknown. `StopEffectType` has no admitted width/domain evidence in the checked-in correction ledger. | `blocked_by_evidence`: obtain exact declarations or independent native/reflection evidence and payload comparisons. Do not reinterpret FastArray IDs as actor IDs or infer an enum's domain from its name. |
| Section arithmetic, scalar warnings, healing and owner references | The historical 15 packet-eligible disagreements and 46 scalar warnings remain observations with uncertainty, not an accepted formula to normalize away. Existing analytical tools retain raw/typed conflicts, ordering and lifecycle uncertainty. | `blocked_by_evidence`: reproduce the disagreeing windows with declaration, bit width, order and lifecycle context; identify the relation independently before changing arithmetic or assigning player credit. |
| Unresolved RPCs and post-RepLayout tails | The current pipeline has separate preserved-payload and loss accounting. A zero RepLayout handle hands the exact remaining window to the sink; unverified tails remain raw, and the narrowly verified AbilitiesAndBuffs path emits a raw structural body. | Preserve the existing boundary contract. Extend decoding only after an exact route and complete payload are independently established; malformed or unverified windows must remain recoverable. |

The three pinned public CI fixtures (12.10, 12.11 and 13.00) are explicitly
documented as containing no `ReplicatedMovement` rows in
[DATA.md](DATA.md#replicatedmovementlocation-is-world-units-at-a-per-class-level).
A green check on those fixtures cannot justify one of the movement types above.
An unobserved candidate is not a passing candidate comparison.

## Existing boundaries checked in this review

The generic array grammar was inspected at `crates/vrf-decode/src/array.rs`.
Both object-reference arrays and the ordinary struct-array path use the
existing final-zero-byte helper. The strict `decode_struct_array_exact` path
does not admit it. The helper accepts only an exactly eight-bit remaining
zero IntPacked word; nonzero trailers remain in the residual tally and
truncated IntPacked trailers increment an error. Route qualification is
separate from that grammar and remains required.

The raw-tail path was inspected at
`crates/vrf-net/src/pipeline/framing.rs` and
`crates/vrfkit/src/sink/stream.rs`. Existing tests exercise a 13-bit tail at
an unaligned start, preservation-versus-loss counters, missing component
provenance, malformed chained framing and raw-body recovery. No new
semantic route was enabled by this review.

## Validation receipt

On the baseline above, with Python warnings treated as errors:

```sh
python -W error -m unittest discover -b -s tools/tests -p 'test_extract_fastarray_observations.py'
python -W error -m unittest discover -b -s tools/tests -p 'test_check_rep_movement_levels.py'
python -W error -m unittest discover -b -s tools/tests -p 'test_extract_section_observations.py'
python -W error -m unittest discover -b -s tools/tests -p 'test_section_timeline.py'
```

Results: 19, 5, 17 and 31 tests passed respectively (72 total, zero failures).
These are synthetic/tool regressions, not a fresh private-corpus audit or
proof of the untyped candidate semantics. No performance benchmark was run.

The FastArray suffix rejection was also mutation-checked. Temporarily
removing the final `reader.pos != reader.end` guard from
`tools/extract_fastarray_observations.py` made
`FastArrayTests.test_suffix_and_count_mutations_are_rejected` fail with
`WireError not raised` (exit 1). Restoring the exact original source made the
same test pass (exit 0). No mutant is retained. This demonstrates that the
test detects silently accepted suffix bits.

Rust boundary-test results and any new bounded accessible-public-input
inventory must be recorded separately in the final Phase C integration
evidence. This receipt does not claim either check succeeded. No private
replay bytes or player identifiers belong in this document.

## Follow-up acceptance gate

A later decoding PR must identify the exact input build and stream, group,
field and checksum; explain every output change; prove full payload
consumption and raw-byte preservation; show meaningful malformed negative
controls; and compare previous typed values and proposed values with an
independent reader. Unavailable inputs remain `blocked_by_evidence` rather
than being filled with a synthetic assumption.
