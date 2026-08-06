//! The overlay table's resolution order, and the hash index's agreement
//! with the binary search it replaced.

use crate::checksum_table::CHECKSUM_TYPES;
use crate::decode::FieldType;
use crate::overlay::{
    OverlayEntry, OverlayHandleEntry, OverlayStats, OverlayTable, apply_overlay,
    apply_overlay_with_handle, canonical_group, group_hash_state, lookup_checksum,
    resolve_field_type, resolve_field_type_with_checksum,
};
use crate::{OVERLAY_HANDLE_TABLE, OVERLAY_TABLE};

const BOMB_GS: &str = "/Game/GameModes/Bomb/BombGameState.BombGameState_C";
const BOMB_PS: &str = "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C";
const SWIFT_GS: &str = "/Game/GameModes/_Development/Swiftplay_EndOfRoundCredits\
/Swiftplay_EoRCredits_GameState.Swiftplay_EoRCredits_GameState_C";
const SWIFT_PS: &str = "/Game/GameModes/_Development/Swiftplay_EndOfRoundCredits\
/Swiftplay_EoRCredits_PlayerState.Swiftplay_EoRCredits_PlayerState_C";

/// A Bomb class is already canonical and must not be rewritten.
#[test]
fn canonical_group_leaves_a_bomb_class_alone() {
    assert_eq!(canonical_group(BOMB_GS), BOMB_GS);
    assert_eq!(canonical_group(BOMB_PS), BOMB_PS);
    assert_eq!(
        canonical_group("/Game/Whatever.Whatever_C"),
        "/Game/Whatever.Whatever_C"
    );
}

#[test]
fn canonical_group_maps_the_swiftplay_siblings() {
    assert_eq!(canonical_group(SWIFT_GS), BOMB_GS);
    assert_eq!(canonical_group(SWIFT_PS), BOMB_PS);
}

/// Suffixed forms are deliberately NOT aliased: the table holds no entries for
/// the Bomb spellings of `_ClassNetCache` or `<Class>:<Function>`, so aliasing
/// them would be an untested claim buying nothing. Pinned so a later "make it
/// consistent" edit has to argue with a test.
#[test]
fn canonical_group_does_not_alias_the_suffixed_forms() {
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
    let table = OverlayTable::with_handles(&OVERLAY_TABLE, &OVERLAY_HANDLE_TABLE);
    for field in ["ChosenCeremonyForRound", "RoundResults", "BombState"] {
        let bomb = resolve_field_type(&table, BOMB_GS, Some(field), None);
        let swift = resolve_field_type(&table, SWIFT_GS, Some(field), None);
        assert_eq!(swift, bomb, "{field} must resolve the same on both classes");
        assert!(bomb.is_some(), "{field} should be in the table at all");
    }
}

/// The alias must not invent types. A name in neither class stays unresolved.
#[test]
fn the_alias_does_not_invent_a_type() {
    let table = OverlayTable::with_handles(&OVERLAY_TABLE, &OVERLAY_HANDLE_TABLE);
    assert_eq!(
        resolve_field_type(&table, SWIFT_GS, Some("NoSuchFieldAnywhere"), None),
        None,
    );
    // and an unaliased group gains nothing
    assert_eq!(
        resolve_field_type(
            &table,
            "/Game/Nope.Nope_C",
            Some("ChosenCeremonyForRound"),
            None
        ),
        None,
    );
}

#[test]
fn table_is_sorted() {
    let table = &OVERLAY_TABLE;
    for window in table.windows(2) {
        let cmp = window[0]
            .group_path
            .cmp(window[1].group_path)
            .then_with(|| window[0].field_name.cmp(window[1].field_name));
        assert!(
            cmp.is_lt() || cmp.is_eq(),
            "table not sorted at {:?} vs {:?}",
            (window[0].group_path, window[0].field_name),
            (window[1].group_path, window[1].field_name)
        );
    }
}

#[test]
fn lookup_finds_known_field() {
    let table = OverlayTable::new(&OVERLAY_TABLE);
    let ft = table.lookup(
        "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C",
        "CompetitiveTier",
    );
    assert_eq!(ft, Some(FieldType::Int32));
}

/// Live per-player economy is replicated under `MoneyManagementComponent` on
/// BOTH 13.01 and 13.02, but no C# descriptor declares the group, so without
/// these entries the fields ship untyped even though their raw bits decode to
/// real credits -- `Money` is 800 across all actors at pistol-round start and
/// runs 0..9000 in multiples of 50; `StartOfRoundMoney` is 800 active / 0
/// inactive; `TotalMoneyGranted` is cumulative 800..34200. `StartOfRoundMoney`'s
/// type is descriptor-corroborated (declared Int32 under OwnerExclusivePlayerInfo
/// at OwnerExclusivePlayerInfoDescriptor.cs:93, a separate end-of-round path).
/// See tools/apply_type_corrections.py ADDITIONS.
#[test]
fn money_management_economy_is_typed() {
    let table = OverlayTable::new(&OVERLAY_TABLE);
    let group = "/Script/ShooterGame.MoneyManagementComponent";
    assert_eq!(table.lookup(group, "Money"), Some(FieldType::Int32));
    assert_eq!(
        table.lookup(group, "StartOfRoundMoney"),
        Some(FieldType::Int32)
    );
    assert_eq!(
        table.lookup(group, "TotalMoneyGranted"),
        Some(FieldType::Int32)
    );
}

