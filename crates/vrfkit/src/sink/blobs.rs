//! Additive decoders for two payload shapes the field stream hands over whole.
//!
//! Both are *additive*: the parent row keeps its `raw_bits` and is emitted
//! either way, and these only add rows. A decoder that failed leaves the export
//! exactly as it would have been without it.
//!
//! - **Flattened arrays.** UE serialises a `TArray` of structs by flattening
//!   each element's members onto consecutive handles of the enclosing group, so
//!   the group's own net field exports name them. `decode_struct_array` walks
//!   that, and this module types and emits the leaves.
//! - **Struct blobs.** `RoundResults`, `TeamEconomy` and `RoundInfos` are
//!   opaque to the overlay table but have dedicated decoders in `vrf-decode`.

use smallvec::SmallVec;
use vrf_bitio::BitReader;
use vrf_decode::{ABILITY_CASTS_SCHEMA, COMBAT_ROUNDS_SCHEMA, FieldType};
use vrf_schema::NetGuidCache;

use super::intern::put;
use super::{ExportSink, FieldValues, TABLE};

/// The four typed columns a decoded value lands in. At most one is ever
/// populated; see the crate-level note on why this is four nullable columns
/// rather than a union.
type DecodedColumns = (Option<i64>, Option<f64>, Option<bool>, Option<String>);

#[derive(Clone, Copy)]
enum VerifiedArrayLeaf {
    Field(FieldType),
    KillWeaponTheme,
    TrackedRewardLocalizedText,
}

struct VerifiedNestedLeaf {
    path: String,
    handle: u32,
    bit_count: u32,
    raw_bits: Vec<u8>,
    value_i64: i64,
}

/// These identities were measured with exact consumption on all 714 replays.
/// They qualify wire windows only, without assigning gameplay ownership or
/// ordering semantics to the references.
fn verified_nested_container(
    parent: &str,
    handle: u32,
    name: Option<&str>,
    checksum: Option<u32>,
    resolved: Option<FieldType>,
) -> bool {
    let expected = match parent {
        "SelectedV2" => (13, "EquippableAttachments", 3_137_596_882),
        "KillData" => (6, "AssistingPlayers", 1_689_463_717),
        _ => return false,
    };
    (handle, name, checksum) == (expected.0, Some(expected.1), Some(expected.2))
        && resolved.is_none()
}

fn verified_nested_member(
    parent: &str,
    handle: u32,
    name: Option<&str>,
    checksum: Option<u32>,
    resolved: Option<FieldType>,
) -> bool {
    let expected = match (parent, handle) {
        ("SelectedV2", 14) => ("SocketAsset", 3_666_994_016),
        ("SelectedV2", 15) => ("AttachmentAsset", 856_446_005),
        ("KillData", 7) => ("AssistingPlayers", 1_417_448_159),
        _ => return false,
    };
    name == Some(expected.0)
        && checksum == Some(expected.1)
        && (resolved.is_none() || resolved == Some(FieldType::ObjectNetGuid))
}

pub(super) fn strict_nested_array_preflight(raw: &[u8], bit_count: u32, allowed: &[u32]) -> bool {
    // The generic walker tolerates EOF terminators and skips zero-width fields
    // for older routes. These new measured routes require explicit terminators
    // and inspect every handle before a zero-width member could disappear.
    if bit_count == 0 {
        return false;
    }
    let Ok(mut reader) = BitReader::with_bit_len(raw, u64::from(bit_count)) else {
        return false;
    };
    let Ok(capacity) = reader.read_int_packed() else {
        return false;
    };
    if capacity > vrf_decode::MAX_ELEMENTS {
        return false;
    }
    let mut elements = 0;
    loop {
        if reader.at_end() {
            return false;
        }
        let Ok(encoded_index) = reader.read_int_packed() else {
            return false;
        };
        if encoded_index == 0 {
            return reader.at_end();
        }
        if encoded_index > capacity || elements == vrf_decode::MAX_ELEMENTS {
            return false;
        }
        elements += 1;
        let mut fields = 0;
        loop {
            if reader.at_end() {
                return false;
            }
            let Ok(encoded_handle) = reader.read_int_packed() else {
                return false;
            };
            if encoded_handle == 0 {
                break;
            }
            if fields == vrf_decode::MAX_FIELDS_PER_ELEMENT {
                return false;
            }
            fields += 1;
            let handle = encoded_handle - 1;
            if !allowed.contains(&handle) {
                return false;
            }
            let Ok(payload_bits) = reader.read_int_packed() else {
                return false;
            };
            if payload_bits == 0 || u64::from(payload_bits) > reader.bits_remaining() {
                return false;
            }
            if reader.sub_reader(u64::from(payload_bits)).is_err() {
                return false;
            }
        }
    }
}

fn decode_verified_nested_array(
    parent: &str,
    container: &vrf_decode::FlattenedField,
    declared_names: &[Option<&str>],
    declared_checksums: &[Option<u32>],
    group_path: &str,
) -> (
    Option<Vec<VerifiedNestedLeaf>>,
    vrf_decode::ArrayDecodeStats,
    u64,
) {
    let declared_name = declared_names
        .get(container.handle as usize)
        .copied()
        .flatten();
    let declared_checksum = declared_checksums
        .get(container.handle as usize)
        .copied()
        .flatten();
    let resolved =
        vrf_decode::resolve_field_type(&TABLE, group_path, declared_name, Some(container.handle));
    if !verified_nested_container(
        parent,
        container.handle,
        declared_name,
        declared_checksum,
        resolved,
    ) {
        return (None, vrf_decode::ArrayDecodeStats::default(), 0);
    }

    let allowed: &[u32] = if parent == "SelectedV2" {
        &[14, 15]
    } else {
        &[7]
    };
    if !strict_nested_array_preflight(&container.raw_bits, container.bit_count, allowed) {
        let stats = vrf_decode::ArrayDecodeStats {
            errors: 1,
            ..Default::default()
        };
        return (None, stats, 0);
    }

    let mut stats = vrf_decode::ArrayDecodeStats::default();
    let flattened = vrf_decode::decode_struct_array_exact(
        &container.raw_bits,
        container.bit_count,
        declared_names,
        &mut stats,
    );
    let complete = stats.truncations == 0
        && stats.errors == 0
        && stats.implicit_terminations == 0
        && stats.unconsumed_nested_bits == 0
        && stats.unconsumed_root_bits == 0;
    if !complete {
        return (None, stats, 0);
    }

    let mut decoded = Vec::with_capacity(flattened.len());
    for leaf in flattened {
        let declared_name = declared_names.get(leaf.handle as usize).copied().flatten();
        let declared_checksum = declared_checksums
            .get(leaf.handle as usize)
            .copied()
            .flatten();
        let resolved =
            vrf_decode::resolve_field_type(&TABLE, group_path, declared_name, Some(leaf.handle));
        if !verified_nested_member(
            parent,
            leaf.handle,
            declared_name,
            declared_checksum,
            resolved,
        ) {
            return (None, stats, 1);
        }
        let mut failures = 0;
        let (value_i64, value_f64, value_bool, value_str) = decode_leaf_with_stats(
            FieldType::ObjectNetGuid,
            &leaf.raw_bits,
            leaf.bit_count,
            &mut failures,
        );
        let Some(value_i64) = value_i64 else {
            return (None, stats, failures.max(1));
        };
        if value_f64.is_some() || value_bool.is_some() || value_str.is_some() {
            return (None, stats, failures.max(1));
        }
        decoded.push(VerifiedNestedLeaf {
            path: leaf.path,
            handle: leaf.handle,
            bit_count: leaf.bit_count,
            raw_bits: leaf.raw_bits,
            value_i64,
        });
    }
    (Some(decoded), stats, 0)
}

/// New structural-array routes may type only leaf windows independently
/// validated across the corpus. Everything else remains an exact raw child.
fn verified_array_leaf_type(
    parent: &str,
    checksum: Option<u32>,
    handle: u32,
    resolved: Option<FieldType>,
    declared_name: Option<&str>,
) -> Option<FieldType> {
    let wanted = match (parent, checksum, handle) {
        ("AllPlayersObfuscatedPlayerInformation", Some(1_349_268_968), 49) => FieldType::Bool,
        ("AllPlayersObfuscatedPlayerInformation", Some(1_349_268_968), 50) => FieldType::EnumByte,
        ("ServerActiveEffects", Some(3_301_618_856), 5 | 6) => FieldType::Bool,
        ("ServerActiveEffects", Some(3_301_618_856), 7 | 8) => FieldType::ObjectNetGuid,
        ("ServerActiveEffects", Some(3_301_618_856), 30 | 31) => FieldType::VectorDouble,
        ("ServerActiveEffects", Some(3_301_618_856), 33) => FieldType::Float,
        ("ServerActiveEffects", Some(3_301_618_856), 34) => FieldType::EnumByte,
        _ => return None,
    };
    // These two members have no top-level property overlay. Their exact
    // 192-bit windows were independently decoded across all measured builds;
    // keep this typing scoped to the qualified parent array and declared leaf.
    let measured_vector = parent == "ServerActiveEffects"
        && checksum == Some(3_301_618_856)
        && matches!(
            (handle, declared_name),
            (30, Some("Translation")) | (31, Some("Scale3D"))
        );
    (resolved == Some(wanted) || (resolved.is_none() && measured_vector)).then_some(wanted)
}

/// ActiveBlinds has no top-level overlay for its struct members. Both the
/// enclosing checksum and each member declaration were observed unchanged in
/// 13.02 and 13.05. A changed name/checksum or conflicting future overlay
/// refuses typing, while the parent's raw_bits stay available.
fn verified_blind_leaf_type(
    handle: u32,
    name: Option<&str>,
    checksum: Option<u32>,
    resolved: Option<FieldType>,
) -> Option<FieldType> {
    let (wanted_name, wanted_checksum, wanted_type) = match handle {
        3 => ("BlindId", 2_836_858_544, FieldType::UInt32),
        4 => ("EffectID", 3_321_413_110, FieldType::UInt64),
        5 => ("SourceID", 4_130_766_059, FieldType::FName),
        6 => ("bLocalEffect", 2_802_682_995, FieldType::Bool),
        7 => ("bTransient", 815_378_154, FieldType::Bool),
        8 => ("InitialDuration", 1_370_668_337, FieldType::Float),
        9 => ("StartNetMovementTime", 2_358_118_895, FieldType::Float),
        10 => ("BlindConfig", 4_121_438_116, FieldType::ObjectNetGuid),
        11 => ("CausingActor", 2_370_661_694, FieldType::ObjectNetGuid),
        _ => return None,
    };
    (name == Some(wanted_name)
        && checksum == Some(wanted_checksum)
        && (resolved.is_none() || resolved == Some(wanted_type)))
    .then_some(wanted_type)
}

fn blind_member_width_valid(handle: u32, width: u32) -> bool {
    match handle {
        3 => width == 32,
        4 => width == 64,
        5 => width == 297,
        6 | 7 => width == 1,
        8 | 9 => width == 32,
        10 => width == 16,
        // Object references use IntPacked. A null actor is the one-byte zero,
        // observed in 59 main/checkpoint windows across the 81-file audit.
        11 => matches!(width, 8 | 16 | 24),
        _ => false,
    }
}

/// ActiveBlinds empty deltas may carry one extra zero IntPacked after the
/// index terminator (57 observed windows in 13.01/13.02/13.04/13.05). Consume
/// that exact trailer before the strict walker; the exported parent still
/// retains the original bit count and bytes. Populated arrays, nonzero tails
/// and multiple/truncated trailers keep the ordinary exact-window checks.
fn active_blind_array_bits(raw: &[u8], bit_count: u32) -> u32 {
    let without_empty_trailer = (|| {
        let mut reader = BitReader::with_bit_len(raw, u64::from(bit_count)).ok()?;
        let capacity = reader.read_int_packed().ok()?;
        if capacity > vrf_decode::MAX_ELEMENTS || reader.read_int_packed().ok()? != 0 {
            return None;
        }
        if reader.bits_remaining() != 8 || reader.read_int_packed().ok()? != 0 {
            return None;
        }
        Some(bit_count - 8)
    })();
    without_empty_trailer.unwrap_or(bit_count)
}

/// `TrackedRewards` leaf types have independent full-corpus evidence. The
/// enclosing exact-array route proves the framing; each typed leaf still needs
/// its own declared name, handle, checksum, and overlay type to agree.
fn verified_reward_leaf_type(
    handle: u32,
    declared_name: Option<&str>,
    declared_checksum: Option<u32>,
    resolved: Option<FieldType>,
) -> Option<FieldType> {
    let (name, checksum, wanted) = match handle {
        28 => ("RewardName", 1_337_472_711, FieldType::FName),
        30 => ("InstancesOfReward", 2_922_243_316, FieldType::Int32),
        31 => ("RewardGrantStrategy", 3_589_631_714, FieldType::EnumByte),
        32 => ("Source", 1_118_571_008, FieldType::EnumByte),
        _ => return None,
    };
    (declared_name == Some(name) && declared_checksum == Some(checksum) && resolved == Some(wanted))
        .then_some(wanted)
}

