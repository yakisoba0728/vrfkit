//! The overlay table's resolution order, and the hash index's agreement
//! with the binary search it replaced.

use std::collections::{BTreeMap, BTreeSet};

use crate::checksum_table::CHECKSUM_TYPES;
use crate::decode::{DecodeError, FieldType, decode_field};
use crate::overlay::{
    OverlayEntry, OverlayHandleEntry, OverlayResult, OverlayStats, OverlayTable, apply_overlay,
    apply_overlay_with_checksum, canonical_group, group_hash_state, lookup_checksum,
    resolve_field_type, resolve_field_type_with_checksum,
};
use crate::scoped_types::SCOPED_TYPES;
use crate::types::{RotatorQuantization, VectorQuantization};
use crate::{OVERLAY_HANDLE_TABLE, OVERLAY_TABLE};
use vrf_testkit::{BitWrite, BitWriter};

/// The table production resolves through: every entry and the explicit-handle
/// fallback. One static, so its hash index is built once for the suite.
pub(super) static TABLE: OverlayTable =
    OverlayTable::with_handles(&OVERLAY_TABLE, &OVERLAY_HANDLE_TABLE);

/// The table entry for `(group, field)`, reported at the caller's line.
#[track_caller]
fn assert_typed(group: &str, field: &str, want: Option<FieldType>) {
    assert_eq!(TABLE.lookup(group, field), want, "{group} {field}");
}

/// The checksum table's answer for `checksum`, reported at the caller's line.
#[track_caller]
fn assert_checksum(checksum: u32, want: Option<FieldType>) {
    assert_eq!(lookup_checksum(checksum), want, "{checksum}");
}

/// What [`TABLE`] resolves `field` of `group` to, with no handle.
pub(super) fn resolve(group: &str, field: &str, checksum: Option<u32>) -> Option<FieldType> {
    resolve_field_type_with_checksum(&TABLE, group, Some(field), None, checksum)
}

/// A `/test` entry, for the synthetic tables below.
const fn entry(field_name: &'static str, field_type: FieldType) -> OverlayEntry {
    OverlayEntry {
        group_path: "/test",
        field_name,
        field_type,
    }
}

/// A `/test` explicit-handle entry.
const fn at(handle: u32, field_name: &'static str) -> OverlayHandleEntry {
    OverlayHandleEntry {
        group_path: "/test",
        handle,
        field_name,
    }
}

/// One field through `table` as the export path applies it, into fresh stats;
/// no `handle` is the name-only [`apply_overlay`] path.
fn run(
    table: &OverlayTable,
    group: &str,
    name: Option<&str>,
    handle: Option<u32>,
    checksum: Option<u32>,
    raw: Option<&[u8]>,
    bits: u32,
) -> (Option<OverlayResult>, OverlayStats) {
    let mut stats = OverlayStats::default();
    let state = group_hash_state(group);
    let result = match handle {
        None => apply_overlay(table, group, state, name, raw, bits, &mut stats),
        Some(handle) => apply_overlay_with_checksum(
            table, group, state, name, handle, checksum, raw, bits, &mut stats,
        ),
    };
    (result, stats)
}

/// One field through [`TABLE`] with every key the export path has, counted
/// into `stats`. The overlay declining the field fails the test.
#[track_caller]
pub(super) fn apply_scoped(
    stats: &mut OverlayStats,
    group: &str,
    field: &str,
    handle: u32,
    checksum: u32,
    raw: &[u8],
    bits: u32,
) -> OverlayResult {
    let (applied, counts) = run(
        &TABLE,
        group,
        Some(field),
        Some(handle),
        Some(checksum),
        Some(raw),
        bits,
    );
    stats.merge_counts_from(&counts);
    let Some(applied) = applied else {
        // In the body, not a closure, so #[track_caller] names the test's line.
        panic!("{group} {field}: the overlay declined it");
    };
    applied
}

pub(super) const BOMB_GS: &str = "/Game/GameModes/Bomb/BombGameState.BombGameState_C";
const BOMB_PS: &str = "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C";
pub(super) const SWIFT_GS: &str = "/Game/GameModes/_Development/Swiftplay_EndOfRoundCredits\
/Swiftplay_EoRCredits_GameState.Swiftplay_EoRCredits_GameState_C";
const SWIFT_PS: &str = "/Game/GameModes/_Development/Swiftplay_EndOfRoundCredits\
/Swiftplay_EoRCredits_PlayerState.Swiftplay_EoRCredits_PlayerState_C";
const PLAY_CONTINUOUS: &str =
    "/Script/ShooterGame.EffectManagerComponent:MulticastPlayContinuousEffect";
const STOP_CONTINUOUS: &str =
    "/Script/ShooterGame.EffectManagerComponent:MulticastStopContinuousEffect";
const FROM_CLIENT: &str =
    "/Script/ShooterGame.AresEquippable:MulticastPlayContinuousEffectFromClient";
const REPLAY_AT_LOCATION: &str =
    "/Script/ShooterGame.ReplayEffectComponent:ReplayPlayContinuousEffectAtLocation";
const DAMAGE_BASE: &str = "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Base";
const DAMAGE_POINT: &str = "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Point";
const FORCE_APPLY: &str =
    "/Script/ShooterGame.ForceModuleManagerComponent:NetMulticastApplyForceModule";
const FORCE_REMOVE: &str =
    "/Script/ShooterGame.ForceModuleManagerComponent:NetMulticastRemoveForceModule";
const HAWK: &str = "/Game/Characters/Guide/S0/Ability_E/\
Projectile_Guide_E_HawkFlash.Projectile_Guide_E_HawkFlash_C";
const SEEKER_NADE: &str = "/Game/Characters/AggroBot/S0/Ability_Q/\
Pawn_Aggrobot_SeekerNade.Pawn_Aggrobot_SeekerNade_C";
const CLAY_SATCHEL_ABILITY: &str =
    "/Game/Characters/Clay/S0/Ability_Q/Ability_Clay_Q_Satchel.Ability_Clay_Q_Satchel_C";
const CLAY_SATCHEL: &str = "/Game/Characters/Clay/S0/Ability_Q/\
Projectile_Clay_Q_Satchel_Arming.Projectile_Clay_Q_Satchel_Arming_C";
const CLAY_BOOMBOT: &str =
    "/Game/Characters/Clay/S0/Ability_E/Pawn_Clay_E_Boomba.Pawn_Clay_E_Boomba_C";
const HEAL_PARAMS: &str = "/Script/ShooterGame.DamageableComponent:MulticastNotifyHeal";
const DECAY_PARAMS: &str = "/Script/ShooterGame.DamageableComponent:MulticastNotifyOverhealDecay";

#[test]
fn scoped_bytes_decode_exactly_and_reject_a_wider_payload() {
    let mut stats = OverlayStats::default();
    let value = apply_scoped(&mut stats, BOMB_PS, "B", 39, 379198054, &[255], 8);
    assert_eq!(value.value_i64, Some(255));
    let rejected = apply_scoped(&mut stats, BOMB_PS, "B", 39, 379198054, &[255, 0, 0, 0], 32);
    assert_eq!(rejected.value_i64, None);
    assert_eq!(stats.decoded_ok, 1);
    assert_eq!(stats.decoded_err, 1);
}

/// Every scoped identity resolves to its own type, and only at its exact
/// identity. The table and its aliases resolve a name before `SCOPED_TYPES`, so
/// an entry whose group has a same-name table entry is never read, silently;
/// and without its checksum, with a neighbouring one, under another name or on
/// a group nothing declares, an entry must add nothing an unrelated group would
/// not get: a Blueprint renames, retypes or moves properties between builds.
/// The lookup is a binary search on `(name, group, checksum)`, so the order is
/// asserted too.
#[test]
fn no_scoped_identity_is_shadowed_by_the_table() {
    for pair in SCOPED_TYPES.windows(2) {
        let (a, b) = (
            (pair[0].0, pair[0].1, pair[0].2),
            (pair[1].0, pair[1].1, pair[1].2),
        );
        assert!(a < b, "SCOPED_TYPES not strictly sorted: {a:?} then {b:?}");
    }
    let elsewhere = |name, checksum| resolve("/Unobserved", name, checksum);
    for &(name, group, checksum, field_type) in &SCOPED_TYPES {
        let id = format!("{name} / {group} / {checksum}");
        let own = resolve(group, name, Some(checksum));
        assert_eq!(own, Some(field_type), "unreachable scoped identity {id}");
        for (probe, other) in [
            (name, None),
            (name, Some(checksum ^ 1)),
            ("Unobserved", Some(checksum)),
        ] {
            let got = resolve(group, probe, other);
            assert_eq!(got, elsewhere(probe, other), "{id}: {probe} {other:?}");
        }
        let alone = elsewhere(name, Some(checksum));
        assert_eq!(
            alone,
            elsewhere(name, None),
            "{id}: typed by checksum alone"
        );
    }
}

