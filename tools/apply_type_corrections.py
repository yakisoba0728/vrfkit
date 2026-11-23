"""Pin measured field types in crates/vrf-decode/src/table.rs.

A pin is `(group, field, FieldType)` with its evidence beside it. A CORRECTION
types an entry the wire contradicted; an ADDITION types a field no declaration
covered, under the bar written above `ADDITIONS`. An exact pin is set, and
inserted when absent; a glob pin (`*`) sets every entry it matches and must
match at least one. Applying also sorts the table and resyncs its length.

Usage:
    python tools/apply_type_corrections.py            # apply every pin
    python tools/apply_type_corrections.py --check    # fail unless applying changes nothing
"""
import argparse
import difflib
import fnmatch
import re
import sys
from collections import Counter
from pathlib import Path

from atomic_io import atomic_write_text

TABLE_RS = Path(__file__).parent.parent / "crates" / "vrf-decode" / "src" / "table.rs"


def pins(spec) -> list[tuple[str, str, str]]:
    """`(group, field, FieldType::T)` rows from `(group, T, field or fields)`."""
    return [(group, field, f"FieldType::{ftype}")
            for group, ftype, fields in spec
            for field in ((fields,) if isinstance(fields, str) else fields)]


def rep_movement(rotation: str, location: str) -> str:
    return (f"RepMovement {{ rotation: RotatorQuantization::{rotation}, "
            f"location: VectorQuantization::{location} }}")


BYTE_WHOLE = rep_movement("ByteComponents", "RoundWholeNumber")
TIMED_BOMB = "/Game/GameModes/Bomb/TimedBomb.TimedBomb_C"
COMP_BOMB_EVENTS = "/Game/GameModes/Components/Comp_BombEvents.Comp_BombEvents_C"
DAMAGE = "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_"

#: Gekko's Wingman, the table's one two-decimal ReplicatedMovement (the other
#: pawns are scoped): its first update is 100x the actors.parquet spawn on all
#: 932 actors (1,018 replays, 15 builds), every component within 0.0502 after /100.
SEEKER_NADE_GROUP = (
    "/Game/Characters/AggroBot/S0/Ability_Q/Pawn_Aggrobot_SeekerNade."
    "Pawn_Aggrobot_SeekerNade_C"
)

#: Five AGameObject classes read with byte rotator components. None ever sends
#: a rotation, so the wire cannot choose: the prior is the native class
#: (AGameObject rotations decode Byte 4/4, AShooterCharacter Short 7/7), and all
#: 903 replays declaring them export byte-identically either way. A rotation
#: seen on the wire overrules it.
GAME_OBJECT_BYTE_ROTATOR_GROUPS = (
    "/Game/Characters/Mage/S0/Ability_E/GameObject_Mage_E_WorldSmoke."
    "GameObject_Mage_E_WorldSmoke_C",
    "/Game/Characters/Smonk/S0/Ability_E/MapTargetSmoke/GameObject_Smonk_NewSmoke."
    "GameObject_Smonk_NewSmoke_C",
    "/Game/Characters/Smonk/S0/Ability_E/MapTargetSmoke/GameObject_Smonk_NewSmoke_PDS."
    "GameObject_Smonk_NewSmoke_PDS_C",
    "/Game/Characters/Smonk/S0/Ability_Q/DebuffKnife/DecayLauncher/"
    "GameObject_Smonk_Q_DecayExplosion.GameObject_Smonk_Q_DecayExplosion_C",
    "/Game/Characters/Wraith/S0/Ability_4/Zone_Wraith_4_Smoke.Zone_Wraith_4_Smoke_C",
)

#: The four `EffectID`s, `FEffectID.EffectID: int64`: each checksum reproduces
#: with int64, not uint64 (test_compatible_checksum_facts.py), and every row over
#: the 1,018 replays is 64 bits with bit 63 clear, so no value changes.
#: checksum_table.rs learns 2340855891 from them (`extract_checksum_types.py --retype`).
EFFECT_ID_INT64 = (
    "/Script/ShooterGame.EffectManagerComponent",
    "/Script/ShooterGame.EffectManagerComponent:MulticastPlayContinuousEffect",
    "/Script/ShooterGame.EffectManagerComponent:MulticastUpdateContinuousEffect",
    "/Script/ShooterGame.ReplayEffectComponent:ReplayPlayContinuousEffectAtLocation",
)