/// Concussion state is replicated under a SHARED component path
/// (`/Game/Characters/Components/Comp_Actor_Concussable.Comp_Actor_Concussable_C`)
/// that is attached to every player character -- it is NOT agent-specific.
/// On the 98605b1b Demos export the component appears on 9 distinct actors
/// spanning eight agents (Phoenix, Breach, Smonk, Clay, Guide, Wushu, Terra,
/// Pandemic, Deadeye) plus Guide's PossessableScout pawn, and the field
/// names and bit widths are identical on every one of them.
///
/// The widths are self-checking across all 375 rows: ConcussStartTime and
/// ConcussEndTime are 32 bits on all 39 rows each (Float), and ConcussLevel
/// is 64 bits on all 297 rows (Double). Read as Float, the start/end times
/// are game-seconds (389.5/392.0 ... 1916.7/1919.2 -- the ~2.5 s gap is the
/// concussion duration); read as Double, ConcussLevel runs the 0..1
/// intensity ramp. No descriptor declares this group, so the entries are
/// ADDITIONS in the same wire-evidence class as `Money` and `Ping`.
#[test]
fn concussion_fields_are_typed() {
    let table = OverlayTable::new(&OVERLAY_TABLE);
    let group = "/Game/Characters/Components/Comp_Actor_Concussable\
.Comp_Actor_Concussable_C";
    assert_eq!(
        table.lookup(group, "ConcussStartTime"),
        Some(FieldType::Float)
    );
    assert_eq!(
        table.lookup(group, "ConcussEndTime"),
        Some(FieldType::Float)
    );
    assert_eq!(table.lookup(group, "ConcussLevel"), Some(FieldType::Double));
}

/// `Comp_AbilityFuelSystem` is a generic component attached to a handful of
/// fuel-burning ability actors -- on 98605b1b that is Sage/Guide's heal
/// (`Ability_Guide_4_Heal`) and Viper/Pandemic's smoke screen. It is
/// per-ability, not per-player, but the component path is shared so a single
/// entry covers every actor that carries it.
///
/// CurrentFuel is 64 bits on all 5702 rows and reads as Double a smooth
/// 1.0 -> 0.0 drain (1.0, 0.9993, 0.9909, 0.9824, ...). IsFuelDraining is
/// 1 bit on all 60 rows, raw 0x00/0x01 -- an unambiguous Bool. The task note
/// guessed CurrentFuel as Float, but the wire is 64-bit; Double is what
/// decodes. No descriptor declares this group, so these are ADDITIONS.
#[test]
fn ability_fuel_fields_are_typed() {
    let table = OverlayTable::new(&OVERLAY_TABLE);
    let group = "/Game/Characters/Components/Comp_AbilityFuelSystem\
.Comp_AbilityFuelSystem_C";
    assert_eq!(table.lookup(group, "CurrentFuel"), Some(FieldType::Double));
    assert_eq!(table.lookup(group, "IsFuelDraining"), Some(FieldType::Bool));
}

/// `Ping` on BombPlayerState is a 16-bit LE unsigned integer that behaves
/// like latency in milliseconds (min ~6, p50 ~15, p90 ~19, max ~473 on
/// 02d4d478). No descriptor declares it, but the wire evidence is
/// overwhelming and the encoding was settled
/// (docs/archive/PROJECT_STATUS.md 18). Typed as `SerializedInt{65536}`
/// (16 bits LSB-first) -- the same wire-evidence ADDITION class as `Money`.
#[test]
fn ping_latency_is_typed() {
    let table = OverlayTable::new(&OVERLAY_TABLE);
    assert_eq!(
        table.lookup(
            "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C",
            "Ping"
        ),
        Some(FieldType::SerializedInt { max: 65536 })
    );
}

#[test]
fn equippable_used_is_an_object_net_guid() {
    // The C# descriptor attaches a custom decoder
    // (DamageParameters.cs:51 -> ValorantPayloadDecoders.Equippable), which
    // extract_descriptors.py cannot see through, so it lands in table.rs as
    // Raw. That decoder is exactly archive.ReadIntPacked(), i.e. our
    // ObjectNetGuid. Leaving it Raw forces consumers to guess the encoding;
    // the adapter guessed a fixed 16-bit LE integer and produced values that
    // were never valid NetGUIDs. tools/apply_type_corrections.py restores
    // the real type.
    let table = OverlayTable::new(&OVERLAY_TABLE);
    for group in [
        "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Base",
        "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Point",
    ] {
        assert_eq!(
            table.lookup(group, "EquippableUsed"),
            Some(FieldType::ObjectNetGuid),
            "EquippableUsed must decode as a net GUID in {group}",
        );
    }
}