/// Payloads an independent parser's tests recorded from replay 42e03082 (not
/// in the local corpus), decoded through the scoped identities: exact widths,
/// exact values.
#[test]
fn recorded_raze_payloads_decode_through_their_scoped_identities() {
    let mut stats = OverlayStats::default();
    let seed = apply_scoped(
        &mut stats,
        CLAY_SATCHEL_ABILITY,
        "CosmeticRandomSeed",
        56,
        2_863_861_815,
        &[0xE1, 0xE9, 0x4B, 0x40],
        32,
    );
    assert_eq!(seed.value_i64, Some(1_078_716_897));
    let offset = apply_scoped(
        &mut stats,
        CLAY_SATCHEL,
        "LocationOffset",
        5,
        111_823_753,
        &[0xD3, 0x20, 0x67, 0xB7, 0xA8, 0x97, 0x48, 0x00],
        64,
    );
    assert_eq!(offset.value_str.as_deref(), Some("(-782.71,-1366.59,5.8)"));
    let rotation = apply_scoped(
        &mut stats,
        CLAY_SATCHEL,
        "RotationOffset",
        7,
        1_473_289_183,
        &[0x01, 0x80, 0xEE, 0x27, 0xF7, 0xFF, 0x07],
        51,
    );
    assert_eq!(
        rotation.value_str.as_deref(),
        Some("rot(90,284.03503,359.989)")
    );
    // `ModuleType` resolves through its table entry (EnumByte); the recorded
    // 3-bit payload must still read 2.
    let module_type = apply_scoped(
        &mut stats,
        FORCE_APPLY,
        "ModuleType",
        1,
        3_263_282_135,
        &[0x02],
        3,
    );
    assert_eq!(module_type.value_i64, Some(2));
    assert_eq!(stats.decoded_ok, 4);
    // The recorded truncation case: one byte cannot carry the rotator its
    // presence bits announce. Rejected and counted, not truncated.
    let truncated = apply_scoped(
        &mut stats,
        CLAY_SATCHEL,
        "RotationOffset",
        7,
        1_473_289_183,
        &[0x01],
        8,
    );
    assert_eq!(truncated.value_str, None);
    assert_eq!(stats.decoded_err, 1);
}

/// The player state's `D` word, one of four `uint32` FGuid members scoped per
/// group and checksum, end to end (scoped lookup, the `UInt32` read,
/// `value_i64`), with a high-bit word staying positive: `Int32` would read the
/// real value 0xe28c69d7 as -494114345, a plausible wrong number a width check
/// alone accepts. Any other width is a decode error, not a truncated or padded
/// value.
#[test]
fn player_state_guid_parts_decode_unsigned_and_exactly() {
    let mut stats = OverlayStats::default();
    let mut apply = |group: &str, raw: &[u8], bits| {
        apply_scoped(&mut stats, group, "D", 210, 1_032_080_829, raw, bits).value_i64
    };
    for group in [BOMB_PS, SWIFT_PS] {
        assert_eq!(
            apply(group, &[0xd7, 0x69, 0x8c, 0xe2], 32),
            Some(3_800_852_951)
        );
        assert_eq!(apply(group, &[0xd7, 0x69, 0x8c], 24), None, "{group}");
        assert_eq!(
            apply(group, &[0xd7, 0x69, 0x8c, 0xe2, 0x00], 40),
            None,
            "{group}"
        );
    }
    assert_eq!((stats.decoded_ok, stats.decoded_err), (2, 4));
}

/// Only the two Swiftplay siblings map to their Bomb twins; a Bomb class and
/// the suffixed forms stay as they are (see `GROUP_ALIASES`). Pinned so a later
/// "make it consistent" edit has to argue with a test.
#[test]
fn canonical_group_maps_only_the_swiftplay_siblings() {
    assert_eq!(canonical_group(SWIFT_GS), BOMB_GS);
    assert_eq!(canonical_group(SWIFT_PS), BOMB_PS);
    for path in [BOMB_GS, BOMB_PS, "/Game/Whatever.Whatever_C"] {
        assert_eq!(canonical_group(path), path);
    }
    for suffix in ["_ClassNetCache", ":SomeFunction"] {
        let path = format!("{SWIFT_GS}{suffix}");
        assert_eq!(canonical_group(&path), path, "{suffix} should not alias");
    }
}

/// The point of the alias: a Swiftplay field resolves to the type its Bomb
/// twin has. `ChosenCeremonyForRound` is the live case -- it is in the table
/// exactly once, under the Bomb game state.
#[test]
fn a_swiftplay_field_resolves_through_its_bomb_twin() {
    for field in ["ChosenCeremonyForRound", "RoundResults", "BombState"] {
        let bomb = resolve_field_type(&TABLE, BOMB_GS, Some(field), None);
        let swift = resolve_field_type(&TABLE, SWIFT_GS, Some(field), None);
        assert_eq!(swift, bomb, "{field} must resolve the same on both classes");
        assert!(bomb.is_some(), "{field} should be in the table at all");
    }
}

/// The alias must not invent types. A name in neither class stays unresolved.
#[test]
fn the_alias_does_not_invent_a_type() {
    assert_eq!(
        resolve_field_type(&TABLE, SWIFT_GS, Some("NoSuchFieldAnywhere"), None),
        None,
    );
    // and an unaliased group gains nothing
    assert_eq!(
        resolve_field_type(
            &TABLE,
            "/Game/Nope.Nope_C",
            Some("ChosenCeremonyForRound"),
            None
        ),
        None,
    );
}

/// Both tables' contract: strictly sorted by their key, which
/// the reference binary searches need, and no key twice (a duplicate could
/// resolve differently in the hash index and in the search).
#[test]
fn table_is_sorted() {
    let unsorted = OVERLAY_TABLE
        .windows(2)
        .position(|w| (w[0].group_path, w[0].field_name) >= (w[1].group_path, w[1].field_name));
    assert_eq!(
        unsorted, None,
        "OVERLAY_TABLE not strictly sorted at that index"
    );
    let unsorted = OVERLAY_HANDLE_TABLE
        .windows(2)
        .position(|w| (w[0].group_path, w[0].handle) >= (w[1].group_path, w[1].handle));
    assert_eq!(
        unsorted, None,
        "OVERLAY_HANDLE_TABLE not strictly sorted at that index"
    );
}

