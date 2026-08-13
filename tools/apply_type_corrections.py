"""Apply known type corrections to the generated overlay table.

These corrections represent empirically-verified differences between the C#
descriptor declarations and the actual wire format in build 13.01:
  - Time-related Float fields are actually Double (64 bits on wire)
  - Actor bookkeeping fields "215"/"216" in non-weapon groups are variable-width
    (3 bits in 13.01), not Int32

Run after extract_descriptors.py regenerates table.rs, and BEFORE cargo fmt.

That ordering is load-bearing and used to be silent. Two of the passes below
match one-line literals, which is the shape extract_descriptors.py emits but
not the shape rustfmt leaves behind, so running the corrector on an already
formatted table applies nothing. The old script printed "Applied 0" and
exited 0 for that case -- indistinguishable from "everything was already
correct".

So the script no longer trusts its own operation count. After writing, it
verifies the END STATE of every correction against the parsed table and fails
loudly if any is missing. That check is format-independent: if the
application patterns ever rot again, the verification still catches it.

`--check` verifies the FILE, not the corrected copy. It used to apply every
correction in memory and verify that, suppressing only the write, so it could
not tell "already corrected" from "correctable and nobody corrected it" -- and
CI runs `--check`, which is how a regenerated table.rs could go green while the
Rust build used the uncorrected one. The two now report separately: a
correction missing from the corrected copy is a DEAD PATTERN, and one present
there but absent from the file means the file was NEVER CORRECTED.

Usage:
    python tools/apply_type_corrections.py            # apply, then verify
    python tools/apply_type_corrections.py --check    # verify the file, no write
"""
import argparse
import re
import sys
from collections import Counter
from pathlib import Path

if __package__:
    from .atomic_io import atomic_write_text
else:  # direct script execution
    from atomic_io import atomic_write_text

TABLE_RS = Path(__file__).parent.parent / "crates" / "vrf-decode" / "src" / "table.rs"

#: (group_path substring, field_name, required FieldType -- IN FULL).
#: One entry per correction the passes below make. Checked against the file
#: after writing; a miss is a hard failure.
#:
#: The type is the WHOLE `FieldType::...` expression and `verify` compares it
#: with `==`, because a substring test cannot see the two mistakes a type table
#: is most likely to make. `"Int32" in "FieldType::UInt32"` is True and
#: `"Byte" in "FieldType::EnumByte"` is True, so under the old check an entry
#: with the wrong signedness or the wrong byte variant verified clean -- the
#: check passed on exactly the errors it existed to catch.
EXPECTED = [
    ("TimedBomb.TimedBomb_C", "TimeRemainingToExplode", "FieldType::Double"),
    ("TimedBomb.TimedBomb_C", "DefuseProgress", "FieldType::Double"),
    ("Comp_Ability_CooldownComponent_C", "StartTimeStamp", "FieldType::Double"),
    ("Comp_Ability_CooldownComponent_C", "CooldownSeconds", "FieldType::Double"),
]
for _group in (
    "TimedBomb.TimedBomb_C",
    "EquippablePickupProjectile.EquippablePickupProjectile_C",
    "EquippableGroundPickup.EquippableGroundPickup_C",
    "OwnerExclusivePlayerInfo",
    "Projectile_Phoenix_Q_FlameWall_ThroughWall.Projectile_Phoenix_Q_FlameWall_ThroughWall_C",
):
    for _field in ("215", "216"):
        EXPECTED.append((_group, _field, "FieldType::EnumRemainingBits"))
EXPECTED += [
    ("SmokeScreen", "ReplicatedMovement",
     "FieldType::RepMovement { rotation: RotatorQuantization::ByteComponents }"),
    ("AresEquippableDataTracker", "OriginalBuyerTeam", "FieldType::Raw"),
    ("MulticastNotifyDamage_Base", "EquippableUsed", "FieldType::ObjectNetGuid"),
    ("MulticastNotifyDamage_Point", "EquippableUsed", "FieldType::ObjectNetGuid"),
    ("MulticastNotifyDamage_Base", "DamageOrigin",
     "FieldType::VectorNetQuantize { scale: 100 }"),
    ("MulticastNotifyDamage_Point", "DamageOrigin",
     "FieldType::VectorNetQuantize { scale: 100 }"),
    ("MulticastNotifyDamage_Point", "DamageImpactLocation",
     "FieldType::VectorNetQuantize { scale: 1 }"),
    ("MulticastNotifyDamage_Point", "DamageImpactBoneRelativeLocation",
     "FieldType::VectorNetQuantize { scale: 1 }"),
    ("MulticastNotifyDamage_Point", "DamageDirection", "FieldType::VectorNetQuantizeNormal"),
    ("MulticastNotifyDamage_Point", "DamageImpactNormal", "FieldType::VectorNetQuantizeNormal"),
]