/// RequestedIgnoreActors is an exact array route, but its child reference has
/// no descriptor overlay. Admit only the measured declaration and only while
/// resolution is absent or agrees; explicit Raw, Skip, and conflicts stay raw.
fn verified_requested_ignore_actor_leaf(
    handle: u32,
    declared_name: Option<&str>,
    declared_checksum: Option<u32>,
    resolved: Option<FieldType>,
) -> Option<FieldType> {
    (handle == 5
        && declared_name == Some("RequestedIgnoreActors")
        && declared_checksum == Some(3_344_674_359)
        && matches!(resolved, None | Some(FieldType::ObjectNetGuid)))
    .then_some(FieldType::ObjectNetGuid)
}

/// This descriptor is deliberately Raw in the generated table. The full FText
/// reader is admitted only for this measured parent leaf; Skip, a conflict, or
/// any future declared type change remains raw.
fn verified_reward_localized_text(
    handle: u32,
    declared_name: Option<&str>,
    declared_checksum: Option<u32>,
    resolved: Option<FieldType>,
) -> bool {
    handle == 29
        && declared_name == Some("LocalizedRewardName")
        && declared_checksum == Some(483_770_233)
        && resolved == Some(FieldType::Raw)
}

fn decode_tracked_reward_localized_text(
    raw: &[u8],
    bit_count: u32,
    failures: &mut u64,
) -> DecodedColumns {
    match vrf_decode::decode_ftext_tree(raw, bit_count) {
        Ok(value) => (None, None, None, Some(value.to_json())),
        Err(_) => {
            *failures = failures.saturating_add(1);
            (None, None, None, None)
        }
    }
}

/// `SelectedV2` has six observed, declaration-qualified IntPacked NetGUID
/// leaves. The overlay has no entry for these members today, which is allowed
/// only here; an overlay that later names one as a different type is a refusal,
/// not a reason to silently prefer this measured route.
fn verified_selected_v2_leaf_type(
    handle: u32,
    declared_name: Option<&str>,
    declared_checksum: Option<u32>,
    resolved: Option<FieldType>,
) -> Option<FieldType> {
    let (name, checksum) = match handle {
        3 => ("EquippableDataAsset", 1_793_937_854),
        4 => ("EquippableSkinDataAsset", 3_765_038_216),
        5 => ("EquippableSkinLevelDataAsset", 603_923_741),
        6 => ("EquippableSkinChromaDataAsset", 3_166_589_204),
        7 => ("EquippableCharmDataAsset", 3_345_806_642),
        8 => ("EquippableCharmLevelDataAsset", 1_087_985_310),
        _ => return None,
    };
    (declared_name == Some(name)
        && declared_checksum == Some(checksum)
        && matches!(resolved, None | Some(FieldType::ObjectNetGuid)))
    .then_some(FieldType::ObjectNetGuid)
}

/// KillData primitive windows were measured over 714 replays. This establishes
/// their wire types, not the gameplay meaning or units of the numeric values.
/// Every admission remains scoped to the exact parent route and to the replay's
/// declared handle, name, and checksum. An explicit overlay disagreement,
/// including Raw or Skip, refuses the measured type.
fn verified_kill_data_leaf(
    handle: u32,
    declared_name: Option<&str>,
    declared_checksum: Option<u32>,
    resolved: Option<FieldType>,
) -> Option<VerifiedArrayLeaf> {
    let (name, checksum, kind) = match handle {
        3 => (
            "Victim",
            3_990_035_472,
            VerifiedArrayLeaf::Field(FieldType::ObjectNetGuid),
        ),
        4 => (
            "KillingEquippableClass",
            2_071_131_011,
            VerifiedArrayLeaf::Field(FieldType::ObjectNetGuid),
        ),
        5 => (
            "WeaponTheme",
            1_839_952_321,
            VerifiedArrayLeaf::KillWeaponTheme,
        ),
        9 => (
            "DamageType",
            2_992_423_760,
            VerifiedArrayLeaf::Field(FieldType::ObjectNetGuid),
        ),
        10 => (
            "DamageTaken",
            2_001_471_495,
            VerifiedArrayLeaf::Field(FieldType::Float),
        ),
        11 => (
            "DamageRegion",
            3_229_265_809,
            // Preserve the observed byte code; no enum-label meaning is claimed.
            VerifiedArrayLeaf::Field(FieldType::Byte),
        ),
        12 => (
            "GameTimeElapsed",
            3_684_431_363,
            VerifiedArrayLeaf::Field(FieldType::Float),
        ),
        13 => (
            "RoundTimestamp",
            2_328_473_242,
            VerifiedArrayLeaf::Field(FieldType::Float),
        ),
        14 => (
            "RoundNumber",
            843_024_485,
            VerifiedArrayLeaf::Field(FieldType::Int32),
        ),
        15 => (
            "bDidKillTriggerFinisher",
            2_795_684_046,
            VerifiedArrayLeaf::Field(FieldType::Bool),
        ),
        _ => return None,
    };
    if declared_name != Some(name) || declared_checksum != Some(checksum) {
        return None;
    }
    match kind {
        VerifiedArrayLeaf::Field(wanted) if resolved.is_none() || resolved == Some(wanted) => {
            Some(kind)
        }
        VerifiedArrayLeaf::KillWeaponTheme if resolved.is_none() => Some(kind),
        _ => None,
    }
}

fn decode_kill_weapon_theme(raw: &[u8], bit_count: u32, failures: &mut u64) -> DecodedColumns {
    let decoded = (|| {
        let mut reader = BitReader::with_bit_len(raw, u64::from(bit_count)).map_err(|_| ())?;
        if !reader.read_bit().map_err(|_| ())? {
            return Err(());
        }
        // The generic FString reader tolerates a missing null terminator.
        // This measured shape requires one for every nonzero length. Check it
        // explicitly, including valid UTF characters in the terminator slot,
        // before allocating or decoding the text.
        let mut framing = reader.clone();
        let length = framing.read_i32().map_err(|_| ())?;
        let units = i64::from(length).unsigned_abs();
        let unit_bits = if length < 0 { 16 } else { 8 };
        if units * (unit_bits / 8) > 64 * 1024 {
            return Err(());
        }
        if units != 0 {
            framing.skip_bits((units - 1) * unit_bits).map_err(|_| ())?;
            if framing.read_bits(unit_bits as u32).map_err(|_| ())? != 0 {
                return Err(());
            }
        }
        if framing.bits_remaining() != 0 {
            return Err(());
        }
        let value = reader.read_fstring(64 * 1024).map_err(|_| ())?;
        if reader.bits_remaining() != 0 {
            return Err(());
        }
        Ok(value)
    })();
    match decoded {
        Ok(value) => (None, None, None, Some(value)),
        Err(()) => {
            *failures = failures.saturating_add(1);
            (None, None, None, None)
        }
    }
}

fn measured_array_route(group: &str, parent: &str, checksum: Option<u32>) -> bool {
    matches!(
        (group, parent, checksum),
        (
            "/Script/ShooterGame.OwnerExclusivePlayerInfo",
            "AllPlayersObfuscatedPlayerInformation",
            Some(1_349_268_968)
        ) | (
            "/Script/ShooterGame.OwnerExclusivePlayerInfo",
            "TrackedRewards",
            Some(976_048_801)
        ) | (
            "/Script/ShooterGame.PersonalizationComponent",
            "SelectedV2",
            Some(4_218_721_055)
        ) | (
            "/Script/ShooterGame.PlayerMatchStatsComponent",
            "KillData",
            Some(1_493_759_848)
        ) | (
            "/Script/ShooterGame.EffectManagerComponent",
            "ServerActiveEffects",
            Some(3_301_618_856)
        ) | (
            "/Script/ShooterGame.FiniteSpeedMovementComponent",
            "RequestedIgnoreActors",
            Some(1_063_739_204)
        ) | (
            "/Script/ShooterGame.BlindManagerComponent",
            "ActiveBlinds",
            Some(3_853_965_310)
        )
    )
}

/// The sole measured empty `TrackedRewards` variant is a 24-bit `02 00 00`
/// window. Its first two packed values are capacity one and its index-zero
/// terminator; the final zero byte remains opaque. This is deliberately a
/// literal route, not a relaxation of `decode_struct_array_exact`: no other
/// suffix byte, bit length, or identity is accepted.
fn is_tracked_rewards_opaque_empty_variant(
    group: &str,
    parent: &str,
    checksum: Option<u32>,
    raw: &[u8],
    bit_count: u32,
) -> bool {
    matches!(
        (group, parent, checksum, bit_count, raw),
        (
            "/Script/ShooterGame.OwnerExclusivePlayerInfo",
            "TrackedRewards",
            Some(976_048_801),
            24,
            [0x02, 0x00, 0x00]
        )
    )
}

fn merge_array_stats(
    target: &mut vrf_decode::ArrayDecodeStats,
    source: &vrf_decode::ArrayDecodeStats,
) {
    target.elements_decoded += source.elements_decoded;
    target.fields_emitted += source.fields_emitted;
    target.truncations += source.truncations;
    target.errors += source.errors;
    target.unconsumed_nested_bits += source.unconsumed_nested_bits;
    target.unconsumed_root_bits += source.unconsumed_root_bits;
    target.implicit_terminations += source.implicit_terminations;
}

/// The struct-blob fields that have a dedicated decoder in `vrf-decode`.
#[derive(Clone, Copy)]
enum StructBlob {
    RoundResults,
    TeamEconomy,
    RoundInfos,
}