/// Pins no other guard holds. The rows are what `apply_type_corrections.py
/// --check` does not verify: descriptor-declared types, the bare `EffectID`
/// donor (its EXPECTED substring also matches the RPC groups), and receivers
/// typed only by checksum, where a same-name entry (`None` here) would shadow
/// the checksum. The checksums are learned types CI's one-group checksum guard
/// cannot see; `None` where the donors disagree, so the checksum stays dropped.
#[test]
fn pins_no_other_guard_holds() {
    use FieldType::{Bool, Double, EnumByte, FName, Int32, Int64, ObjectNetGuid, VectorDouble};
    const VIPER: &str = "/Game/Characters/Pandemic/S0/Ability_E/\
GameObject_Pandemic_E_SmokeScreenManager.GameObject_Pandemic_E_SmokeScreenManager_C\
:MulticastAddSmokeScreenPoint";
    const CAGE_TRAP: &str = "/Game/Characters/Gumshoe/S0/Ability_4/\
Ability_Gumshoe_4_CageTrap.Ability_Gumshoe_4_CageTrap_C";
    const TRIP_WIRE: &str = "/Game/Characters/Gumshoe/S0/Ability_E/\
GameObject_Gumshoe_E_TripWire.GameObject_Gumshoe_E_TripWire_C";
    const SECOND_WIRE: &str = "/Game/Characters/Gumshoe/S0/Ability_E/\
GameObject_Gumshoe_E_TripWire_SecondWire.GameObject_Gumshoe_E_TripWire_SecondWire_C";
    const TRIP_ABILITY: &str = "/Game/Characters/Gumshoe/S0/Ability_E/\
Ability_Gumshoe_E_TripWire.Ability_Gumshoe_E_TripWire_C";
    for (group, field, want) in [
        (BOMB_PS, "CompetitiveTier", Some(Int32)),
        (
            REPLAY_AT_LOCATION,
            "Rotation",
            Some(FieldType::RotationShort),
        ),
        (
            "/Script/ShooterGame.AmmoComponent",
            "AuthResourceAmount",
            Some(Int32),
        ),
        (
            DAMAGE_BASE,
            "bDeathMontageEffectOverrideIsQueued",
            Some(Bool),
        ),
        (
            DAMAGE_POINT,
            "bDeathMontageEffectOverrideIsQueued",
            Some(Bool),
        ),
        (VIPER, "Translation", Some(VectorDouble)),
        (VIPER, "Scale3D", Some(VectorDouble)),
        // Cypher's pre-13.01 paths keep their entries.
        (TRIP_WIRE, "Deployed", Some(Bool)),
        (SECOND_WIRE, "Deployed", Some(Bool)),
        (TRIP_ABILITY, "CreatedByCharacter", Some(ObjectNetGuid)),
        (CAGE_TRAP, "CreatedByCharacter", Some(ObjectNetGuid)),
        (
            CAGE_TRAP,
            "RelativeScale3D",
            Some(FieldType::VectorNetQuantize { scale: 100 }),
        ),
        // Donors that must agree, or the checksum is dropped again.
        (PLAY_CONTINUOUS, "AllianceFilter", Some(EnumByte)),
        (
            "/Script/ShooterGame.EffectManagerComponent:MulticastPlayOneShotEffect",
            "AllianceFilter",
            Some(EnumByte),
        ),
        (
            "/Script/ShooterGame.EffectManagerComponent",
            "EffectID",
            Some(Int64),
        ),
        (FROM_CLIENT, "AllianceFilter", None),
        (FORCE_REMOVE, "ModuleType", None),
        (STOP_CONTINUOUS, "EffectID", None),
        // The component's own property, which the RPC parameter's entry must
        // not reach.
        (
            "/Script/ShooterGame.ForceModuleManagerComponent",
            "RespawnNumber",
            None,
        ),
    ] {
        assert_typed(group, field, want);
    }
    for (group, field, checksum, want) in [
        (FROM_CLIENT, "AllianceFilter", 2270825073, EnumByte),
        (FORCE_REMOVE, "ModuleType", 3263282135, EnumByte),
        (STOP_CONTINUOUS, "EffectID", 2340855891, Int64),
    ] {
        assert_eq!(
            resolve(group, field, Some(checksum)),
            Some(want),
            "{group} {field}"
        );
    }
    for (checksum, want) in [
        // DeathMontageEffectOverride and its Context.
        (1712763745, Some(ObjectNetGuid)),
        (2397897524, Some(ObjectNetGuid)),
        (255019476, Some(FName)),          // OriginalBuyerTeam
        (677106858, Some(Double)),         // HawkFlash Banking
        (2270825073, Some(EnumByte)),      // AllianceFilter
        (1051633025, Some(ObjectNetGuid)), // the weapon RPCs' EffectManagerComponent
        (3263282135, Some(EnumByte)),      // the force module's ModuleType
        // AuthEquipSpeed and the inventory's two correction counters.
        (3151779304, Some(EnumByte)),
        (3198546915, Some(Int32)),
        (1076231069, Some(Int32)),
        // Cypher's trap fields other than `Deployed`.
        (2035145197, None),
        (1992268157, None),
        (2749104612, None), // ReplicatedMovement: short and byte donors
    ] {
        assert_checksum(checksum, want);
    }
    assert!(
        !OVERLAY_TABLE
            .iter()
            .any(|e| e.field_type == FieldType::UInt64),
        "no entry should still read an int64 property as UInt64"
    );
}

#[test]
fn overlay_uses_an_explicit_property_handle_when_the_wire_name_is_missing() {
    const ENTRIES: &[OverlayEntry] = &[entry("Health", FieldType::Int32)];
    const HANDLES: &[OverlayHandleEntry] = &[at(9, "Health")];
    let table = OverlayTable::with_handles(ENTRIES, HANDLES);
    let data = 100i32.to_le_bytes();
    let (result, stats) = run(&table, "/test", None, Some(9), None, Some(&data), 32);
    assert_eq!(result.and_then(|value| value.value_i64), Some(100));
    let counts = (stats.decoded_ok, stats.no_field_name, stats.not_in_table);
    assert_eq!(counts, (1, 0, 0));
}

#[test]
fn overlay_keeps_direct_name_lookup_ahead_of_the_handle_fallback() {
    const ENTRIES: &[OverlayEntry] = &[
        entry("DeclaredName", FieldType::Int32),
        entry("RuntimeName", FieldType::Bool),
    ];
    const HANDLES: &[OverlayHandleEntry] = &[at(9, "DeclaredName")];
    let table = OverlayTable::with_handles(ENTRIES, HANDLES);
    let (result, stats) = run(
        &table,
        "/test",
        Some("RuntimeName"),
        Some(9),
        None,
        Some(&[1]),
        1,
    );
    assert_eq!(result.and_then(|value| value.value_bool), Some(true));
    assert_eq!(stats.decoded_ok, 1);
}

/// A REAL, different name declared at a handle is not typed through the
/// descriptor's stale mapping: a patch moving `OldField: Int32` to `NewField`
/// (a `Float`) would read `1.0f32` as 1065353216 with `Decode errors` at zero
/// (docs/OVERLAY_RESOLUTION.md "Fail-closed on a handle conflict").
#[test]
fn a_conflicting_declared_name_refuses_the_stale_handle_mapping() {
    const ENTRIES: &[OverlayEntry] = &[entry("OldField", FieldType::Int32)];
    const HANDLES: &[OverlayHandleEntry] = &[at(7, "OldField")];
    let table = OverlayTable::with_handles(ENTRIES, HANDLES);
    let data = 1.0f32.to_le_bytes();
    let (result, stats) = run(
        &table,
        "/test",
        Some("NewField"),
        Some(7),
        None,
        Some(&data),
        32,
    );
    assert!(result.is_none(), "must not type a conflicting declaration");
    assert_eq!(stats.decoded_ok, 0, "1065353216 must not be reported");
    assert_eq!(stats.handle_conflicts_refused, 1, "{stats:?}");
    assert_eq!(stats.not_in_table, 1, "the field is untyped, not failed");
}

/// The refusal must NOT fire on a bare decimal wire name: `"248"`, an
/// unresolved hardcoded FName index, declares nothing, and the handle fallback
/// is the only thing that can type such a field.
#[test]
fn a_bare_fname_index_still_reaches_the_handle_fallback() {
    const ENTRIES: &[OverlayEntry] = &[entry("Health", FieldType::Int32)];
    const HANDLES: &[OverlayHandleEntry] = &[at(7, "Health")];
    let table = OverlayTable::with_handles(ENTRIES, HANDLES);
    let data = 100i32.to_le_bytes();
    for wire_name in ["248", "0"] {
        let (result, stats) = run(
            &table,
            "/test",
            Some(wire_name),
            Some(7),
            None,
            Some(&data),
            32,
        );
        let value = result.and_then(|v| v.value_i64);
        assert_eq!(value, Some(100), "wire name {wire_name}");
        assert_eq!(stats.handle_conflicts_refused, 0, "wire name {wire_name}");
    }
}