#: Entries the WIRE carries that the C# descriptors cannot declare, because the
#: group did not exist when the reference was pinned.
#:
#: This is a different kind of claim from every correction above. Those say
#: "the descriptor declares X and the wire disagrees". This says "the descriptor
#: is SILENT and here is the type anyway", so the bar is higher and only one
#: case clears it today.
#:
#: `/Script/ShooterGame.BaseTeamState` is new in build 13.02, which deleted
#: `BombGameState.TeamEconomy` and moved team economy into a separately
#: replicated actor. The property NAMES did not change, and the reference
#: declares their types at the pinned commit --
#: `GameState/AresTeamEconomy.cs:11-12`:
#:
#:     public sealed record AresTeamEconomyUpdate(
#:         int Index, uint? ReplicationId,
#:         int? LoadoutValue,            <- Int32
#:         int? AverageLoadoutValue);    <- Int32
#:
#: so this is a descriptor-sourced type for a relocated property, not a type
#: guessed from values. `OwnerExclusivePlayerInfo.{Start,End}OfRoundLoadoutValue`
#: are Int32 in the same reference, which corroborates the family.
#:
#: Corroborated on the wire, not decided by it: all 44+44 payloads are 32 bits,
#: read as little-endian i32 they run 4300/4150 at round 1 to 34300, and
#: AverageLoadoutValue is EXACTLY LoadoutValue/5 on every row (a five-player
#: team).
#:
#: DELIBERATELY NOT ADDED: `Wins`, `Points`, `InitialRole`, `TeamRole`,
#: `TeamPlayerStates`, `TeamComponent`, `TeamExclusiveTeamInfo`. The same
#: 13.02 group declares all of them and the reference declares NONE of them
#: under any group, so there is no source for their types. They keep their raw
#: bits and stay untyped. That line is what makes this addition defensible;
#: widening it by eye would undo the reason it is allowed at all.
#: `ChosenCeremonyForRound` is the second entry of this kind, and it rests on
#: wire evidence alone -- no descriptor names it, in either build. It is added
#: because that evidence is unusually complete and completely self-checking:
#:
#:   246 occurrences across 6 replays and BOTH builds (13.01 and 13.02)
#:   126 are 16 bits and read as IntPacked resolve, 126 of 126, to an actor
#:       whose class path ends in `Ceremony_C` -- Default, Clutch, Closer,
#:       Flawless, Ace, TeamAce
#:   120 are 8 bits of `00`, i.e. GUID 0, the null reference, written at round
#:       start and replaced by a real ceremony at round end
#:     0 odd GUIDs, and a dynamic NetGUID must be even
#:
#: An ObjectNetGuid reads an IntPacked, so it covers both widths without a
#: special case. A wrong type here cannot hide: every non-zero value has to
#: name a ceremony actor that the same export already lists in actors.parquet.
#:
#: This is a WEAKER justification than the BaseTeamState pair above, where the
#: reference declares the property's type and only its group moved. Recorded as
#: such deliberately -- see docs/archive/PROJECT_STATUS.md 31-C and 32.
#: Swiftplay's copy of this field is NOT listed here. It was, briefly, and the
#: entry was removed when `GROUP_ALIASES` landed in `vrf-decode/src/overlay.rs`
#: -- Swiftplay's game state now falls back to the Bomb class for every field,
#: so a second entry would state the same fact twice and drift from it.
#:
#: `MoneyManagementComponent.{Money,StartOfRoundMoney,TotalMoneyGranted}` are
#: the live per-player credits, replicated under this group on BOTH 13.01 and
#: 13.02 (verified on 02d4d478 and 13.02 Demos files). No descriptor declares
#: the group: `Money`/`TotalMoneyGranted` appear under no group in the C#
#: reference, and `StartOfRoundMoney` is declared only under
#: `OwnerExclusivePlayerInfo` (OwnerExclusivePlayerInfoDescriptor.cs:93), a
#: separate end-of-round snapshot path. So the extractor emits nothing and the
#: fields ship untyped.
#:
#: All three are 32-bit on every row. Read as little-endian i32 they are
#: unambiguous credits: `Money` is 800 across all ten actors at pistol-round
#: start (t=8ms) and runs 0..9000 in multiples of 50; `StartOfRoundMoney` is
#: 800 for active players and 0 otherwise; `TotalMoneyGranted` is cumulative,
#: 800..34200. `StartOfRoundMoney`'s type is descriptor-sourced (Int32), the
#: same strength as the BaseTeamState pair above; `Money` and
#: `TotalMoneyGranted` rest on wire evidence alone, like ChosenCeremonyForRound.
#: No other MoneyManagementComponent field exists on the wire, so this does not
#: widen by eye -- the DELIBERATELY NOT ADDED line above stays intact.
#:
#: `BombPlayerState.Ping` is the largest untyped wire-declared field (20193 rows
#: on 02d4d478, 222855 across the 11-replay bundle). No descriptor declares it.
#: The encoding was settled in docs/archive/PROJECT_STATUS.md 18: bit_count is
#: always 16, and the value is a 16-bit little-endian unsigned integer that
#: behaves like latency in
#: milliseconds -- on 02d4d478: min 6, p5 10, p50 15, p90 19, p99 25, max 473,
#: 57 distinct values. Typed as `SerializedInt{65536}` (16 bits LSB-first), which
#: read_serialized_int reads as exactly 16 bits and satisfies decode_field's
#: full-consumption guard. This is the same wire-evidence ADDITION class as Money
#: -- 18-D's "no descriptor declares it" objection is exactly the gate Money
#: cleared, and the per-player latency series is now wanted.
#:
#: Status-effect components. `Comp_Actor_Concussable` and
#: `Comp_AbilityFuelSystem` are GENERIC Blueprint components shipped under
#: `/Game/Characters/Components/`, not agent-specific classes -- no C#
#: descriptor declares either group, so without these entries the fields
#: ship as raw bits even though they decode cleanly to real values.
#:
#: Concussion was surveyed for agent-commonality on the 98605b1b Demos
#: export: the component is on 9 distinct actors spanning eight agents
#: (Phoenix, Breach, Smonk, Clay, Guide, Wushu, Terra, Pandemic, Deadeye)
#: plus Guide's PossessableScout pawn, with identical field names and bit
#: widths on every actor -- typing the group once covers every agent. The
#: widths are self-checking across all 375 rows: ConcussStartTime and
#: ConcussEndTime are 32 bits on all 39 rows each (Float), ConcussLevel is
#: 64 bits on all 297 rows (Double). Read as Float the start/end times are
#: game-seconds (389.5/392.0 ... 1916.7/1919.2 -- the ~2.5 s gap is the
#: concussion duration); read as Double ConcussLevel runs the 0..1
#: intensity ramp.
#:
#: AbilityFuel is per-ability rather than per-player (on 98605b1b: Sage/
#: Guide's heal `Ability_Guide_4_Heal` and Viper/Pandemic's smoke), but the
#: component path is shared so one entry covers every actor that carries
#: it. CurrentFuel is 64 bits on all 5702 rows and reads as Double a smooth
#: 1.0 -> 0.0 drain (1.0, 0.9993, 0.9909, 0.9824, ...); IsFuelDraining is
#: 1 bit on all 60 rows, raw 0x00/0x01 -- an unambiguous Bool. CurrentFuel
#: was guessed Float in the task brief, but the wire is 64-bit; Double is
#: what decodes.
#:
#: DELIBERATELY NOT ADDED: `Comp_AbilityStatisticsReplicator.AbilityCasts
#: ThisRound`. It is on all ten player characters (agent-common, like
#: Concussion), but the brief's "Int32" guess is wrong: the payload runs 16
#: to 6712 bits across 955 rows and the bytes after the 16-bit `0000` empty
#: case decode to ASCII GUID strings -- it is a variable-width array of
#: struct entries, not a 32-bit integer. Forcing Int32 would decode only
#: the smallest rows and mislabel every other row, so it stays raw until
#: the array element type is worked out. Also deliberately not added:
#: `FuelFull`/`FuelEmpty` (0-bit payloads, no value to decode) and
#: `BlindManagerComponent.ActiveBlinds`/`LongestActiveBlindDuration`
#: (ActiveBlinds is a variable-width array; the duration was outside the
#: brief and its Float read is not yet corroborated against gameplay).
#: Final-sweep ADDITIONS. Four descriptor-silent groups whose raw bits decode
#: cleanly to real game values on the 98605b1b Demos export. Each is held to
#: the same wire-evidence bar as Money/Ping: a single consistent bit width on
#: every row, and a value distribution that a wrong type cannot reproduce.
#:
#: `DamageableComponent:MulticastNotifyHeal.HealTaken` and
#: `:MulticastNotifyOverhealDecay.DecayApplied`. The DamageableComponent C#
#: descriptor (DamageableComponentClassNetCacheDescriptor.cs) declares only the
#: two MulticastNotifyDamage_* handles, so the heal/decay RPC parameter groups
#: -- which the wire still carries by name -- have no declared type. The RPC
#: sink resolves these under their colon-group paths
#: (`/Script/ShooterGame.DamageableComponent:MulticastNotifyHeal` etc.) with
#: the bare parameter name, the same shape as the EquippableUsed correction.
#: HealTaken is 32 bits on all 1252 rows and reads as Float a 0.05..400 heal
#: magnitude; the 0x3f800000 pattern (1.0f) recurs and is a float signature no
#: int produces. DecayApplied is 32 bits on all 699 rows and reads as Float a
#: 0.07..50 overheal-decay amount clustering at 0.195. The sibling
#: LifeChangeBySection (177 bits, a struct array) and the *Instigator/*Causer
#: actor refs are deliberately NOT added: the first is variable-width and the
#: second are metadata refs this project does not type.
#:
#: `PlayerScoreComponent.Score` is the per-player combat score. No descriptor
#: declares the group. 32 bits on all 430 rows. Read as Float the bytes are
#: denormal slop (~1e-44) -- the float read rules itself out -- but read as
#: Int32 the values run 21..5833 with 415 distinct values, the exact shape of
#: a cumulative combat score across a full match.
#:
#: `ZoomMultiplierComponent` drives the ADS/scope FOV transition. No descriptor
#: declares the group. The five fields below are 32 bits on every row with zero
#: NaN: SourceFov/TargetFov run 20.6..103.0 and 103.0 is Valorant's documented
#: default hip-fire FOV; SourceFov1P/TargetFov1P run 5.0..70.0 and 70.0 is the
#: default 1P FOV; TotalTransitionTimeDuration runs 0.0..0.25 (the ADS
#: transition seconds). A wrong type cannot yield 103.0. Deliberately NOT added:
#: SourceZoomLevel/TargetZoomLevel (~70% of rows are the 0xFFFFFFFF sentinel,
#: which Float reads as NaN -- an enum-or-sentinel, not a clean float) and
#: CooldownOption/TransitionState (2-bit enums the wire count alone cannot
#: disambiguate from a SerializedInt).
#:
#: `FiniteSpeedMovementComponent.MaximumRange` is a projectile's max travel
#: distance in Unreal units. No descriptor declares the group. 32 bits on all
#: 11699 rows, reads as Float 397.6..49986.1 with the mode at ~19993 UU
#: (~500 m), the right order of magnitude for a Valorant projectile.
#: Deliberately NOT added: bIsActive (1 bit but all 574 rows are 0x01 -- a
#: constant that carries no consumer information), ServerMovementTime (a
#: movement-clock timestamp whose epoch is undocumented, so the Float read is
#: correct but the values are not interpretable), NumCollisions (32 bits but
#: the i32 values are all 0/1, indistinguishable from a Bool widened to 32
#: bits by the property block -- not worth a guess), and RequestedIgnoreActors
#: (a variable-width array).
ADDITIONS = [
    ("/Game/GameModes/Bomb/BombGameState.BombGameState_C",
     "ChosenCeremonyForRound", "FieldType::ObjectNetGuid"),
    # Phoenix's wall, the other class declaring `MulticastAddSmokeScreenPoint`.
    # Viper's `SmokeScreenManager` was typed and this one was not, so 2,791 rows
    # over 31 replays came out null while decode errors stayed at 0 -- and
    # because Viper's side worked, the ability looked handled.
    #
    # The checksum fallback could not carry the type across and was right not
    # to: Unreal hashes the property, and these are different properties
    # (2794273677 / 1639439377 against Viper's 2235276067 / 2983776962). It
    # refused instead of guessing, which is the behaviour that made this a
    # missing name rather than a wrong type.
    #
    # Admitted on wire evidence, same bar as the rest: every row is 192 bits
    # (3 x f64); `Translation` reads as map coordinates (7211.7, 1670.3, 96.0)
    # on an Ascent replay; `Scale3D` is (1,1,1) on every row, which no other
    # reading of those bits produces. The third parameter of the same RPC
    # arrives as handle `249` with no name from the replay and is left raw --
    # naming a handle is what HANDLE_ADDITIONS is for, and its bar is higher.
    ("/Game/Characters/Phoenix/S0/Ability_Q/Production/"
     "GameObject_Phoenix_Q_FlameWallManager_Production."
     "GameObject_Phoenix_Q_FlameWallManager_Production_C:MulticastAddSmokeScreenPoint",
     "Translation", "FieldType::VectorDouble"),
    ("/Game/Characters/Phoenix/S0/Ability_Q/Production/"
     "GameObject_Phoenix_Q_FlameWallManager_Production."
     "GameObject_Phoenix_Q_FlameWallManager_Production_C:MulticastAddSmokeScreenPoint",
     "Scale3D", "FieldType::VectorDouble"),
    # `249` on the effect-placement RPCs: the rotation that pairs with `248`.
    #
    # `248` is already here as the placement location. `249` follows it,
    # unnamed, on 441,814 rows over 20 replays, and three things settle it as a
    # `RotationShort`. The widths are 3, 19, 35 and 51 bits, which is exactly
    # `3 + 16 x (flags set)` for that type's three conditional components and
    # is not a shape any other type produces. Decoding all 441,814 that way
    # consumes every payload exactly, no leftover at any width. And the table
    # already carries `ReplayPlayContinuousEffectAtLocation.Rotation` as
    # `RotationShort` -- the same UFunction parameter, from a replay that named
    # it instead of sending the number.
    #
    # Decoded, yaw is set on 92.5% of rows, pitch on 14.5% and roll on 0.1%,
    # all on a 0.0055-degree lattice: a ground-placed effect facing somewhere.
    #
    # Not to be confused with the other `249` above, which is a `VectorDouble`.
    # That one is 192 bits under a different checksum; this family shares
    # 2526428638 and is 19 bits on most rows. Same number, different property --
    # which is the whole reason the checksum is the thing to check.
    ("/Script/ShooterGame.LocationalEffectManagerComponent:ClientPlayOneShotEffectAtLocation",
     "249", "FieldType::RotationShort"),
    ("/Script/ShooterGame.ReplayEffectComponent:ReplayPlayContinuousEffectAtLocation",
     "249", "FieldType::RotationShort"),
    ("/Script/ShooterGame.ReplayEffectComponent:ReplayPlayOneShotEffectAtLocation",
     "249", "FieldType::RotationShort"),
    ("/Script/ShooterGame.EffectManagerComponent:ReplayRecordOneShotEffect",
     "249", "FieldType::RotationShort"),
    ("/Script/ShooterGame.EffectManagerComponent:ReplayRecordContinuousEffect",
     "249", "FieldType::RotationShort"),
    # The RNG component's seed. 120,853 rows, one group, one checksum, 32 bits
    # on every row, and 120,852 of the values distinct across the full i32
    # range -- which is what a seed looks like and what a counter, a time or a
    # GUID does not. The sibling `AuthInitialRandomSeed` matches in width and
    # in that near-total distinctness.
    #
    # Int32 rather than UInt32 is not settled by the data: the same 32 bits
    # read either way. It follows Unreal's `FRandomStream`, whose seed is an
    # `int32`, and the choice only changes the sign of half the values.
    ("/Script/ShooterGame.NetworkedRandomNumberGeneratorComponent",
     "AuthCurrentRandomSeed", "FieldType::Int32"),
    ("/Script/ShooterGame.BaseTeamState", "AverageLoadoutValue", "FieldType::Int32"),
    ("/Script/ShooterGame.BaseTeamState", "LoadoutValue", "FieldType::Int32"),
    ("/Script/ShooterGame.DamageableComponent:MulticastNotifyHeal",
     "HealTaken", "FieldType::Float"),
    ("/Script/ShooterGame.DamageableComponent:MulticastNotifyOverhealDecay",
     "DecayApplied", "FieldType::Float"),
    ("/Script/ShooterGame.FiniteSpeedMovementComponent",
     "MaximumRange", "FieldType::Float"),
    ("/Script/ShooterGame.MoneyManagementComponent", "Money", "FieldType::Int32"),
    ("/Script/ShooterGame.MoneyManagementComponent", "StartOfRoundMoney", "FieldType::Int32"),
    ("/Script/ShooterGame.MoneyManagementComponent", "TotalMoneyGranted", "FieldType::Int32"),
    ("/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C",
     "Ping", "FieldType::SerializedInt { max: 65536 }"),
    ("/Script/ShooterGame.PlayerScoreComponent", "Score", "FieldType::Int32"),
    ("/Game/Characters/Components/Comp_Actor_Concussable.Comp_Actor_Concussable_C",
     "ConcussStartTime", "FieldType::Float"),
    ("/Game/Characters/Components/Comp_Actor_Concussable.Comp_Actor_Concussable_C",
     "ConcussEndTime", "FieldType::Float"),
    ("/Game/Characters/Components/Comp_Actor_Concussable.Comp_Actor_Concussable_C",
     "ConcussLevel", "FieldType::Double"),
    ("/Game/Characters/Components/Comp_AbilityFuelSystem.Comp_AbilityFuelSystem_C",
     "CurrentFuel", "FieldType::Double"),
    ("/Game/Characters/Components/Comp_AbilityFuelSystem.Comp_AbilityFuelSystem_C",
     "IsFuelDraining", "FieldType::Bool"),
    ("/Script/ShooterGame.BlindManagerComponent",
     "LongestActiveBlindDuration", "FieldType::Float"),
    ("/Script/ShooterGame.ZoomMultiplierComponent",
     "SourceFov", "FieldType::Float"),
    ("/Script/ShooterGame.ZoomMultiplierComponent",
     "SourceFov1P", "FieldType::Float"),
    ("/Script/ShooterGame.ZoomMultiplierComponent",
     "TargetFov", "FieldType::Float"),
    ("/Script/ShooterGame.ZoomMultiplierComponent",
     "TargetFov1P", "FieldType::Float"),
    ("/Script/ShooterGame.ZoomMultiplierComponent",
     "TotalTransitionTimeDuration", "FieldType::Float"),
    # UsableComponent drives every hold-to-interact object: spike plant/defuse,
    # ultimate-orb pickup, doors. HighestProgress is a 0..1 float that advances
    # 1/128 per tick (a u32 read is non-monotonic; only f32 ramps linearly);
    # bIsActive is the 1-bit "someone is interacting" flag. No C# descriptor --
    # typed from wire evidence, same bar as Ping/Money.
    ("/Script/ShooterGame.UsableComponent", "HighestProgress", "FieldType::Float"),
    ("/Script/ShooterGame.UsableComponent", "bIsActive", "FieldType::Bool"),
    # The 192-bit RPC vectors. Unreal serialises an FTransform parameter as
    # three separate double vectors on this wire -- rotation, translation,
    # scale -- and no descriptor declares any of them, so 54,859 rows arrived
    # raw. The table's `MulticastPlayContinuousEffect:Transform` entry
    # (FieldType::Transform, 320 bits) is dead against this stream: no
    # `Transform` parameter exists in the replay's own schema.
    #
    # Read as 3 x f64 they are unambiguous. `Scale3D` is exactly
    # (1.0, 1.0, 1.0) on every row, which no other reading produces -- 6 x f32
    # gives (0, 1.875, 0, 1.875, 0, 1.875). The `248` locations are map
    # coordinates in Unreal units with plausible floor heights, and `249` is a
    # rotator carrying negative zero, which a wrong split would not produce.
    #
    # Independently cross-checked: `BombPlantedRPC.PlantLocation` and
    # `MulticastActivateBombSiteEffects.BombLocation` are two unrelated RPCs
    # that report byte-identical coordinates, 9 rows each against 9
    # `spikePlanted` events.
    #
    # The replay's own `compatible_checksum` agrees with the grouping and was
    # not used to derive it: every `248` is 598402184, every `249` is
    # 747197698, every `Translation` 2235276067, every `Scale3D` 2983776962,
    # across all the groups below.
    ("/Script/ShooterGame.LocationalEffectManagerComponent:ClientPlayOneShotEffectAtLocation",
     "248", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.ReplayEffectComponent:ReplayPlayOneShotEffectAtLocation",
     "248", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.EffectManagerComponent:ReplayRecordOneShotEffect",
     "248", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.EffectManagerComponent:ReplayRecordContinuousEffect",
     "248", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.EffectManagerComponent:MulticastPlayContinuousEffect",
     "249", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.EffectManagerComponent:MulticastPlayContinuousEffect",
     "Translation", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.EffectManagerComponent:MulticastPlayContinuousEffect",
     "Scale3D", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.AresEquippable:MulticastPlayContinuousEffectFromClient",
     "249", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.AresEquippable:MulticastPlayContinuousEffectFromClient",
     "Translation", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.AresEquippable:MulticastPlayContinuousEffectFromClient",
     "Scale3D", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.EffectManagerComponent:MulticastPlayOneShotEffect",
     "249", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.EffectManagerComponent:MulticastPlayOneShotEffect",
     "Translation", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.EffectManagerComponent:MulticastPlayOneShotEffect",
     "Scale3D", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.AresEquippable:MulticastPlayOneShotEffectFromClient",
     "249", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.AresEquippable:MulticastPlayOneShotEffectFromClient",
     "Translation", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.AresEquippable:MulticastPlayOneShotEffectFromClient",
     "Scale3D", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.AresGameStateBase:MulticastResetForRespawn",
     "249", "FieldType::VectorDouble"),
    ("/Script/ShooterGame.ForceModuleManagerComponent:NetMulticastApplyForceModule",
     "SourceLocation", "FieldType::VectorDouble"),
    ("/Game/Abilities/GrenadeExplodeIndicator.GrenadeExplodeIndicator_C:MulticastTriggerExplodeIndicator",
     "IndicatorLocation", "FieldType::VectorDouble"),
    ("/Game/GameModes/Bomb/BombDestination.BombDestination_C:MulticastActivateBombSiteEffects",
     "BombLocation", "FieldType::VectorDouble"),
    ("/Game/GameModes/Components/Comp_BombEvents.Comp_BombEvents_C:BombPlantedRPC",
     "PlantLocation", "FieldType::VectorDouble"),
    ("/Game/Equippables/Finishers/Rogue/Desturctible/FXC_Rogue_Finisher_Destructible.FXC_Rogue_Finisher_Destructible_C:Set skeletal Collision",
     "Collision Static Mesh Scale", "FieldType::VectorDouble"),
    # `StopMovementTime` is the other half of the pair whose `StartMovementTime`
    # is already Float: same RPC family, same 32-bit width on all 13,316 rows,
    # and the same shape when read as f32 -- a -1.0 sentinel on 5,371 of them
    # and 0.76..136.98 on the rest, against -1.0..1771.83 for the declared
    # sibling. One entry per checksum is enough; 244888268 carries it to
    # `ReplayStopContinuousEffectAtLocation` as well.
    ("/Script/ShooterGame.EffectManagerComponent:MulticastStopContinuousEffect",
     "StopMovementTime", "FieldType::Float"),
    # `HandleNumber` identifies a force module for the later Remove/Cleanup RPC
    # to name. Read as u32 the 3,741 rows hold 1..765 with every value in that
    # range present -- a dense sequential id, which no other reading of these
    # bits produces. Checksum 3336285386 shares it with `NetMulticastRemove`
    # `ForceModule`.
    ("/Script/ShooterGame.ForceModuleManagerComponent:NetMulticastApplyForceModule",
     "HandleNumber", "FieldType::Int32"),
    # Which named area of the map a player is standing in -- "A Site", "Mid",
    # "Heaven" and so on, the same callouts the game announces. The group only
    # became reachable when the `CalloutRegionTracker` leaf was remapped, and
    # the field is an ObjectNetGuid: unpacking the raw bits of all 1,957
    # non-zero rows and looking them up in net_guids resolves 1,957 of 1,957 to
    # a `CalloutRegion_*` path, 22 distinct regions. Nothing else in the export
    # names where a player is in map terms.
    ("/Script/ShooterGame.CalloutRegionTrackingComponent",
     "CurrentRegion", "FieldType::ObjectNetGuid"),
    # The per-cast ability log: who cast what, when, and where. vrfkit already
    # flattens the array into `AbilityCastsThisRound[i].<member>` rows and the
    # replay declares every member name -- but every value arrived raw, so a
    # survey that scans typed columns walks straight past it. That is how this
    # repo concluded twice that no exact cast count exists on the wire.
    #
    # The names carry Blueprint property GUIDs, which are stable: byte-identical
    # on 13.01 and 13.02.
    #
    # Each member checks out against something outside itself. `Player` is an
    # FString whose 352 values are all 36-char UUIDs, and all 352 match a
    # `manifest.players.subject`. `Round` covers exactly 0..17, the replay's 18
    # rounds. `CastLocation` reads as 3 x f64 inside the map bounds that
    # `movement.parquet` describes. `Slot` takes four values (3, 4, 5, 9) --
    # three abilities and an ultimate. `CastTime` is seconds within the round.
    ("/Game/Characters/_Core/Comp_AbilityStatisticsReplicator.Comp_AbilityStatisticsReplicator_C",
     "Player_11_0963330440D68BDF1A8E34B035420342", "FieldType::FString"),
    ("/Game/Characters/_Core/Comp_AbilityStatisticsReplicator.Comp_AbilityStatisticsReplicator_C",
     "Slot_12_22D571914FAFD5F0EBD400B7E2F28B36", "FieldType::Byte"),
    ("/Game/Characters/_Core/Comp_AbilityStatisticsReplicator.Comp_AbilityStatisticsReplicator_C",
     "Round_22_905E6CC0448D2C6270A94C9690101E49", "FieldType::Int32"),
    ("/Game/Characters/_Core/Comp_AbilityStatisticsReplicator.Comp_AbilityStatisticsReplicator_C",
     "RoundPhase_25_84478C0047988409FEEC9E95C15DFB02", "FieldType::Byte"),
    ("/Game/Characters/_Core/Comp_AbilityStatisticsReplicator.Comp_AbilityStatisticsReplicator_C",
     "CastTime_4_5AE288704801A9B74D6D159DFC2BD147", "FieldType::Float"),
    ("/Game/Characters/_Core/Comp_AbilityStatisticsReplicator.Comp_AbilityStatisticsReplicator_C",
     "CastLocation_21_61F4B6BC47A10FE8CD34D29141FC9B88", "FieldType::VectorDouble"),
    ("/Game/Characters/_Core/Comp_AbilityStatisticsReplicator.Comp_AbilityStatisticsReplicator_C",
     "DestroyedCount_36_5936AB33418F8A2AB3A52DBF4492CF7F", "FieldType::Int32"),
    # Inside each cast, `Effects[]` records what the cast did and to whom. This
    # is the authoritative debuff log -- `EnemiesSuppressed`, `EnemiesSlowed`,
    # `EnemiesVulnerabled` and 28 more -- where the cosmetic-effect channel is
    # only a proxy for it.
    #
    # NOT the way `LifeChangeEvents`' members get their types.
    #
    # `table.rs` carries `ChangedComponent`, `LifeResult`, `DeltaLife` and
    # `bAliveAfterChange` under the `MulticastNotifyDamage_*` groups, and those
    # entries are unreachable: the names are members inside the array payload
    # and never arrive as top-level RPC parameters, so the name lookup never
    # asks for them. Changing `LifeResult` there from `Raw` to `Float` compiles,
    # passes every test, and moves no row.
    #
    # The members are typed in `crates/vrfkit/src/sink/rpc.rs`, by
    # `life_change_member_type`, on the path the array walker gives each leaf.
    # That is where to look, and where to change it.

    # `LocalizedStat`: an FText, and now typed as one.
    #
    # It was `FString` once and returned null on 3,011 of 3,011 rows, which is
    # why it was removed. The wire is an `FText` carrying a string-table entry,
    # and the key it carries is the statistic's name -- `EnemiesBlinded`,
    # `DamageDealt`, 29 distinct values across 4,341 rows, each mapping 1:1 to
    # a `Statistic` enum value.
    #
    # The note that removed it said this was not worth doing because
    # `Statistic` already carried the same fact. That was wrong in the way that
    # matters: `Statistic` decodes to a bare integer and this repository ships
    # no table mapping those integers to names -- they exist in a comment
    # below and in README prose, and nowhere a consumer can reach. This column
    # is the only machine-readable source of them.
    #
    # Evidence: 4,341 of 4,341 rows decode with zero residual bits, on the
    # layout in `decode_ftext`. The keys agree with `Statistic` without a
    # single collision.
    ("/Game/Characters/_Core/Comp_AbilityStatisticsReplicator"
     ".Comp_AbilityStatisticsReplicator_C",
     "LocalizedStat_14_C3A26F5E46CDAD94571AE6B0EDEA058B", "FieldType::FText"),

    # `AffectedPlayer` is the check: all 224 values resolve to a
    # `manifest.players.actor_net_guid`, across exactly 10 distinct players.
    # `Statistic` is a small enum whose observed values line up with the named
    # statistics (0 EnemiesBlinded, 7 EnemiesBlocked, 8 EnemiesNearsighted, ...),
    # and `Time` reads as seconds within the round like its sibling `CastTime`.
    ("/Game/Characters/_Core/Comp_AbilityStatisticsReplicator.Comp_AbilityStatisticsReplicator_C",
     "Statistic_2_0868666A4F6501815AB301BB615B2B5C", "FieldType::EnumByte"),
    ("/Game/Characters/_Core/Comp_AbilityStatisticsReplicator.Comp_AbilityStatisticsReplicator_C",
     "Value_7_E46F38AE4D245059AF7BB09E301C3C65", "FieldType::Float"),
    ("/Game/Characters/_Core/Comp_AbilityStatisticsReplicator.Comp_AbilityStatisticsReplicator_C",
     "Time_8_6CD58DD7441CBD0407DD1F89FDD05167", "FieldType::Float"),
    ("/Game/Characters/_Core/Comp_AbilityStatisticsReplicator.Comp_AbilityStatisticsReplicator_C",
     "AffectedPlayer_2_BAF988E34EAAE6B7A1D4758455186559", "FieldType::ObjectNetGuid"),
    ("/Game/Characters/_Core/Comp_AbilityStatisticsReplicator.Comp_AbilityStatisticsReplicator_C",
     "Value_5_203891704B7EF064EDB5528BFECC4807", "FieldType::Float"),
]
EXPECTED += [(g, f, t) for g, f, t in ADDITIONS]