CORRECTIONS = pins([
    # Time fields 64 bits on the wire.
    (TIMED_BOMB, "Double", ("TimeRemainingToExplode", "DefuseProgress")),
    ("/Game/Characters/Components/Comp_Ability_CooldownComponent."
     "Comp_Ability_CooldownComponent_C", "Double", ("StartTimeStamp", "CooldownSeconds")),
    # "215"/"216" are hardcoded FName indices, not handles: on every group that
    # sends them they carry 1710918439 / 4109980037 at 3 bits and decode to 3
    # and 1. The weapon groups are WEAPON_PINS.
    *[(group, "EnumRemainingBits", ("215", "216")) for group in (
        TIMED_BOMB,
        "/Game/Weapons/WeaponPickups/EquippablePickupProjectile.EquippablePickupProjectile_C",
        "/Game/Weapons/WeaponPickups/EquippableGroundPickup.EquippableGroundPickup_C",
        "/Script/ShooterGame.OwnerExclusivePlayerInfo",
        "/Game/Characters/Phoenix/S0/Ability_Q/Production/"
        "Projectile_Phoenix_Q_FlameWall_ThroughWall.Projectile_Phoenix_Q_FlameWall_ThroughWall_C",
    )],
    # 32 bits on all 32,978,229 rows in 42 character/pawn groups (714 exports):
    # a finite, actor-monotonic Float in seconds of server time, offset from
    # replay time per file (~10 s on 672 of 714), so estimate the offset.
    ("/Game/Characters/*", "Float", "ReplayLastTransformUpdateTimeStamp"),
    # The smoke-screen projectile's 113-124-bit payloads: a Short rotator read
    # runs off the end (137 EOF failures on one replay).
    ("*SmokeScreen*", BYTE_WHOLE, "ReplicatedMovement"),
    (SEEKER_NADE_GROUP, rep_movement("ShortComponents", "RoundTwoDecimals"),
     "ReplicatedMovement"),
    *[(group, BYTE_WHOLE, "ReplicatedMovement") for group in GAME_OBJECT_BYTE_ROTATOR_GROUPS],
    *[(group, "Int64", "EffectID") for group in EFFECT_ID_INT64],
    # No row is 8 bits: the only two payloads, 97 and 105 bits, are the inline
    # FNames "Red" and "Blue", which an independent reader consumes exactly on
    # all 748,381 rows. The team name as sent, mapped to no side.
    ("/Script/ShooterGame.AresEquippableDataTracker", "FName", "OriginalBuyerTeam"),
    # The damage RPCs' payload types. EquippableUsed: all 632 values on 02d4d478
    # are 8/16/24 bits and even (dynamic NetGUIDs), 114 of 115 resolving to a
    # weapon. Scales: impact location whole units, origin two decimals,
    # direction and normal unit vectors.
    (DAMAGE + "Base", "ObjectNetGuid", "EquippableUsed"),
    (DAMAGE + "Point", "ObjectNetGuid", "EquippableUsed"),
    (DAMAGE + "Base", "VectorNetQuantize { scale: 100 }", "DamageOrigin"),
    (DAMAGE + "Point", "VectorNetQuantize { scale: 100 }", "DamageOrigin"),
    (DAMAGE + "Point", "VectorNetQuantize { scale: 1 }",
     ("DamageImpactLocation", "DamageImpactBoneRelativeLocation")),
    (DAMAGE + "Point", "VectorNetQuantizeNormal", ("DamageDirection", "DamageImpactNormal")),
    # Consumed exactly as IntPacked; resolved IDs name transition-context
    # objects, and null stays zero.
    ("/Script/ShooterGame.EquippableStateMachineComponent", "ObjectNetGuid",
     "TransitionContext"),
    # One property (checksum 2270825073) whose other donors are EnumByte: all
    # 16,030,813 rows are 3 bits within EAresAlliance 0..5, so EnumByte keeps
    # every value and lets the checksum type the five RPCs that carry it.
    ("/Script/ShooterGame.ReplayEffectComponent:ReplayPlayContinuousEffectAtLocation",
     "EnumByte", "AllianceFilter"),
    # 959,445 rows each, every one exact IntPacked, 8-bit rows 0x00. Override's
    # non-zero values are odd (static), each resolving to one of 190 FXC_*
    # finisher classes; Context's are even and resolve to a *_PC_C pawn on
    # 62,162 of 62,162. The handle drifts by build, so the name keys it.
    (DAMAGE + "Base", "ObjectNetGuid",
     ("DeathMontageEffectOverride", "DeathMontageEffectOverrideContext")),
    (DAMAGE + "Point", "ObjectNetGuid",
     ("DeathMontageEffectOverride", "DeathMontageEffectOverrideContext")),
    # No DamagedBone pin: its FName decodes, 9-bit hardcoded names included
    # (177 of 581 payloads).
])

#: The weapon half of "215"/"216": every `/Game/Equippables/` entry carrying
#: one (36 today), each counted in `expectation_count`.
WEAPON_PINS = pins([("/Game/Equippables/*", "EnumRemainingBits", ("215", "216"))])

BOMB_PLAYER_STATE = "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C"
TIDAL_WAVE_CHUNK = ("/Game/Characters/Mage/S0/Ability_X/GameObject_Mage_X_TidalWave_Chunk."
                    "GameObject_Mage_X_TidalWave_Chunk_C")
FLAME_WALL = ("/Game/Characters/Phoenix/S0/Ability_Q/Production/"
              "GameObject_Phoenix_Q_FlameWallManager_Production."
              "GameObject_Phoenix_Q_FlameWallManager_Production_C:MulticastAddSmokeScreenPoint")
FORCE_MODULE = "/Script/ShooterGame.ForceModuleManagerComponent:NetMulticastApplyForceModule"
ABILITY_STATS = ("/Game/Characters/_Core/Comp_AbilityStatisticsReplicator."
                 "Comp_AbilityStatisticsReplicator_C")