/// The `b`-prefix probe resolves before the handle fallback, so a `bFoo`/`Foo`
/// spelling difference never reaches the refusal.
#[test]
fn a_b_prefixed_spelling_difference_is_not_treated_as_a_conflict() {
    const ENTRIES: &[OverlayEntry] = &[entry("bIsQueued", FieldType::Bool)];
    const HANDLES: &[OverlayHandleEntry] = &[at(7, "bIsQueued")];
    let table = OverlayTable::with_handles(ENTRIES, HANDLES);
    let (result, stats) = run(
        &table,
        "/test",
        Some("IsQueued"),
        Some(7),
        None,
        Some(&[1]),
        1,
    );
    assert_eq!(result.and_then(|v| v.value_bool), Some(true));
    assert_eq!(stats.handle_conflicts_refused, 0, "{stats:?}");
}

#[test]
fn apply_overlay_returns_none_for_no_field_name() {
    const ENTRIES: &[OverlayEntry] = &[entry("Health", FieldType::Int32)];
    let table = OverlayTable::new(ENTRIES);
    let (result, stats) = run(&table, "/test", None, None, None, Some(&[0; 4]), 32);
    assert!(result.is_none());
    assert_eq!(stats.no_field_name, 1);
}

/// A zero-bit payload is the value 0 for `EnumRemainingBits` (see
/// `scalar::decode_enum_remaining_bits`) and a `ZeroBits` failure for every
/// other type; a field with no payload takes the same arm, and is `ZeroBits`
/// for every type once it claims a nonzero width.
#[test]
fn a_zero_bit_payload_is_zero_only_for_enum_remaining_bits() {
    const ENTRIES: &[OverlayEntry] = &[
        entry("Empty", FieldType::Int32),
        entry("Zero", FieldType::EnumRemainingBits),
    ];
    let table = OverlayTable::new(ENTRIES);
    let empty: &[u8] = &[];
    for (field, raw_bits, bit_count, want) in [
        ("Zero", Some(empty), 0, Some(0)),
        ("Zero", None, 0, Some(0)),
        ("Zero", None, 8, None),
        ("Empty", Some(empty), 0, None),
        ("Empty", None, 8, None),
    ] {
        let case = format!("{field} {raw_bits:?} {bit_count}");
        let (r, stats) = run(
            &table,
            "/test",
            Some(field),
            None,
            None,
            raw_bits,
            bit_count,
        );
        let columns = r.expect("a typed field is attempted").into_columns();
        assert_eq!(columns, (want, None, None, None), "{case}");
        let kinds: Vec<String> = stats
            .error_report
            .top_n(usize::MAX)
            .iter()
            .map(|row| row.error_kind.to_string())
            .collect();
        if want.is_some() {
            assert_eq!((stats.decoded_ok, stats.decoded_err), (1, 0), "{case}");
            assert!(kinds.is_empty(), "{case}: {kinds:?}");
        } else {
            assert_eq!((stats.decoded_ok, stats.decoded_err), (0, 1), "{case}");
            assert_eq!(kinds, ["ZeroBits"], "{case}");
        }
    }
}

/// The error report's `kind` is the only column that says WHY a field failed,
/// so each failure must print the label of its own cause, asserted on the
/// printed label an operator reads. Every `DecodeError` variant the overlay can
/// meet has a case, checked by the test itself: `decode_field` names the
/// variant each fixture fails with, and the variants reached must equal the
/// list the wildcard-free match below is generated from.
#[test]
fn the_error_report_names_the_cause_of_each_failure() {
    /// A field payload: its bytes and its bit count.
    type Payload = (Vec<u8>, u32);
    // Generated from one list, so the list and the match cannot disagree.
    macro_rules! variants {
        ($($variant:ident),+ $(,)?) => {
            (
                [$(stringify!($variant)),+],
                |err: &DecodeError| -> &'static str {
                    match err {
                        $(DecodeError::$variant { .. } => stringify!($variant),)+
                    }
                },
            )
        };
    }
    let (every_variant, variant_of) = variants!(
        BitIo,
        NotFullyConsumed,
        RawOrSkip,
        UnsignedOverflow,
        NonFiniteComponent,
        InvalidQuantizationScale,
        InvalidFNameNumber,
        UnsupportedTextHistory,
        ByteArrayLengthCapExceeded,
        FTextTree,
    );
    static ENTRIES: [OverlayEntry; 16] = [
        entry("BadTextBool", FieldType::FTextTree),
        entry("BadUtf8", FieldType::FString),
        entry("ByteArrayOverCap", FieldType::ByteArray { max_bytes: 1 }),
        entry(
            "ByteArrayOverlongPrefix",
            FieldType::ByteArray { max_bytes: 8 },
        ),
        entry("EmptyFText", FieldType::FText),
        entry("LongInt", FieldType::Int32),
        entry("LongTextString", FieldType::FTextTree),
        entry("NaNVector", FieldType::VectorNetQuantize { scale: 100 }),
        entry("NegativeFNameNumber", FieldType::FName),
        entry("OverlongPrefix", FieldType::FString),
        entry("RunawayIntPacked", FieldType::ObjectNetGuid),
        entry("ShortInt", FieldType::Int32),
        entry("U64PastI64", FieldType::UInt64),
        entry("UnseenTextHistory", FieldType::FTextTree),
        entry(
            "ZeroQuantizeScale",
            FieldType::VectorNetQuantize { scale: 0 },
        ),
        entry("ZeroSerializedIntMax", FieldType::SerializedInt { max: 0 }),
    ];
    // An inline FName (hardcoded bit clear), "Source", instance number -1.
    let fname = BitWriter::new()
        .bit(false)
        .fstring("Source")
        .i32(-1)
        .finish();
    // A 7-bit SerializedInt(128) header of 0 -- no component bits, no extra
    // info -- takes the raw-f32 fallback, and the first word is a NaN.
    let nan_vector = BitWriter::new()
        .bits(0, 7)
        .u32(0x7fc0_0000)
        .f32(1.0)
        .f32(2.0)
        .finish();
    let bytes = |data: &[u8]| (data.to_vec(), data.len() as u32 * 8);
    // (field, payload, variant decode_field fails with, printed label)
    let cases: Vec<(&str, Payload, &str, &str)> = vec![
        // The payload is shorter than the type: a real EOF.
        ("ShortInt", bytes(&[0x01, 0x00]), "BitIo", "EOF"),
        // The type finished with bits to spare.
        (
            "LongInt",
            bytes(&[0x01, 0, 0, 0, 0]),
            "NotFullyConsumed",
            "Residual",
        ),
        // Length 3, then three bytes that are not UTF-8: consumed exactly,
        // nothing ran out.
        (
            "BadUtf8",
            bytes(&[0x03, 0, 0, 0, 0xff, 0xfe, 0x00]),
            "BitIo",
            "Malformed",
        ),
        // A length prefix of 100 with one byte behind it.
        (
            "OverlongPrefix",
            bytes(&[0x64, 0, 0, 0, 0x41]),
            "BitIo",
            "Malformed",
        ),
        // The same cause in a byte array: a count of 4, within the cap of 8,
        // with one byte behind it.
        (
            "ByteArrayOverlongPrefix",
            bytes(&[0x08, 0xaa]),
            "BitIo",
            "Malformed",
        ),
        // Three declared where the table allows one, and no byte behind
        // them: the cap is checked first, so the table constant is what the
        // report names.
        (
            "ByteArrayOverCap",
            bytes(&[0x06]),
            "ByteArrayLengthCapExceeded",
            "Rejected",
        ),
        // Five IntPacked bytes that never clear the continuation bit.
        ("RunawayIntPacked", bytes(&[0xff; 5]), "BitIo", "Malformed"),
        // The cap's boundary: two declared where the table allows one.
        (
            "ByteArrayOverCap",
            bytes(&[0x04, 0xaa, 0xbb]),
            "ByteArrayLengthCapExceeded",
            "Rejected",
        ),
        // Reads fine; the value has no i64 spelling.
        (
            "U64PastI64",
            bytes(&[0, 0, 0, 0, 0, 0, 0, 0x80]),
            "UnsignedOverflow",
            "Rejected",
        ),
        // A table parameter no value can be read against. No bit is wrong.
        ("ZeroSerializedIntMax", bytes(&[0x00]), "BitIo", "Rejected"),
        // The same for a quantized vector's divisor.
        (
            "ZeroQuantizeScale",
            bytes(&[0x00]),
            "InvalidQuantizationScale",
            "Rejected",
        ),
        // A valid empty tree: no string-table key to return.
        (
            "EmptyFText",
            bytes(&[0, 0, 0, 0, 0xff, 0, 0, 0, 0]),
            "UnsupportedTextHistory",
            "Rejected",
        ),
        // The full-tree reader's own causes, sorted the same way: a history
        // it has never seen laid out is refused ...
        (
            "UnseenTextHistory",
            bytes(&[0, 0, 0, 0, 5]),
            "FTextTree",
            "Rejected",
        ),
        // ... and an archive bool that is neither 0 nor 1 breaks the framing.
        (
            "BadTextBool",
            bytes(&[1, 0, 0, 0, 4, 3, 0, 0, 0, 0, 0, 0, 0xf0, 0x3f, 2, 0, 0, 0]),
            "FTextTree",
            "Malformed",
        ),
        // A string-table name of 1,000 bytes in a 201-bit window, as for
        // `OverlongPrefix`.
        (
            "LongTextString",
            BitWriter::new()
                .u32(0)
                .u8(11)
                .bit(false)
                .i32(1000)
                .repeat(false, 128)
                .finish(),
            "FTextTree",
            "Malformed",
        ),
        // Read in full; the value has no JSON spelling.
        ("NaNVector", nan_vector, "NonFiniteComponent", "Rejected"),
        // Read in full; a negative instance number has no display spelling.
        (
            "NegativeFNameNumber",
            fname,
            "InvalidFNameNumber",
            "Rejected",
        ),
    ];
    let table = OverlayTable::new(&ENTRIES);
    let mut printed = Vec::new();
    for (field, (data, bits), variant, _) in &cases {
        let field_type = ENTRIES
            .iter()
            .find(|entry| entry.field_name == *field)
            .map(|entry| entry.field_type)
            .unwrap_or_else(|| panic!("{field}: no entry"));
        let err = decode_field(field_type, data, *bits)
            .expect_err(&format!("{field}: the fixture must not decode"));
        assert_eq!(variant_of(&err), *variant, "{field}: {err:?}");

        let (result, stats) = run(&table, "/test", Some(field), None, None, Some(data), *bits);
        let columns = result.map(OverlayResult::into_columns);
        assert_eq!(
            columns,
            Some((None, None, None, None)),
            "{field}: must fail to decode"
        );
        assert_eq!(stats.decoded_err, 1, "{field}");
        let rows = stats.error_report.top_n(2);
        assert_eq!(rows.len(), 1, "{field}: one bucket");
        printed.push((*field, rows[0].error_kind.to_string()));
    }
    let wanted: Vec<(&str, String)> = cases
        .iter()
        .map(|(field, _, _, want)| (*field, (*want).to_owned()))
        .collect();
    assert_eq!(printed, wanted);

    // `Raw` and `Skip` return before `decode_field` is reached, so its
    // `RawOrSkip` arm cannot fire from the overlay and has no case.
    let reached: BTreeSet<&str> = cases.iter().map(|(_, _, variant, _)| *variant).collect();
    let expected: BTreeSet<&str> = every_variant
        .into_iter()
        .filter(|variant| *variant != "RawOrSkip")
        .collect();
    assert_eq!(
        reached, expected,
        "every DecodeError the overlay can meet needs a case"
    );
}