#[test]
fn damage_geometry_fields_are_quantized_vectors() {
    // Same trap as EquippableUsed: DamageParameters attaches
    // ValorantPayloadDecoders.VectorNetQuantize* to these four, so
    // extract_descriptors.py cannot see the type and they land as Raw --
    // even though vrf-decode already implements the exact quantization.
    // Scales are the C# call sites: VectorNetQuantize = 1,
    // VectorNetQuantize100 = 100, VectorNetQuantizeNormal = unit vector.
    const BASE: &str = "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Base";
    const POINT: &str = "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Point";

    // DamageOrigin is on the shared base; the impact geometry only exists
    // for point damage, which is why the two groups differ here.
    let expected = [
        (
            BASE,
            "DamageOrigin",
            FieldType::VectorNetQuantize { scale: 100 },
        ),
        (
            POINT,
            "DamageOrigin",
            FieldType::VectorNetQuantize { scale: 100 },
        ),
        (
            POINT,
            "DamageImpactLocation",
            FieldType::VectorNetQuantize { scale: 1 },
        ),
        (
            POINT,
            "DamageImpactBoneRelativeLocation",
            FieldType::VectorNetQuantize { scale: 1 },
        ),
        (POINT, "DamageDirection", FieldType::VectorNetQuantizeNormal),
        (
            POINT,
            "DamageImpactNormal",
            FieldType::VectorNetQuantizeNormal,
        ),
    ];

    let table = OverlayTable::new(&OVERLAY_TABLE);
    for (group, field, want) in expected {
        assert_eq!(table.lookup(group, field), Some(want), "{field} in {group}");
    }
}

#[test]
fn overlay_falls_back_to_the_b_prefixed_boolean_name() {
    // The C# descriptors bind by handle and treat the name as a label, so
    // one spells a boolean `bDeathMontageEffectOverrideIsQueued` while the
    // replay declares it without the prefix. A name-keyed lookup misses,
    // and the field stayed raw on 581 rows.
    let entries: &[OverlayEntry] = &[OverlayEntry {
        group_path: "/test",
        field_name: "bIsQueued",
        field_type: FieldType::Bool,
    }];
    let table = OverlayTable::new(entries);
    let mut stats = OverlayStats::default();
    let data = [0x01u8];
    let result = apply_overlay(
        &table,
        "/test",
        group_hash_state("/test"),
        Some("IsQueued"),
        Some(&data),
        1,
        &mut stats,
    );
    assert_eq!(
        result.and_then(|r| r.value_bool),
        Some(true),
        "the unprefixed wire name must resolve to the b-prefixed entry",
    );
}

#[test]
fn overlay_falls_back_to_an_explicit_property_handle_when_the_wire_name_differs() {
    const GROUP: &str =
        "/Script/ShooterGame.ReplayEffectComponent:ReplayPlayContinuousEffectAtLocation";
    let entries: &[OverlayEntry] = &[OverlayEntry {
        group_path: GROUP,
        field_name: "Location",
        field_type: FieldType::VectorDouble,
    }];
    let handle_entries: &[OverlayHandleEntry] = &[OverlayHandleEntry {
        group_path: GROUP,
        handle: 26,
        field_name: "Location",
    }];
    let table = OverlayTable::with_handles(entries, handle_entries);
    let mut data = Vec::new();
    data.extend_from_slice(&1.25f64.to_le_bytes());
    data.extend_from_slice(&(-2.5f64).to_le_bytes());
    data.extend_from_slice(&3.75f64.to_le_bytes());
    let mut stats = OverlayStats::default();

    let result = apply_overlay_with_handle(
        &table,
        GROUP,
        group_hash_state(GROUP),
        Some("248"),
        26,
        Some(&data),
        192,
        &mut stats,
    );

    assert_eq!(
        result.and_then(|value| value.value_str),
        Some("(1.25,-2.5,3.75)".to_owned()),
    );
    assert_eq!(stats.decoded_ok, 1);
    assert_eq!(stats.not_in_table, 0);
}

#[test]
fn overlay_uses_an_explicit_property_handle_when_the_wire_name_is_missing() {
    let entries: &[OverlayEntry] = &[OverlayEntry {
        group_path: "/test",
        field_name: "Health",
        field_type: FieldType::Int32,
    }];
    let handle_entries: &[OverlayHandleEntry] = &[OverlayHandleEntry {
        group_path: "/test",
        handle: 9,
        field_name: "Health",
    }];
    let table = OverlayTable::with_handles(entries, handle_entries);
    let mut stats = OverlayStats::default();
    let data = 100i32.to_le_bytes();

    let result = apply_overlay_with_handle(
        &table,
        "/test",
        group_hash_state("/test"),
        None,
        9,
        Some(&data),
        32,
        &mut stats,
    );

    assert_eq!(result.and_then(|value| value.value_i64), Some(100));
    assert_eq!(stats.decoded_ok, 1);
    assert_eq!(stats.no_field_name, 0);
    assert_eq!(stats.not_in_table, 0);
}