HAWK_FLASH = ("/Game/Characters/Guide/S0/Ability_E/Projectile_Guide_E_HawkFlash."
              "Projectile_Guide_E_HawkFlash_C")

#: Fields no declaration types, typed anyway. The bar is higher than for a
#: correction: complete, self-checking wire evidence -- one bit width on every
#: row and a value distribution a wrong type cannot produce -- or a checksum or
#: sibling that fixes the type. Never widen this list by eye. Keys are group +
#: name, not checksum: decode_byte fails only at 0 or >8 bits, so a reused name
#: of <=8 bits would decode silently wrong. Swiftplay's game state reaches the
#: Bomb class through GROUP_ALIASES.
#:
#: DELIBERATELY NOT ADDED -- each failed the bar and keeps its raw bits:
#: * BaseTeamState Wins, Points, InitialRole, TeamRole, TeamPlayerStates,
#:   TeamComponent, TeamExclusiveTeamInfo: no source for a type.
#: * AbilityCastsThisRound as a scalar: 16 to 6712 bits, a struct array (its
#:   members are typed below).
#: * Comp_AbilityFuelSystem FuelFull/FuelEmpty: 0-bit payloads.
#: * BlindManagerComponent.ActiveBlinds, DamageableComponent LifeChangeBySection
#:   and FiniteSpeedMovementComponent.RequestedIgnoreActors: variable-width arrays.
#: * ZoomMultiplierComponent CooldownOption/TransitionState: 2 bits, where
#:   width cannot tell an enum from a SerializedInt.
#: * FiniteSpeedMovementComponent bIsActive: all 574 rows 0x01; NumCollisions:
#:   i32 values only 0/1, indistinguishable from a widened Bool.
ADDITIONS = pins([
    # Both declared CombatReport handles: all 84,867 payloads consume exactly
    # as packed NetGUIDs and resolve to icon paths; no other group is admitted.
    ("/Game/GameModes/Bomb/Bomb_CombatReportComponent.Bomb_CombatReportComponent_C",
     "ObjectNetGuid", "DamageIcon"),
    # Crosshair settings, each decoded with exact consumption on 13.01-13.05
    # payloads. B also names a 32-bit property (checksum 943211507), so it is
    # typed by exact identity in tools/fixtures/scoped_type_evidence.json.
    (BOMB_PLAYER_STATE, "Bool", (
        "bHasOutline", "bDisplayCenterDot", "bFadeCrosshairWithFiringError",
        "bShowSpectatedPlayerCrosshair", "bFixMinErrorAcrossWeapons", "bAllowVertScaling",
        "bShowMovementError", "bShowShootingError", "bShowMinError", "bShowLines",
        "bUsePrimaryCrosshairForADS", "bUseCustomCrosshairOnAllPrimary", "bUseAdvancedOptions")),
    (BOMB_PLAYER_STATE, "Float", (
        "OutlineThickness", "OutlineOpacity", "CenterDotSize", "CenterDotOpacity",
        "LineThickness", "LineLength", "LineLengthVertical", "LineOffset", "Opacity",
        "FiringErrorScale", "MovementErrorScale")),
    (BOMB_PLAYER_STATE, "Byte", ("G", "R")),
    (BOMB_PLAYER_STATE, "FString", "ProfileName"),
    # Tidal Wave, each type checked against payload widths and ranges (names
    # with spaces are the replay's); AliveChunks is variable-width and stays raw.
    (TIDAL_WAVE_CHUNK + ":MulticastInitialize", "Int32",
     ("ChunkIndex", "Generation", "Num Chunks", "Num Crossfade Anchors In")),
    (TIDAL_WAVE_CHUNK + ":MulticastInitialize", "Float", "ChunkSpacing"),
    (TIDAL_WAVE_CHUNK + ":MulticastInitialize", "Double", ("Velocity In", "Anchor Spacing In")),
    (TIDAL_WAVE_CHUNK + ":MulticastInitialize", "ObjectNetGuid", "PreviousChunk"),
    (TIDAL_WAVE_CHUNK + ":MulticastWallStartLinger", "Double", "LingerWallStopPosition"),
    (TIDAL_WAVE_CHUNK + ":MulticastWallStartLinger", "Bool", "FinalEndpointReached"),
    ("/Game/Characters/Mage/S0/Ability_X/GameObject_Mage_X_TidalWave."
     "GameObject_Mage_X_TidalWave_C:MulticastStopWave", "Bool", "FinalEndpointReached"),
    # 246 rows: 126 of 16 bits resolve as IntPacked to a *Ceremony_C actor
    # (126/126), 120 are the 8-bit null written at round start; none is odd.
    ("/Game/GameModes/Bomb/BombGameState.BombGameState_C", "ObjectNetGuid",
     "ChosenCeremonyForRound"),
    # Phoenix's wall: every row 192 bits (3 x f64); Translation reads as map
    # coordinates and Scale3D is (1, 1, 1) on every row. Its checksums differ from
    # Viper's, so the checksum fallback cannot carry Viper's types over. Its
    # `249` (an FQuat, 177696787) stays raw.
    (FLAME_WALL, "VectorDouble", ("Translation", "Scale3D")),
    # `249` beside a `248` location: 441,814 rows at 3/19/35/51 bits, exactly
    # 3 + 16 x (flags set), which only RotationShort produces, consumed exactly;
    # the named twin ReplayPlayContinuousEffectAtLocation.Rotation is RotationShort.
    # The other `249` (747197698, below) is an FTransform's FQuat.
    ("/Script/ShooterGame.LocationalEffectManagerComponent:ClientPlayOneShotEffectAtLocation",
     "RotationShort", "249"),
    ("/Script/ShooterGame.ReplayEffectComponent:ReplayPlayContinuousEffectAtLocation",
     "RotationShort", "249"),
    ("/Script/ShooterGame.ReplayEffectComponent:ReplayPlayOneShotEffectAtLocation",
     "RotationShort", "249"),
    ("/Script/ShooterGame.EffectManagerComponent:ReplayRecordOneShotEffect", "RotationShort", "249"),
    ("/Script/ShooterGame.EffectManagerComponent:ReplayRecordContinuousEffect",
     "RotationShort", "249"),
    # A seed: 120,853 rows of 32 bits, 120,852 distinct over the full range.
    # Int32 follows FRandomStream's int32 seed.
    ("/Script/ShooterGame.NetworkedRandomNumberGeneratorComponent", "Int32",
     "AuthCurrentRandomSeed"),
    # Moved from BombGameState.TeamEconomy in 13.02 with its declared int type;
    # all payloads 32 bits, AverageLoadoutValue = LoadoutValue / 5 on every row.
    ("/Script/ShooterGame.BaseTeamState", "Int32", ("AverageLoadoutValue", "LoadoutValue")),
    # 32 bits on every row: HealTaken Float 0.05..400 with 1.0f recurring,
    # DecayApplied 0.07..50. Their *Instigator/*Causer references are typed by
    # exact identity (scoped_type_evidence.json): the damage RPCs reuse the names.
    ("/Script/ShooterGame.DamageableComponent:MulticastNotifyHeal", "Float", "HealTaken"),
    ("/Script/ShooterGame.DamageableComponent:MulticastNotifyOverhealDecay", "Float",
     "DecayApplied"),
    # A projectile's travel limit: 32 bits on all 11,699 rows, 397.6..49986.1.
    ("/Script/ShooterGame.FiniteSpeedMovementComponent", "Float", "MaximumRange"),
    # A movement clock: 32 bits on all 4,571,175 rows (714 exports), a finite
    # Float monotonic per actor; its epoch (actor spawn or not) is not established.
    ("/Script/ShooterGame.FiniteSpeedMovementComponent", "Float", "ServerMovementTime"),
    ("/Script/ShooterGame.SplineMovementComponent", "Float", "ServerMovementTime"),
    ("/Script/ShooterGame.PrecalculatedProjectileMovementComponent", "Float",
     "ServerMovementTime"),
    ("/Game/Characters/Components/Comp_Projectile_FloatCurveMovement."
     "Comp_Projectile_FloatCurveMovement_C", "Float", "ServerMovementTime"),
    # ReplayLastTransformUpdateTimeStamp on nine pawn groups that lacked it; the
    # evidence is at its /Game/Characters/ correction.
    *[(group, "Float", "ReplayLastTransformUpdateTimeStamp") for group in (
        "/Game/Characters/Cashew/S0/Ability_E/AIPawn_Cashew_E_SeekingTargetMissile."
        "AIPawn_Cashew_E_SeekingTargetMissile_C",
        "/Game/Characters/Clay/S0/Ability_E/Pawn_Clay_E_Boomba.Pawn_Clay_E_Boomba_C",
        "/Game/Characters/Guide/S0/Ability_Q/Pawn_Guide_Q_PossessableScout."
        "Pawn_Guide_Q_PossessableScout_C",
        "/Game/Characters/Gumshoe/S0/Ability_E/Pawn_Gumshoe_E_PossessableCamera."
        "Pawn_Gumshoe_E_PossessableCamera_C",
        "/Game/Characters/Killjoy/S0/Ability_E/Pawn_Killjoy_E_Turret.Pawn_Killjoy_E_Turret_C",
        "/Game/Characters/Killjoy/S0/Ability_Q/Pawn_Killjoy_Q_StealthAlarmbot."
        "Pawn_Killjoy_Q_StealthAlarmbot_C",
        "/Game/Characters/Pine/S0/Ability_E/Pawn_Pine_E_RadEater.Pawn_Pine_E_RadEater_C",
        "/Game/Characters/Rift/S0/Ability_X/WorldTargeting/Rift_TargetingForm_PC."
        "Rift_TargetingForm_PC_C",
        "/Game/Characters/Stealth/S0/Ability_4/Pawn_Stealth_4_Decoy_V2.Pawn_Stealth_4_Decoy_V2_C",
    )],
    # Live per-player credits, 32-bit LE i32 on every row: Money is 800 on all
    # ten actors at pistol-round start and 0..9000 in steps of 50;
    # StartOfRoundMoney 800 for active players, else 0; TotalMoneyGranted cumulative.
    ("/Script/ShooterGame.MoneyManagementComponent", "Int32",
     ("Money", "StartOfRoundMoney", "TotalMoneyGranted")),
    # Ping: always 16 bits, a LE u16 latency in ms (02d4d478: p50 15, max 473);
    # SerializedInt{65536} reads exactly those bits.
    (BOMB_PLAYER_STATE, "SerializedInt { max: 65536 }", "Ping"),
    # The scoreboard K/D/A: all 407 updates on a 13.02 replay are 32 bits, and
    # the final Int32 counters match the in-game scoreboard for all ten players.
    ("/Script/ShooterGame.BasicCombatStatsComponent", "Int32",
     ("AggregateKills", "AggregateDeaths", "AggregateAssists")),
    # 32 bits on all 430 rows: Int32 21..5833, 415 distinct; as Float a denormal.
    ("/Script/ShooterGame.PlayerScoreComponent", "Int32", "Score"),
    # 375 rows on 9 actors: Start/EndTime 32-bit Float game-seconds ~2.5 s
    # apart; Level 64 bits, a Double 0..1 intensity ramp.
    ("/Game/Characters/Components/Comp_Actor_Concussable.Comp_Actor_Concussable_C", "Float",
     ("ConcussStartTime", "ConcussEndTime")),
    ("/Game/Characters/Components/Comp_Actor_Concussable.Comp_Actor_Concussable_C", "Double",
     "ConcussLevel"),
    # CurrentFuel: 64 bits on all 5,702 rows, a Double draining 1.0 -> 0.0;
    # IsFuelDraining: 1 bit on all 60 rows.
    ("/Game/Characters/Components/Comp_AbilityFuelSystem.Comp_AbilityFuelSystem_C", "Double",
     "CurrentFuel"),
    ("/Game/Characters/Components/Comp_AbilityFuelSystem.Comp_AbilityFuelSystem_C", "Bool",
     "IsFuelDraining"),
    # Seconds: all 1,727 main and 2,431 checkpoint rows are 32-bit finite
    # Floats, 0.0..3.0 (1.5 s x176, 2.25 s x67).
    ("/Script/ShooterGame.BlindManagerComponent", "Float", "LongestActiveBlindDuration"),
    # 32 bits on every row, no NaN: Source/TargetFov 20.6..103.0 (103 is the
    # default hip-fire FOV), the 1P pair 5.0..70.0, the duration 0.0..0.25 s.
    ("/Script/ShooterGame.ZoomMultiplierComponent", "Float",
     ("SourceFov", "SourceFov1P", "TargetFov", "TargetFov1P", "TotalTransitionTimeDuration")),
    # Signed: the seed TargetFov (float) implies reproduces both checksums only as
    # int32 (check_checksum_types.py sibling seeds). All 64,273 rows (60 exports,
    # 11.06-13.10) are 32 bits in {-1, 0, 1}; -1 (0xFFFFFFFF) pairs with Fov 103.0
    # and 1 with 20.6 on every same-packet pair.
    ("/Script/ShooterGame.ZoomMultiplierComponent", "Int32",
     ("SourceZoomLevel", "TargetZoomLevel")),
    # Hold-to-interact objects (plant/defuse, orbs, doors): HighestProgress
    # advances 1/128 per tick as a 0..1 float (a u32 read is not monotonic);
    # bIsActive is the 1-bit interacting flag.
    ("/Script/ShooterGame.UsableComponent", "Float", "HighestProgress"),
    ("/Script/ShooterGame.UsableComponent", "Bool", "bIsActive"),
    # 3 bits on all 1,015,515 rows, values {0, 1, 2}, like the EnumByte sibling
    # AutoEquipSpeed. Every checkpoint row reads 0, so a checkpoint value is not
    # readying state.
    ("/Script/ShooterGame.ReadyingStateComponent", "EnumByte", "AuthEquipSpeed"),
    # Correction counters, 32 bits on all 24 builds: LE i32 1..2011, strictly
    # increasing per (actor, object); LastSeenClientCorrectionIndex is never >=
    # CorrectionIndex at the same point. The handle moved at 12.04; the name did not.
    ("/Script/ShooterGame.AresInventory", "Int32",
     ("CorrectionIndex", "LastSeenClientCorrectionIndex")),
    # An FTransform parameter arrives as three double vectors, none declared:
    # as 3 x f64 Scale3D is exactly (1, 1, 1) on every row, and `248` is a map
    # location. `249` is the Rotation FQuat's X/Y/Z (747197698 reproduces
    # `Transform: FTransform -> Rotation: FQuat`); W is not sent, sqrt(1 - |xyz|^2):
    # all 12,698,371 rows are 192 bits with |xyz| <= 1. Its value_str is the
    # quaternion's (x,y,z), not Euler angles.
    *[(group, "VectorDouble", "248") for group in (
        "/Script/ShooterGame.LocationalEffectManagerComponent:ClientPlayOneShotEffectAtLocation",
        "/Script/ShooterGame.ReplayEffectComponent:ReplayPlayOneShotEffectAtLocation",
        "/Script/ShooterGame.EffectManagerComponent:ReplayRecordOneShotEffect",
        "/Script/ShooterGame.EffectManagerComponent:ReplayRecordContinuousEffect",
    )],
    *[(group, "VectorDouble", ("249", "Translation", "Scale3D")) for group in (
        "/Script/ShooterGame.EffectManagerComponent:MulticastPlayContinuousEffect",
        "/Script/ShooterGame.AresEquippable:MulticastPlayContinuousEffectFromClient",
        "/Script/ShooterGame.EffectManagerComponent:MulticastPlayOneShotEffect",
        "/Script/ShooterGame.AresEquippable:MulticastPlayOneShotEffectFromClient",
    )],
    # The holder's EffectManager (checksum 1051633025): 3,112,054 rows at 16/24
    # bits, exact IntPacked, every value resolving to an `EffectManager` whose
    # outer is the pawn holding the weapon. Both twins: typing one leaves the
    # other raw at 0 decode errors.
    ("/Script/ShooterGame.AresEquippable:MulticastPlayContinuousEffectFromClient",
     "ObjectNetGuid", "EffectManagerComponent"),
    ("/Script/ShooterGame.AresEquippable:MulticastPlayOneShotEffectFromClient",
     "ObjectNetGuid", "EffectManagerComponent"),
    # The same FQuat under `SpawnTransform` (1874998526): 183,577 rows, all 192
    # bits, |xyz| <= 1. The vectors after it are 3 x f64 too; PlantLocation and
    # BombLocation report byte-identical coordinates on all 9 plants.
    ("/Script/ShooterGame.AresGameStateBase:MulticastResetForRespawn", "VectorDouble", "249"),
    (FORCE_MODULE, "VectorDouble", "SourceLocation"),
    ("/Game/Abilities/GrenadeExplodeIndicator.GrenadeExplodeIndicator_C:"
     "MulticastTriggerExplodeIndicator", "VectorDouble", "IndicatorLocation"),
    ("/Game/GameModes/Bomb/BombDestination.BombDestination_C:MulticastActivateBombSiteEffects",
     "VectorDouble", "BombLocation"),
    (COMP_BOMB_EVENTS + ":BombPlantedRPC", "VectorDouble", "PlantLocation"),
    # 873 plants over 76 replays, 11.06-13.06. BombPlanter: 16/24-bit IntPacked,
    # the spike's BombEquippable_C.Owner at the RPC on 873/873 (Gekko's Wingman
    # when it plants). PlantSite: 2 bits when sent, equal to PlantedAtSite of the
    # TimedBomb opened at the same time_ms on 873/873; both absent together is
    # the default site 0, so it takes PlantedAtSite's type.
    (COMP_BOMB_EVENTS + ":BombPlantedRPC", "ObjectNetGuid", "BombPlanter"),
    (COMP_BOMB_EVENTS + ":BombPlantedRPC", "EnumByte", "PlantSite"),
    # The latest non-null TimedBomb.CurrentDefuser at the RPC on 224/224.
    (COMP_BOMB_EVENTS + ":BombDefusedRPC", "ObjectNetGuid", "DefusingCharacter"),
    # 1,976 calls: Collectable Orb is an UltPointOrb_C actor on every one; Orb
    # Gatherer a player pawn whose PlayerState's NumUltimatePoints rises by one
    # within 1 s on 1,944, the other 32 already at 7 or 8 points.
    ("/Game/BaseGameState.BaseGameState_C:OrbPickedUpRPC", "ObjectNetGuid",
     ("Orb Gatherer", "Collectable Orb")),
    ("/Game/Equippables/Finishers/Rogue/Desturctible/FXC_Rogue_Finisher_Destructible."
     "FXC_Rogue_Finisher_Destructible_C:Set skeletal Collision",
     "VectorDouble", "Collision Static Mesh Scale"),
    # 32 bits on all 13,316 rows with the declared sibling StartMovementTime's
    # -1.0 sentinel; 244888268 carries it to ReplayStopContinuousEffectAtLocation.
    ("/Script/ShooterGame.EffectManagerComponent:MulticastStopContinuousEffect", "Float",
     "StopMovementTime"),
    # A dense id, 1..765 on 3,741 rows; 3336285386 reproduces only as
    # `HandleNumber: uint32`, and bit 31 is never set.
    (FORCE_MODULE, "UInt32", "HandleNumber"),
    # The other NetMulticastApplyForceModule parameters (665,519 calls), each
    # checksum this parameter's alone at one width. A row's `handle` is the
    # function slot, 1 for Apply.
    # RespawnNumber: i32 0..38, equal to the character's AresInventory.RespawnNumber
    # on 665,363 of 665,370 rows. Not ForceModuleManagerComponent.RespawnNumber
    # (3044239005): do not merge them.
    (FORCE_MODULE, "Int32", "RespawnNumber"),
    # NetTimestamp: finite f32 0.0..252.7 agreeing with the character's typed
    # clocks; its epoch is per actor and life, NOT replay time.
    (FORCE_MODULE, "Float", "NetTimestamp"),
    # ModuleType: 3 bits, {0, 2}; each of the 39 module classes maps to one
    # value, and Remove's ModuleType (typed through this donor) agrees with the
    # paired Apply on 647,381 of 647,381. Typing Remove's differently would drop
    # the checksum.
    (FORCE_MODULE, "EnumByte", "ModuleType"),
    # Module: 16-bit IntPacked, always odd (static), resolving to a ForceModule_*
    # class; Character: IntPacked, equal to the row's own actor_net_guid on every row.
    (FORCE_MODULE, "ObjectNetGuid", ("Module", "Character")),
    # All 1,957 non-zero rows resolve to a CalloutRegion_* actor (22 distinct):
    # the region actor's object name, not the callout label shown in game.
    ("/Script/ShooterGame.CalloutRegionTrackingComponent", "ObjectNetGuid", "CurrentRegion"),
    # The per-cast ability log's members, names with Blueprint property GUIDs.
    # Player's 352 values are UUIDs matching manifest.players.subject; Round is
    # 0..17 over 18 rounds; CastLocation lies in the map bounds; Slot is 3/4/5/9.
    (ABILITY_STATS, "FString", "Player_11_0963330440D68BDF1A8E34B035420342"),
    (ABILITY_STATS, "Byte", "Slot_12_22D571914FAFD5F0EBD400B7E2F28B36"),
    (ABILITY_STATS, "Int32", "Round_22_905E6CC0448D2C6270A94C9690101E49"),
    (ABILITY_STATS, "Byte", "RoundPhase_25_84478C0047988409FEEC9E95C15DFB02"),
    (ABILITY_STATS, "Float", "CastTime_4_5AE288704801A9B74D6D159DFC2BD147"),
    (ABILITY_STATS, "VectorDouble", "CastLocation_21_61F4B6BC47A10FE8CD34D29141FC9B88"),
    (ABILITY_STATS, "Int32", "DestroyedCount_36_5936AB33418F8A2AB3A52DBF4492CF7F"),
    # Each cast's Effects[]. LocalizedStat: an FText keyed by the statistic's
    # name, 29 distinct over 4,341 rows, 1:1 with Statistic, all decoded with
    # zero residual bits. AffectedPlayer: all 224 values resolve to a
    # manifest.players.actor_net_guid. Time is seconds within the round.
    (ABILITY_STATS, "FText", "LocalizedStat_14_C3A26F5E46CDAD94571AE6B0EDEA058B"),
    (ABILITY_STATS, "EnumByte", "Statistic_2_0868666A4F6501815AB301BB615B2B5C"),
    (ABILITY_STATS, "Float", "Value_7_E46F38AE4D245059AF7BB09E301C3C65"),
    (ABILITY_STATS, "Float", "Time_8_6CD58DD7441CBD0407DD1F89FDD05167"),
    (ABILITY_STATS, "ObjectNetGuid", "AffectedPlayer_2_BAF988E34EAAE6B7A1D4758455186559"),
    (ABILITY_STATS, "Float", "Value_5_203891704B7EF064EDB5528BFECC4807"),
    # Three finite LE f64 in every 192-bit window, on this exact group only.
    (HAWK_FLASH, "VectorDouble", "PostControlVelocity"),
    # ByteComponents consumes all 1,033,952 rows exactly, where Short fails on
    # 54.6%. Location in whole units: the first update lies within 0.87 cm of
    # the actors.parquet spawn on all 8,265 actors.
    # Checksum 2749104612 stays dropped: Gekko's Wingman is Short.
    (HAWK_FLASH, BYTE_WHOLE, "ReplicatedMovement"),
    # 64 bits on all 801,700 rows: LE f64 in [-180, 180] with the low 29 mantissa
    # bits zero (a widened f32), wrapping like an angle. Its handle moved from
    # 17 to 18, so only the name keys it.
    (HAWK_FLASH, "Double", "Banking"),
    # Cypher's trapwire and cage, moved in 13.01 from Ability_E/TripWire and
    # Ability_4/CageTrap (whose entries stay for 11.06-13.00) with identical
    # fields, handles and checksums; no replay declares both. Only Deployed's
    # checksum is donated (these are its only carriers).
    # Deployed: 1 bit, always true; false is the default and never sent.
    ("/Game/Characters/Gumshoe/S0/Ability_4/GameObject_Gumshoe_4_TripWire."
     "GameObject_Gumshoe_4_TripWire_C", "Bool", "Deployed"),
    ("/Game/Characters/Gumshoe/S0/Ability_4/GameObject_Gumshoe_4_TripWire_SecondWire."
     "GameObject_Gumshoe_4_TripWire_SecondWire_C", "Bool", "Deployed"),
    # CreatedByCharacter: 16-bit IntPacked or the 8-bit null, every non-null
    # value the Gumshoe_PC_C actor.
    ("/Game/Characters/Gumshoe/S0/Ability_4/Ability_Gumshoe_4_TripWire."
     "Ability_Gumshoe_4_TripWire_C", "ObjectNetGuid", "CreatedByCharacter"),
    ("/Game/Characters/Gumshoe/S0/Ability_Q/Ability_Gumshoe_Q_CageTrap."
     "Ability_Gumshoe_Q_CageTrap_C", "ObjectNetGuid", "CreatedByCharacter"),
    # RelativeScale3D: 31 bits, (1, 1, 1) on every row, the old cage's type.
    ("/Game/Characters/Gumshoe/S0/Ability_Q/Ability_Gumshoe_Q_CageTrap."
     "Ability_Gumshoe_Q_CageTrap_C", "VectorNetQuantize { scale: 100 }", "RelativeScale3D"),
])
EXPECTED = CORRECTIONS + ADDITIONS