/// The hash index must answer exactly what the binary search answered, on
/// every key in the table and on keys that are not in it. A wrong
/// overlay type moves NO counter, so only this equivalence would catch an
/// index that quietly disagrees on a handful of entries.
#[test]
fn the_hash_index_answers_exactly_what_the_binary_search_answered() {
    for entry in &OVERLAY_TABLE {
        assert_eq!(
            TABLE.lookup(entry.group_path, entry.field_name),
            TABLE.lookup_by_binary_search(entry.group_path, entry.field_name),
            "direct lookup disagrees for {}::{}",
            entry.group_path,
            entry.field_name,
        );

        // The `b`-prefix fallback, asked the way the overlay asks it.
        for probe in [
            entry.field_name,
            entry
                .field_name
                .strip_prefix('b')
                .unwrap_or(entry.field_name),
        ] {
            assert_eq!(
                TABLE.lookup_b_prefixed(entry.group_path, probe),
                TABLE.lookup_b_prefixed_by_binary_search(entry.group_path, probe),
                "b-prefixed lookup disagrees for {}::b{}",
                entry.group_path,
                probe,
            );
        }

        // Keys that must miss: a real group with a name nothing declares,
        // and a real name under a group that does not carry it.
        for (group, name) in [
            (entry.group_path, "NoSuchFieldNameAnywhere"),
            ("/Game/NoSuchGroupPathAnywhere", entry.field_name),
            (entry.group_path, ""),
        ] {
            assert_eq!(
                TABLE.lookup(group, name),
                TABLE.lookup_by_binary_search(group, name),
                "miss disagrees for {group}::{name}",
            );
            assert_eq!(
                TABLE.lookup_b_prefixed(group, name),
                TABLE.lookup_b_prefixed_by_binary_search(group, name),
                "b-prefixed miss disagrees for {group}::b{name}",
            );
        }
    }
}

/// Same equivalence for the handle fallback table, including handles that are
/// not declared for a group that is.
#[test]
fn the_handle_index_answers_exactly_what_the_binary_search_answered() {
    for entry in &OVERLAY_HANDLE_TABLE {
        for handle in [entry.handle, entry.handle.wrapping_add(1000), u32::MAX, 0] {
            assert_eq!(
                TABLE.lookup_handle(entry.group_path, handle),
                TABLE.lookup_handle_by_binary_search(entry.group_path, handle),
                "handle lookup disagrees for {}::{handle}",
                entry.group_path,
            );
        }
        assert_eq!(
            TABLE.lookup_handle("/Game/NoSuchGroupPathAnywhere", entry.handle),
            None,
        );
    }
}

/// The four engine object references resolve by name once the table has
/// missed on the group and its alias (see `ENGINE_OBJECT_REFS`).
#[test]
fn the_engine_fallback_covers_every_one_of_the_four_names() {
    const BOMB_EQUIPPABLE: &str = "/Game/Equippables/Bomb/BombEquippable.BombEquippable_C";
    assert_eq!(
        TABLE.lookup(BOMB_EQUIPPABLE, "Owner"),
        None,
        "not in the table"
    );
    for group in ["/Game/NeverSeen.NeverSeen_C", BOMB_EQUIPPABLE] {
        for name in ["Owner", "Instigator", "AttachParent", "Controller"] {
            assert_eq!(
                resolve_field_type(&TABLE, group, Some(name), None),
                Some(FieldType::ObjectNetGuid),
                "{name} should resolve by name on {group}",
            );
        }
    }
}

/// The fallback is a fixed list, not "anything that looks like a reference".
#[test]
fn the_engine_fallback_does_not_invent_other_names() {
    for name in ["OwnerId", "Owner2", "MyOwner", "Parent", "Target"] {
        assert_eq!(
            resolve_field_type(&TABLE, "/Game/NeverSeen.NeverSeen_C", Some(name), None),
            None,
            "{name} must stay unresolved",
        );
    }
}

/// Checksum propagation: the undeclared `PlayerID` of
/// `ReplayPlayerController:ClientReplayReceiveInputEventProcessingCapture`
/// shares checksum 2396673102 with the declared Int32
/// `BombPlayerState_C.PlayerId`, and read as Int32 its rows hold exactly the
/// declared column's ten values. A Cypher trap wire under a path no build has
/// shipped resolves `Deployed` the same way.
#[test]
fn a_checksum_types_a_field_the_table_never_declared() {
    const UNDECLARED: &str =
        "/Script/ShooterGame.ReplayPlayerController:ClientReplayReceiveInputEventProcessingCapture";
    const FUTURE_WIRE: &str = "/Game/Characters/Gumshoe/S0/Ability_C/\
GameObject_Gumshoe_C_TripWire.GameObject_Gumshoe_C_TripWire_C";
    assert_eq!(TABLE.lookup(UNDECLARED, "PlayerID"), None, "not declared");
    assert_eq!(
        resolve(UNDECLARED, "PlayerID", Some(2396673102)),
        Some(FieldType::Int32),
    );
    assert_eq!(TABLE.lookup(FUTURE_WIRE, "Deployed"), None, "not declared");
    assert_eq!(
        resolve(FUTURE_WIRE, "Deployed", Some(3902815170)),
        Some(FieldType::Bool)
    );
}