#[test]
fn overlay_keeps_direct_name_lookup_ahead_of_the_handle_fallback() {
    let entries: &[OverlayEntry] = &[
        OverlayEntry {
            group_path: "/test",
            field_name: "DeclaredName",
            field_type: FieldType::Int32,
        },
        OverlayEntry {
            group_path: "/test",
            field_name: "RuntimeName",
            field_type: FieldType::Bool,
        },
    ];
    let handle_entries: &[OverlayHandleEntry] = &[OverlayHandleEntry {
        group_path: "/test",
        handle: 9,
        field_name: "DeclaredName",
    }];
    let table = OverlayTable::with_handles(entries, handle_entries);
    let mut stats = OverlayStats::default();

    let result = apply_overlay_with_handle(
        &table,
        "/test",
        group_hash_state("/test"),
        Some("RuntimeName"),
        9,
        Some(&[1]),
        1,
        &mut stats,
    );

    assert_eq!(result.and_then(|value| value.value_bool), Some(true));
    assert_eq!(stats.decoded_ok, 1);
}

#[test]
fn lookup_returns_none_for_unknown() {
    let table = OverlayTable::new(&OVERLAY_TABLE);
    let ft = table.lookup("nonexistent", "field");
    assert_eq!(ft, None);
}

#[test]
fn apply_overlay_decodes_int32() {
    let entries: &[OverlayEntry] = &[OverlayEntry {
        group_path: "/test",
        field_name: "Health",
        field_type: FieldType::Int32,
    }];
    let table = OverlayTable::new(entries);
    let mut stats = OverlayStats::default();
    let data = 100i32.to_le_bytes();
    let result = apply_overlay(
        &table,
        "/test",
        group_hash_state("/test"),
        Some("Health"),
        Some(&data),
        32,
        &mut stats,
    );
    assert!(result.is_some());
    let r = result.unwrap();
    assert_eq!(r.value_i64, Some(100));
    assert_eq!(stats.decoded_ok, 1);
}

#[test]
fn apply_overlay_returns_none_for_no_field_name() {
    let entries: &[OverlayEntry] = &[OverlayEntry {
        group_path: "/test",
        field_name: "Health",
        field_type: FieldType::Int32,
    }];
    let table = OverlayTable::new(entries);
    let mut stats = OverlayStats::default();
    let result = apply_overlay(
        &table,
        "/test",
        group_hash_state("/test"),
        None,
        Some(&[0; 4]),
        32,
        &mut stats,
    );
    assert!(result.is_none());
    assert_eq!(stats.no_field_name, 1);
}

#[test]
fn apply_overlay_graceful_on_decode_failure() {
    let entries: &[OverlayEntry] = &[OverlayEntry {
        group_path: "/test",
        field_name: "Broken",
        field_type: FieldType::FString, // needs more than 1 bit
    }];
    let table = OverlayTable::new(entries);
    let mut stats = OverlayStats::default();
    let data = [0x01u8]; // only 1 bit -- FString needs at least 32 bits for length
    let result = apply_overlay(
        &table,
        "/test",
        group_hash_state("/test"),
        Some("Broken"),
        Some(&data),
        1,
        &mut stats,
    );
    // Should return Some but with all values None (decode failure)
    assert!(result.is_some());
    let r = result.unwrap();
    assert_eq!(r.value_i64, None);
    assert_eq!(r.value_str, None);
    assert_eq!(stats.decoded_err, 1);
}

/// Byte-sized properties nested inside replicated arrays are written with
/// only their significant bits, so the decoder must take its width from the
/// payload rather than assuming 8.
///
/// This is not hypothetical: `CombatReport` `AssistType` arrives as a 5-bit
/// payload, and a fixed 8-bit read left all 364 of its rows in a real replay
/// untyped while every neighbouring field decoded fine.
#[test]
fn byte_takes_its_width_from_the_payload() {
    use crate::decode::{DecodedValue, FieldType, decode_field};

    // 5 significant bits holding 9 (0b01001), padded to one byte.
    let data = [0b0000_1001u8];
    for width in [1u32, 3, 5, 8] {
        let v = decode_field(FieldType::EnumByte, &data, width)
            .unwrap_or_else(|e| panic!("width {width} should decode: {e:?}"));
        let mask = ((1u16 << width) - 1) as u8;
        let expected = i64::from(0b0000_1001u8 & mask);
        assert_eq!(v, DecodedValue::I64(expected), "width {width}");
    }
}

/// A payload wider than a byte is not a byte field. Truncating to the low 8
/// bits would emit a plausible wrong number, so it is reported instead.
#[test]
fn byte_rejects_payloads_wider_than_eight_bits() {
    use crate::decode::{FieldType, decode_field};

    let data = [0xFFu8, 0xFF];
    // 12 bits declared: the nominal 8-bit read leaves 4 unconsumed, which
    // decode_field turns into an error rather than a truncated value.
    assert!(decode_field(FieldType::Byte, &data, 12).is_err());
}