#: The OVERLAY_TABLE literal: text before the length, text up to the body, the
#: body, and the rest of the file, which applying leaves untouched.
TABLE_RE = re.compile(r"(.*?pub static OVERLAY_TABLE: \[OverlayEntry; )\d+(\] = \[\n)(.*?)"
                      r"(\n\];\n.*)", re.S)
ENTRY_RE = re.compile(r'    OverlayEntry \{ group_path: "([^"\\]+)", field_name: "([^"\\]+)", '
                      r"field_type: (FieldType::\w+(?: \{ [^{}]+ \})?) \},")


def parse_table(content: str):
    """`(match, {(group, field): type})`. Anything but one entry per line, each
    key once, exits naming the line."""
    table = TABLE_RE.fullmatch(content)
    if table is None:
        raise SystemExit(f"{TABLE_RS}: no `pub static OVERLAY_TABLE: [OverlayEntry; N] = [` "
                         f"... `];` literal")
    entries = {}
    first = content.count("\n", 0, table.start(3)) + 1
    for number, line in enumerate(table[3].split("\n"), first):
        entry = ENTRY_RE.fullmatch(line)
        if entry is None:
            raise SystemExit(f"{TABLE_RS}:{number}: not a one-line OverlayEntry: {line[:80]!r}")
        if entry.group(1, 2) in entries:
            raise SystemExit(f"{TABLE_RS}:{number}: {entry.group(1, 2)} is declared twice")
        entries[entry.group(1, 2)] = entry[3]
    return table, entries