/// A declared entry outranks the engine references and the checksum, which run
/// only after the table misses, so a declared `Owner` or `PlayerID` keeps its
/// type. Declared `Raw` or `Skip` is a decision not to decode, reported only by
/// `raw_or_skip`, so that counter must move once per field.
#[test]
fn a_declared_entry_outranks_the_engine_and_checksum_fallbacks() {
    static RAW: [OverlayEntry; 2] = [
        entry("Owner", FieldType::Raw),
        entry("PlayerID", FieldType::Raw),
    ];
    static SKIP: [OverlayEntry; 2] = [
        entry("Owner", FieldType::Skip),
        entry("PlayerID", FieldType::Skip),
    ];
    for (entries, field_type) in [(&RAW, FieldType::Raw), (&SKIP, FieldType::Skip)] {
        let table = OverlayTable::new(entries);
        assert_eq!(
            resolve_field_type(&table, "/test", Some("Owner"), None),
            Some(field_type)
        );
        // This checksum types an undeclared `PlayerID` as `Int32` elsewhere.
        let donated = Some(2396673102);
        assert_eq!(
            resolve_field_type_with_checksum(&table, "/test", Some("PlayerID"), None, donated),
            Some(field_type),
        );
        let (result, stats) = run(&table, "/test", Some("Owner"), None, None, Some(&[1]), 1);
        assert!(result.is_none(), "{field_type:?} must not be decoded");
        let counts = (
            stats.raw_or_skip,
            stats.decoded_ok,
            stats.decoded_err,
            stats.not_in_table,
            stats.no_field_name,
            stats.handle_conflicts_refused,
        );
        assert_eq!(counts, (1, 0, 0, 0, 0, 0), "{stats:?}");
    }
}