/// The hash index must answer exactly what the binary search answered, on
/// every key in the generated table and on keys that are not in it.
///
/// A wrong overlay type moves NO summary counter -- the row still emits and
/// the block still walks -- so nothing but an equivalence check like this
/// one would catch an index that quietly disagrees on a handful of entries.
#[test]
fn the_hash_index_answers_exactly_what_the_binary_search_answered() {
    let table = OverlayTable::with_handles(&OVERLAY_TABLE, &OVERLAY_HANDLE_TABLE);

    for entry in &OVERLAY_TABLE {
        // Every real key.
        assert_eq!(
            table.lookup(entry.group_path, entry.field_name),
            table.lookup_by_binary_search(entry.group_path, entry.field_name),
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
                table.lookup_b_prefixed(entry.group_path, probe),
                table.lookup_b_prefixed_by_binary_search(entry.group_path, probe),
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
                table.lookup(group, name),
                table.lookup_by_binary_search(group, name),
                "miss disagrees for {group}::{name}",
            );
            assert_eq!(
                table.lookup_b_prefixed(group, name),
                table.lookup_b_prefixed_by_binary_search(group, name),
                "b-prefixed miss disagrees for {group}::b{name}",
            );
        }
    }
}

/// Same equivalence for the 84-entry handle fallback table, including
/// handles that are not declared for a group that is.
#[test]
fn the_handle_index_answers_exactly_what_the_binary_search_answered() {
    let table = OverlayTable::with_handles(&OVERLAY_TABLE, &OVERLAY_HANDLE_TABLE);

    for entry in &OVERLAY_HANDLE_TABLE {
        for handle in [entry.handle, entry.handle.wrapping_add(1000), u32::MAX, 0] {
            assert_eq!(
                table.lookup_handle(entry.group_path, handle),
                table.lookup_handle_by_binary_search(entry.group_path, handle),
                "handle lookup disagrees for {}::{handle}",
                entry.group_path,
            );
        }
        assert_eq!(
            table.lookup_handle("/Game/NoSuchGroupPathAnywhere", entry.handle),
            None,
        );
    }
}

/// `BlindManagerComponent.LongestActiveBlindDuration` is a 32-bit float giving
/// the longest active flash-blind duration in seconds (0.0..2.1 on observed
/// data). Common to all player characters. Typed as Float.
#[test]
fn blind_duration_is_typed() {
    let table = OverlayTable::new(&OVERLAY_TABLE);
    assert_eq!(
        table.lookup(
            "/Script/ShooterGame.BlindManagerComponent",
            "LongestActiveBlindDuration"
        ),
        Some(FieldType::Float)
    );
}

/// `MulticastNotifyHeal` and `MulticastNotifyOverhealDecay` are RPC parameter
/// blocks whose group paths the wire registers as
/// `/Script/ShooterGame.DamageableComponent:MulticastNotifyHeal` and
/// `:MulticastNotifyOverhealDecay`. The DamageableComponent C# descriptor
/// (DamageableComponentClassNetCacheDescriptor.cs) only declares the two
/// `MulticastNotifyDamage_*` handles, so the heal/decay parameter groups ship
/// untyped even though their scalars decode cleanly. The RPC sink resolves
/// these under their colon-group path with the bare parameter name, the same
/// shape as the EquippableUsed correction.
///
/// On the 98605b1b Demos export `HealTaken` is 32 bits on all 1252 rows and
/// reads as Float a 0.05..400 heal magnitude -- the 0x3f800000 bit pattern
/// (1.0f, the IEEE-754 identity) recurs, which is a float signature no int
/// read produces. `DecayApplied` is 32 bits on all 699 rows and reads as Float
/// a 0.07..50 overheal-decay amount, clustering tightly around 0.195 (a
/// per-tick decay). No descriptor declares either field, so these are
/// ADDITIONS in the same wire-evidence class as `Money` and `Ping`.
#[test]
fn heal_and_overheal_decay_scalars_are_typed() {
    let table = OverlayTable::new(&OVERLAY_TABLE);
    assert_eq!(
        table.lookup(
            "/Script/ShooterGame.DamageableComponent:MulticastNotifyHeal",
            "HealTaken"
        ),
        Some(FieldType::Float)
    );
    assert_eq!(
        table.lookup(
            "/Script/ShooterGame.DamageableComponent:MulticastNotifyOverhealDecay",
            "DecayApplied"
        ),
        Some(FieldType::Float)
    );
}

/// `PlayerScoreComponent.Score` is the per-player combat score. No C#
/// descriptor declares the group. On 98605b1b it is 32 bits on all 430 rows.
/// Read as Float the bytes are denormal slop (~1e-44) -- the float read
/// rejects itself -- but read as Int32 the values run 21..5833 with 415
/// distinct values, exactly the shape of a cumulative combat score across a
/// full match. Typed as Int32. ADDITION, same wire-evidence class as `Money`.
#[test]
fn player_score_is_typed() {
    let table = OverlayTable::new(&OVERLAY_TABLE);
    assert_eq!(
        table.lookup("/Script/ShooterGame.PlayerScoreComponent", "Score"),
        Some(FieldType::Int32)
    );
}