#: Handle -> field_name additions for groups the replay never names. Each pairs
#: with an ADDITION of the same (group_path, field_name) so the overlay can type
#: the newly-named handle. Keyed on (group_path, handle) and inserted at the
#: sorted position the binary search over OVERLAY_HANDLE_TABLE requires.
HANDLE_ADDITIONS = [

]

GROUP_RE = re.compile(r'group_path: "([^"]+)"')
FIELD_RE = re.compile(r'field_name: "([^"]+)"')
TYPE_MARKER = "field_type:"


def normalize_type(field_type: str) -> str:
    """One spelling for a field type, whichever layout it was written in.

    `verify` compares types with `==`, so the two spellings rustfmt can produce
    have to collapse to one first. A struct-like type written on a single line
    is `RepMovement { rotation: X }`; broken across lines rustfmt adds a
    trailing comma before the brace, so the same type parses as
    `RepMovement { rotation: X, }`. Without this the exact comparison would
    report every braced type as wrong on a formatted table and right on a
    freshly generated one -- a check that depends on formatting is not a check.
    """
    collapsed = " ".join(field_type.rstrip().rstrip(",").split())
    return re.sub(r",\s*\}", " }", collapsed)


def _field_type_of(block: str) -> str | None:
    """The `field_type` value of one `OverlayEntry { ... }` block.

    Brace-counted rather than pattern-matched. Two things defeat a regex here,
    and both are live:

    * several field types contain their own braces
      (`VectorNetQuantize { scale: 100 }`), so a non-greedy match truncates;
    * the table exists in TWO layouts -- one entry per line as
      extract_descriptors.py emits it, and the rustfmt'd multi-line form that
      gets committed. A pattern anchored on a newline works on one and
      silently matches nothing on the other.

    That second case is not hypothetical: the first version of this function
    required a newline, so it reported all 25 corrections missing on a
    freshly generated table -- exactly when this script is supposed to run.

    The block has already had its opening `OverlayEntry {` consumed, so the
    first unmatched `}` is the one that closes the entry.
    """
    start = block.find(TYPE_MARKER)
    if start == -1:
        return None
    start += len(TYPE_MARKER)
    depth = 0
    for i, ch in enumerate(block[start:], start):
        if ch == "{":
            depth += 1
        elif ch == "}":
            if depth == 0:
                return normalize_type(block[start:i])
            depth -= 1
    return None