def render_table(table, entries: dict) -> str:
    body = "\n".join(f'    OverlayEntry {{ group_path: "{group}", field_name: "{field}", '
                     f"field_type: {ftype} }},"
                     for (group, field), ftype in sorted(entries.items()))
    return f"{table[1]}{len(entries)}{table[2]}{body}{table[4]}"


def targets(entries: dict, group: str, field: str) -> list[tuple[str, str]]:
    """The keys a pin sets: its own for an exact pin, present or not; every
    present key it matches for a glob."""
    if "*" not in group:
        return [(group, field)]
    return [key for key in entries if key[1] == field and fnmatch.fnmatchcase(key[0], group)]


def apply(entries: dict) -> tuple[dict, list[str]]:
    """`(entries with every pin set, problems)`: a glob that matches nothing,
    or one entry pinned two ways."""
    fixed, pinned, problems = dict(entries), {}, []
    for group, field, ftype in EXPECTED + WEAPON_PINS:
        keys = targets(fixed, group, field)
        if not keys:
            problems.append(f"{field} in {group}: the pin matches no entry")
        for key in keys:
            if pinned.setdefault(key, ftype) != ftype:
                problems.append(f"{key}: pinned as {pinned[key]} and as {ftype}")
            fixed[key] = ftype
    return fixed, problems