/// `ZoomMultiplierComponent` drives the ADS/scope FOV transition. No C#
/// descriptor declares the group, so its properties ship raw even though the
/// values are textbook floats. On 98605b1b all five fields below are 32 bits
/// on every row with zero NaN:
///   - SourceFov/TargetFov: 20.6..103.0, and 103.0 is Valorant's documented
///     default hip-fire FOV (the mode the player is in when not ADS), so a
///     wrong type cannot produce it.
///   - SourceFov1P/TargetFov1P: 5.0..70.0, and 70.0 is the default 1P FOV.
///   - TotalTransitionTimeDuration: 0.0..0.25, the ADS transition time.
///
/// SourceZoomLevel/TargetZoomLevel are deliberately NOT typed: ~70% of their
/// rows are the 0xFFFFFFFF sentinel, which Float reads as NaN, and the rest
/// are 0.0, so the field is an enum-or-sentinel, not a clean float. These
/// five are ADDITIONS, same wire-evidence class as `Money`.
#[test]
fn zoom_multiplier_fov_fields_are_typed() {
    let table = OverlayTable::new(&OVERLAY_TABLE);
    let group = "/Script/ShooterGame.ZoomMultiplierComponent";
    assert_eq!(table.lookup(group, "SourceFov"), Some(FieldType::Float));
    assert_eq!(table.lookup(group, "TargetFov"), Some(FieldType::Float));
    assert_eq!(table.lookup(group, "SourceFov1P"), Some(FieldType::Float));
    assert_eq!(table.lookup(group, "TargetFov1P"), Some(FieldType::Float));
    assert_eq!(
        table.lookup(group, "TotalTransitionTimeDuration"),
        Some(FieldType::Float)
    );
}

/// `UsableComponent` drives every hold-to-interact object: spike plant/defuse,
/// ultimate-orb pickup, doors. No C# descriptor declares the group. On a bomb
/// replay `HighestProgress` is 32 bits on ~12k rows and reads as Float a clean
/// 0..1 ramp advancing 1/128 per tick (a u32 read is non-monotonic; only the
/// float read is linear), and `bIsActive` is a single 0x01 bit on ~150 rows --
/// the "someone is interacting" flag. ADDITIONS, same wire-evidence class as
/// `Money`.
#[test]
fn usable_component_interaction_is_typed() {
    let table = OverlayTable::new(&OVERLAY_TABLE);
    let group = "/Script/ShooterGame.UsableComponent";
    assert_eq!(
        table.lookup(group, "HighestProgress"),
        Some(FieldType::Float)
    );
    assert_eq!(table.lookup(group, "bIsActive"), Some(FieldType::Bool));
}

/// `MagazineAmmo` is a bare group the replay never names -- every row is
/// handle 2 with field_name None. `HANDLE_ADDITIONS` names it `AmmoCount` and
/// the overlay types it Int32. On a bomb replay the u32 steps down 25,24,23...
/// per weapon as the magazine empties: classic ammo. This pins the full path
/// (handle table -> name -> type) that a name-only ADDITION cannot exercise.
#[test]
fn magazine_ammo_is_typed_via_handle_addition() {
    let table = OverlayTable::with_handles(&OVERLAY_TABLE, &OVERLAY_HANDLE_TABLE);
    let mut stats = OverlayStats::default();
    let data = 12i32.to_le_bytes();
    // field_name is None on the wire; the handle table must supply "AmmoCount".
    let result = apply_overlay_with_handle(
        &table,
        "MagazineAmmo",
        group_hash_state("MagazineAmmo"),
        None,
        2,
        Some(&data),
        32,
        &mut stats,
    );
    assert_eq!(result.and_then(|v| v.value_i64), Some(12));
    assert_eq!(stats.decoded_ok, 1);
    assert_eq!(stats.not_in_table, 0);
}

/// `FiniteSpeedMovementComponent` drives projectile travel. No C# descriptor
/// declares the group. `MaximumRange` is the projectile's max travel distance
/// in Unreal units: 32 bits on all 11699 rows on 98605b1b, reads as Float a
/// 397.6..49986.1 range with the mode at ~19993 UU (~500 m), which is the
/// right order of magnitude for a Valorant projectile. Typed as Float.
/// (bIsActive is deliberately NOT typed despite a clean 1-bit width: all 574
/// rows are 0x01, so the field carries no information a consumer can use, and
/// widening the table for a constant is not worth it.) ADDITION, same
/// wire-evidence class as `Money`.
#[test]
fn finite_speed_movement_max_range_is_typed() {
    let table = OverlayTable::new(&OVERLAY_TABLE);
    assert_eq!(
        table.lookup(
            "/Script/ShooterGame.FiniteSpeedMovementComponent",
            "MaximumRange"
        ),
        Some(FieldType::Float)
    );
}