impl ExportSink<'_> {
    /// Every name the replay declares for `group_path`, indexed by handle.
    ///
    /// This is field-name resolution for a whole group at once, borrowed rather
    /// than cloned. The array walker needs the declaration for each of an
    /// element's flattened members, and resolving per leaf would re-resolve the
    /// group and allocate for every one.
    ///
    /// Empty when the group is unknown, which is exactly the "no declaration"
    /// case `decode_struct_array` falls back from.
    ///
    /// An associated function over `&NetGuidCache` rather than a `&self` method
    /// on purpose: the result borrows for as long as the walker runs, and a
    /// `&self` method would hold all of `self` and collide with the `&mut
    /// self.stats` the same call site needs.
    fn declared_handle_names<'g>(
        cache: &'g NetGuidCache,
        group_path: &str,
    ) -> Vec<Option<&'g str>> {
        let Some(group) = cache.get_group_by_path(group_path) else {
            return Vec::new();
        };
        group
            .fields
            .iter()
            .map(|slot| slot.as_ref().map(|f| f.name.as_str()))
            .collect()
    }

    /// Compatible checksums are declarations on the current enclosing group,
    /// just like the names above. Keep them indexed by handle so leaf typing
    /// cannot silently infer a checksum from the child position.
    fn declared_handle_checksums(cache: &NetGuidCache, group_path: &str) -> Vec<Option<u32>> {
        let Some(group) = cache.get_group_by_path(group_path) else {
            return Vec::new();
        };
        group
            .fields
            .iter()
            .map(|slot| slot.as_ref().map(|field| field.compatible_checksum))
            .collect()
    }

    /// Check if a field name is a known DynamicArray that should be flattened.
    pub(super) fn is_known_array_field(
        &self,
        field_name: Option<&str>,
        checksum: Option<u32>,
    ) -> bool {
        match (field_name, checksum) {
            (Some("Rounds"), _) => self.current_group_path.contains("CombatReportComponent"),
            // A RepLayout dynamic array of ability-cast structs. Each element
            // carries a GUID FString (handle 3), ints, floats and vectors;
            // `decode_struct_array` walks it with no hardcoded schema, naming
            // leaves from the replay's own declarations or `_h{N}`.
            (Some("AbilityCastsThisRound"), _) => self
                .current_group_path
                .contains("AbilityStatisticsReplicator"),
            (Some("AllPlayersObfuscatedPlayerInformation"), Some(1_349_268_968)) => {
                self.measured_array_routes
                    && self.current_group_path.as_ref()
                        == "/Script/ShooterGame.OwnerExclusivePlayerInfo"
            }
            (Some("TrackedRewards"), Some(976_048_801)) => {
                self.measured_array_routes
                    && self.current_group_path.as_ref()
                        == "/Script/ShooterGame.OwnerExclusivePlayerInfo"
            }
            (Some("SelectedV2"), Some(4_218_721_055)) => {
                self.measured_array_routes
                    && self.current_group_path.as_ref()
                        == "/Script/ShooterGame.PersonalizationComponent"
            }
            (Some("KillData"), Some(1_493_759_848)) => {
                self.measured_array_routes
                    && self.current_group_path.as_ref()
                        == "/Script/ShooterGame.PlayerMatchStatsComponent"
            }
            (Some("ServerActiveEffects"), Some(3_301_618_856)) => {
                self.measured_array_routes
                    && self.current_group_path.as_ref()
                        == "/Script/ShooterGame.EffectManagerComponent"
            }
            (Some("RequestedIgnoreActors"), Some(1_063_739_204)) => {
                self.measured_array_routes
                    && self.current_group_path.as_ref()
                        == "/Script/ShooterGame.FiniteSpeedMovementComponent"
            }
            (Some("ActiveBlinds"), Some(3_853_965_310)) => {
                self.measured_array_routes
                    && self.current_group_path.as_ref()
                        == "/Script/ShooterGame.BlindManagerComponent"
            }
            _ => false,
        }
    }

    /// Get the array schema for a known DynamicArray field.
    fn get_array_schema(
        &self,
        field_name: Option<&str>,
    ) -> Option<&'static vrf_decode::ArrayFieldSchema> {
        match field_name {
            Some("Rounds") if self.current_group_path.contains("CombatReportComponent") => {
                Some(&COMBAT_ROUNDS_SCHEMA)
            }
            // One cast per element, and each cast carries an `Effects` array of
            // the statistics it produced -- which in turn names the players each
            // one landed on. Without a schema the walker cannot see that
            // nesting, so `Effects` came out as one opaque leaf and the
            // authoritative debuff log stayed raw.
            Some("AbilityCastsThisRound")
                if self
                    .current_group_path
                    .contains("Comp_AbilityStatisticsReplicator") =>
            {
                Some(&ABILITY_CASTS_SCHEMA)
            }
            _ => None,
        }
    }

    /// Flatten a known DynamicArray field and emit one row per leaf.
    pub(super) fn emit_flattened_array(
        &mut self,
        field_name: Option<&str>,
        checksum: Option<u32>,
        raw: &[u8],
        bit_count: u32,
    ) {
        let schema = self.get_array_schema(field_name);
        let declared = Self::declared_handle_names(self.cache, &self.current_group_path);
        let declared_checksums =
            Self::declared_handle_checksums(self.cache, &self.current_group_path);
        let parent_name = field_name.unwrap_or("_array");
        let measured = self.measured_array_routes
            && measured_array_route(&self.current_group_path, parent_name, checksum);
        let array_bits = if measured && parent_name == "ActiveBlinds" {
            active_blind_array_bits(raw, bit_count)
        } else {
            bit_count
        };
        if measured
            && parent_name == "ActiveBlinds"
            && !strict_nested_array_preflight(raw, array_bits, &[3, 4, 5, 6, 7, 8, 9, 10, 11])
        {
            self.stats.array.errors += 1;
            return;
        }
        if measured
            && is_tracked_rewards_opaque_empty_variant(
                &self.current_group_path,
                parent_name,
                checksum,
                raw,
                bit_count,
            )
        {
            self.stats.tracked_rewards_opaque_empty_variants += 1;
            return;
        }
        let mut isolated = vrf_decode::ArrayDecodeStats::default();
        let flattened = if measured {
            vrf_decode::decode_struct_array_exact(raw, array_bits, &declared, &mut isolated)
        } else {
            vrf_decode::decode_struct_array(
                raw,
                bit_count,
                schema,
                &declared,
                &mut self.stats.array,
            )
        };
        if measured {
            let complete = isolated.truncations == 0
                && isolated.errors == 0
                && isolated.implicit_terminations == 0
                && isolated.unconsumed_nested_bits == 0
                && isolated.unconsumed_root_bits == 0;
            merge_array_stats(&mut self.stats.array, &isolated);
            if !complete {
                return;
            }
            if parent_name == "ActiveBlinds"
                && flattened
                    .iter()
                    .any(|field| !blind_member_width_valid(field.handle, field.bit_count))
            {
                self.stats.array_leaf_decode_errors += 1;
                return;
            }
            if parent_name == "ActiveBlinds"
                && flattened.iter().any(|field| {
                    let name = declared.get(field.handle as usize).copied().flatten();
                    let checksum = declared_checksums
                        .get(field.handle as usize)
                        .copied()
                        .flatten();
                    let resolved = vrf_decode::resolve_field_type(
                        &TABLE,
                        &self.current_group_path,
                        name,
                        Some(field.handle),
                    );
                    verified_blind_leaf_type(field.handle, name, checksum, resolved).is_none()
                })
            {
                self.stats.array_leaf_decode_errors += 1;
                return;
            }
        }

        // Resolve every leaf's type before touching `self.records`.
        //
        // The overlay table is asked first, keyed on the name the REPLAY
        // declares for that handle. UE flattens an array's element members into
        // consecutive handles on the enclosing group, so the group's own net
        // field export names them -- and the generated table already carries a
        // type for most of them, straight from the C# descriptor.
        //
        // Before this, the only source was `decode_array_leaf`'s hardcoded
        // handle->type match, which is a second copy of knowledge the table
        // already holds and was missing entries. `DeathLocation` is the
        // demonstrable case: handle 104, declared by the replay, typed
        // `VectorDouble` in the table, arriving 3,492 times on 02d4d478 and
        // emitted as an all-null `_h104` because the match had no arm for it.
        //
        // The hardcoded match stays as the fallback: it covers handles whose
        // declared name has no table entry, and dropping it would trade one gap
        // for another.
        let leaf_types: Vec<Option<VerifiedArrayLeaf>> = flattened
            .iter()
            .map(|f| {
                // The FULL resolution order, not a bare name lookup. An
                // ordinary field gets three steps -- name, b-prefixed name,
                // then handle -> descriptor name -> type -- and a flattened
                // leaf was getting only the first, so the same property could
                // be typed outside an array and untyped inside one.
                let name = declared.get(f.handle as usize).copied().flatten();
                let declared_resolved = vrf_decode::resolve_field_type(
                    &TABLE,
                    &self.current_group_path,
                    name,
                    Some(f.handle),
                );
                let resolved =
                    declared_resolved.filter(|ft| !matches!(ft, FieldType::Raw | FieldType::Skip));
                // Same lookup regardless of which branch below fires, so it is
                // done once here instead of once per branch. Pure and cheap --
                // an `Option` read off a slice -- so evaluating it on the
                // branches that never use it (the `verified_array_leaf_type`
                // and plain-`resolved` fallbacks) costs nothing observable.
                let declared_checksum =
                    declared_checksums.get(f.handle as usize).copied().flatten();
                if measured && parent_name == "TrackedRewards" {
                    if verified_reward_localized_text(
                        f.handle,
                        name,
                        declared_checksum,
                        declared_resolved,
                    ) {
                        Some(VerifiedArrayLeaf::TrackedRewardLocalizedText)
                    } else {
                        verified_reward_leaf_type(f.handle, name, declared_checksum, resolved)
                            .map(VerifiedArrayLeaf::Field)
                    }
                } else if measured && parent_name == "RequestedIgnoreActors" {
                    verified_requested_ignore_actor_leaf(
                        f.handle,
                        name,
                        declared_checksum,
                        declared_resolved,
                    )
                    .map(VerifiedArrayLeaf::Field)
                } else if measured && parent_name == "ActiveBlinds" {
                    verified_blind_leaf_type(f.handle, name, declared_checksum, declared_resolved)
                        .map(VerifiedArrayLeaf::Field)
                } else if measured && parent_name == "SelectedV2" {
                    verified_selected_v2_leaf_type(
                        f.handle,
                        name,
                        declared_checksum,
                        declared_resolved,
                    )
                    .map(VerifiedArrayLeaf::Field)
                } else if measured && parent_name == "KillData" {
                    verified_kill_data_leaf(f.handle, name, declared_checksum, declared_resolved)
                } else if measured {
                    verified_array_leaf_type(parent_name, checksum, f.handle, resolved, name)
                        .map(VerifiedArrayLeaf::Field)
                } else {
                    resolved.map(VerifiedArrayLeaf::Field)
                }
            })
            .collect();

        let nested_results: Vec<_> = flattened
            .iter()
            .map(|f| {
                if measured
                    && matches!(parent_name, "SelectedV2" | "KillData")
                    && matches!(
                        (parent_name, f.handle),
                        ("SelectedV2", 13) | ("KillData", 6)
                    )
                {
                    decode_verified_nested_array(
                        parent_name,
                        f,
                        &declared,
                        &declared_checksums,
                        &self.current_group_path,
                    )
                } else {
                    (None, vrf_decode::ArrayDecodeStats::default(), 0)
                }
            })
            .collect();
        for (_, nested_stats, nested_failures) in &nested_results {
            merge_array_stats(&mut self.stats.array, nested_stats);
            self.stats.array_leaf_decode_errors = self
                .stats
                .array_leaf_decode_errors
                .saturating_add(*nested_failures);
        }

        for ((f, declared_type), (nested, _, _)) in
            flattened.iter().zip(leaf_types).zip(nested_results)
        {
            // Build full field name: "Rounds[0].RoundNumber" etc. `f.path`
            // already carries its own leading separator.
            let full_name = self.channel_state.names.intern_fmt(|out| {
                out.push_str(parent_name);
                out.push_str(&f.path);
            });

            let (vi, vf, vb, vs) = match declared_type {
                Some(VerifiedArrayLeaf::Field(ft)) => decode_leaf_with_stats(
                    ft,
                    &f.raw_bits,
                    f.bit_count,
                    &mut self.stats.array_leaf_decode_errors,
                ),
                Some(VerifiedArrayLeaf::KillWeaponTheme) => decode_kill_weapon_theme(
                    &f.raw_bits,
                    f.bit_count,
                    &mut self.stats.array_leaf_decode_errors,
                ),
                Some(VerifiedArrayLeaf::TrackedRewardLocalizedText) => {
                    decode_tracked_reward_localized_text(
                        &f.raw_bits,
                        f.bit_count,
                        &mut self.stats.array_leaf_decode_errors,
                    )
                }
                // The hardcoded handle->type map is CombatReport-specific:
                // handle 3 is an Int32 there and an FString in
                // AbilityCastsThisRound, so applying it to any other array
                // forces the wrong type. Only Rounds falls through to it.
                None if parent_name == "Rounds" => decode_array_leaf(
                    f.handle,
                    &f.raw_bits,
                    f.bit_count,
                    &mut self.stats.array_leaf_decode_errors,
                ),
                None => (None, None, None, None),
            };

            self.push_field(FieldValues {
                handle: f.handle,
                field_name: Some(full_name),
                // An array leaf is addressed by its position inside the array
                // payload, not by a handle the group declares, so there is no
                // checksum for it to carry. The null says exactly that.
                compatible_checksum: None,
                bit_count: f.bit_count,
                raw_bits: Some(SmallVec::from_slice(&f.raw_bits)),
                value_i64: vi,
                value_f64: vf,
                value_bool: vb,
                value_str: vs,
            });
            self.stats.fields_emitted += 1;

            // Nested rows follow their preserved raw container row immediately.
            // The entire nested window was validated and decoded before the
            // container was emitted, so a malformed member cannot leak a prefix.
            if let Some(nested) = nested {
                for leaf in nested {
                    let field_name = self.channel_state.names.intern_fmt(|out| {
                        out.push_str(parent_name);
                        out.push_str(&f.path);
                        out.push_str(&leaf.path);
                    });
                    self.push_field(FieldValues {
                        handle: leaf.handle,
                        field_name: Some(field_name),
                        compatible_checksum: None,
                        bit_count: leaf.bit_count,
                        raw_bits: Some(SmallVec::from_slice(&leaf.raw_bits)),
                        value_i64: Some(leaf.value_i64),
                        ..FieldValues::default()
                    });
                    self.stats.fields_emitted += 1;
                }
            }
        }
    }

    /// The group this block belongs to, with game-mode sibling classes mapped
    /// to the class everything here is keyed on.
    ///
    /// A Swiftplay replay carries `RoundResults` and `TeamEconomy` on
    /// `Swiftplay_EoRCredits_GameState_C`, so a bare `contains("BombGameState")`
    /// silently skips the struct-blob decoders for it -- the export looked
    /// clean and the match had no score, which is section 26 happening again
    /// one game mode over. `vrf_decode::canonical_group` is the single alias
    /// table the overlay uses, so the two cannot disagree about what a game
    /// state is.
    fn canonical_group(&self) -> &str {
        vrf_decode::canonical_group(&self.current_group_path)
    }

    /// Which dedicated decoder owns this field on this group, if any.
    ///
    /// One classifier rather than a predicate and a dispatcher that each spell
    /// the gate out. The field stream asks the predicate whether to hand the
    /// blob over at all and then asks the dispatcher to decode it, so two
    /// copies that disagreed would take the blob off the ordinary path and
    /// then decline it -- the row would lose its decoded leaves and no counter
    /// would move. Section 33 changed this gate for Swiftplay; it had to be
    /// changed in three places.
    fn struct_blob_kind(&self, field_name: Option<&str>) -> Option<StructBlob> {
        match field_name? {
            "RoundResults" if self.canonical_group().contains("BombGameState") => {
                Some(StructBlob::RoundResults)
            }
            "TeamEconomy" if self.canonical_group().contains("BombGameState") => {
                Some(StructBlob::TeamEconomy)
            }
            "RoundInfos" if self.current_group_path.contains("OwnerExclusivePlayerInfo") => {
                Some(StructBlob::RoundInfos)
            }
            _ => None,
        }
    }

    /// Check if a field is a struct blob that has a dedicated decoder.
    pub(super) fn is_struct_blob_field(&self, field_name: Option<&str>) -> bool {
        self.struct_blob_kind(field_name).is_some()
    }

    /// Is this a `MultiItemSlot.MultiContents` blob the additive decoder should
    /// flatten? The parent row stays `Raw` (the overlay does not type it), and
    /// the items are emitted as extra `MultiContents[i]` rows.
    pub(super) fn is_multi_contents_field(&self, field_name: Option<&str>) -> bool {
        matches!(field_name, Some("MultiContents"))
            && self.current_group_path.contains("MultiItemSlot")
    }

    /// Decode a `MultiContents` blob and emit one row per item NetGUID.
    ///
    /// The blob is a RepLayout dynamic array of object references
    /// (`TArray<AAresItem*>`); [`vrf_decode::decode_object_ref_array`] walks the
    /// framing and returns `(wire element index, NetGUID)` pairs. Each lands as
    /// a `MultiContents[index]` row with the NetGUID in `value_i64`, the same
    /// column a single `ItemSlot.Contents` decode populates. The wire index,
    /// not arrival order, has to label the row: dynamic arrays are
    /// delta-replicated per element, so a re-send can carry only the changed
    /// slot and an enumerate-based label would put that item's GUID in slot 0.
    pub(super) fn emit_multi_contents(&mut self, raw: &[u8], bit_count: u32) {
        let guids =
            vrf_decode::decode_object_ref_array_with_stats(raw, bit_count, &mut self.stats.array);
        for (index, guid) in &guids {
            self.emit_struct_sub_field(
                |out| put(out, format_args!("MultiContents[{index}]")),
                Some(i64::from(*guid)),
                None,
            );
            self.stats.multi_contents_items_emitted += 1;
        }
    }

    /// Decode a struct blob and emit flattened sub-field rows.
    /// Returns true if decoding succeeded and sub-fields were emitted.
    pub(super) fn decode_struct_blob(
        &mut self,
        field_name: &str,
        raw: &[u8],
        bit_count: u32,
    ) -> bool {
        let emitted = match self.struct_blob_kind(Some(field_name)) {
            Some(StructBlob::RoundResults) => self.decode_round_results_blob(raw, bit_count),
            Some(StructBlob::TeamEconomy) => self.decode_team_economy_blob(raw, bit_count),
            Some(StructBlob::RoundInfos) => self.decode_round_infos_blob(raw, bit_count),
            None => false,
        };
        if emitted {
            self.stats.struct_blobs_decoded += 1;
        }
        emitted
    }

    /// Record a struct-blob decode failure instead of dropping it.
    ///
    /// Returns `false` so a call site can `return self.record_blob_failure(..)`
    /// -- the decoders are additive and a failure emits no rows, which is the
    /// same return value the discarding version produced. What is new is that
    /// the run says so.
    fn record_blob_failure(&mut self, err: &dyn std::fmt::Display) -> bool {
        self.stats.struct_blobs_failed += 1;
        if self.stats.struct_blob_first_error.is_none() {
            self.stats.struct_blob_first_error = Some(err.to_string());
        }
        false
    }

    /// Open a bit reader over a struct blob's declared bit length, or record
    /// the failure and hand back `None`. All three struct-blob decoders start
    /// this way; one guard here keeps the "declared bit length exceeds
    /// buffer" wording from drifting between copies.
    fn blob_bit_reader<'a>(&mut self, raw: &'a [u8], bit_count: u32) -> Option<BitReader<'a>> {
        match BitReader::with_bit_len(raw, u64::from(bit_count)) {
            Ok(reader) => Some(reader),
            Err(_) => {
                self.record_blob_failure(&"declared bit length exceeds buffer");
                None
            }
        }
    }

    /// Decode RoundResults blob and emit sub-field rows.
    fn decode_round_results_blob(&mut self, raw: &[u8], bit_count: u32) -> bool {
        use vrf_decode::structs::decode_round_results;

        let Some(mut reader) = self.blob_bit_reader(raw, bit_count) else {
            return false;
        };
        // Scoped so the borrow of `self.cache` ends before the emit loop needs
        // `&mut self`. The decoded elements own their strings, so nothing
        // outlives the declaration.
        let decoded = {
            let declared = Self::declared_handle_names(self.cache, &self.current_group_path);
            decode_round_results(&mut reader, &declared)
        };
        let results = match decoded {
            Ok(results) => results,
            Err(err) => return self.record_blob_failure(&err),
        };

        for rr in &results {
            let index = rr.round_number;
            self.emit_struct_sub_field(
                |out| put(out, format_args!("RoundResults[{index}].RoundNumber")),
                Some(i64::from(rr.round_number)),
                None,
            );
            if let Some(ref team) = rr.winning_team {
                self.emit_struct_sub_field(
                    |out| put(out, format_args!("RoundResults[{index}].WinningTeam")),
                    None,
                    Some(team.clone()),
                );
            }
            // `as_str` lives on the enums in `vrf-decode`; these used to be two
            // fully-qualified matches here, a second copy of the variant list in
            // a crate that does not own the types.
            for (member, text) in [
                ("WinningTeamRole", rr.winning_team_role.map(|r| r.as_str())),
                ("RoundResult", rr.round_result.map(|o| o.as_str())),
            ] {
                if let Some(text) = text {
                    self.emit_struct_sub_field(
                        |out| put(out, format_args!("RoundResults[{index}].{member}")),
                        None,
                        Some(text.to_owned()),
                    );
                }
            }
        }

        !results.is_empty()
    }

    /// Decode TeamEconomy blob and emit sub-field rows.
    fn decode_team_economy_blob(&mut self, raw: &[u8], bit_count: u32) -> bool {
        use vrf_decode::structs::decode_team_economy_declared;

        let Some(mut reader) = self.blob_bit_reader(raw, bit_count) else {
            return false;
        };
        let decoded = {
            let declared = Self::declared_handle_names(self.cache, &self.current_group_path);
            decode_team_economy_declared(&mut reader, &declared)
        };
        let results = match decoded {
            Ok(results) => results,
            Err(err) => return self.record_blob_failure(&err),
        };

        for te in &results {
            let index = te.index;
            self.emit_struct_sub_field(
                |out| put(out, format_args!("TeamEconomy[{index}].Index")),
                Some(i64::from(te.index)),
                None,
            );
            // Widened to i64 here rather than in the loop body: the members
            // are a mix of u32 and i32 and the array has to be one type.
            for (member, value) in [
                ("ReplicationId", te.replication_id.map(i64::from)),
                ("LoadoutValue", te.loadout_value.map(i64::from)),
                (
                    "AverageLoadoutValue",
                    te.average_loadout_value.map(i64::from),
                ),
            ] {
                if let Some(v) = value {
                    self.emit_struct_sub_field(
                        |out| put(out, format_args!("TeamEconomy[{index}].{member}")),
                        Some(v),
                        None,
                    );
                }
            }
        }

        !results.is_empty()
    }

    /// Decode RoundInfos blob and emit sub-field rows.
    fn decode_round_infos_blob(&mut self, raw: &[u8], bit_count: u32) -> bool {
        use vrf_decode::structs::decode_round_infos;

        let Some(mut reader) = self.blob_bit_reader(raw, bit_count) else {
            return false;
        };
        let decoded = {
            let declared = Self::declared_handle_names(self.cache, &self.current_group_path);
            decode_round_infos(&mut reader, &declared)
        };
        let results = match decoded {
            Ok(results) => results,
            Err(err) => return self.record_blob_failure(&err),
        };

        for ri in &results {
            let index = ri.index;
            for (member, value) in [
                ("RoundNumber", ri.round_number),
                ("StartOfRoundMoney", ri.start_of_round_money),
                ("StartOfRoundLoadoutValue", ri.start_of_round_loadout_value),
                ("EndOfRoundMoney", ri.end_of_round_money),
                ("EndOfRoundLoadoutValue", ri.end_of_round_loadout_value),
            ] {
                if let Some(v) = value {
                    self.emit_struct_sub_field(
                        |out| put(out, format_args!("RoundInfos[{index}].{member}")),
                        Some(i64::from(v)),
                        None,
                    );
                }
            }
        }

        !results.is_empty()
    }

    /// Emit a single sub-field row for a decoded struct blob element.
    ///
    /// The name is built by a closure straight into the interner's scratch
    /// buffer rather than passed as a `&str`, so a row costs no allocation for
    /// its name at all -- the callers used to build a prefix `String` and then
    /// a second `String` per member.
    ///
    /// Only the i64 and str columns are reachable: no struct-blob member
    /// decodes to a float or a bool today, and a parameter for a column no
    /// caller can fill reads as if the shape were open when it is not.
    fn emit_struct_sub_field(
        &mut self,
        name: impl FnOnce(&mut String),
        value_i64: Option<i64>,
        value_str: Option<String>,
    ) {
        let field_name = self.channel_state.names.intern_fmt(name);
        self.push_field(FieldValues {
            handle: 0,
            field_name: Some(field_name),
            bit_count: 0,
            raw_bits: None,
            value_i64,
            value_str,
            ..FieldValues::default()
        });
        self.stats.fields_emitted += 1;
    }
}

