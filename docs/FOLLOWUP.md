# Follow-up

The open work, the decisions deliberately not taken, and the one record the
reference export lacks.

## Recorded review findings

[Pre-fix code review findings](CODE_REVIEW.md) preserves the fourteen findings
from the 2026-10-16 review, with synthetic reproductions, observed behavior,
fix acceptance criteria and the separate GitHub Issue/PR snapshot.
Its [resolution ledger](CODE_REVIEW.md#resolution-ledger) records the F01--F14
working-tree fixes, the separate Windows CI reader-lifetime fix, failing-before
regressions, integrated checks and the remaining Windows/private-corpus limits.

## Remaining work

- Establish meanings before adding typed fields or game-action labels: the
  AbilitiesAndBuffs FastArray item/property schema
  ([GAS observations](GAS_AND_PATCHVOLUME_INVESTIGATION.md)), the
  `InputEventData` tags (DATA's seven tags come from a 53,605-row sample and
  are not revalidated on the 42,545,425-row population), whole raw
  post-RepLayout tails, unresolved RPC payloads, and `StopEffectType`'s width
  and domain. A bit walk that closes is not evidence: pilot with an exact
  raw/value comparison and malformed controls, then a full-corpus comparison
  that keeps every previous value and prints zero and nonzero failures.
- Resolve the 15 packet-eligible section arithmetic disagreements and the 46
  scalar warnings ([section observations](SECTION_OBSERVATIONS.md)) before
  claiming effective HP or healing totals.
- Associate healing, ability and GAS `OwnerActor`/`AvatarActor` references
  with their role conflicts, missing references and lifecycle uncertainty
  kept; an association is not player credit.
- Type the six `ReplicatedMovement` classes still raw
  ([DATA.md](DATA.md#replicatedmovementlocation-is-world-units-at-a-per-class-level)):
  four need their rotator width from the native class, and Cypher's cage
  projectile needs its table `Skip` retyped with its 13.01 successor.
- Extend `Clay_PC_C.FocusProjectiles` only when additional build-specific
  evidence supports it. 0.6.0 added the exact
  [12.05 measured route](PHASE_C_FOCUS_PROJECTILES.md), preserving raw parents
  and accepting the proved empty-array zero trailer. The earlier broad audit
  found 24,409 of 25,197 main payloads parsed exactly and all 12,837 elements
  resolved to Raze actors; the other 788 carried an empty-array zero trailer.
  Those historical figures are not a fresh audit of the route or evidence for
  enabling this route on other builds. Further support needs exact live
  declarations, complete payload consumption and independent malformed and
  row-preservation comparisons.

## Considered and not done

- **The `BitReader::with_bit_len` failure arm after `take_completed`**
  (`crates/vrf-net/src/pipeline/mod.rs`). The accumulator sizes the buffer to
  the bit count it returns, so no input reaches it; preserving its payload
  would need a new `PartialPayloadReason` for a path nothing reaches.
- **Deleting the packet reader's partial tracker** (`track_partial_bunch`,
  `crates/vrf-net/src/packet.rs`). Nothing in the workspace reads it, but
  `RawPacketReader::read_packet` and `PacketReadResult::partial_error_count`
  are published API; deleting it would leave a field that is always 0.
- **Retiring a live actor whose channel a refused or discarded partial
  fragment reopened.** Every other arm that stops an open before it is read
  retires the displaced actor (`retire_after_failed_open`); this one needs a
  rule per discard mode inside a path the verdict leaves unscored.
- **Counting the bits after a cleanly read package-map export list.**
  `process_complete_payload` returns after the exports, so what follows is
  never read. No replay carries such a bunch to check reading on or a bit
  counter against, so the bunch fails `validate` and `verify_build_corpus.py`.
- **A team roster.** `Rounds[].Reports[].Interactions[].{ParticipantSubject,ParticipantTeamName}`
  gives one: in the 1,015 corpus replays that carry it (Bomb and Swiftplay; the
  public fixtures carry none), 364,312 pairs name `Red` or `Blue`, 0 subjects
  under both and 5 manifest players under neither, and the spike carrier's team
  is `extract_rounds.py`'s `attacker_team` on all 36,277 held custody intervals.
  valplay derives teams itself, and a second file would repeat account UUIDs.
- **Committing raw bits cut from private replays** so CI's real-bytes type
  check covers more fields: it would put private replay content in this
  public repository. The machine-local corpus sweeps cover them.

The reopen and package-map items have no input: over the 1,018 exports of the common audit,
`package_map_exports`, `must_be_mapped_guids`, `bunch_header_failures`,
`channel_state_limit_failures`, `channel_reopens_while_open` and
`partial_errors` are 0 on 563,030,549 main and 4,346,884 checkpoint bunches;
`vrfkit diag` on the same replays agrees, `rep_layout_export_bunches` included.

## The damage record only vrfkit emits

On `02d4d478` vrfkit exports one `MulticastNotifyDamage_Point` the reference
export `compare_rpc_params.py` reads lacks: packet 391880, actor 27232,
subobject 27244, channel 194, a killing blow of 29.45 dealt / 20 taken on
Gekko's Dizzy (`Projectile_E_Aggrobot_DiscTurret_PowerWave_C`). The record is
on the wire. Subobject 27244 is stably named `Damageable`, so its block header
carries no class NetGUID; vrfkit binds the class through
`resolve_cnc_for_instance_name` (`crates/vrf-schema/src/resolve.rs`), whose
`Component_ClassNetCache` suffix finds the only matching declared group.
Streams not derived from the record agree with it:

- The shooter, pawn 1466 (`Hunter_PC_C`), has `PlayerState` 268 (the record's
  `DamagerPlayerState` and `KillCreditPlayerState`) and `CurrentEquippable`
  26796 (the record's `EquippableUsed`).
- `DamageOrigin` is 1466's `movement.parquet` position raised 76.0 units, its
  view matches `DamageDirection`, and that direction is within 0.31 degrees of
  the vector to `DamageImpactLocation`.
- The victim is the projectile: its damage section goes from 20 to 0, and two
  packets later the actor's `Destroyed Logic` RPC arrives and its channel
  closes as `Destroyed`.

`compare_rpc_params.py` lists this record as its one expected difference, keyed
by the replay's SHA-256 (from the reference `manifest.json`), packet, actor,
subobject, channel, function and the four compared values. Anything else on
this replay -- the reference gaining it, vrfkit losing it, other values, a
second record -- is reported STALE and exits 1; without the manifest a run
that otherwise matches exits 2. The same shape covers every RPC on a
subobject named `Damageable`: 1,989 invocations in 391 of the 1,018 replays,
on six projectile classes; only this one was checked against the reference.