/// `Owner`, `Instigator`, `AttachParent` and `Controller` are `AActor` /
/// `USceneComponent` object references Unreal replicates on every actor, always
/// as a NetGUID. The C# descriptors declare them only for the classes they
/// happen to cover, so on 02d4d478 they are typed on 129 group/field pairs
/// (4,601 rows) and untyped on 203 more (6,048 rows) -- same four names, same
/// encoding, no table entry. The type does not vary by class, so it resolves by
/// name once the table has missed on both the group and its alias.
#[test]
fn an_engine_object_ref_resolves_on_a_group_the_table_never_saw() {
    let table = OverlayTable::with_handles(&OVERLAY_TABLE, &OVERLAY_HANDLE_TABLE);
    const BOMB_EQUIPPABLE: &str = "/Game/Equippables/Bomb/BombEquippable.BombEquippable_C";
    assert_eq!(
        table.lookup(BOMB_EQUIPPABLE, "Owner"),
        None,
        "not in the table"
    );
    assert_eq!(
        resolve_field_type(&table, BOMB_EQUIPPABLE, Some("Owner"), None),
        Some(FieldType::ObjectNetGuid),
    );
}

#[test]
fn the_engine_fallback_covers_every_one_of_the_four_names() {
    let table = OverlayTable::with_handles(&OVERLAY_TABLE, &OVERLAY_HANDLE_TABLE);
    for name in ["Owner", "Instigator", "AttachParent", "Controller"] {
        assert_eq!(
            resolve_field_type(&table, "/Game/NeverSeen.NeverSeen_C", Some(name), None),
            Some(FieldType::ObjectNetGuid),
            "{name} should resolve by name",
        );
    }
}

/// The fallback is a fixed list, not "anything that looks like a reference".
#[test]
fn the_engine_fallback_does_not_invent_other_names() {
    let table = OverlayTable::with_handles(&OVERLAY_TABLE, &OVERLAY_HANDLE_TABLE);
    for name in ["OwnerId", "Owner2", "MyOwner", "Parent", "Target"] {
        assert_eq!(
            resolve_field_type(&table, "/Game/NeverSeen.NeverSeen_C", Some(name), None),
            None,
            "{name} must stay unresolved",
        );
    }
}

/// A declared entry still wins: the fallback only runs after the table misses,
/// so a class that really does spell one of these names differently keeps its
/// declared type.
#[test]
fn a_table_entry_outranks_the_engine_fallback() {
    let entries: &[OverlayEntry] = &[OverlayEntry {
        group_path: "/test",
        field_name: "Owner",
        field_type: FieldType::Raw,
    }];
    let table = OverlayTable::new(entries);
    assert_eq!(
        resolve_field_type(&table, "/test", Some("Owner"), None),
        Some(FieldType::Raw),
    );
}

/// The 192-bit RPC vectors. Unreal splits an `FTransform` parameter into three
/// separate double vectors on this wire, and no descriptor declares any of
/// them, so 54,859 rows on 02d4d478 arrived raw. Read as 3 x f64 they are
/// unambiguous -- `Scale3D` is exactly (1,1,1) on every row, which no other
/// split produces. ADDITIONS, same wire-evidence class as `Money`.
///
/// The replay's own `compatible_checksum` agrees with the grouping and was not
/// used to derive it: `248` is 598402184 wherever it appears, `249` is
/// 747197698, `Translation` 2235276067, `Scale3D` 2983776962.
#[test]
fn the_rpc_transform_vectors_are_typed() {
    let table = OverlayTable::new(&OVERLAY_TABLE);
    for (group, field) in [
        (
            "/Script/ShooterGame.EffectManagerComponent:MulticastPlayContinuousEffect",
            "Scale3D",
        ),
        (
            "/Script/ShooterGame.EffectManagerComponent:MulticastPlayContinuousEffect",
            "Translation",
        ),
        (
            "/Script/ShooterGame.EffectManagerComponent:MulticastPlayContinuousEffect",
            "249",
        ),
        (
            "/Script/ShooterGame.LocationalEffectManagerComponent:ClientPlayOneShotEffectAtLocation",
            "248",
        ),
        (
            "/Game/GameModes/Components/Comp_BombEvents.Comp_BombEvents_C:BombPlantedRPC",
            "PlantLocation",
        ),
        (
            "/Game/GameModes/Bomb/BombDestination.BombDestination_C:MulticastActivateBombSiteEffects",
            "BombLocation",
        ),
    ] {
        assert_eq!(
            table.lookup(group, field),
            Some(FieldType::VectorDouble),
            "{group}:{field}",
        );
    }
}

/// The decode that makes the reading unambiguous: the bytes below are the
/// `Scale3D` payload every row carries, and only a 3 x f64 split reads them as
/// (1,1,1). Six f32s would give (0, 1.875, 0, 1.875, 0, 1.875).
#[test]
fn a_192_bit_rpc_vector_decodes_as_three_doubles() {
    const GROUP: &str = "/Script/ShooterGame.EffectManagerComponent:MulticastPlayContinuousEffect";
    let table = OverlayTable::new(&OVERLAY_TABLE);
    let mut stats = OverlayStats::default();
    let mut bits = Vec::new();
    for _ in 0..3 {
        bits.extend_from_slice(&1.0f64.to_le_bytes());
    }
    let result = apply_overlay(
        &table,
        GROUP,
        group_hash_state(GROUP),
        Some("Scale3D"),
        Some(&bits),
        192,
        &mut stats,
    );
    assert_eq!(result.and_then(|v| v.value_str).as_deref(), Some("(1,1,1)"));
    assert_eq!(stats.decoded_ok, 1);
    assert_eq!(stats.decoded_err, 0);
}