def parse_entries(content: str):
    """Yield (group_path, field_name, field_type) for every OverlayEntry.

    Split per `OverlayEntry {` block first so each lookup is scoped to one
    entry and cannot bleed into its neighbour.
    """
    for block in content.split("    OverlayEntry {")[1:]:
        group = GROUP_RE.search(block)
        field = FIELD_RE.search(block)
        ftype = _field_type_of(block)
        if group and field and ftype:
            yield group.group(1), field.group(1), ftype


def apply_additions(content: str) -> tuple[str, int]:
    """Insert every entry in `ADDITIONS` that is not already present.

    Insertion, not replacement, so it cannot reuse the `.replace()` shape the
    corrections above use -- there is nothing to replace.

    The slice is sorted by `(group_path, field_name)` and
    `tests::overlay::table_is_sorted` enforces it, so a new entry goes at its
    sorted position rather than at the end. We find the first existing entry
    that sorts AFTER ours and splice in before it; if no such entry exists we
    fail loudly rather than append, because appending past the final block
    would write into the `];` that closes the slice.
    """
    added = 0
    for group, field, ftype in ADDITIONS:
        blocks = content.split("    OverlayEntry {")
        keys = []
        for block in blocks[1:]:
            g, f = GROUP_RE.search(block), FIELD_RE.search(block)
            keys.append((g.group(1), f.group(1)) if g and f else ("", ""))
        if (group, field) in keys:
            continue

        target = next((i for i, k in enumerate(keys) if k > (group, field)), None)
        entry = (
            "    OverlayEntry {\n"
            f'        group_path: "{group}",\n'
            f'        field_name: "{field}",\n'
            f"        field_type: {ftype},\n"
            "    },\n"
        )
        if target is None:
            # The new entry is the new tail of OVERLAY_TABLE. Splice it in
            # before the `];` that closes that slice. The split puts the final
            # block (blocks[-1]) as the last OverlayEntry's text followed by
            # `];` and the OVERLAY_HANDLE_TABLE that comes after, so the first
            # `\n];` in that block is the OVERLAY_TABLE close. Refusing here
            # used to be the safe choice, but a sorted table whose last group
            # is new (e.g. ZoomMultiplierComponent) cannot gain entries without
            # it, so the append is handled rather than rejected.
            last = blocks[-1]
            close = last.find("\n];")
            if close == -1:
                raise SystemExit(
                    f"{TABLE_RS}: {group}/{field} would append but the "
                    f"OVERLAY_TABLE closing '];' could not be located."
                )
            head = (
                "    OverlayEntry {".join(blocks[:-1])
                + "    OverlayEntry {"
                + last[:close]
            )
            content = head + entry + last[close:]
        else:
            # blocks[target + 1] is the block for `keys[target]`; put the new
            # entry in front of the marker that introduces it.
            head = "    OverlayEntry {".join(blocks[: target + 1])
            tail = (
                "    OverlayEntry {"
                + "    OverlayEntry {".join(blocks[target + 1:])
            )
            content = head + entry + tail
        added += 1
    return content, added