def expectation_count(content: str) -> int:
    """How many pins hold: EXPECTED, plus each weapon entry WEAPON_PINS reaches."""
    _table, entries = parse_table(content)
    return len(EXPECTED) + sum(len(targets(entries, g, f)) for g, f, _t in WEAPON_PINS)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify without writing")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    check = parse_args(argv).check
    on_disk = TABLE_RS.read_text(encoding="utf-8")
    table, entries = parse_table(on_disk)
    fixed, problems = apply(entries)
    content = render_table(table, fixed)
    for line in problems:
        print(f"FAILED: {line}", file=sys.stderr)
    if check and content != on_disk:
        diff = list(difflib.unified_diff(on_disk.splitlines(), content.splitlines(),
                                         "table.rs", "applied", n=0, lineterm=""))
        print(f"FAILED: applying the pins changes {TABLE_RS}; run without --check:\n"
              + "\n".join(diff[:60]) + (f"\n... {len(diff) - 60} more" if len(diff) > 60 else ""),
              file=sys.stderr)
        return 1
    if problems:
        return 1
    if content != on_disk:
        atomic_write_text(TABLE_RS, content)
    changed = sum(entries.get(key) != ftype for key, ftype in fixed.items())
    kinds = Counter(ftype if ftype in ("FieldType::Raw", "FieldType::Skip") else "typed"
                    for ftype in fixed.values())
    print(f"{'verified' if check else 'applied'}: all {expectation_count(content)} pins hold "
          f"in {TABLE_RS}, {changed} entries changed; {len(fixed)} entries: {kinds['typed']} "
          f"typed, {kinds['FieldType::Skip']} Skip, {kinds['FieldType::Raw']} Raw")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