/// Decode one array leaf with a type the caller already resolved.
///
/// Split out so the overlay-driven path and the hardcoded-handle fallback share
/// one decode-and-widen, rather than each growing its own copy of the match on
/// `DecodedValue`.
pub(super) fn decode_leaf_with_stats(
    field_type: FieldType,
    raw: &[u8],
    bit_count: u32,
    failures: &mut u64,
) -> DecodedColumns {
    use vrf_decode::{DecodedValue, decode_field};

    match decode_field(field_type, raw, bit_count) {
        Ok(DecodedValue::I64(v)) => (Some(v), None, None, None),
        Ok(DecodedValue::F64(v)) => (None, Some(v), None, None),
        Ok(DecodedValue::Bool(v)) => (None, None, Some(v), None),
        Ok(DecodedValue::Str(v)) => (None, None, None, Some(v)),
        Err(_) => {
            *failures = failures.saturating_add(1);
            (None, None, None, None)
        }
    }
}

/// Fallback leaf typing for handles the overlay table cannot name.
///
/// The caller asks the table first, keyed on the name the replay declares for
/// the handle. This map only sees what that misses, so it is a floor rather
/// than the source of truth it used to be.
///
/// Returns (value_i64, value_f64, value_bool, value_str). All None if the
/// handle is not recognized or decoding fails.
///
/// Handle->type mapping derived from `CombatRoundReportsDecoder`:
/// - Int32 handles: 3, 5, 19, 21, 46, 81, 96
/// - Float handles: 18, 20, 47, 82
/// - Bool handles: 22, 25, 48, 49, 83, 84, 103
/// - EnumByte handles: 23, 45, 80
/// - ObjectNetGuid handles: 13, 24, 50, 85, 98
/// - FString handles: 11
/// - FName handles: 12
fn decode_array_leaf(
    handle: u32,
    raw: &[u8],
    bit_count: u32,
    failures: &mut u64,
) -> DecodedColumns {
    let field_type = match handle {
        3 | 5 | 19 | 21 | 46 | 81 | 96 => FieldType::Int32,
        18 | 20 | 47 | 82 => FieldType::Float,
        22 | 25 | 48 | 49 | 83 | 84 | 103 => FieldType::Bool,
        23 | 45 | 80 => FieldType::EnumByte,
        13 | 24 | 50 | 85 | 98 => FieldType::ObjectNetGuid,
        11 => FieldType::FString,
        12 => FieldType::FName,
        _ => return (None, None, None, None),
    };

    decode_leaf_with_stats(field_type, raw, bit_count, failures)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::{ChannelState, ExportStats, RecordBuffers};
    use std::sync::Arc;
    use vrf_net::field::FieldSink;

    const OWNER: &str = "/Script/ShooterGame.OwnerExclusivePlayerInfo";
    const OWNER_PARENT: &str = "AllPlayersObfuscatedPlayerInformation";
    const OWNER_CHECKSUM: u32 = 1_349_268_968;
    const REWARDS_PARENT: &str = "TrackedRewards";
    const REWARDS_CHECKSUM: u32 = 976_048_801;
    const SELECTED_GROUP: &str = "/Script/ShooterGame.PersonalizationComponent";
    const SELECTED_PARENT: &str = "SelectedV2";
    const SELECTED_CHECKSUM: u32 = 4_218_721_055;
    const KILL_GROUP: &str = "/Script/ShooterGame.PlayerMatchStatsComponent";
    const KILL_PARENT: &str = "KillData";
    const KILL_CHECKSUM: u32 = 1_493_759_848;
    const MEASURED_BUILD: &str = "++Ares-Core+release-13.05";

    fn packed(bits: &mut Vec<bool>, mut value: u32) {
        loop {
            let byte = ((value & 127) << 1) | u32::from(value > 127);
            bits.extend((0..8).map(|bit| byte & (1 << bit) != 0));
            value >>= 7;
            if value == 0 {
                break;
            }
        }
    }

    fn bytes(bits: &[bool]) -> Vec<u8> {
        let mut raw = vec![0; bits.len().div_ceil(8)];
        for (index, bit) in bits.iter().enumerate() {
            raw[index / 8] |= u8::from(*bit) << (index % 8);
        }
        raw
    }

    fn bits_from_bytes(raw: &[u8]) -> Vec<bool> {
        raw.iter()
            .flat_map(|byte| (0..8).map(move |bit| byte & (1 << bit) != 0))
            .collect()
    }

    fn one_leaf(handle: u32, payload: &[bool]) -> Vec<bool> {
        let mut bits = Vec::new();
        for value in [1, 1, handle + 1, payload.len() as u32] {
            packed(&mut bits, value);
        }
        bits.extend_from_slice(payload);
        packed(&mut bits, 0);
        packed(&mut bits, 0);
        bits
    }

    fn one_element(fields: &[(u32, Vec<bool>)]) -> Vec<bool> {
        let mut bits = Vec::new();
        packed(&mut bits, 1);
        packed(&mut bits, 1);
        for (handle, payload) in fields {
            packed(&mut bits, handle + 1);
            packed(&mut bits, payload.len() as u32);
            bits.extend_from_slice(payload);
        }
        packed(&mut bits, 0);
        packed(&mut bits, 0);
        bits
    }

    fn kill_weapon_theme_payload(value: &str, utf16: bool) -> Vec<bool> {
        let mut bits = vec![true];
        let (length, body) = if value.is_empty() && !utf16 {
            (0, Vec::new())
        } else if utf16 {
            let units: Vec<u16> = value.encode_utf16().chain([0]).collect();
            let body = units.iter().flat_map(|v| v.to_le_bytes()).collect();
            (-(units.len() as i32), body)
        } else {
            let mut body = value.as_bytes().to_vec();
            body.push(0);
            (body.len() as i32, body)
        };
        bits.extend(bits_from_bytes(&length.to_le_bytes()));
        bits.extend(bits_from_bytes(&body));
        bits
    }

    fn export_array(
        identity: (&str, &str, u32),
        leaf: (u32, &str),
        bits: &[bool],
        branch: Option<&str>,
    ) -> (RecordBuffers, ExportStats) {
        export_array_with_child_checksum(identity, (leaf.0, leaf.1, 0), bits, branch)
    }

    fn export_array_with_child_checksum(
        identity: (&str, &str, u32),
        leaf: (u32, &str, u32),
        bits: &[bool],
        branch: Option<&str>,
    ) -> (RecordBuffers, ExportStats) {
        export_array_with_declarations(identity, &[leaf], bits, branch)
    }

    fn export_array_with_declarations(
        identity: (&str, &str, u32),
        leaves: &[(u32, &str, u32)],
        bits: &[bool],
        branch: Option<&str>,
    ) -> (RecordBuffers, ExportStats) {
        let (group, parent, checksum) = identity;
        let mut cache = NetGuidCache::new();
        cache
            .add_export_group(vrf_schema::NetFieldExportGroup::new(group.into(), 7, 128))
            .unwrap();
        for (handle, name, compatible_checksum) in
            std::iter::once((0, parent, checksum)).chain(leaves.iter().copied())
        {
            assert!(cache.set_field_on_group(
                7,
                vrf_schema::NetFieldExport {
                    handle,
                    compatible_checksum,
                    name: name.into(),
                }
            ));
        }
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
        sink.set_current_group_path(Arc::from(group));
        if let Some(branch) = branch {
            sink.enable_measured_array_routes(branch);
        }
        let raw = bytes(bits);
        sink.on_field(
            0,
            bits.len() as u32,
            BitReader::with_bit_len(&raw, bits.len() as u64).unwrap(),
        );
        let stats = sink.stats;
        (records, stats)
    }

    #[test]
    fn measured_array_emits_typed_child_before_exact_raw_parent() {
        let bits = one_leaf(49, &[true]);
        let (records, stats) = export_array(
            (OWNER, OWNER_PARENT, OWNER_CHECKSUM),
            (49, "bIsAfk"),
            &bits,
            Some(MEASURED_BUILD),
        );
        assert_eq!(records.fields.len(), 2);
        let child = &records.fields[0];
        assert_eq!(
            child.field_name.as_deref(),
            Some("AllPlayersObfuscatedPlayerInformation[0].bIsAfk")
        );
        assert_eq!(child.value_bool, Some(true));
        assert_eq!(child.raw_bits.as_deref(), Some([1u8].as_slice()));
        assert_eq!(child.bit_count, 1);
        assert_eq!(child.compatible_checksum, None);
        let parent = &records.fields[1];
        assert_eq!(parent.field_name.as_deref(), Some(OWNER_PARENT));
        assert_eq!(parent.raw_bits.as_deref(), Some(bytes(&bits).as_slice()));
        assert_eq!(parent.bit_count, bits.len() as u32);
        assert_eq!(stats.array.fields_emitted, 1);
        assert_eq!(stats.fields_emitted, 2);
    }

    #[test]
    fn measured_array_rejects_unmeasured_build_and_wrong_identity() {
        let bits = one_leaf(49, &[true]);
        for (group, name, checksum, branch) in [
            (OWNER, OWNER_PARENT, OWNER_CHECKSUM, None),
            (
                OWNER,
                OWNER_PARENT,
                OWNER_CHECKSUM,
                Some("++Ares-Core+release-13.07"),
            ),
            (
                OWNER,
                OWNER_PARENT,
                OWNER_CHECKSUM + 1,
                Some(MEASURED_BUILD),
            ),
            (
                "/Script/ShooterGame.Other",
                OWNER_PARENT,
                OWNER_CHECKSUM,
                Some(MEASURED_BUILD),
            ),
            (
                OWNER,
                "DifferentArray",
                OWNER_CHECKSUM,
                Some(MEASURED_BUILD),
            ),
        ] {
            let (records, stats) =
                export_array((group, name, checksum), (49, "bIsAfk"), &bits, branch);
            assert_eq!(
                records.fields.len(),
                1,
                "{group}/{name}/{checksum}/{branch:?}"
            );
            assert_eq!(
                records.fields[0].raw_bits.as_deref(),
                Some(bytes(&bits).as_slice())
            );
            assert_eq!(stats.array.fields_emitted, 0);
        }
    }

    #[test]
    fn measured_array_rejects_suffix_and_missing_terminator_transactionally() {
        let valid = one_leaf(49, &[true]);
        let mut suffix = valid.clone();
        suffix.extend([false; 8]);
        let truncated = valid[..valid.len() - 8].to_vec();
        for bits in [suffix, truncated] {
            let (records, stats) = export_array(
                (OWNER, OWNER_PARENT, OWNER_CHECKSUM),
                (49, "bIsAfk"),
                &bits,
                Some(MEASURED_BUILD),
            );
            assert_eq!(records.fields.len(), 1, "no partially accepted children");
            assert_eq!(
                records.fields[0].raw_bits.as_deref(),
                Some(bytes(&bits).as_slice())
            );
            assert!(
                stats.array.unconsumed_root_bits > 0
                    || stats.array.implicit_terminations > 0
                    || stats.array.errors > 0
            );
        }
    }

    #[test]
    fn measured_array_unknown_leaf_stays_raw() {
        let bits = one_leaf(48, &[true, false, true]);
        let (records, _) = export_array(
            (OWNER, OWNER_PARENT, OWNER_CHECKSUM),
            (48, "SubjectUniqueId"),
            &bits,
            Some(MEASURED_BUILD),
        );
        assert_eq!(records.fields.len(), 2);
        let child = &records.fields[0];
        assert_eq!(child.raw_bits.as_deref(), Some([5u8].as_slice()));
        assert_eq!(
            (
                child.value_i64,
                child.value_f64,
                child.value_bool,
                child.value_str.as_deref()
            ),
            (None, None, None, None)
        );
    }

    #[test]
    fn tracked_rewards_unverified_children_stay_raw() {
        let bits = one_leaf(19, &[true]);
        let (records, stats) = export_array(
            (OWNER, REWARDS_PARENT, REWARDS_CHECKSUM),
            (19, "AdditionalRawReward"),
            &bits,
            Some(MEASURED_BUILD),
        );
        assert_eq!(records.fields.len(), 2);
        let child = &records.fields[0];
        assert_eq!(
            child.field_name.as_deref(),
            Some("TrackedRewards[0].AdditionalRawReward")
        );
        assert_eq!(child.raw_bits.as_deref(), Some([1u8].as_slice()));
        assert_eq!(
            (
                child.value_i64,
                child.value_f64,
                child.value_bool,
                child.value_str.as_deref()
            ),
            (None, None, None, None)
        );
        assert_eq!(stats.tracked_rewards_opaque_empty_variants, 0);
    }

    #[test]
    fn tracked_rewards_types_only_the_four_verified_leaf_identities() {
        let fname_zero = vec![true, false, false, false, false, false, false, false, false];
        for (handle, name, checksum, payload, want_i64, want_str) in [
            (28, "RewardName", 1_337_472_711, fname_zero, None, Some("0")),
            (
                30,
                "InstancesOfReward",
                2_922_243_316,
                vec![false; 32],
                Some(0),
                None,
            ),
            (
                31,
                "RewardGrantStrategy",
                3_589_631_714,
                vec![false; 2],
                Some(0),
                None,
            ),
            (
                32,
                "Source",
                1_118_571_008,
                vec![true, true, false],
                Some(3),
                None,
            ),
        ] {
            let bits = one_leaf(handle, &payload);
            let (records, _) = export_array_with_child_checksum(
                (OWNER, REWARDS_PARENT, REWARDS_CHECKSUM),
                (handle, name, checksum),
                &bits,
                Some(MEASURED_BUILD),
            );
            let child = &records.fields[0];
            assert_eq!(child.value_i64, want_i64, "{name}");
            assert_eq!(child.value_str.as_deref(), want_str, "{name}");
            assert_eq!(child.raw_bits.as_deref(), Some(bytes(&payload).as_slice()));
        }
    }

    #[test]
    fn tracked_rewards_refuses_wrong_child_identity_or_resolved_type() {
        let bits = one_leaf(30, &[false; 32]);
        for (name, checksum) in [("OtherName", 2_922_243_316), ("InstancesOfReward", 0)] {
            let (records, _) = export_array_with_child_checksum(
                (OWNER, REWARDS_PARENT, REWARDS_CHECKSUM),
                (30, name, checksum),
                &bits,
                Some(MEASURED_BUILD),
            );
            assert_eq!(records.fields[0].value_i64, None, "{name}/{checksum}");
        }
        assert_eq!(
            verified_reward_leaf_type(
                30,
                Some("InstancesOfReward"),
                Some(2_922_243_316),
                Some(FieldType::Float),
            ),
            None
        );
    }

    #[test]
    fn tracked_rewards_localized_text_requires_its_raw_declaration() {
        assert!(verified_reward_localized_text(
            29,
            Some("LocalizedRewardName"),
            Some(483_770_233),
            Some(FieldType::Raw),
        ));
        for (handle, name, checksum, resolved) in [
            (29, Some("Other"), Some(483_770_233), Some(FieldType::Raw)),
            (
                29,
                Some("LocalizedRewardName"),
                Some(0),
                Some(FieldType::Raw),
            ),
            (
                29,
                Some("LocalizedRewardName"),
                Some(483_770_233),
                Some(FieldType::Skip),
            ),
            (
                29,
                Some("LocalizedRewardName"),
                Some(483_770_233),
                Some(FieldType::FText),
            ),
            (
                28,
                Some("LocalizedRewardName"),
                Some(483_770_233),
                Some(FieldType::Raw),
            ),
        ] {
            assert!(!verified_reward_localized_text(
                handle, name, checksum, resolved
            ));
        }
    }

    #[test]
    fn tracked_rewards_bad_typed_width_keeps_raw_leaf_and_counts_error() {
        let bits = one_leaf(30, &[false; 8]);
        let (records, stats) = export_array_with_child_checksum(
            (OWNER, REWARDS_PARENT, REWARDS_CHECKSUM),
            (30, "InstancesOfReward", 2_922_243_316),
            &bits,
            Some(MEASURED_BUILD),
        );
        assert_eq!(records.fields.len(), 2);
        assert_eq!(records.fields[0].raw_bits.as_deref(), Some([0].as_slice()));
        assert_eq!(records.fields[0].value_i64, None);
        assert_eq!(stats.array_leaf_decode_errors, 1);
    }

    #[test]
    fn localized_reward_text_keeps_raw_on_success_and_failure() {
        let empty = bits_from_bytes(&[0, 0, 0, 0, 255, 0, 0, 0, 0]);
        for (payload, errors, expected) in [
            (
                empty.clone(),
                0,
                Some(r#"{"flags":0,"history":255,"kind":"empty"}"#),
            ),
            (empty[..empty.len() - 1].to_vec(), 1, None),
        ] {
            let bits = one_leaf(29, &payload);
            let (records, stats) = export_array_with_child_checksum(
                (OWNER, REWARDS_PARENT, REWARDS_CHECKSUM),
                (29, "LocalizedRewardName", 483_770_233),
                &bits,
                Some(MEASURED_BUILD),
            );
            assert_eq!(records.fields.len(), 2);
            assert_eq!(
                records.fields[0].raw_bits.as_deref(),
                Some(bytes(&payload).as_slice())
            );
            assert_eq!(records.fields[0].value_str.as_deref(), expected);
            assert_eq!(records.fields[0].value_i64, None);
            assert_eq!(records.fields[0].value_f64, None);
            assert_eq!(records.fields[0].value_bool, None);
            assert_eq!(stats.array_leaf_decode_errors, errors);
            assert_eq!(
                records.fields[1].raw_bits.as_deref(),
                Some(bytes(&bits).as_slice())
            );
        }
    }

    #[test]
    fn selected_v2_and_kill_data_are_exact_raw_child_routes() {
        for (group, parent, checksum) in [
            (SELECTED_GROUP, SELECTED_PARENT, SELECTED_CHECKSUM),
            (KILL_GROUP, KILL_PARENT, KILL_CHECKSUM),
        ] {
            let bits = one_leaf(19, &[true, false, true]);
            let (records, stats) = export_array(
                (group, parent, checksum),
                (19, "NestedRawMember"),
                &bits,
                Some(MEASURED_BUILD),
            );
            assert_eq!(records.fields.len(), 2, "{parent}");
            assert_eq!(records.fields[0].raw_bits.as_deref(), Some([5].as_slice()));
            assert_eq!(
                (
                    records.fields[0].value_i64,
                    records.fields[0].value_f64,
                    records.fields[0].value_bool,
                    records.fields[0].value_str.as_deref()
                ),
                (None, None, None, None),
                "{parent}"
            );
            assert_eq!(stats.array.fields_emitted, 1);
        }
    }

    #[test]
    fn selected_v2_types_only_the_six_qualified_object_net_guid_leaves() {
        let mut multi_byte = Vec::new();
        packed(&mut multi_byte, 128);
        let (records, _) = export_array_with_child_checksum(
            (SELECTED_GROUP, SELECTED_PARENT, SELECTED_CHECKSUM),
            (3, "EquippableDataAsset", 1_793_937_854),
            &one_leaf(3, &multi_byte),
            Some(MEASURED_BUILD),
        );
        assert_eq!(records.fields[0].value_i64, Some(128));
        assert_eq!(
            records.fields[0].raw_bits.as_deref(),
            Some([1, 2].as_slice())
        );

        let zero = vec![false; 8];
        let (records, _) = export_array_with_child_checksum(
            (SELECTED_GROUP, SELECTED_PARENT, SELECTED_CHECKSUM),
            (8, "EquippableCharmLevelDataAsset", 1_087_985_310),
            &one_leaf(8, &zero),
            Some(MEASURED_BUILD),
        );
        assert_eq!(records.fields[0].value_i64, Some(0));

        for (name, checksum) in [("Other", 1_793_937_854), ("EquippableDataAsset", 0)] {
            let (records, _) = export_array_with_child_checksum(
                (SELECTED_GROUP, SELECTED_PARENT, SELECTED_CHECKSUM),
                (3, name, checksum),
                &one_leaf(3, &multi_byte),
                Some(MEASURED_BUILD),
            );
            assert_eq!(records.fields[0].value_i64, None, "{name}/{checksum}");
        }
        for (handle, name, checksum) in [
            (9, "A", 3_055_317_389),
            (13, "EquippableAttachments", 3_137_596_882),
            (14, "SocketAsset", 3_666_994_016),
            (15, "AttachmentAsset", 856_446_005),
        ] {
            let (records, _) = export_array_with_child_checksum(
                (SELECTED_GROUP, SELECTED_PARENT, SELECTED_CHECKSUM),
                (handle, name, checksum),
                &one_leaf(handle, &multi_byte),
                Some(MEASURED_BUILD),
            );
            assert_eq!(records.fields[0].value_i64, None, "{name} remains raw");
        }
        assert_eq!(
            verified_selected_v2_leaf_type(
                3,
                Some("EquippableDataAsset"),
                Some(1_793_937_854),
                Some(FieldType::Float),
            ),
            None,
            "a contradictory overlay must leave the leaf raw"
        );
        for blocked in [FieldType::Raw, FieldType::Skip] {
            assert_eq!(
                verified_selected_v2_leaf_type(
                    3,
                    Some("EquippableDataAsset"),
                    Some(1_793_937_854),
                    Some(blocked),
                ),
                None,
                "an explicit raw/skip declaration is not an absent overlay"
            );
        }
    }

    #[test]
    fn selected_v2_bad_object_net_guid_windows_stay_raw_and_count_errors() {
        for payload in [bits_from_bytes(&[1]), bits_from_bytes(&[1, 1, 1, 1, 0x20])] {
            let (records, stats) = export_array_with_child_checksum(
                (SELECTED_GROUP, SELECTED_PARENT, SELECTED_CHECKSUM),
                (3, "EquippableDataAsset", 1_793_937_854),
                &one_leaf(3, &payload),
                Some(MEASURED_BUILD),
            );
            assert_eq!(records.fields[0].value_i64, None);
            assert_eq!(stats.array_leaf_decode_errors, 1);
            assert_eq!(
                records.fields[0].raw_bits.as_deref(),
                Some(bytes(&payload).as_slice())
            );
        }
    }

    #[test]
    fn kill_data_types_only_qualified_primitive_leaves() {
        let mut object = Vec::new();
        packed(&mut object, 128);
        let float = bits_from_bytes(&(-1.5f32).to_le_bytes());
        let int = bits_from_bytes(&(-2i32).to_le_bytes());
        for (handle, name, checksum, payload, vi, vf, vb) in [
            (
                3,
                "Victim",
                3_990_035_472,
                object.clone(),
                Some(128),
                None,
                None,
            ),
            (
                4,
                "KillingEquippableClass",
                2_071_131_011,
                vec![false; 8],
                Some(0),
                None,
                None,
            ),
            (
                9,
                "DamageType",
                2_992_423_760,
                object.clone(),
                Some(128),
                None,
                None,
            ),
            (
                10,
                "DamageTaken",
                2_001_471_495,
                float.clone(),
                None,
                Some(-1.5),
                None,
            ),
            (
                11,
                "DamageRegion",
                3_229_265_809,
                vec![true, false, true],
                Some(5),
                None,
                None,
            ),
            (
                12,
                "GameTimeElapsed",
                3_684_431_363,
                float.clone(),
                None,
                Some(-1.5),
                None,
            ),
            (
                13,
                "RoundTimestamp",
                2_328_473_242,
                float,
                None,
                Some(-1.5),
                None,
            ),
            (14, "RoundNumber", 843_024_485, int, Some(-2), None, None),
            (
                15,
                "bDidKillTriggerFinisher",
                2_795_684_046,
                vec![false],
                None,
                None,
                Some(false),
            ),
        ] {
            let (records, _) = export_array_with_child_checksum(
                (KILL_GROUP, KILL_PARENT, KILL_CHECKSUM),
                (handle, name, checksum),
                &one_leaf(handle, &payload),
                Some(MEASURED_BUILD),
            );
            let child = &records.fields[0];
            assert_eq!(child.value_i64, vi, "{name}");
            assert_eq!(child.value_f64, vf, "{name}");
            assert_eq!(child.value_bool, vb, "{name}");
            assert_eq!(child.raw_bits.as_deref(), Some(bytes(&payload).as_slice()));
        }
    }

    #[test]
    fn kill_data_weapon_theme_decodes_exact_prefixed_fstrings() {
        for (value, utf16) in [
            ("/Game/Themes/Standard", false),
            ("\u{d14c}\u{b9c8}", true),
            ("", false),
        ] {
            let payload = kill_weapon_theme_payload(value, utf16);
            let (records, stats) = export_array_with_child_checksum(
                (KILL_GROUP, KILL_PARENT, KILL_CHECKSUM),
                (5, "WeaponTheme", 1_839_952_321),
                &one_leaf(5, &payload),
                Some(MEASURED_BUILD),
            );
            assert_eq!(records.fields[0].value_str.as_deref(), Some(value));
            assert_eq!(
                records.fields[0].raw_bits.as_deref(),
                Some(bytes(&payload).as_slice())
            );
            assert_eq!(stats.array_leaf_decode_errors, 0);
        }
    }

    #[test]
    fn kill_data_weapon_theme_rejects_bad_flag_terminator_and_residual() {
        let mut bad_flag = kill_weapon_theme_payload("x", false);
        bad_flag[0] = false;
        let mut bad_terminator = kill_weapon_theme_payload("x", false);
        // Use a valid UTF-8 byte so rejection cannot come from UTF decoding.
        let last = bad_terminator.len() - 8;
        bad_terminator[last] = true;
        let mut bad_wide_terminator = kill_weapon_theme_payload("x", true);
        let last_wide = bad_wide_terminator.len() - 16;
        bad_wide_terminator[last_wide] = true;
        let mut residual = kill_weapon_theme_payload("x", false);
        residual.push(false);
        let mut invalid_utf8 = vec![true];
        invalid_utf8.extend(bits_from_bytes(&2i32.to_le_bytes()));
        invalid_utf8.extend(bits_from_bytes(&[0xff, 0]));
        for payload in [
            bad_flag,
            bad_terminator,
            bad_wide_terminator,
            residual,
            invalid_utf8,
        ] {
            let (records, stats) = export_array_with_child_checksum(
                (KILL_GROUP, KILL_PARENT, KILL_CHECKSUM),
                (5, "WeaponTheme", 1_839_952_321),
                &one_leaf(5, &payload),
                Some(MEASURED_BUILD),
            );
            assert_eq!(records.fields[0].value_str, None);
            assert_eq!(
                records.fields[0].raw_bits.as_deref(),
                Some(bytes(&payload).as_slice())
            );
            assert_eq!(stats.array_leaf_decode_errors, 1);
        }
    }

    #[test]
    fn kill_data_refuses_wrong_child_identity_and_any_overlay_disagreement() {
        assert!(
            verified_kill_data_leaf(10, Some("DamageTaken"), Some(2_001_471_495), None).is_some()
        );
        for (name, checksum) in [("Other", 2_001_471_495), ("DamageTaken", 0)] {
            assert!(verified_kill_data_leaf(10, Some(name), Some(checksum), None).is_none());
            let payload = bits_from_bytes(&1.5f32.to_le_bytes());
            let (records, _) = export_array_with_child_checksum(
                (KILL_GROUP, KILL_PARENT, KILL_CHECKSUM),
                (10, name, checksum),
                &one_leaf(10, &payload),
                Some(MEASURED_BUILD),
            );
            assert_eq!(records.fields[0].value_f64, None);
            assert_eq!(
                records.fields[0].raw_bits.as_deref(),
                Some(bytes(&payload).as_slice())
            );
        }
        let payload = kill_weapon_theme_payload("theme", false);
        let (records, _) = export_array_with_child_checksum(
            (KILL_GROUP, KILL_PARENT, KILL_CHECKSUM),
            (5, "Other", 1_839_952_321),
            &one_leaf(5, &payload),
            Some(MEASURED_BUILD),
        );
        assert_eq!(records.fields[0].value_str, None);
        for blocked in [FieldType::Int32, FieldType::Raw, FieldType::Skip] {
            assert!(
                verified_kill_data_leaf(
                    10,
                    Some("DamageTaken"),
                    Some(2_001_471_495),
                    Some(blocked)
                )
                .is_none()
            );
        }
        assert!(
            verified_kill_data_leaf(
                5,
                Some("WeaponTheme"),
                Some(1_839_952_321),
                Some(FieldType::FString)
            )
            .is_none()
        );
    }

    #[test]
    fn measured_nested_arrays_emit_after_their_preserved_raw_containers() {
        let mut first = Vec::new();
        packed(&mut first, 128);
        let mut second = Vec::new();
        packed(&mut second, 9);
        let selected_nested = one_element(&[(14, first.clone()), (15, second.clone())]);
        let (records, stats) = export_array_with_declarations(
            (SELECTED_GROUP, SELECTED_PARENT, SELECTED_CHECKSUM),
            &[
                (13, "EquippableAttachments", 3_137_596_882),
                (14, "SocketAsset", 3_666_994_016),
                (15, "AttachmentAsset", 856_446_005),
            ],
            &one_leaf(13, &selected_nested),
            Some(MEASURED_BUILD),
        );
        assert_eq!(records.fields.len(), 4);
        assert_eq!(
            records.fields[0].field_name.as_deref(),
            Some("SelectedV2[0].EquippableAttachments")
        );
        assert_eq!(
            records.fields[0].raw_bits.as_deref(),
            Some(bytes(&selected_nested).as_slice())
        );
        assert_eq!(
            records.fields[1].field_name.as_deref(),
            Some("SelectedV2[0].EquippableAttachments[0].SocketAsset")
        );
        assert_eq!(records.fields[1].value_i64, Some(128));
        assert_eq!(
            records.fields[1].raw_bits.as_deref(),
            Some(bytes(&first).as_slice())
        );
        assert_eq!(
            records.fields[2].field_name.as_deref(),
            Some("SelectedV2[0].EquippableAttachments[0].AttachmentAsset")
        );
        assert_eq!(records.fields[2].value_i64, Some(9));
        assert_eq!(
            records.fields[3].field_name.as_deref(),
            Some(SELECTED_PARENT)
        );
        assert_eq!(stats.array.fields_emitted, 3);

        let kill_nested = one_element(&[(7, first.clone())]);
        let (records, stats) = export_array_with_declarations(
            (KILL_GROUP, KILL_PARENT, KILL_CHECKSUM),
            &[
                (6, "AssistingPlayers", 1_689_463_717),
                (7, "AssistingPlayers", 1_417_448_159),
            ],
            &one_leaf(6, &kill_nested),
            Some(MEASURED_BUILD),
        );
        assert_eq!(records.fields.len(), 3);
        assert_eq!(
            records.fields[0].field_name.as_deref(),
            Some("KillData[0].AssistingPlayers")
        );
        assert_eq!(
            records.fields[1].field_name.as_deref(),
            Some("KillData[0].AssistingPlayers[0].AssistingPlayers")
        );
        assert_eq!(records.fields[1].value_i64, Some(128));
        assert_eq!(records.fields[2].field_name.as_deref(), Some(KILL_PARENT));
        assert_eq!(stats.array.fields_emitted, 2);
    }

    #[test]
    fn nested_array_preflight_requires_bounds_nonzero_windows_and_terminators() {
        let valid = one_element(&[(7, bits_from_bytes(&[2]))]);
        assert!(strict_nested_array_preflight(
            &bytes(&valid),
            valid.len() as u32,
            &[7]
        ));
        assert!(!strict_nested_array_preflight(&[], 0, &[7]));
        assert!(!strict_nested_array_preflight(
            &bytes(&valid[..valid.len() - 8]),
            (valid.len() - 8) as u32,
            &[7]
        ));
        let mut suffix = valid.clone();
        suffix.extend([false; 8]);
        assert!(!strict_nested_array_preflight(
            &bytes(&suffix),
            suffix.len() as u32,
            &[7]
        ));
        let zero_width = one_element(&[(7, Vec::new())]);
        assert!(!strict_nested_array_preflight(
            &bytes(&zero_width),
            zero_width.len() as u32,
            &[7]
        ));
        let unexpected = one_element(&[(8, bits_from_bytes(&[2]))]);
        assert!(!strict_nested_array_preflight(
            &bytes(&unexpected),
            unexpected.len() as u32,
            &[7]
        ));
        // The generic array walker skips zero-width members. The projectile
        // route must reject one even when all three expected members follow.
        let path_with_unknown_zero = one_element(&[
            (4, Vec::new()),
            (1, bits_from_bytes(&[0; 4])),
            (2, bits_from_bytes(&[0; 24])),
            (3, bits_from_bytes(&[0; 24])),
        ]);
        assert!(!strict_nested_array_preflight(
            &bytes(&path_with_unknown_zero),
            path_with_unknown_zero.len() as u32,
            &[1, 2, 3]
        ));
        let mut capacity_limit = Vec::new();
        packed(&mut capacity_limit, vrf_decode::MAX_ELEMENTS + 1);
        packed(&mut capacity_limit, 0);
        assert!(!strict_nested_array_preflight(
            &bytes(&capacity_limit),
            capacity_limit.len() as u32,
            &[7]
        ));
        let mut index_range = Vec::new();
        packed(&mut index_range, 1);
        packed(&mut index_range, 2);
        assert!(!strict_nested_array_preflight(
            &bytes(&index_range),
            index_range.len() as u32,
            &[7]
        ));
        let fields = (0..=vrf_decode::MAX_FIELDS_PER_ELEMENT)
            .map(|_| (7, bits_from_bytes(&[0])))
            .collect::<Vec<_>>();
        let field_limit = one_element(&fields);
        assert!(!strict_nested_array_preflight(
            &bytes(&field_limit),
            field_limit.len() as u32,
            &[7]
        ));
    }

    #[test]
    fn active_blinds_empty_delta_with_zero_trailer_is_complete() {
        let identity = (
            "/Script/ShooterGame.BlindManagerComponent",
            "ActiveBlinds",
            3_853_965_310,
        );
        // Captured 57 times across 13.01/13.02/13.04/13.05: capacity
        // one (56 cases) or two (one case), no changed elements, zero trailer.
        for capacity in [1, 2] {
            let mut bits = Vec::new();
            packed(&mut bits, capacity);
            packed(&mut bits, 0);
            let (_, control) =
                export_array_with_declarations(identity, &[], &bits, Some(MEASURED_BUILD));
            assert_eq!(control.array.errors, 0);
            packed(&mut bits, 0);
            let (records, stats) =
                export_array_with_declarations(identity, &[], &bits, Some(MEASURED_BUILD));
            assert_eq!(stats.array.errors, 0, "capacity {capacity}");
            assert_eq!(stats.array.unconsumed_root_bits, 0);
            assert_eq!(stats.array_leaf_decode_errors, 0);
            assert_eq!(
                records.fields.len(),
                1,
                "unchanged elements add no children"
            );
            assert_eq!(
                records.fields[0].raw_bits.as_deref(),
                Some(bytes(&bits).as_slice())
            );
        }
    }

    #[test]
    fn active_blinds_null_causing_actor_is_a_decoded_reference() {
        let identity = (
            "/Script/ShooterGame.BlindManagerComponent",
            "ActiveBlinds",
            3_853_965_310,
        );
        // The 59 rejected value windows contain a one-byte IntPacked zero.
        // Keep the positive reference as a control through the same sink.
        for reference in [257, 0] {
            let mut payload = Vec::new();
            packed(&mut payload, reference);
            let bits = one_leaf(11, &payload);
            let (records, stats) = export_array_with_declarations(
                identity,
                &[(11, "CausingActor", 2_370_661_694)],
                &bits,
                Some(MEASURED_BUILD),
            );
            assert_eq!(stats.array.errors, 0);
            assert_eq!(stats.array_leaf_decode_errors, 0, "reference {reference}");
            assert_eq!(records.fields.len(), 2);
            assert_eq!(records.fields[0].value_i64, Some(i64::from(reference)));
            assert_eq!(
                records.fields[0].raw_bits.as_deref(),
                Some(bytes(&payload).as_slice())
            );
            assert_eq!(
                records.fields[1].raw_bits.as_deref(),
                Some(bytes(&bits).as_slice())
            );
        }
    }

    #[test]
    fn active_blinds_invalid_trailers_and_references_still_fail() {
        let identity = (
            "/Script/ShooterGame.BlindManagerComponent",
            "ActiveBlinds",
            3_853_965_310,
        );
        let mut empty = Vec::new();
        packed(&mut empty, 1);
        packed(&mut empty, 0);
        for trailer in [
            vec![true; 8],
            bits_from_bytes(&[2]),
            bits_from_bytes(&[0, 0]),
        ] {
            let mut bits = empty.clone();
            bits.extend(trailer);
            let (records, stats) =
                export_array_with_declarations(identity, &[], &bits, Some(MEASURED_BUILD));
            assert_eq!(stats.array.errors, 1);
            assert_eq!(records.fields.len(), 1);
            assert_eq!(
                records.fields[0].raw_bits.as_deref(),
                Some(bytes(&bits).as_slice())
            );
        }
        for payload in [
            bits_from_bytes(&[1]),
            bits_from_bytes(&[0, 0]),
            vec![false; 7],
        ] {
            let bits = one_leaf(11, &payload);
            let (records, stats) = export_array_with_declarations(
                identity,
                &[(11, "CausingActor", 2_370_661_694)],
                &bits,
                Some(MEASURED_BUILD),
            );
            assert_eq!(stats.array_leaf_decode_errors, 1);
            assert!(records.fields.iter().all(|row| row.value_i64.is_none()));
            assert_eq!(
                records.fields.last().unwrap().raw_bits.as_deref(),
                Some(bytes(&bits).as_slice())
            );
        }
        let mut populated = one_leaf(11, &bits_from_bytes(&[0]));
        packed(&mut populated, 0);
        let (_, stats) = export_array_with_declarations(
            identity,
            &[(11, "CausingActor", 2_370_661_694)],
            &populated,
            Some(MEASURED_BUILD),
        );
        assert_eq!(
            stats.array.errors, 1,
            "only empty deltas admit the zero trailer"
        );
        assert!(
            !strict_nested_array_preflight(&[2, 0, 0], 24, &[11]),
            "other array routes retain the exact-window contract"
        );
    }

    #[test]
    fn active_blinds_changed_member_declaration_retains_only_raw_parent() {
        const GROUP: &str = "/Script/ShooterGame.BlindManagerComponent";
        const PARENT: &str = "ActiveBlinds";
        let declarations = [
            (3, "BlindId", 2_836_858_544),
            (4, "EffectID", 3_321_413_110),
            (5, "SourceID", 4_130_766_059),
            (6, "bLocalEffect", 2_802_682_995),
            (7, "bTransient", 815_378_154),
            (8, "InitialDuration", 1_370_668_337),
            (9, "StartNetMovementTime", 2_358_118_895),
            (10, "BlindConfig", 4_121_438_116),
            (11, "CausingActor", 2_370_661_694),
        ];
        let mut source_id = vec![false];
        source_id.extend(bits_from_bytes(&(29i32).to_le_bytes()));
        source_id.extend(bits_from_bytes(b"DedicatedServerWorldSourceID\0"));
        source_id.extend(bits_from_bytes(&0i32.to_le_bytes()));
        assert_eq!(source_id.len(), 297);
        let mut blind_config = Vec::new();
        packed(&mut blind_config, 256);
        let mut causing_actor = Vec::new();
        packed(&mut causing_actor, 257);
        let bits = one_element(&[
            (3, bits_from_bytes(&7u32.to_le_bytes())),
            (4, bits_from_bytes(&8u64.to_le_bytes())),
            (5, source_id),
            (6, vec![true]),
            (7, vec![false]),
            (8, bits_from_bytes(&1.5f32.to_le_bytes())),
            (9, bits_from_bytes(&10.0f32.to_le_bytes())),
            (10, blind_config),
            (11, causing_actor),
        ]);
        let identity = (GROUP, PARENT, 3_853_965_310);
        let (valid, clean) =
            export_array_with_declarations(identity, &declarations, &bits, Some(MEASURED_BUILD));
        assert_eq!(valid.fields.len(), 10);
        assert_eq!(clean.array_leaf_decode_errors, 0);
        assert_eq!(valid.fields[0].value_i64, Some(7));
        assert_eq!(valid.fields[8].value_i64, Some(257));

        let mut changed = declarations;
        changed[0].2 += 1;
        let (refused, stats) =
            export_array_with_declarations(identity, &changed, &bits, Some(MEASURED_BUILD));
        assert_eq!(refused.fields.len(), 1);
        assert_eq!(refused.fields[0].field_name.as_deref(), Some(PARENT));
        assert_eq!(
            refused.fields[0].raw_bits.as_deref(),
            Some(bytes(&bits).as_slice())
        );
        assert_eq!(stats.array_leaf_decode_errors, 1);
    }

    #[test]
    fn nested_array_identity_and_overlay_disagreements_are_refused() {
        assert!(verified_nested_container(
            "KillData",
            6,
            Some("AssistingPlayers"),
            Some(1_689_463_717),
            None
        ));
        assert!(!verified_nested_container(
            "KillData",
            6,
            Some("Other"),
            Some(1_689_463_717),
            None
        ));
        assert!(!verified_nested_container(
            "KillData",
            6,
            Some("AssistingPlayers"),
            Some(0),
            None
        ));
        for blocked in [FieldType::ObjectNetGuid, FieldType::Raw, FieldType::Skip] {
            assert!(!verified_nested_container(
                "KillData",
                6,
                Some("AssistingPlayers"),
                Some(1_689_463_717),
                Some(blocked)
            ));
        }
        for blocked in [FieldType::Float, FieldType::Raw, FieldType::Skip] {
            assert!(!verified_nested_member(
                "SelectedV2",
                14,
                Some("SocketAsset"),
                Some(3_666_994_016),
                Some(blocked)
            ));
        }
    }

    #[test]
    fn malformed_nested_value_is_transactional_and_keeps_outer_raw() {
        let malformed = one_element(&[(7, bits_from_bytes(&[2])), (7, bits_from_bytes(&[1]))]);
        let (records, stats) = export_array_with_declarations(
            (KILL_GROUP, KILL_PARENT, KILL_CHECKSUM),
            &[
                (6, "AssistingPlayers", 1_689_463_717),
                (7, "AssistingPlayers", 1_417_448_159),
            ],
            &one_leaf(6, &malformed),
            Some(MEASURED_BUILD),
        );
        assert_eq!(records.fields.len(), 2);
        assert_eq!(
            records.fields[0].raw_bits.as_deref(),
            Some(bytes(&malformed).as_slice())
        );
        assert_eq!(records.fields[1].field_name.as_deref(), Some(KILL_PARENT));
        assert_eq!(stats.array_leaf_decode_errors, 1);
        assert_eq!(
            stats.array.fields_emitted, 3,
            "walker count includes attempted leaves, while no row prefix leaks"
        );

        let valid = one_element(&[(14, bits_from_bytes(&[2])), (15, bits_from_bytes(&[4]))]);
        let (records, stats) = export_array_with_declarations(
            (SELECTED_GROUP, SELECTED_PARENT, SELECTED_CHECKSUM),
            &[
                (13, "EquippableAttachments", 3_137_596_882),
                (14, "SocketAsset", 3_666_994_016),
                (15, "AttachmentAsset", 0),
            ],
            &one_leaf(13, &valid),
            Some(MEASURED_BUILD),
        );
        assert_eq!(
            records.fields.len(),
            2,
            "wrong nested checksum emits no prefix"
        );
        assert_eq!(
            records.fields[0].raw_bits.as_deref(),
            Some(bytes(&valid).as_slice())
        );
        assert_eq!(stats.array_leaf_decode_errors, 1);
    }

    #[test]
    fn nested_fixture_is_not_enabled_outside_the_exact_parent_gate() {
        let nested = one_element(&[(7, bits_from_bytes(&[2]))]);
        for (group, checksum, branch) in [
            (
                "/Script/ShooterGame.Other",
                KILL_CHECKSUM,
                Some(MEASURED_BUILD),
            ),
            (KILL_GROUP, KILL_CHECKSUM + 1, Some(MEASURED_BUILD)),
            (KILL_GROUP, KILL_CHECKSUM, None),
        ] {
            let (records, stats) = export_array_with_declarations(
                (group, KILL_PARENT, checksum),
                &[
                    (6, "AssistingPlayers", 1_689_463_717),
                    (7, "AssistingPlayers", 1_417_448_159),
                ],
                &one_leaf(6, &nested),
                branch,
            );
            assert_eq!(records.fields.len(), 1, "{group}/{checksum}/{branch:?}");
            assert_eq!(records.fields[0].field_name.as_deref(), Some(KILL_PARENT));
            assert_eq!(stats.array.fields_emitted, 0);
        }
    }

    #[test]
    fn selected_v2_and_kill_data_refuse_wrong_identity_and_exact_residuals() {
        for (group, parent, checksum) in [
            (SELECTED_GROUP, SELECTED_PARENT, SELECTED_CHECKSUM),
            (KILL_GROUP, KILL_PARENT, KILL_CHECKSUM),
        ] {
            let valid = one_leaf(19, &[true]);
            for (actual_group, actual_checksum, branch, bits) in [
                (
                    "/Script/ShooterGame.Other",
                    checksum,
                    Some(MEASURED_BUILD),
                    valid.clone(),
                ),
                (group, checksum + 1, Some(MEASURED_BUILD), valid.clone()),
                (group, checksum, None, valid.clone()),
                (group, checksum, Some(MEASURED_BUILD), {
                    let mut suffix = valid.clone();
                    suffix.extend([false; 8]);
                    suffix
                }),
                (
                    group,
                    checksum,
                    Some(MEASURED_BUILD),
                    valid[..valid.len() - 8].to_vec(),
                ),
            ] {
                let (records, stats) = export_array(
                    (actual_group, parent, actual_checksum),
                    (19, "NestedRawMember"),
                    &bits,
                    branch,
                );
                assert_eq!(
                    records.fields.len(),
                    1,
                    "{parent}/{actual_group}/{actual_checksum}"
                );
                if actual_group == group && actual_checksum == checksum && branch.is_some() {
                    assert!(
                        stats.array.unconsumed_root_bits > 0
                            || stats.array.errors > 0
                            || stats.array.implicit_terminations > 0,
                        "exact residual lost its diagnostic: {stats:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn tracked_rewards_literal_opaque_empty_variant_keeps_only_parent_raw() {
        let bits = bits_from_bytes(&[0x02, 0x00, 0x00]);
        let (records, stats) = export_array(
            (OWNER, REWARDS_PARENT, REWARDS_CHECKSUM),
            (49, "Rewards"),
            &bits,
            Some(MEASURED_BUILD),
        );
        assert_eq!(records.fields.len(), 1);
        assert_eq!(
            records.fields[0].field_name.as_deref(),
            Some(REWARDS_PARENT)
        );
        assert_eq!(
            records.fields[0].raw_bits.as_deref(),
            Some([2, 0, 0].as_slice())
        );
        assert_eq!(stats.tracked_rewards_opaque_empty_variants, 1);
        assert_eq!(stats.array.fields_emitted, 0);
    }

    #[test]
    fn tracked_rewards_refuses_wrong_identity_and_any_other_trailer_shape() {
        let literal = bits_from_bytes(&[0x02, 0x00, 0x00]);
        for (group, parent, checksum, branch, bits) in [
            (
                OWNER,
                REWARDS_PARENT,
                REWARDS_CHECKSUM + 1,
                Some(MEASURED_BUILD),
                literal.clone(),
            ),
            (
                "/Script/ShooterGame.Other",
                REWARDS_PARENT,
                REWARDS_CHECKSUM,
                Some(MEASURED_BUILD),
                literal.clone(),
            ),
            (
                OWNER,
                "OtherRewards",
                REWARDS_CHECKSUM,
                Some(MEASURED_BUILD),
                literal.clone(),
            ),
            (
                OWNER,
                REWARDS_PARENT,
                REWARDS_CHECKSUM,
                None,
                literal.clone(),
            ),
            // A different trailing byte and a nonempty extension cannot become
            // an accepted optional trailer.
            (
                OWNER,
                REWARDS_PARENT,
                REWARDS_CHECKSUM,
                Some(MEASURED_BUILD),
                bits_from_bytes(&[0x02, 0x00, 0x01]),
            ),
            (
                OWNER,
                REWARDS_PARENT,
                REWARDS_CHECKSUM,
                Some(MEASURED_BUILD),
                bits_from_bytes(&[0x02, 0x00, 0x00, 0x00]),
            ),
            // No zero index terminator: the exact decoder must reject it.
            (
                OWNER,
                REWARDS_PARENT,
                REWARDS_CHECKSUM,
                Some(MEASURED_BUILD),
                bits_from_bytes(&[0x02]),
            ),
        ] {
            let (records, stats) =
                export_array((group, parent, checksum), (49, "Rewards"), &bits, branch);
            assert_eq!(
                records.fields.len(),
                1,
                "{group}/{parent}/{checksum}/{branch:?}"
            );
            assert_eq!(stats.tracked_rewards_opaque_empty_variants, 0);
            assert_eq!(stats.array.fields_emitted, 0);
        }
    }

    #[test]
    fn tracked_rewards_residual_variants_keep_exact_diagnostics() {
        for bits in [
            bits_from_bytes(&[0x02, 0x00, 0x01]),
            bits_from_bytes(&[0x02, 0x00, 0x00, 0x00]),
            bits_from_bytes(&[0x02]),
        ] {
            let (records, stats) = export_array(
                (OWNER, REWARDS_PARENT, REWARDS_CHECKSUM),
                (49, "Rewards"),
                &bits,
                Some(MEASURED_BUILD),
            );
            assert_eq!(records.fields.len(), 1);
            assert_eq!(stats.tracked_rewards_opaque_empty_variants, 0);
            assert_eq!(stats.array.fields_emitted, 0);
            assert!(
                stats.array.unconsumed_root_bits > 0
                    || stats.array.errors > 0
                    || stats.array.implicit_terminations > 0,
                "residual window lost its exact-decoder diagnostic: {stats:?}"
            );
        }
        // A complete nonempty array followed by a zero byte is not the measured
        // empty variant. Walking a child does not authorize emitting it when
        // the enclosing array retains unexplained bits.
        let mut bits = one_leaf(19, &[false; 32]);
        bits.extend(bits_from_bytes(&[0]));
        let (records, stats) = export_array(
            (OWNER, REWARDS_PARENT, REWARDS_CHECKSUM),
            (19, "Rewards"),
            &bits,
            Some(MEASURED_BUILD),
        );
        assert_eq!(records.fields.len(), 1);
        assert_eq!(stats.tracked_rewards_opaque_empty_variants, 0);
        assert_eq!(stats.array.fields_emitted, 1);
        assert_eq!(stats.array.unconsumed_root_bits, 8);
    }

    #[test]
    fn measured_effect_and_ignore_routes_emit_expected_leaf_values() {
        let payload: Vec<bool> = (0..32)
            .map(|bit| (-0.0f32).to_bits() & (1 << bit) != 0)
            .collect();
        let bits = one_leaf(33, &payload);
        let (records, _) = export_array(
            (
                "/Script/ShooterGame.EffectManagerComponent",
                "ServerActiveEffects",
                3_301_618_856,
            ),
            (33, "StartTimeStamp"),
            &bits,
            Some(MEASURED_BUILD),
        );
        assert_eq!(records.fields.len(), 2);
        assert_eq!(
            records.fields[0].value_f64.unwrap().to_bits(),
            (-0.0f64).to_bits()
        );

        let mut payload = Vec::new();
        packed(&mut payload, 700);
        let bits = one_leaf(5, &payload);
        let (records, _) = export_array_with_child_checksum(
            (
                "/Script/ShooterGame.FiniteSpeedMovementComponent",
                "RequestedIgnoreActors",
                1_063_739_204,
            ),
            (5, "RequestedIgnoreActors", 3_344_674_359),
            &bits,
            Some(MEASURED_BUILD),
        );
        assert_eq!(records.fields.len(), 2);
        let child = &records.fields[0];
        assert_eq!(child.raw_bits.as_deref(), Some(bytes(&payload).as_slice()));
        assert_eq!(
            (
                child.value_i64,
                child.value_f64,
                child.value_bool,
                child.value_str.as_deref()
            ),
            (Some(700), None, None, None)
        );
    }

    #[test]
    fn existing_ability_array_keeps_its_float_type_without_measured_route() {
        let payload: Vec<bool> = (0..32)
            .map(|bit| 12.5f32.to_bits() & (1 << bit) != 0)
            .collect();
        let bits = one_leaf(7, &payload);
        let (records, _) = export_array(
            (
                "/Game/Characters/_Core/Comp_AbilityStatisticsReplicator.Comp_AbilityStatisticsReplicator_C",
                "AbilityCastsThisRound",
                0,
            ),
            (7, "CastTime_4_5AE288704801A9B74D6D159DFC2BD147"),
            &bits,
            None,
        );
        assert_eq!(records.fields.len(), 2);
        assert_eq!(records.fields[0].value_f64, Some(12.5));
        assert_eq!(
            records.fields[0].field_name.as_deref(),
            Some("AbilityCastsThisRound[0].CastTime_4_5AE288704801A9B74D6D159DFC2BD147")
        );
    }

    #[test]
    fn measured_routes_require_the_full_qualified_identity() {
        assert!(measured_array_route(
            "/Script/ShooterGame.FiniteSpeedMovementComponent",
            "RequestedIgnoreActors",
            Some(1_063_739_204)
        ));
        assert!(measured_array_route(
            OWNER,
            REWARDS_PARENT,
            Some(REWARDS_CHECKSUM)
        ));
        assert!(!measured_array_route(
            "/Script/ShooterGame.FiniteSpeedMovementComponent",
            "RequestedIgnoreActors",
            Some(1)
        ));
        assert!(!measured_array_route(
            "/Script/ShooterGame.Other",
            "RequestedIgnoreActors",
            Some(1_063_739_204)
        ));
    }

    #[test]
    fn new_routes_type_only_the_verified_leaf_windows() {
        assert_eq!(
            verified_array_leaf_type(
                "ServerActiveEffects",
                Some(3_301_618_856),
                33,
                Some(FieldType::Float),
                None
            ),
            Some(FieldType::Float)
        );
        assert_eq!(
            verified_array_leaf_type(
                "RequestedIgnoreActors",
                Some(1_063_739_204),
                5,
                Some(FieldType::ObjectNetGuid),
                None
            ),
            None,
            "packed wire integers remain raw without an identity claim"
        );
        assert_eq!(
            verified_array_leaf_type(
                "ServerActiveEffects",
                Some(3_301_618_856),
                33,
                Some(FieldType::Double),
                None
            ),
            None
        );
    }

    #[test]
    fn requested_ignore_actor_requires_exact_child_identity_and_non_raw_resolution() {
        assert_eq!(
            verified_requested_ignore_actor_leaf(
                5,
                Some("RequestedIgnoreActors"),
                Some(3_344_674_359),
                None,
            ),
            Some(FieldType::ObjectNetGuid)
        );
        for (handle, name, checksum, resolved) in [
            (
                5,
                Some("RequestedIgnoreActors"),
                Some(3_344_674_359),
                Some(FieldType::Raw),
            ),
            (
                5,
                Some("RequestedIgnoreActors"),
                Some(3_344_674_359),
                Some(FieldType::Skip),
            ),
            (
                5,
                Some("RequestedIgnoreActors"),
                Some(3_344_674_359),
                Some(FieldType::Int32),
            ),
            (5, Some("Other"), Some(3_344_674_359), None),
            (5, Some("RequestedIgnoreActors"), Some(0), None),
            (4, Some("RequestedIgnoreActors"), Some(3_344_674_359), None),
        ] {
            assert_eq!(
                verified_requested_ignore_actor_leaf(handle, name, checksum, resolved),
                None
            );
        }
    }

    #[test]
    fn requested_ignore_actor_bad_packed_child_keeps_raw_rows_and_counts_error() {
        // 0xff is an unterminated IntPacked value: its continuation bit is
        // set, but the exact leaf window ends before another packed byte.
        let bits = one_leaf(5, &[true; 8]);
        let (records, stats) = export_array_with_child_checksum(
            (
                "/Script/ShooterGame.FiniteSpeedMovementComponent",
                "RequestedIgnoreActors",
                1_063_739_204,
            ),
            (5, "RequestedIgnoreActors", 3_344_674_359),
            &bits,
            Some(MEASURED_BUILD),
        );
        assert_eq!(records.fields.len(), 2);
        let child = &records.fields[0];
        assert_eq!(child.raw_bits.as_deref(), Some([0xff].as_slice()));
        assert_eq!(child.value_i64, None);
        assert_eq!(child.value_f64, None);
        assert_eq!(child.value_bool, None);
        assert_eq!(child.value_str, None);
        let parent = &records.fields[1];
        assert_eq!(parent.raw_bits.as_deref(), Some(bytes(&bits).as_slice()));
        assert_eq!(stats.array_leaf_decode_errors, 1);
    }

    #[test]
    fn a_typed_array_leaf_failure_is_counted_while_its_raw_input_survives() {
        let raw = [0x7a];
        let mut failures = 0;

        let decoded = decode_leaf_with_stats(FieldType::Int32, &raw, 8, &mut failures);

        assert_eq!(decoded, (None, None, None, None));
        assert_eq!(failures, 1);
        assert_eq!(raw, [0x7a]);
    }

    #[test]
    fn measured_effect_vectors_are_typed_without_a_top_level_overlay() {
        let payload: Vec<bool> = [1.25f64, -0.0, -2.5]
            .iter()
            .flat_map(|value| (0..64).map(move |bit| value.to_bits() & (1 << bit) != 0))
            .collect();
        for (handle, name) in [(30, "Translation"), (31, "Scale3D")] {
            let bits = one_leaf(handle, &payload);
            let (records, _) = export_array(
                (
                    "/Script/ShooterGame.EffectManagerComponent",
                    "ServerActiveEffects",
                    3_301_618_856,
                ),
                (handle, name),
                &bits,
                Some(MEASURED_BUILD),
            );
            assert_eq!(records.fields.len(), 2);
            assert_eq!(
                records.fields[0].value_str.as_deref(),
                Some("(1.25,-0,-2.5)")
            );
        }
        assert_eq!(
            verified_array_leaf_type(
                "ServerActiveEffects",
                Some(3_301_618_856),
                30,
                None,
                Some("DifferentMember")
            ),
            None
        );
    }
}