#: The weapon half of the "215"/"216" correction, which EXPECTED cannot list.
#:
#: The pass discovers its targets from the table itself -- every group under
#: `/Game/Equippables/` that carries one of these fields, 18 of them today --
#: so a hardcoded list would be a second, drifting copy of that discovery.
#: EXPECTED's "215"/"216" rows name the five HARDCODED non-weapon groups and
#: nothing else, so before this the weapon pass had no check at all: it could
#: match nothing, print "Applied 0" and exit 0, which is indistinguishable from
#: having had nothing to do. The expectation is therefore derived from the same
#: table the pass ran over.
WEAPON_GROUP_MARKER = "/Game/Equippables/"
ACTOR_BOOKKEEPING_FIELDS = ("215", "216")
ACTOR_BOOKKEEPING_TYPE = "FieldType::EnumRemainingBits"


def weapon_expectations(content: str) -> list[tuple[str, str, str]]:
    """Every `/Game/Equippables/` "215"/"216" entry, with the type it has."""
    return sorted(
        (g, f, t)
        for g, f, t in parse_entries(content)
        if WEAPON_GROUP_MARKER in g and f in ACTOR_BOOKKEEPING_FIELDS
    )


def expectation_count(content: str) -> int:
    """How many corrections `verify` actually checks against `content`."""
    return len(EXPECTED) + len(weapon_expectations(content))