/// Every group the overlay assigns a `RepMovement` type (table or scoped), by
/// the location level `tools/check_rep_movement_levels.py` measures for it.
const REP_MOVEMENT_LOCATION_EVIDENCE: [(VectorQuantization, &[&str]); 2] = [
    (
        VectorQuantization::RoundTwoDecimals,
        &[
            SEEKER_NADE,
            "/Game/Characters/BountyHunter/S0/Ability_4/Pawn_BountyHunter_4_WolfHound.Pawn_BountyHunter_4_WolfHound_C",
            "/Game/Characters/Cashew/S0/Ability_E/AIPawn_Cashew_E_SeekingTargetMissile.AIPawn_Cashew_E_SeekingTargetMissile_C",
            CLAY_BOOMBOT,
            "/Game/Characters/Guide/S0/Ability_X/Pawn_Guide_X_Pack.Pawn_Guide_X_Pack_C",
            "/Game/Characters/Killjoy/S0/Ability_E/Pawn_Killjoy_E_Turret.Pawn_Killjoy_E_Turret_C",
            "/Game/Characters/Killjoy/S0/Ability_Q/Pawn_Killjoy_Q_StealthAlarmbot.Pawn_Killjoy_Q_StealthAlarmbot_C",
            "/Game/Characters/Pine/S0/Ability_E/Pawn_Pine_E_RadEater.Pawn_Pine_E_RadEater_C",
            "/Game/Characters/Stealth/S0/Ability_4/Pawn_Stealth_4_Decoy_V2.Pawn_Stealth_4_Decoy_V2_C",
            "/Game/Characters/Stealth/S0/Ability_E/Pawn_Stealth_E_TeleporterMoving_FakeTP.Pawn_Stealth_E_TeleporterMoving_FakeTP_C",
            "/Game/Characters/Stealth/S0/Ability_E/Pawn_Stealth_E_TeleporterStationary_FakeTP.Pawn_Stealth_E_TeleporterStationary_FakeTP_C",
        ],
    ),
    (
        VectorQuantization::RoundWholeNumber,
        &[
            "/Game/Characters/AggroBot/S0/Ability_4/Projectile_Aggrobot_C_ExplodeyPatch.Projectile_Aggrobot_C_ExplodeyPatch_C",
            "/Game/Characters/AggroBot/S0/Ability_E/Projectile_Aggrobot_Zamboni_Rocket.Projectile_Aggrobot_Zamboni_Rocket_C",
            "/Game/Characters/AggroBot/S0/Ability_E/Projectile_E_Aggrobot_DiscTurret_PowerWave.Projectile_E_Aggrobot_DiscTurret_PowerWave_C",
            "/Game/Characters/AggroBot/S0/Ability_E/Projectile_E_Aggrobot_OrbSpawner.Projectile_E_Aggrobot_OrbSpawner_C",
            "/Game/Characters/BountyHunter/S0/Ability_E/Projectile_E_BountyHunter_Divebomb.Projectile_E_BountyHunter_Divebomb_C",
            "/Game/Characters/BountyHunter/S0/Ability_Q/Projectile_Q_BountyHunter_TetherGrenade_SphereExpansion.Projectile_Q_BountyHunter_TetherGrenade_SphereExpansion_C",
            "/Game/Characters/Breach/S0/Ability_4/Projectile_Breach_4_FusionBlast.Projectile_Breach_4_FusionBlast_C",
            "/Game/Characters/Breach/S0/Ability_Q/Projectile_Breach_Q_ThroughWalls_Flash.Projectile_Breach_Q_ThroughWalls_Flash_C",
            "/Game/Characters/Cable/S0/Ability_4/Projectile_Cable_4_NetToss.Projectile_Cable_4_NetToss_C",
            "/Game/Characters/Cable/S0/Ability_4/Projectile_NetToss.Projectile_NetToss_C",
            "/Game/Characters/Cable/S0/Ability_E/Projectile_CableJam_InAir.Projectile_CableJam_InAir_C",
            "/Game/Characters/Cashew/S0/Ability_Q/Projectile_Cashew_Q_ShellShockGrenade.Projectile_Cashew_Q_ShellShockGrenade_C",
            "/Game/Characters/Cashew/S0/Ability_Q/Projectile_Cashew_Q_ShellShockGrenade_Bounce.Projectile_Cashew_Q_ShellShockGrenade_Bounce_C",
            "/Game/Characters/Cashew/S0/Ability_X/GameObject_Cashew_X_SegmentManager.GameObject_Cashew_X_SegmentManager_C",
            "/Game/Characters/Clay/S0/Ability_4/Projectile_Clay_4_ProjectilePrimary.Projectile_Clay_4_ProjectilePrimary_C",
            "/Game/Characters/Clay/S0/Ability_4/Projectile_Clay_4_ProjectileSecondary.Projectile_Clay_4_ProjectileSecondary_C",
            "/Game/Characters/Clay/S0/Ability_4/Projectile_Clay_4_Projectile_Primary.Projectile_Clay_4_Projectile_Primary_C",
            "/Game/Characters/Clay/S0/Ability_4/Projectile_Clay_4_Projectile_Secondary.Projectile_Clay_4_Projectile_Secondary_C",
            "/Game/Characters/Clay/S0/Ability_4/Projectile_Clay_4_Projectile_SecondarySpawner.Projectile_Clay_4_Projectile_SecondarySpawner_C",
            "/Game/Characters/Clay/S0/Ability_4/Projectile_Clay_4_SecondarySpawner.Projectile_Clay_4_SecondarySpawner_C",
            "/Game/Characters/Clay/S0/Ability_Q/Projectile_Clay_Q_Satchel_Arming.Projectile_Clay_Q_Satchel_Arming_C",
            "/Game/Characters/Clay/S0/Ability_X/Projectile_Clay_X_Rocket.Projectile_Clay_X_Rocket_C",
            "/Game/Characters/Deadeye/S0/Ability_4/Projectile_Deadeye_4_Trap_Dart.Projectile_Deadeye_4_Trap_Dart_C",
            "/Game/Characters/Grenadier/S0/Ability_4/Projectile_C_Grenadier_Flash.Projectile_C_Grenadier_Flash_C",
            "/Game/Characters/Grenadier/S0/Ability_4/Projectile_C_Grenadier_Flash_Underhand.Projectile_C_Grenadier_Flash_Underhand_C",
            "/Game/Characters/Grenadier/S0/Ability_E/Gameobject_Grenadier_E_SuppressionPulse.Gameobject_Grenadier_E_SuppressionPulse_C",
            "/Game/Characters/Grenadier/S0/Ability_E/Projectile_Grenadier_E_SuppressionBlade.Projectile_Grenadier_E_SuppressionBlade_C",
            "/Game/Characters/Grenadier/S0/Ability_Q/Projectile_Grenadier_Q_SemtexBasic.Projectile_Grenadier_Q_SemtexBasic_C",
            HAWK,
            "/Game/Characters/Gumshoe/S0/Ability_E/Projectile_Gumshoe_E_CameraTrackingDart.Projectile_Gumshoe_E_CameraTrackingDart_C",
            "/Game/Characters/Gumshoe/S0/Ability_Q/Projectile_Gumshoe_Q_CameraTrackingDart.Projectile_Gumshoe_Q_CameraTrackingDart_C",
            "/Game/Characters/Gumshoe/S0/Ability_Q/Zone_Gumshoe_Q_Cage.Zone_Gumshoe_Q_Cage_C",
            "/Game/Characters/Hunter/S0/Ability_4/AnimationUpdatePrototype/Projectile_Hunter_4_ExplosiveBolt_PrototypeBalance.Projectile_Hunter_4_ExplosiveBolt_PrototypeBalance_C",
            "/Game/Characters/Hunter/S0/Ability_4/Projectile_Hunter_4_ExplosiveBolt.Projectile_Hunter_4_ExplosiveBolt_C",
            "/Game/Characters/Hunter/S0/Ability_Q/Projectile_Hunter_Q_RevealBolt.Projectile_Hunter_Q_RevealBolt_C",
            "/Game/Characters/Iris/S0/Ability_4/GameObject_Thumper_Concuss.GameObject_Thumper_Concuss_C",
            "/Game/Characters/Iris/S0/Ability_4/GameObject_Thumper_Heal.GameObject_Thumper_Heal_C",
            "/Game/Characters/Iris/S0/Ability_4/Projectile_Thumper_Concuss.Projectile_Thumper_Concuss_C",
            "/Game/Characters/Iris/S0/Ability_4/Projectile_Thumper_Heal.Projectile_Thumper_Heal_C",
            "/Game/Characters/Killjoy/S0/Ability_4/Projectile_Killjoy_4_RemoteBees_MultiDetonate.Projectile_Killjoy_4_RemoteBees_MultiDetonate_C",
            "/Game/Characters/Mage/S0/Ability_4/Projectile_Mage_4_SplashGrenade.Projectile_Mage_4_SplashGrenade_C",
            "/Game/Characters/Mage/S0/Ability_E/GameObject_Mage_E_WorldSmoke.GameObject_Mage_E_WorldSmoke_C",
            "/Game/Characters/Mage/S0/Ability_E/Projectile_Mage_E_WorldSmoke.Projectile_Mage_E_WorldSmoke_C",
            "/Game/Characters/Mage/S0/Ability_Q/Projectile_Mage_Q_Wall.Projectile_Mage_Q_Wall_C",
            "/Game/Characters/Nox/S0/Ability_4/Projectile_Nox_BarbedWire.Projectile_Nox_BarbedWire_C",
            "/Game/Characters/Pandemic/S0/Ability_4/Projectile_Pandemic_4_SmokeGrenade.Projectile_Pandemic_4_SmokeGrenade_C",
            "/Game/Characters/Pandemic/S0/Ability_E/Projectile_Pandemic_E_SmokeScreen_NoCollision.Projectile_Pandemic_E_SmokeScreen_NoCollision_C",
            "/Game/Characters/Pandemic/S0/Ability_Q/Projectile_Pandemic_Q_AcidGrenade.Projectile_Pandemic_Q_AcidGrenade_C",
            "/Game/Characters/Paprika/S0/Ability_E/Projectile_Paprika_E_RedirectPull.Projectile_Paprika_E_RedirectPull_C",
            "/Game/Characters/Paprika/S0/Ability_Q/Projectile_Paprika_Q_RedirectFlash.Projectile_Paprika_Q_RedirectFlash_C",
            "/Game/Characters/Paprika/S0/Ability_X/Projectile_Paprika_X_FullBendPop.Projectile_Paprika_X_FullBendPop_C",
            "/Game/Characters/Phoenix/S0/Ability_4/Production/Projectile_Phoenix_4_Molotov_Production.Projectile_Phoenix_4_Molotov_Production_C",
            "/Game/Characters/Phoenix/S0/Ability_E/Production/Projectile_Phoenix_E_FlareCurve_Synced.Projectile_Phoenix_E_FlareCurve_Synced_C",
            "/Game/Characters/Phoenix/S0/Ability_E/Production/Projectile_Phoenix_E_FlareCurve_Synced_Right.Projectile_Phoenix_E_FlareCurve_Synced_Right_C",
            "/Game/Characters/Phoenix/S0/Ability_Q/Production/Projectile_Phoenix_Q_FlameWall_ThroughWall.Projectile_Phoenix_Q_FlameWall_ThroughWall_C",
            "/Game/Characters/Pine/S0/Ability_E/Projectile_Pine_E_RadEaterSpawner.Projectile_Pine_E_RadEaterSpawner_C",
            "/Game/Characters/Pine/S0/Ability_Q/Projectile_Pine_Q_TrapGrenade.Projectile_Pine_Q_TrapGrenade_C",
            "/Game/Characters/Pine/S0/Ability_Q/Projectile_Pine_Q_TrapGrenadeUnderhand.Projectile_Pine_Q_TrapGrenadeUnderhand_C",
            "/Game/Characters/Sarge/S0/Ability_Molotov/Projectile_Sarge_Q_Molotov_Production.Projectile_Sarge_Q_Molotov_Production_C",
            "/Game/Characters/Sarge/S0/Ability_SpeedStim/Projectile_Sarge_E_SpeedStim.Projectile_Sarge_E_SpeedStim_C",
            "/Game/Characters/Sequoia/S0/Ability_Q/Projectile_Sequoia_Q_FragileMissile.Projectile_Sequoia_Q_FragileMissile_C",
            "/Game/Characters/Smonk/S0/Ability_E/MapTargetSmoke/GameObject_Smonk_NewSmoke.GameObject_Smonk_NewSmoke_C",
            "/Game/Characters/Smonk/S0/Ability_E/MapTargetSmoke/GameObject_Smonk_NewSmoke_PDS.GameObject_Smonk_NewSmoke_PDS_C",
            "/Game/Characters/Smonk/S0/Ability_Q/DebuffKnife/DecayLauncher/GameObject_Smonk_Q_DecayExplosion.GameObject_Smonk_Q_DecayExplosion_C",
            "/Game/Characters/Smonk/S0/Ability_Q/DebuffKnife/DecayLauncher/Projectile_Smonk_DecayNade.Projectile_Smonk_DecayNade_C",
            "/Game/Characters/Sprinter/S0/Ability_4/Projectile_Neon_C_Tunnel.Projectile_Neon_C_Tunnel_C",
            "/Game/Characters/Sprinter/S0/Ability_4/Projectile_Neon_C_Tunnel_Cosmetic.Projectile_Neon_C_Tunnel_Cosmetic_C",
            "/Game/Characters/Sprinter/S0/Ability_Q/Projectile_Sprinter_4_GroundStrike.Projectile_Sprinter_4_GroundStrike_C",
            "/Game/Characters/Stealth/S0/Ability_Q/Projectile_Stealth_Q_BounceFlash.Projectile_Stealth_Q_BounceFlash_C",
            "/Game/Characters/Terra/S0/Ability_4/GameObject_Terra_C_TimeSlowGrenade_Explosion.GameObject_Terra_C_TimeSlowGrenade_Explosion_C",
            "/Game/Characters/Terra/S0/Ability_4/Projectile_Terra_C_TimeSlowGrenade.Projectile_Terra_C_TimeSlowGrenade_C",
            "/Game/Characters/Thorne/S0/Ability_4/Projectile_Thorne_4_SlowFIeld_Production.Projectile_Thorne_4_SlowFIeld_Production_C",
            "/Game/Characters/Vampire/S0/Ability_4/Projectile_Vampire_4_NearsightAoE.Projectile_Vampire_4_NearsightAoE_C",
            "/Game/Characters/Wraith/S0/Ability_4/Projectile_Wraith_4_Smoke.Projectile_Wraith_4_Smoke_C",
            "/Game/Characters/Wraith/S0/Ability_4/Zone_Wraith_4_Smoke.Zone_Wraith_4_Smoke_C",
            "/Game/Characters/Wraith/S0/Ability_Q/Projectile_Wraith_Q_NearsightMissile.Projectile_Wraith_Q_NearsightMissile_C",
            "/Game/Characters/Wushu/S0/Ability_4/Projectile_Wushu_4_Smoke.Projectile_Wushu_4_Smoke_C",
            "/Game/Weapons/WeaponPickups/EquippablePickupProjectile.EquippablePickupProjectile_C",
        ],
    ),
];