/// Checksum propagation: a parameter no descriptor declares takes the type of
/// a declared field sharing its `compatible_checksum`.
///
/// `PlayerID` on `ReplayPlayerController:ClientReplayReceiveInputEvent`
/// `ProcessingCapture` is undeclared, and `BombPlayerState_C.PlayerId` is
/// declared `Int32`. They share checksum 2396673102, and reading the
/// undeclared rows as `Int32` yields exactly the ten values the declared column
/// holds.
#[test]
fn a_checksum_types_a_field_the_table_never_declared() {
    const UNDECLARED: &str =
        "/Script/ShooterGame.ReplayPlayerController:ClientReplayReceiveInputEventProcessingCapture";
    let table = OverlayTable::with_handles(&OVERLAY_TABLE, &OVERLAY_HANDLE_TABLE);
    assert_eq!(table.lookup(UNDECLARED, "PlayerID"), None, "not declared");
    assert_eq!(
        resolve_field_type_with_checksum(
            &table,
            UNDECLARED,
            Some("PlayerID"),
            None,
            Some(2396673102)
        ),
        Some(FieldType::Int32),
    );
}

/// The checksum runs last, so anything the table declares still wins.
#[test]
fn a_declared_entry_outranks_the_checksum() {
    let entries: &[OverlayEntry] = &[OverlayEntry {
        group_path: "/test",
        field_name: "PlayerID",
        field_type: FieldType::Raw,
    }];
    let table = OverlayTable::new(entries);
    assert_eq!(
        resolve_field_type_with_checksum(&table, "/test", Some("PlayerID"), None, Some(2396673102)),
        Some(FieldType::Raw),
    );
}

/// A checksum nothing donated types nothing -- the map asserts only what it
/// learned.
#[test]
fn an_unlearned_checksum_resolves_nothing() {
    let table = OverlayTable::with_handles(&OVERLAY_TABLE, &OVERLAY_HANDLE_TABLE);
    assert_eq!(
        resolve_field_type_with_checksum(
            &table,
            "/Game/Nope.Nope_C",
            Some("Whatever"),
            None,
            Some(1)
        ),
        None,
    );
}

/// The safety property: a checksum whose donors disagree is not in the table at
/// all, so the mechanism declines the cases it cannot settle. `ReplicatedMovement`
/// is the one that matters -- `ByteComponents` on 18 groups and `ShortComponents`
/// on 6, which differ in width, so guessing would desync the block rather than
/// read a wrong value.
#[test]
fn checksums_whose_donors_disagree_are_omitted() {
    for (checksum, why) in [
        (
            2749104612u32,
            "ReplicatedMovement: Byte vs Short components",
        ),
        (2270825073, "AllianceFilter: EnumByte vs EnumRemainingBits"),
    ] {
        assert_eq!(lookup_checksum(checksum), None, "{why}");
    }
}

/// The map is only useful if it holds something; a silently empty generated
/// table would make every test above pass for the wrong reason.
#[test]
fn the_checksum_table_is_populated_and_sorted() {
    assert!(CHECKSUM_TYPES.len() > 300, "{}", CHECKSUM_TYPES.len());
    assert!(CHECKSUM_TYPES.windows(2).all(|w| w[0].0 < w[1].0));
}

/// `StopMovementTime` is the other half of a pair whose `StartMovementTime` is
/// already Float: same RPC family, 32 bits on all 13,316 rows, and the same
/// shape read as f32 -- a -1.0 sentinel on 5,371 of them and 0.76..136.98 on
/// the rest, against -1.0..1771.83 for the declared sibling.
///
/// `HandleNumber` identifies a force module so a later Remove/Cleanup RPC can
/// name it. Read as u32 its 3,741 rows hold 1..765 with every value present --
/// a dense sequential id, which no other reading of those bits produces.
///
/// One entry each: checksum propagation carries `StopMovementTime` to
/// `ReplayStopContinuousEffectAtLocation` (244888268) and `HandleNumber` to
/// `NetMulticastRemoveForceModule` (3336285386).
#[test]
fn the_movement_time_pair_and_force_module_handle_are_typed() {
    let table = OverlayTable::new(&OVERLAY_TABLE);
    assert_eq!(
        table.lookup(
            "/Script/ShooterGame.EffectManagerComponent:MulticastStopContinuousEffect",
            "StopMovementTime"
        ),
        Some(FieldType::Float),
    );
    assert_eq!(
        table.lookup(
            "/Script/ShooterGame.ForceModuleManagerComponent:NetMulticastApplyForceModule",
            "HandleNumber"
        ),
        Some(FieldType::Int32),
    );
}