def verify(content: str) -> list[str]:
    """Return one line per correction that is NOT present in `content`.

    EVERY hit has to carry the required type, not merely one of them. The group
    is matched by substring, so `"SmokeScreen"` reaches both `SmokeScreen` and
    `SmokeScreenManager`; under an `any()` a wrong-typed entry verified clean
    whenever a sibling group happened to be right -- the same shape of hole as
    the substring type test this function used to do.
    """
    entries = list(parse_entries(content))
    problems = []
    for group_part, field, required in EXPECTED:
        hits = [ft for g, f, ft in entries if group_part in g and f == field]
        if not hits:
            problems.append(f"{field} in *{group_part}*: entry not found at all")
        elif any(ft != required for ft in hits):
            problems.append(
                f"{field} in *{group_part}*: expected {required}, found {hits}"
            )

    weapons = weapon_expectations(content)
    if not weapons:
        problems.append(
            f"no {WEAPON_GROUP_MARKER} entry named "
            f"{' or '.join(ACTOR_BOOKKEEPING_FIELDS)} exists at all: the weapon "
            f"pass had nothing to convert, which is exactly what its discovery "
            f"going dead looks like"
        )
    for group, field, ftype in weapons:
        if ftype != ACTOR_BOOKKEEPING_TYPE:
            problems.append(
                f"{field} in {group}: expected {ACTOR_BOOKKEEPING_TYPE}, "
                f"found {ftype}"
            )
    return problems


#: The two generated header lines this script has to keep true, in file order.
#: `rewrite_header` recounts both from the parsed entries and `--check` fails
#: when either disagrees.
HEADER_RES = (
    re.compile(r"^// \d+ entries from \d+ groups\.$", re.MULTILINE),
    re.compile(r"^// Raw/Custom: \d+, Skip: \d+, Typed: \d+\.$", re.MULTILINE),
)

#: The slice is declared with an explicit length, so an addition that does not
#: update it does not compile. That is the good failure -- it is caught by
#: `cargo build` rather than by anyone noticing -- but the script should not
#: hand over a file that cannot build.
TABLE_LEN_RE = re.compile(r"(pub static OVERLAY_TABLE: \[OverlayEntry; )(\d+)(\])")


def resync_table_len(content: str) -> str:
    """Rewrite the declared `OVERLAY_TABLE` length to the entries present."""
    n = sum(1 for _ in parse_entries(content))
    new_content, hits = TABLE_LEN_RE.subn(
        lambda m: f"{m.group(1)}{n}{m.group(3)}", content, count=1
    )
    if hits != 1:
        raise SystemExit(
            f"{TABLE_RS}: expected exactly one OVERLAY_TABLE length declaration, "
            f"found {hits}."
        )
    return new_content


HANDLE_GROUP_RE = re.compile(r'group_path: "([^"]+)"')
HANDLE_NUM_RE = re.compile(r"handle: (\d+)")


def parse_handle_entries(content: str):
    """Yield (group_path, handle, field_name) for every OverlayHandleEntry."""
    for block in content.split("    OverlayHandleEntry {")[1:]:
        g = HANDLE_GROUP_RE.search(block)
        h = HANDLE_NUM_RE.search(block)
        f = FIELD_RE.search(block)
        if g and h and f:
            yield g.group(1), int(h.group(1)), f.group(1)


def apply_handle_additions(content: str) -> tuple[str, int]:
    """Insert every OverlayHandleEntry in HANDLE_ADDITIONS not already present.

    Mirrors `apply_additions` but keys on `(group_path, handle)` and writes into
    the OVERLAY_HANDLE_TABLE slice. Some groups (e.g. `MagazineAmmo`) are never
    given field names by the replay or the C# descriptors, so the handle table
    is the only place that can name them -- and without a name the overlay
    cannot type the handle.
    """
    added = 0
    for group, handle, field in HANDLE_ADDITIONS:
        blocks = content.split("    OverlayHandleEntry {")
        keys = []
        for block in blocks[1:]:
            g, h = HANDLE_GROUP_RE.search(block), HANDLE_NUM_RE.search(block)
            keys.append((g.group(1), int(h.group(1))) if g and h else ("", -1))
        if (group, handle) in keys:
            continue

        target = next((i for i, k in enumerate(keys) if k > (group, handle)), None)
        entry = (
            "    OverlayHandleEntry {\n"
            f'        group_path: "{group}",\n'
            f"        handle: {handle},\n"
            f'        field_name: "{field}",\n'
            "    },\n"
        )
        if target is None:
            # OVERLAY_HANDLE_TABLE is the last slice in the file; splice the new
            # entry in before its closing `];`.
            last = blocks[-1]
            close = last.rfind("\n];")
            if close == -1:
                raise SystemExit(
                    f"{TABLE_RS}: {group}/handle {handle} would append but the "
                    f"OVERLAY_HANDLE_TABLE closing '];' could not be located."
                )
            head = (
                "    OverlayHandleEntry {".join(blocks[:-1])
                + "    OverlayHandleEntry {"
                + last[:close]
            )
            content = head + entry + last[close:]
        else:
            head = "    OverlayHandleEntry {".join(blocks[: target + 1])
            tail = (
                "    OverlayHandleEntry {"
                + "    OverlayHandleEntry {".join(blocks[target + 1:])
            )
            content = head + entry + tail
        added += 1
    return content, added


