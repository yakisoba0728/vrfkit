# Phase C decoder evidence disposition

This preserves the initial review and interruption-era evidence. Execution
subsequently resumed and the exact 12.05 Clay FocusProjectiles route was
implemented in [PR #72](https://github.com/yakisoba0728/vrfkit/pull/72).
See [the focused route report](PHASE_C_FOCUS_PROJECTILES.md) and
[current acceptance](PHASE_C_RESULTS.md). The earlier execution blocker is
historical; the remaining unproven candidate types below retain their stated
evidence requirements.

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
| `Clay_PC_C.FocusProjectiles` | The initial source-only review lacked an exact checked-in route. A later independent check of accepted historical public 12.05 output identified `/Game/Characters/Clay/Clay_PC.Clay_PC_C`, parent handle 45/checksum 957945936 and member handle 46/checksum 831657485. It fully consumed 102 main and 18 checkpoint parent payloads and observed 53 main member values. | The interruption temporarily blocked implementation and fresh acceptance. After recovery, the narrow route was implemented with both exact declarations, raw-parent preservation and 53 expected additional main child rows. Current validation is tracked in [Phase C results](PHASE_C_RESULTS.md); generic array framing remains unchanged. |
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

## Superseding independent public observation

After the initial Track C commit, E independently inspected the accepted
historical Phase B public 12.05 export. The exact parent/member declarations
above were present in the main schema and all 18 checkpoint schema sets.
Main contained 53 populated parents and 49 empty parents; all 18 checkpoint
parents were empty. Main widths were 16, 24, 64 and 72 bits. The bounded
independent parser consumed every selected payload, accepted three positive
synthetic shapes and rejected ten malformed shapes. These are input controls,
not a completed Rust source mutation campaign.

A separate actor-table check corroborated all 53 member references using
actor-open records (12 Boomba, 17 primary projectiles, 21 arming satchels and
3 rockets). The final combined receipt rerun after adding that corroboration
was interrupted by execution-environment loss. The separate observation is
recorded; completion of that updated receipt is not inferred.

These were historical-output checks, not fresh Phase C replay executions.
At the interruption, no FocusProjectiles Rust route had been implemented or
accepted. The initial lack of accessible exact identity evidence was
superseded, then the original environment recovered and implementation
resumed. [Phase C results](PHASE_C_RESULTS.md) records the later route and
fresh-acceptance status. Other candidate rows retain their evidence
requirements. [The recovery checkpoint](PHASE_C_RECOVERY.md) preserves the
earlier source and restart limits as history.
