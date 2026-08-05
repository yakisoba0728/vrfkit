//! The overlay table's resolution order, and the hash index's agreement
//! with the binary search it replaced.

use crate::decode::FieldType;
use crate::overlay::{
    OverlayEntry, OverlayHandleEntry, OverlayStats, OverlayTable, apply_overlay,
    apply_overlay_with_handle, canonical_group, group_hash_state, resolve_field_type,
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

/// `Ping` on BombPlayerState is a 16-bit LE unsigned integer that behaves
/// like latency in milliseconds (min ~6, p50 ~15, p90 ~19, max ~473 on
/// 02d4d478). No descriptor declares it, but the wire evidence is
/// overwhelming and the encoding was settled (PROJECT_STATUS 18). Typed as
/// `SerializedInt{65536}` (16 bits LSB-first) -- the same wire-evidence
/// ADDITION class as `Money`.
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