/// Every `RepMovement` type the overlay can assign carries its measured level,
/// and no unlisted group gets one: the level is not on the wire, so Unreal's
/// default (`RoundWholeNumber`) is a prior, not a measurement. The table and the
/// scoped types are held to the list by group, checksum propagation by admitting
/// no `RepMovement`. Short rotators and two-decimal locations go together (pawns).
#[test]
fn every_rep_movement_entry_carries_its_measured_location_level() {
    let table = OVERLAY_TABLE.iter().map(|e| (e.group_path, e.field_type));
    let scoped = SCOPED_TYPES
        .iter()
        .map(|&(_, group, _, field_type)| (group, field_type));
    let mut declared: BTreeMap<&str, VectorQuantization> = BTreeMap::new();
    let mut short = BTreeSet::new();
    for (group, field_type) in table.chain(scoped) {
        if let FieldType::RepMovement { rotation, location } = field_type {
            let previous = declared.insert(group, location);
            assert!(
                previous.is_none() || previous == Some(location),
                "{group}: RepMovement declared at two location levels"
            );
            if rotation == RotatorQuantization::ShortComponents {
                short.insert(group);
            }
        }
    }
    let mut measured = BTreeMap::new();
    for (level, groups) in REP_MOVEMENT_LOCATION_EVIDENCE {
        for &group in groups {
            let twice = measured.insert(group, level).is_some();
            assert!(!twice, "{group}: the evidence list names it twice");
        }
    }
    let two_decimals: BTreeSet<&str> = measured
        .iter()
        .filter(|&(_, &level)| level == VectorQuantization::RoundTwoDecimals)
        .map(|(&group, _)| group)
        .collect();
    assert_eq!(
        short, two_decimals,
        "short rotators and two decimals differ"
    );
    for (group, level) in &declared {
        assert_eq!(
            measured.get(group),
            Some(level),
            "{group}: declared {level:?}; the measured level differs or was never recorded"
        );
    }
    for group in measured.keys() {
        assert!(
            declared.contains_key(group),
            "{group}: measured but no route assigns it a RepMovement type"
        );
    }
    // The name rule the checksum map would apply is closed for this field
    // (donors disagree), so the table is the only route to a RepMovement type.
    assert!(
        CHECKSUM_TYPES
            .iter()
            .all(|(_, t)| !matches!(t, FieldType::RepMovement { .. })),
        "a checksum-propagated RepMovement type would bypass the per-class evidence"
    );
}

/// The map is only useful if it holds something; a silently empty generated
/// table would make every test above pass for the wrong reason.
#[test]
fn the_checksum_table_is_populated_and_sorted() {
    assert!(CHECKSUM_TYPES.len() > 300, "{}", CHECKSUM_TYPES.len());
    assert!(CHECKSUM_TYPES.windows(2).all(|w| w[0].0 < w[1].0));
}

/// The scoped heal and overheal-decay references decode through the ordinary
/// packed-NetGUID reader, with the payload consumed exactly. The 24-bit window
/// is the widest seen on these parameters (50,360 = 56 + 9*128 + 3*16384,
/// low-bit continuation); the single zero byte is the null reference, which
/// decodes to 0 -- Unreal's null NetGUID, as the damage-side references export
/// it -- not to an actor.
#[test]
fn heal_and_decay_references_decode_packed_guids_exactly() {
    let mut stats = OverlayStats::default();
    let mut apply = |group: &str, field: &str, handle: u32, checksum: u32, raw: &[u8], bits| {
        apply_scoped(&mut stats, group, field, handle, checksum, raw, bits).value_i64
    };
    let pawn = apply(
        HEAL_PARAMS,
        "EventInstigatorPawn",
        8,
        3_901_949_544,
        &[0x71, 0x13, 0x06],
        24,
    );
    assert_eq!(pawn, Some(50_360));
    let null = apply(DECAY_PARAMS, "DecayCauser", 9, 3_648_603_088, &[0x00], 8);
    assert_eq!(null, Some(0));
    // A byte the packed value never claims is a residual, not a value.
    let residual = apply(
        HEAL_PARAMS,
        "EventInstigator",
        7,
        3_087_885_251,
        &[0x71, 0x13, 0x06, 0x00],
        32,
    );
    assert_eq!(residual, None);
    assert_eq!((stats.decoded_ok, stats.decoded_err), (2, 1));
}

/// The bomb-objective and ult-orb RPC parameters, each decoded from a payload
/// cut from a 13.06 replay. The references are packed NetGUIDs consumed
/// exactly; `PlantSite` is read at its 2-bit wire width, the width of every
/// row that carries it (an absent row is the default site 0).
#[test]
fn bomb_objective_and_orb_rpc_parameters_decode_their_wire_payloads() {
    const PLANTED: &str =
        "/Game/GameModes/Components/Comp_BombEvents.Comp_BombEvents_C:BombPlantedRPC";
    const DEFUSED: &str =
        "/Game/GameModes/Components/Comp_BombEvents.Comp_BombEvents_C:BombDefusedRPC";
    const ORB: &str = "/Game/BaseGameState.BaseGameState_C:OrbPickedUpRPC";
    let mut stats = OverlayStats::default();
    let mut apply = |group: &str, field: &str, handle: u32, checksum: u32, raw: &[u8], bits| {
        apply_scoped(&mut stats, group, field, handle, checksum, raw, bits).value_i64
    };
    let planter = apply(PLANTED, "BombPlanter", 1, 2_737_763_130, &[0x0d, 0x0e], 16);
    assert_eq!(planter, Some(902));
    assert_eq!(
        apply(PLANTED, "PlantSite", 2, 2_721_616_182, &[0x02], 2),
        Some(2)
    );
    assert_eq!(
        apply(PLANTED, "PlantSite", 2, 2_721_616_182, &[0x01], 2),
        Some(1)
    );
    let defuser = apply(
        DEFUSED,
        "DefusingCharacter",
        0,
        2_403_630_779,
        &[0xf5, 0x14],
        16,
    );
    assert_eq!(defuser, Some(1402));
    let gatherer = apply(ORB, "Orb Gatherer", 0, 2_930_585_102, &[0x65, 0x12], 16);
    assert_eq!(gatherer, Some(1202));
    let orb = apply(
        ORB,
        "Collectable Orb",
        1,
        2_066_098_074,
        &[0x85, 0x09, 0x04],
        24,
    );
    assert_eq!(orb, Some(33_346));
    // A byte the packed value never claims is a residual, not a value.
    let residual = apply(
        PLANTED,
        "BombPlanter",
        1,
        2_737_763_130,
        &[0x0d, 0x0e, 0x00],
        24,
    );
    assert_eq!(residual, None);
    assert_eq!((stats.decoded_ok, stats.decoded_err), (6, 1));
}