HANDLE_TABLE_LEN_RE = re.compile(
    r"(pub static OVERLAY_HANDLE_TABLE: \[OverlayHandleEntry; )(\d+)(\])"
)


def resync_handle_table_len(content: str) -> str:
    """Rewrite the declared `OVERLAY_HANDLE_TABLE` length to the entries present."""
    n = sum(1 for _ in parse_handle_entries(content))
    new_content, hits = HANDLE_TABLE_LEN_RE.subn(
        lambda m: f"{m.group(1)}{n}{m.group(3)}", content, count=1
    )
    if hits != 1:
        raise SystemExit(
            f"{TABLE_RS}: expected exactly one OVERLAY_HANDLE_TABLE length "
            f"declaration, found {hits}."
        )
    return new_content


def rewrite_header(content: str) -> tuple[str, tuple[str, ...]]:
    """Recount the table and rewrite both generated header lines.

    extract_descriptors.py writes them from the descriptors it read, and then
    this script changes some of those types and ADDS entries the descriptors
    never declared -- so between the two the header is a statement about a table
    that no longer exists.

    The bucket line said "Raw/Custom: 164 ... Typed: 864" while the file held
    157 and 871: the seven corrections that turn a Raw into a real type. The
    shape line above it said "1185 entries from 171 groups" while the file held
    1188 from 172 -- and, worse, while the bucket line one row below it summed
    to 1188. Three consecutive lines, two of them recounted here and the third
    left to rot, contradicting each other in the same paragraph.

    Nothing reads the header, which is exactly why it went unnoticed and why it
    is worth fixing -- a comment on a generated file that quietly disagrees with
    the file is how a reader learns not to trust the comments.

    Counted from the parsed entries rather than by substring, so a type whose
    name contains another's (`FieldType::RawPayload` would contain `Raw`)
    cannot miscount.
    """
    buckets = Counter()
    groups = set()
    for group, _field, ftype in parse_entries(content):
        groups.add(group)
        if ftype == "FieldType::Raw":
            buckets["raw"] += 1
        elif ftype == "FieldType::Skip":
            buckets["skip"] += 1
        else:
            buckets["typed"] += 1

    lines = (
        f"// {sum(buckets.values())} entries from {len(groups)} groups.",
        f"// Raw/Custom: {buckets['raw']}, Skip: {buckets['skip']}, "
        f"Typed: {buckets['typed']}.",
    )
    for pattern, line in zip(HEADER_RES, lines):
        content, n = pattern.subn(lambda _m, _l=line: _l, content, count=1)
        if n != 1:
            raise SystemExit(
                f"{TABLE_RS}: expected exactly one generated header line "
                f"matching {pattern.pattern!r}, found {n}. Regenerate with "
                f"extract_descriptors.py first."
            )
    return content, lines


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify without writing")
    return parser.parse_args(argv)


def main():
    check_only = parse_args().check
    content = TABLE_RS.read_text(encoding="utf-8")
    # The file exactly as committed. Every pass below rewrites `content`, so by
    # the end it is the CORRECTED COPY -- and verifying that copy is what made
    # `--check` unable to tell an already-corrected table from a correctable one.
    on_disk = content
    count = 0

    # Fix: Float -> Double for time-related fields (verified: 64-bit on wire)
    float_to_double = [
        ('TimedBomb.TimedBomb_C", field_name: "TimeRemainingToExplode", field_type: FieldType::Float',
         'TimedBomb.TimedBomb_C", field_name: "TimeRemainingToExplode", field_type: FieldType::Double'),
        ('TimedBomb.TimedBomb_C", field_name: "DefuseProgress", field_type: FieldType::Float',
         'TimedBomb.TimedBomb_C", field_name: "DefuseProgress", field_type: FieldType::Double'),
        ('Comp_Ability_CooldownComponent_C", field_name: "StartTimeStamp", field_type: FieldType::Float',
         'Comp_Ability_CooldownComponent_C", field_name: "StartTimeStamp", field_type: FieldType::Double'),
        ('Comp_Ability_CooldownComponent_C", field_name: "CooldownSeconds", field_type: FieldType::Float',
         'Comp_Ability_CooldownComponent_C", field_name: "CooldownSeconds", field_type: FieldType::Double'),
    ]
    for old, new in float_to_double:
        if old in content:
            content = content.replace(old, new)
            count += 1

    # Fix: every group's "215"/"216" is EnumRemainingBits, weapons included.
    #
    # These are not handles. They are field *names* -- the decimal spelling of
    # a hardcoded Unreal FName index the replay never resolves to text (see
    # `read_fname` in vrf-schema) -- and they appear on 370 groups.
    #
    # The block below used to say "Weapons use Raw (correct)" and nothing had
    # checked. The wire says otherwise, and the checksum is what settles it:
    # `"215"` carries 1710918439 and `"216"` carries 4109980037 on every group
    # that sends them, decoding or not, at a uniform 3 bits. Unreal hashes the
    # property's type into that number, so one checksum is one property; `Raw`
    # on a weapon was never a second type, only an unverified guess. Where it
    # does decode the value is 3 and 1 without exception.
    #
    # It cost 48,010 rows over 20 replays, and it was invisible: a name hit
    # wins in `resolve_in_group` before the checksum fallback is consulted, so
    # `Raw` blocked the very mechanism that would have caught it, and the rows
    # counted as "raw/skip" rather than as an error.
    #
    # Two passes because the two sets arrive spelled differently -- the
    # non-weapon groups are declared Int32 by the C# descriptor, the weapon
    # groups Raw. A single Int32-matching pass silently does nothing to the
    # weapons: the string it looks for is not there, so no substitution is made
    # and no counter moves.
    weapon_groups_215_216 = [
        line.split('group_path: "')[1].split('"')[0]
        for line in content.splitlines()
        if 'group_path: "/Game/Equippables/' in line
    ]
    for g in sorted(set(weapon_groups_215_216)):
        for field in ["215", "216"]:
            old = f'{g}", field_name: "{field}", field_type: FieldType::Raw'
            new = f'{g}", field_name: "{field}", field_type: FieldType::EnumRemainingBits'
            if old in content:
                content = content.replace(old, new)
                count += 1

    groups_215_216 = [
        "TimedBomb.TimedBomb_C",
        "EquippablePickupProjectile.EquippablePickupProjectile_C",
        "EquippableGroundPickup.EquippableGroundPickup_C",
        "OwnerExclusivePlayerInfo",
        "Projectile_Phoenix_Q_FlameWall_ThroughWall.Projectile_Phoenix_Q_FlameWall_ThroughWall_C",
    ]
    for g in groups_215_216:
        for field in ["215", "216"]:
            old = f'{g}", field_name: "{field}", field_type: FieldType::Int32'
            new = f'{g}", field_name: "{field}", field_type: FieldType::EnumRemainingBits'
            if old in content:
                content = content.replace(old, new)
                count += 1

    # Fix: rotation quantization for the Astra smoke-screen projectiles.
    #
    # `ProjectileSmokeScreenDescriptor.cs` calls `.ReplicatedMovement()` with no
    # argument, which defaults to ShortComponents (16 bits per rotation axis).
    # Every other projectile descriptor in the same codebase passes
    # ByteComponents explicitly -- FlameWall, MageWall, NeonTunnel and
    # EquippablePickup -- so the bare call reads as an oversight rather than a
    # deliberate difference.
    #
    # The wire agrees with the other projectiles: on release-13.01 these payloads
    # arrive at 113-124 bits and a ShortComponents read runs off the end (137 EOF
    # failures on one replay, every one of them from this single group).
    #
    # Entries span several lines, so rewrite per `OverlayEntry { .. }` block
    # rather than per line: a line-wise match cannot see the group path and the
    # field type at the same time.
    blocks = content.split("    OverlayEntry {")
    for i, block in enumerate(blocks):
        if i == 0:
            continue
        if "SmokeScreen" not in block or 'field_name: "ReplicatedMovement"' not in block:
            continue
        if "RotatorQuantization::ShortComponents" in block:
            blocks[i] = block.replace(
                "RotatorQuantization::ShortComponents",
                "RotatorQuantization::ByteComponents",
            )
            count += 1
    content = "    OverlayEntry {".join(blocks)

    # REMOVED: FName -> Raw for DamagedBone in MulticastNotifyDamage_Point.
    #
    # This pass existed because 177 of 581 payloads arrive at 9 bits, which
    # the FName decoder could not read. Its comment claimed the value "is
    # always bone index 0" -- generalised from those 177 and wrong: the
    # reference has 22 distinct bone names (Head 69, Spine4 51, L_Shoulder
    # 46, ...) and forcing Raw made us ship mojibake for all 581.
    #
    # The real cause was decode_fname ignoring the isHardcoded bit. Fixed in
    # vrf-decode, so the field decodes as the FName the C# descriptor
    # declares and needs no correction here.

    # Fix: EnumByte -> Raw for AresEquippableDataTracker.OriginalBuyerTeam.
    # C# declares this as EnumByte (single byte), but on wire it arrives as
    # 97-105 bits consistently (248 occurrences in 02d4d478). This is likely
    # a serialized FastArray entry or struct, not a bare enum. Mark Raw.
    blocks = content.split("    OverlayEntry {")
    for i, block in enumerate(blocks):
        if i == 0:
            continue
        if "AresEquippableDataTracker" not in block:
            continue
        if 'field_name: "OriginalBuyerTeam"' not in block:
            continue
        if "FieldType::EnumByte" in block:
            blocks[i] = block.replace("FieldType::EnumByte", "FieldType::Raw")
            count += 1
    content = "    OverlayEntry {".join(blocks)

    # Fix: Raw -> ObjectNetGuid for EquippableUsed on both damage RPCs.
    #
    # Not a wire/declaration mismatch like the others above -- the declaration
    # is simply invisible to the extractor. DamageParameters.cs:51 attaches a
    # custom decoder:
    #
    #   AddPropertyHandle(7, x => x.EquippableUsed, ...)
    #       .Decode(ValorantPayloadDecoders.Equippable)
    #
    # and that decoder (ValorantPayloadDecoders.cs:158) is exactly
    #
    #   var netGuid = archive.ReadIntPacked();
    #
    # which is what FieldType::ObjectNetGuid already implements. Because
    # extract_descriptors.py cannot see through .Decode(...), the field lands
    # here as Raw, and every consumer has to guess the encoding.
    #
    # Verified on 02d4d478 across all 632 occurrences: read as IntPacked the
    # values are 116 distinct and 100% even -- the engine requires dynamic
    # NetGUIDs to be even (IsDynamic => (Value & 1) == 0) -- and 114 of 115
    # resolve to a weapon class path in actors.parquet. The bits are 8, 16 or
    # 24 wide depending on the value, so any fixed-width read is wrong by
    # construction.
    blocks = content.split("    OverlayEntry {")
    for i, block in enumerate(blocks):
        if i == 0:
            continue
        if "DamageableComponent:MulticastNotifyDamage_" not in block:
            continue
        if 'field_name: "EquippableUsed"' not in block:
            continue
        if "FieldType::Raw" in block:
            blocks[i] = block.replace("FieldType::Raw", "FieldType::ObjectNetGuid")
            count += 1
    content = "    OverlayEntry {".join(blocks)

    # Fix: Raw -> quantized vectors for the damage geometry fields.
    #
    # Same invisibility problem as EquippableUsed above: these are attached
    # with .Decode(ValorantPayloadDecoders.VectorNetQuantize*(...)), so the
    # extractor sees a custom decoder and emits Raw, even though vrf-decode
    # already implements the exact quantization.
    #
    # Scales come from the C# call sites, not from guesswork:
    #   DamageParameters.cs:50                    VectorNetQuantize100
    #   MulticastNotifyDamagePointParameters.cs:40 VectorNetQuantizeNormal
    #   MulticastNotifyDamagePointParameters.cs:42 VectorNetQuantize
    #   MulticastNotifyDamagePointParameters.cs:44 VectorNetQuantizeNormal
    #   MulticastNotifyDamagePointParameters.cs:46 VectorNetQuantize
    #
    # Confirmed by the reference bundle's own output: DamageImpactLocation is
    # integral (scale 1), DamageOrigin carries two decimals (scale 100), and
    # DamageDirection / DamageImpactNormal are unit vectors.
    damage_vectors = {
        "DamageOrigin": "FieldType::VectorNetQuantize { scale: 100 }",
        "DamageImpactLocation": "FieldType::VectorNetQuantize { scale: 1 }",
        "DamageImpactBoneRelativeLocation": "FieldType::VectorNetQuantize { scale: 1 }",
        "DamageDirection": "FieldType::VectorNetQuantizeNormal",
        "DamageImpactNormal": "FieldType::VectorNetQuantizeNormal",
    }
    blocks = content.split("    OverlayEntry {")
    for i, block in enumerate(blocks):
        if i == 0:
            continue
        if "DamageableComponent:MulticastNotifyDamage_" not in block:
            continue
        for field, new_type in damage_vectors.items():
            if f'field_name: "{field}"' not in block:
                continue
            if "FieldType::Raw" in block:
                blocks[i] = block.replace("FieldType::Raw", new_type)
                count += 1
            break
    content = "    OverlayEntry {".join(blocks)

    # Additions last, so the bucket recount below sees them.
    content, n_added = apply_additions(content)
    count += n_added
    content = resync_table_len(content)

    content, n_handle_added = apply_handle_additions(content)
    count += n_handle_added
    content = resync_handle_table_len(content)

    content, header_lines = rewrite_header(content)

    # The operation count is a diagnostic, not the verdict. 0 is correct when
    # the table was already corrected and wrong when the patterns are dead;
    # only the end state distinguishes them.
    #
    # TWO end states, and they answer different questions:
    #
    #   dead        `content` is the corrected copy, so a correction missing
    #               HERE could not be applied at all -- the pattern it matches
    #               on is gone. That is the failure this script was rewritten
    #               to catch and its message is unchanged.
    #   uncorrected present in the corrected copy and absent from the FILE, so
    #               the passes can fix it and nobody ran them. `--check` used
    #               to verify only the copy, so this was silently clean --
    #               and CI runs `--check`, which is how a regenerated table.rs
    #               went green while the Rust build used the uncorrected one.
    #
    # A regenerated table trips both at once (the one-line passes are dead in
    # the rustfmt'd layout while the block-based ones apply fine in memory), so
    # both sections print rather than one hiding the other.
    dead = verify(content)
    uncorrected = (
        [p for p in verify(on_disk) if p not in dead]
        if check_only else []
    )
    checked = expectation_count(content)
    if dead or uncorrected:
        if dead:
            print(f"FAILED: {len(dead)} of {checked} corrections are "
                  f"missing from {TABLE_RS}", file=sys.stderr)
            for line in dead:
                print(f"  {line}", file=sys.stderr)
            print("If table.rs was regenerated, run extract_descriptors.py, "
                  "then THIS script, and only then cargo fmt.", file=sys.stderr)
        if uncorrected:
            print(f"FAILED: {len(uncorrected)} of {checked} corrections are "
                  f"absent from {TABLE_RS} but ARE applied by this script -- "
                  f"the file was never corrected.", file=sys.stderr)
            for line in uncorrected:
                print(f"  {line}", file=sys.stderr)
            print("Run this script WITHOUT --check, then cargo fmt, and commit "
                  "the result.", file=sys.stderr)
        return 1

    if check_only:
        for pattern, line in zip(HEADER_RES, header_lines):
            stale = pattern.search(on_disk)
            if stale and stale.group(0) != line:
                print(f"FAILED: the generated header disagrees with the table.\n"
                      f"  file says {stale.group(0)}\n"
                      f"  counted   {line}", file=sys.stderr)
                return 1
    else:
        atomic_write_text(TABLE_RS, content)

    verb = "verified" if check_only else "applied"
    summary = "; ".join(line.lstrip("/ ").rstrip(".") for line in header_lines)
    print(f"{verb}: {count} replacement(s) made, "
          f"all {checked} corrections present in {TABLE_RS}; {summary}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
