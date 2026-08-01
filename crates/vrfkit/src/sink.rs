//! Sink implementation connecting `vrf-net` events to `vrf-export` writers.
//!
//! # Design: no skipping
//!
//! Every field/RPC is emitted, even if we cannot resolve the group path or field
//! name. In that case we emit `group_path = "<unknown:{guid}>"` and
//! `field_name = None`. This ensures the Parquet output is a **lossless**
//! representation of the stream.

use std::collections::HashMap;
use std::sync::Arc;

use vrf_bitio::BitReader;
use vrf_decode::{
    ArrayDecodeStats, COMBAT_ROUNDS_SCHEMA, OVERLAY_TABLE, OverlayStats, OverlayTable,
    apply_overlay, decode_struct_array,
};
use vrf_export::{ActorRecord, FieldRecord, MovementRecord};
use vrf_net::content::ContentBlockHeader;
use vrf_net::field::FieldSink;
use vrf_net::net_guid::GuidPathSink;
use vrf_net::pipeline::{ActorChannelState, ReplicationSink, StreamFailure};
use vrf_net::types::NetworkGuid;
use vrf_schema::{NetGuidCache, class_net_cache_lookup_keys, replay_path_lookup_keys};

/// Static overlay table built from C# descriptors.
static TABLE: OverlayTable = OverlayTable::new(&OVERLAY_TABLE);

/// Well-known subobject leaf names that map to a fixed class path.
///
/// The replay uses short "stably named" identifiers for certain built-in
/// components. When no `class_net_guid` is present we fall back to this table,
/// exactly as the C# reference parser does in `ContentBlockPathResolver`.
const KNOWN_SUBOBJECT_CLASS_PATHS: &[(&str, &str)] = &[
    ("ReplayEffect", "/Script/ShooterGame.ReplayEffectComponent"),
    (
        "EffectManager",
        "/Script/ShooterGame.EffectManagerComponent",
    ),
    (
        "LocationalEffectManager",
        "/Script/ShooterGame.LocationalEffectManagerComponent",
    ),
    (
        "DamageHandlerComponent",
        "/Script/ShooterGame.DamageableComponent",
    ),
];

/// Memo for `ExportSink::find_rpc_param_group_path`.
///
/// That lookup is a pure function of (content-block group path, function name,
/// the set of declared group paths). The third input is what
/// `NetGuidCache::schema_generation` tracks, so stamping the memo with it and
/// clearing on a change makes the memo exactly equivalent to recomputing.
///
/// Why it matters: the fallback branch scans every declared group (475 on
/// 02d4d478) with `ends_with`, once per RPC, and a replay has 342,735 RPCs.
/// The distinct (group path, function name) pairs number in the hundreds.
///
/// Two levels rather than a tuple key so that a hit costs no allocation: both
/// levels are keyed by `String` and probed with `&str`.
#[derive(Debug, Clone, Default)]
struct RpcParamGroupMemo {
    generation: u64,
    by_group: HashMap<String, HashMap<String, Option<Arc<str>>>>,
}

/// Persistent per-channel state that must survive across packets and chunks.
///
/// The replay pipeline creates a fresh `ExportSink` for every packet (to
/// satisfy borrow-checker constraints around `NetGuidCache` mutability). This
/// struct holds the state that *must* persist across those boundaries ??namely
/// the archetype GUID assigned when a channel is opened, which is needed later
/// to resolve ClassNetCache export groups when content blocks arrive.
#[derive(Debug, Clone, Default)]
pub struct ChannelState {
    /// channel_index ??archetype NetworkGuid.
    archetypes: HashMap<u32, NetworkGuid>,
    /// See [`RpcParamGroupMemo`].
    rpc_param_groups: RpcParamGroupMemo,
    /// One line per content block that framed and decoded but whose inner stream
    /// could not be walked.
    ///
    /// Lives here rather than on the sink because the sink is rebuilt for every
    /// packet, so anything recorded on it is lost immediately. Capped: a build
    /// whose transform is wrong would fail on essentially every block, and the
    /// first few dozen say everything the later million would.
    stream_failures: Vec<String>,
}

/// How many stream-failure lines to retain. See `ChannelState::stream_failures`.
const MAX_STREAM_FAILURE_RECORDS: usize = 32;

impl ChannelState {
    /// Create an empty channel state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one stream failure, up to the cap.
    pub fn push_stream_failure(&mut self, line: String) {
        if self.stream_failures.len() < MAX_STREAM_FAILURE_RECORDS {
            self.stream_failures.push(line);
        }
    }

    /// Retained stream-failure lines.
    #[must_use]
    pub fn stream_failures(&self) -> &[String] {
        &self.stream_failures
    }
}
#[derive(Debug, Clone, Default)]
pub struct ExportStats {
    pub fields_emitted: u64,
    pub rpcs_emitted: u64,
    pub actor_opens: u64,
    pub actor_closes: u64,
    pub content_blocks: u64,
    pub overlay: OverlayStats,
    pub array: ArrayDecodeStats,
}

/// The record buffers a sink fills for one packet.
///
/// These live outside the sink and are lent to it. The sink is rebuilt for each
/// of a replay's ~530 k packets, so a `Vec` allocated in its constructor is
/// allocated (and freed) half a million times; that construct-and-drop cost
/// measured at ~290 ms of a 1.79 s export, larger than the whole movement
/// decoder. The buffers are empty at the end of every packet, so keeping their
/// capacity across packets costs one allocation for the entire run.
///
/// [`ExportSink::new`] clears them, so a sink always starts empty no matter what
/// the previous holder did. That is what stops a caller which never drains them
/// -- the validation oracle is one -- from accumulating every record in the
/// replay.
#[derive(Debug, Default)]
pub struct RecordBuffers {
    /// Field records to be drained by the driver.
    pub fields: Vec<FieldRecord>,
    /// Movement records to be drained by the driver.
    pub movement: Vec<MovementRecord>,
    /// Actor lifecycle records to be drained by the driver.
    pub actors: Vec<ActorRecord>,
}

/// The export sink. Receives decoded events from `vrf-net` and produces records
/// for the Parquet writers.
///
/// The sink borrows the `NetGuidCache` mutably because `vrf-net` calls
/// `GuidPathSink::register_path` during packet processing (for package-map
/// export bunches that declare new GUID?뭦ath mappings inline).
pub struct ExportSink<'a> {
    /// Schema cache ??mutable because in-packet path registrations need it.
    pub cache: &'a mut NetGuidCache,
    /// Persistent per-channel state (archetype mappings survive across packets).
    channel_state: &'a mut ChannelState,
    /// Current frame time in milliseconds.
    pub time_ms: u32,
    /// Current packet index.
    pub packet_id: u32,
    /// Output buffers for this packet. See [`RecordBuffers`].
    records: &'a mut RecordBuffers,
    /// Stats.
    pub stats: ExportStats,

    // ?? per-content-block context (set by on_content_block) ??????????????
    current_channel: u32,
    current_actor_guid: u32,
    /// Subobject GUID of the block being walked; `None` for actor blocks.
    current_object_guid: Option<u32>,
    current_group_path: String,
}

impl<'a> ExportSink<'a> {
    /// Build a sink for one packet over caller-owned record buffers.
    ///
    /// The buffers are cleared here rather than trusted to arrive empty: that is
    /// what makes it safe to lend the same buffers to every packet regardless of
    /// whether the caller drains them.
    pub fn new(
        cache: &'a mut NetGuidCache,
        channel_state: &'a mut ChannelState,
        records: &'a mut RecordBuffers,
    ) -> Self {
        records.fields.clear();
        records.movement.clear();
        records.actors.clear();
        Self {
            cache,
            channel_state,
            time_ms: 0,
            packet_id: 0,
            records,
            stats: ExportStats::default(),
            current_channel: 0,
            current_actor_guid: 0,
            current_object_guid: None,
            current_group_path: String::new(),
        }
    }

    /// Resolve a content block to the appropriate export group path.
    ///
    /// Follows the same logic as the C# `ContentBlockPathResolver`:
    /// - For **actor** blocks: derive the class path from the channel's archetype
    ///   GUID (outer path + class-name leaf extracted from archetype path).
    /// - For **subobject** blocks: use `class_net_guid` directly.
    ///
    /// For RepLayout blocks the result must match a RepLayout group; for
    /// ClassNetCache blocks the result must match a `*_ClassNetCache` group.
    fn resolve_group_path(
        &self,
        channel_index: u32,
        guid: u32,
        header: &ContentBlockHeader,
    ) -> String {
        if header.is_actor {
            self.resolve_actor_group_path(channel_index, guid, header)
        } else {
            self.resolve_subobject_group_path(guid, header)
        }
    }

    /// Actor path resolution ??mirrors `ResolveCachedActorExportGroupPath` /
    /// `ResolveCachedActorClassPath` from the C# reference.
    ///
    /// Priority order (matching C# `ResolveActorPackageOrClassPath`):
    /// 1. Archetype outer path (package path for the class)
    /// 2. Archetype path itself (if not a CDO path)
    /// 3. Actor GUID path
    ///
    /// For ClassNetCache blocks we then combine the package path with the class
    /// name extracted from the archetype's leaf (stripping `Default__`).
    fn resolve_actor_group_path(
        &self,
        channel_index: u32,
        actor_guid: u32,
        header: &ContentBlockHeader,
    ) -> String {
        // Step 1: Determine the base "package or class" path.
        let (package_path, archetype_path) =
            self.resolve_actor_package_and_archetype(channel_index);

        // Step 2: Combine package path with class name from archetype.
        let combined =
            self.create_combined_candidate(package_path.as_deref(), archetype_path.as_deref());

        // Step 3: Find matching export group using the combined or package path.
        let lookup_keys_fn = if header.has_rep_layout {
            replay_path_lookup_keys
        } else {
            class_net_cache_lookup_keys
        };

        // For ClassNetCache blocks, only accept groups whose canonical path
        // ends with `_ClassNetCache`. Without this check, a lookup key like
        // `AggroBot_PC.AggroBot_PC_C` would match the RepLayout group (14
        // fields) instead of the ClassNetCache group (4 fields), causing
        // ReadSerializedInt to consume the wrong number of bits.
        let is_cnc = !header.has_rep_layout;

        // Try combined path first (most specific).
        if let Some(ref combined_path) = combined {
            for key in lookup_keys_fn(combined_path) {
                if let Some(g) = self.cache.get_group_by_path(&key) {
                    if !is_cnc || g.path.ends_with("_ClassNetCache") {
                        return g.path.clone();
                    }
                }
            }
        }

        // Try package path directly.
        if let Some(ref pkg) = package_path {
            for key in lookup_keys_fn(pkg) {
                if let Some(g) = self.cache.get_group_by_path(&key) {
                    if !is_cnc || g.path.ends_with("_ClassNetCache") {
                        return g.path.clone();
                    }
                }
            }
        }

        // Try archetype path (if not CDO).
        if let Some(ref arch) = archetype_path {
            if !is_class_default_object_path(arch) {
                for key in lookup_keys_fn(arch) {
                    if let Some(g) = self.cache.get_group_by_path(&key) {
                        if !is_cnc || g.path.ends_with("_ClassNetCache") {
                            return g.path.clone();
                        }
                    }
                }
            }
        }

        // Fallback: try actor GUID path directly.
        if let Some(actor_path) = self.cache.get_path_by_guid(actor_guid) {
            for key in lookup_keys_fn(actor_path) {
                if let Some(g) = self.cache.get_group_by_path(&key) {
                    if !is_cnc || g.path.ends_with("_ClassNetCache") {
                        return g.path.clone();
                    }
                }
            }
            return actor_path.to_owned();
        }

        // Return the best candidate even if it doesn't match a group ??the
        // export format requires a path, and downstream still gets the raw bits.
        combined
            .or(package_path)
            .unwrap_or_else(|| format!("<unknown:{actor_guid}>"))
    }

    /// Determine the package path and archetype path for an actor channel.
    ///
    /// Returns `(package_or_class_path, archetype_path)`. Either may be `None`
    /// if the GUID cache doesn't have the mapping yet.
    fn resolve_actor_package_and_archetype(
        &self,
        channel_index: u32,
    ) -> (Option<String>, Option<String>) {
        let archetype_guid = match self.channel_state.archetypes.get(&channel_index) {
            Some(g) if g.is_valid() => *g,
            _ => return (None, None),
        };

        let archetype_path = self
            .cache
            .get_path_by_guid(archetype_guid.0)
            .map(|s| s.to_owned());

        // C# priority: outer path of archetype first (gives the package path).
        let package_path = self
            .cache
            .get_outer_path(archetype_guid.0)
            .map(|s| s.to_owned());

        (package_path, archetype_path)
    }

    /// Combine the package path with the class name from the archetype path.
    ///
    /// Mirrors `TryCreateCombinedCandidate` in C#:
    /// - Extract the leaf of `archetype_path` (after last `/`, `.`, or `:`).
    /// - Strip `Default__` prefix if present to get the class name.
    /// - If `package_path` already ends with `.{class_name}`, return as-is.
    /// - Otherwise append `.{class_name}` to `package_path`.
    fn create_combined_candidate(
        &self,
        package_path: Option<&str>,
        archetype_path: Option<&str>,
    ) -> Option<String> {
        let pkg = package_path?;
        let class_name = extract_class_name_from_archetype(archetype_path?)?;

        // Check if package path already ends with the class name.
        if ends_with_class_name(pkg, class_name) {
            return Some(pkg.to_owned());
        }

        let mut combined = String::with_capacity(pkg.len() + 1 + class_name.len());
        combined.push_str(pkg);
        combined.push('.');
        combined.push_str(class_name);
        Some(combined)
    }

    /// Subobject path resolution ??mirrors `ResolveSubobjectExportGroupPath` /
    /// `ResolveSubobjectClassPath` from C#.
    fn resolve_subobject_group_path(
        &self,
        _actor_guid: u32,
        header: &ContentBlockHeader,
    ) -> String {
        let lookup_keys_fn = if header.has_rep_layout {
            replay_path_lookup_keys
        } else {
            class_net_cache_lookup_keys
        };

        let is_cnc = !header.has_rep_layout;

        // Primary: use class_net_guid path.
        if header.class_net_guid.0 != 0 {
            if let Some(class_path) = self.cache.get_path_by_guid(header.class_net_guid.0) {
                for key in lookup_keys_fn(class_path) {
                    if let Some(g) = self.cache.get_group_by_path(&key) {
                        if !is_cnc || g.path.ends_with("_ClassNetCache") {
                            return g.path.clone();
                        }
                    }
                }
                // UniqueLeafMatch: if class_path is a bare name (no separators),
                // try to find a group whose path ends with ".{class_path}".
                // Mirrors C# ContentBlockPathResolver.UniqueLeafMatch.
                if let Some(g) = self.cache.unique_leaf_match(class_path) {
                    if !is_cnc || g.path.ends_with("_ClassNetCache") {
                        return g.path.clone();
                    }
                }
                return class_path.to_owned();
            }
        }

        // Secondary: use object_net_guid for path lookup.
        if header.object_net_guid.0 != 0 {
            if let Some(obj_path) = self.cache.get_path_by_guid(header.object_net_guid.0) {
                // Try outer path (component ??owning class).
                if let Some(outer) = self.cache.get_outer_path(header.object_net_guid.0) {
                    for key in lookup_keys_fn(outer) {
                        if let Some(g) = self.cache.get_group_by_path(&key) {
                            if !is_cnc || g.path.ends_with("_ClassNetCache") {
                                return g.path.clone();
                            }
                        }
                    }
                }

                for key in lookup_keys_fn(obj_path) {
                    if let Some(g) = self.cache.get_group_by_path(&key) {
                        if !is_cnc || g.path.ends_with("_ClassNetCache") {
                            return g.path.clone();
                        }
                    }
                }

                // UniqueLeafMatch for object path.
                if let Some(g) = self.cache.unique_leaf_match(obj_path) {
                    if !is_cnc || g.path.ends_with("_ClassNetCache") {
                        return g.path.clone();
                    }
                }

                // Fallback: known subobject class path table.
                if is_cnc {
                    if let Some(known) = resolve_known_subobject_class_path(obj_path) {
                        for key in class_net_cache_lookup_keys(known) {
                            if let Some(g) = self.cache.get_group_by_path(&key) {
                                if g.path.ends_with("_ClassNetCache") {
                                    return g.path.clone();
                                }
                            }
                        }
                        return known.to_owned();
                    }
                }

                return obj_path.to_owned();
            }
        }

        let fallback_guid = if header.class_net_guid.0 != 0 {
            header.class_net_guid.0
        } else {
            header.object_net_guid.0
        };
        format!("<unknown:{fallback_guid}>")
    }

    /// Resolve field name from the cache.
    fn resolve_field_name(&self, handle: u32) -> Option<String> {
        let group = self.cache.get_group_by_path(&self.current_group_path)?;
        group.get_field(handle).map(|f| f.name.clone())
    }

    /// Check if a field name is a known DynamicArray that should be flattened.
    fn is_known_array_field(&self, field_name: Option<&str>) -> bool {
        match field_name {
            Some("Rounds") => self.current_group_path.contains("CombatReportComponent"),
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
            _ => None,
        }
    }

    /// Check if a field is a struct blob that has a dedicated decoder.
    fn is_struct_blob_field(&self, field_name: Option<&str>) -> bool {
        match field_name {
            Some("RoundResults") | Some("TeamEconomy") => {
                self.current_group_path.contains("BombGameState")
            }
            Some("RoundInfos") => self.current_group_path.contains("OwnerExclusivePlayerInfo"),
            _ => false,
        }
    }

    /// Decode a struct blob and emit flattened sub-field rows.
    /// Returns true if decoding succeeded and sub-fields were emitted.
    fn decode_struct_blob(&mut self, field_name: &str, raw: &[u8], bit_count: u32) -> bool {
        match field_name {
            "RoundResults" if self.current_group_path.contains("BombGameState") => {
                self.decode_round_results_blob(raw, bit_count)
            }
            "TeamEconomy" if self.current_group_path.contains("BombGameState") => {
                self.decode_team_economy_blob(raw, bit_count)
            }
            "RoundInfos" if self.current_group_path.contains("OwnerExclusivePlayerInfo") => {
                self.decode_round_infos_blob(raw, bit_count)
            }
            _ => false,
        }
    }

    /// Decode RoundResults blob and emit sub-field rows.
    fn decode_round_results_blob(&mut self, raw: &[u8], bit_count: u32) -> bool {
        use vrf_decode::structs::decode_round_results;

        let mut reader = BitReader::with_bit_len(raw, u64::from(bit_count));
        let results = match decode_round_results(&mut reader) {
            Ok(r) => r,
            Err(_) => return false,
        };

        for rr in &results {
            let prefix = format!("RoundResults[{}]", rr.round_number);

            // RoundNumber
            self.emit_struct_sub_field(
                &format!("{prefix}.RoundNumber"),
                Some(i64::from(rr.round_number)),
                None,
                None,
                None,
            );

            // WinningTeam
            if let Some(ref team) = rr.winning_team {
                self.emit_struct_sub_field(
                    &format!("{prefix}.WinningTeam"),
                    None,
                    None,
                    None,
                    Some(team.clone()),
                );
            }

            // WinningTeamRole
            if let Some(role) = rr.winning_team_role {
                let role_str = match role {
                    vrf_decode::structs::AresTeamRole::None => "none",
                    vrf_decode::structs::AresTeamRole::Attacker => "attacker",
                    vrf_decode::structs::AresTeamRole::Defender => "defender",
                    vrf_decode::structs::AresTeamRole::FreeForAll => "free_for_all",
                    vrf_decode::structs::AresTeamRole::Any => "any",
                    vrf_decode::structs::AresTeamRole::RoleCount => "role_count",
                };
                self.emit_struct_sub_field(
                    &format!("{prefix}.WinningTeamRole"),
                    None,
                    None,
                    None,
                    Some(role_str.to_owned()),
                );
            }

            // RoundResult
            if let Some(outcome) = rr.round_result {
                let outcome_str = match outcome {
                    vrf_decode::structs::AresRoundOutcome::Elimination => "elimination",
                    vrf_decode::structs::AresRoundOutcome::Defuse => "defuse",
                    vrf_decode::structs::AresRoundOutcome::Detonate => "detonate",
                    vrf_decode::structs::AresRoundOutcome::TimeExpired => "time_expired",
                    vrf_decode::structs::AresRoundOutcome::Cheat => "cheat",
                    vrf_decode::structs::AresRoundOutcome::Surrendered => "surrendered",
                    vrf_decode::structs::AresRoundOutcome::RoundOutcomeCount => {
                        "round_outcome_count"
                    }
                    vrf_decode::structs::AresRoundOutcome::Invalid => "invalid",
                };
                self.emit_struct_sub_field(
                    &format!("{prefix}.RoundResult"),
                    None,
                    None,
                    None,
                    Some(outcome_str.to_owned()),
                );
            }
        }

        !results.is_empty()
    }

    /// Decode TeamEconomy blob and emit sub-field rows.
    fn decode_team_economy_blob(&mut self, raw: &[u8], bit_count: u32) -> bool {
        use vrf_decode::structs::decode_team_economy;

        let mut reader = BitReader::with_bit_len(raw, u64::from(bit_count));
        let results = match decode_team_economy(&mut reader) {
            Ok(r) => r,
            Err(_) => return false,
        };

        for te in &results {
            let prefix = format!("TeamEconomy[{}]", te.index);

            // Index
            self.emit_struct_sub_field(
                &format!("{prefix}.Index"),
                Some(i64::from(te.index)),
                None,
                None,
                None,
            );

            // ReplicationId
            if let Some(rep_id) = te.replication_id {
                self.emit_struct_sub_field(
                    &format!("{prefix}.ReplicationId"),
                    Some(i64::from(rep_id)),
                    None,
                    None,
                    None,
                );
            }

            // LoadoutValue
            if let Some(lv) = te.loadout_value {
                self.emit_struct_sub_field(
                    &format!("{prefix}.LoadoutValue"),
                    Some(i64::from(lv)),
                    None,
                    None,
                    None,
                );
            }

            // AverageLoadoutValue
            if let Some(alv) = te.average_loadout_value {
                self.emit_struct_sub_field(
                    &format!("{prefix}.AverageLoadoutValue"),
                    Some(i64::from(alv)),
                    None,
                    None,
                    None,
                );
            }
        }

        !results.is_empty()
    }

    /// Decode RoundInfos blob and emit sub-field rows.
    fn decode_round_infos_blob(&mut self, raw: &[u8], bit_count: u32) -> bool {
        use vrf_decode::structs::decode_round_infos;

        let mut reader = BitReader::with_bit_len(raw, u64::from(bit_count));
        let results = match decode_round_infos(&mut reader) {
            Ok(r) => r,
            Err(_) => return false,
        };

        for ri in &results {
            let prefix = format!("RoundInfos[{}]", ri.index);

            // RoundNumber
            if let Some(rn) = ri.round_number {
                self.emit_struct_sub_field(
                    &format!("{prefix}.RoundNumber"),
                    Some(i64::from(rn)),
                    None,
                    None,
                    None,
                );
            }

            // StartOfRoundMoney
            if let Some(v) = ri.start_of_round_money {
                self.emit_struct_sub_field(
                    &format!("{prefix}.StartOfRoundMoney"),
                    Some(i64::from(v)),
                    None,
                    None,
                    None,
                );
            }

            // StartOfRoundLoadoutValue
            if let Some(v) = ri.start_of_round_loadout_value {
                self.emit_struct_sub_field(
                    &format!("{prefix}.StartOfRoundLoadoutValue"),
                    Some(i64::from(v)),
                    None,
                    None,
                    None,
                );
            }

            // EndOfRoundMoney
            if let Some(v) = ri.end_of_round_money {
                self.emit_struct_sub_field(
                    &format!("{prefix}.EndOfRoundMoney"),
                    Some(i64::from(v)),
                    None,
                    None,
                    None,
                );
            }

            // EndOfRoundLoadoutValue
            if let Some(v) = ri.end_of_round_loadout_value {
                self.emit_struct_sub_field(
                    &format!("{prefix}.EndOfRoundLoadoutValue"),
                    Some(i64::from(v)),
                    None,
                    None,
                    None,
                );
            }
        }

        !results.is_empty()
    }

    /// Emit a single sub-field row for a decoded struct blob element.
    fn emit_struct_sub_field(
        &mut self,
        field_name: &str,
        value_i64: Option<i64>,
        value_f64: Option<f64>,
        value_bool: Option<bool>,
        value_str: Option<String>,
    ) {
        self.records.fields.push(FieldRecord {
            time_ms: self.time_ms,
            packet_id: self.packet_id,
            channel_index: self.current_channel,
            actor_net_guid: self.current_actor_guid,
            object_net_guid: self.current_object_guid,
            group_path: self.current_group_path.clone(),
            handle: 0,
            field_name: Some(field_name.to_owned()),
            bit_count: 0,
            raw_bits: None,
            value_i64,
            value_f64,
            value_bool,
            value_str,
        });
        self.stats.fields_emitted += 1;
    }

    /// Determine function_count for a ClassNetCache block.
    ///
    /// The function count equals `NetFieldExportGroup.len()` for the matching
    /// ClassNetCache group. The C# parser uses
    /// `ReadSerializedInt(FunctionsByHandle.Length)` where `FunctionsByHandle`
    /// is sized to `replayGroup.NetFieldExportsLength`, i.e. the number of
    /// declared export slots in the ClassNetCache group.
    ///
    /// If the group cannot be resolved we return 0, which causes the RPC parser
    /// to skip the bits but NOT silently drop them. The caller still records the
    /// raw payload and the oracle counts this as a stream failure.
    fn resolve_function_count(&mut self, header: &ContentBlockHeader, channel_index: u32) -> u32 {
        // Fast path: current_group_path was already resolved to a CNC group.
        if let Some(group) = self.cache.get_group_by_path(&self.current_group_path) {
            if group.path.ends_with("_ClassNetCache") {
                return group.len();
            }
        }

        // For subobjects: try class_net_guid with ClassNetCache suffix toggle.
        if header.class_net_guid.0 != 0 {
            if let Some(class_path) = self.cache.get_path_by_guid(header.class_net_guid.0) {
                for key in class_net_cache_lookup_keys(class_path) {
                    if let Some(group) = self.cache.get_group_by_path(&key) {
                        if group.path.ends_with("_ClassNetCache") {
                            return group.len();
                        }
                    }
                }
            }
        }

        // For actors: try deriving from archetype.
        if header.is_actor {
            let (package_path, archetype_path) =
                self.resolve_actor_package_and_archetype(channel_index);
            if let Some(combined) =
                self.create_combined_candidate(package_path.as_deref(), archetype_path.as_deref())
            {
                for key in class_net_cache_lookup_keys(&combined) {
                    if let Some(group) = self.cache.get_group_by_path(&key) {
                        if group.path.ends_with("_ClassNetCache") {
                            return group.len();
                        }
                    }
                }
            }
        }

        // Schema-driven fallback: when the resolved path is a bare instance
        // name (no path separators), search the replay's own ClassNetCache
        // groups for one whose leaf matches the instance name after applying
        // Unreal naming conventions.
        //
        // This recovers blocks for static actors (BombDestination_A,
        // WindowShieldA1) and stably-named subobjects (ForceModuleManager,
        // AudDeadeyeVOComponent) that have no archetype GUID or class net GUID
        // on the wire. The capacity comes from the matched group's declared
        // NetFieldExportsLength -- never guessed.
        //
        // The C# reference parser also fails on these same blocks. This lookup
        // goes beyond C# by leveraging the replay's own declared schema.
        if !self.current_group_path.contains('/')
            && !self.current_group_path.contains('.')
            && !self.current_group_path.contains(':')
            && !self.current_group_path.starts_with('<')
        {
            if let Some(group) = self
                .cache
                .resolve_cnc_for_instance_name(&self.current_group_path)
            {
                let resolved_path = group.path.clone();
                let len = group.len();
                // Update current_group_path so downstream field/RPC handle
                // lookups use the correct group. Without this, resolved RPCs
                // would emit handle-indexed names instead of proper field names.
                self.current_group_path = resolved_path;
                return len;
            }
        }

        0
    }

    /// Try to parse an RPC payload as a RepLayout field stream using the
    /// RPC parameter group from the schema.
    ///
    /// # Wire format (confirmed against C# `ParseClassNetCachePayload`)
    ///
    /// The RPC payload is a sub-archive whose contents follow RepLayout
    /// `FunctionParameters` grammar:
    /// ```text
    ///   propertyChecksum : 1 bit (ignored)
    ///   loop:
    ///     encodedHandle  : IntPacked
    ///     [if 0 -> break]
    ///     handle = encodedHandle - 1
    ///     payloadBits    : IntPacked
    ///     fieldPayload   : sub-reader of payloadBits bits
    /// ```
    /// Additionally, if exactly 1 bit remains after reading all fields, it is
    /// a trailing alignment bit that must be consumed (C# grammar check:
    /// `FunctionParameters && BitsRemaining == 1 -> SkipBits(1)`).
    ///
    /// # Group path resolution
    ///
    /// RPC parameter groups are registered with paths like
    /// `/Script/ShooterGame.ShooterCharacter:MulticastNotifyKilledEnemy`.
    /// The content block's CNC group might be an agent-specific path like
    /// `Wushu_PC_C_ClassNetCache`. We cannot simply strip the suffix and
    /// append `:FunctionName` because the parent class differs.
    ///
    /// Strategy: look up the group by function name as a unique leaf suffix
    /// (the part after `:`). Most RPC parameter groups have a unique function
    /// name across all 84 groups. For the rare case where the name is not
    /// unique, we fall back to emitting unnamed handle-indexed rows.
    ///
    /// Returns `true` if parameters were emitted (even if just handle-indexed),
    /// `false` if the payload could not be walked at all (caller should emit
    /// raw_bits row).
    fn try_parse_rpc_params(
        &mut self,
        rpc_handle: u32,
        _bit_count: u32,
        reader: BitReader<'_>,
        function_name: Option<&str>,
    ) -> bool {
        let func_name = match function_name {
            Some(n) => n,
            None => return false,
        };

        // Find the RPC parameter group. Try direct path construction first,
        // then fall back to function-name leaf match.
        let param_group_path = self.find_rpc_param_group_path(func_name);

        // Parse the RepLayout stream inside the RPC payload.
        let mut rpc_reader = reader;

        // Property checksum bit (1 bit) ??always present for FunctionParameters.
        if rpc_reader.read_bit().is_err() {
            return false;
        }

        let mut emitted_any = false;
        let param_group_path_ref = param_group_path.as_deref();

        loop {
            if rpc_reader.at_end() {
                break;
            }

            // FunctionParameters grammar: if exactly 1 bit remains, skip it.
            if rpc_reader.bits_remaining() == 1 {
                let _ = rpc_reader.read_bit();
                break;
            }

            let encoded_handle = match rpc_reader.read_int_packed() {
                Ok(h) => h,
                Err(_) => break,
            };
            if encoded_handle == 0 {
                break;
            }

            let param_handle = encoded_handle - 1;
            let payload_bits = match rpc_reader.read_int_packed() {
                Ok(b) => b,
                Err(_) => break,
            };

            if payload_bits as u64 > rpc_reader.bits_remaining() {
                // Malformed: more bits declared than available. Stop parsing
                // but keep what we have (emitted_any may be true).
                break;
            }

            let sub = match rpc_reader.sub_reader(payload_bits as u64) {
                Ok(s) => s,
                Err(_) => break,
            };

            // Resolve parameter field name from the group.
            let param_name: Option<String> = param_group_path_ref.and_then(|gp| {
                self.cache
                    .get_group_by_path(gp)
                    .and_then(|g| g.get_field(param_handle))
                    .map(|f| f.name.clone())
            });

            // Build field_name: "FunctionName.ParamName" or "FunctionName._h{N}"
            let full_field_name = match param_name.as_deref() {
                Some(pn) => format!("{func_name}.{pn}"),
                None => format!("{func_name}._h{param_handle}"),
            };

            // Extract raw bits for this parameter field.
            let raw_bits = if payload_bits > 0 {
                let byte_count = (payload_bits as usize).div_ceil(8);
                let mut buf = vec![0u8; byte_count];
                let mut sub_copy = sub;
                let _ = sub_copy.copy_bits_to(&mut buf, payload_bits as u64);
                Some(buf)
            } else {
                None
            };

            // Apply type overlay using the parameter group path as group_path.
            let overlay_group = param_group_path_ref.unwrap_or(&self.current_group_path);
            let overlay_field = param_name.as_deref().unwrap_or(&full_field_name);
            let (value_i64, value_f64, value_bool, value_str) = match apply_overlay(
                &TABLE,
                overlay_group,
                Some(overlay_field),
                raw_bits.as_deref(),
                payload_bits,
                &mut self.stats.overlay,
            ) {
                Some(result) => (
                    result.value_i64,
                    result.value_f64,
                    result.value_bool,
                    result.value_str,
                ),
                None => (None, None, None, None),
            };

            self.records.fields.push(FieldRecord {
                time_ms: self.time_ms,
                packet_id: self.packet_id,
                channel_index: self.current_channel,
                actor_net_guid: self.current_actor_guid,
                object_net_guid: self.current_object_guid,
                group_path: self.current_group_path.clone(),
                handle: rpc_handle,
                field_name: Some(full_field_name),
                bit_count: payload_bits,
                raw_bits,
                value_i64,
                value_f64,
                value_bool,
                value_str,
            });
            self.stats.fields_emitted += 1;

            emitted_any = true;
        }

        emitted_any
    }

    /// Find the RPC parameter group path for a given function name.
    ///
    /// Strategy:
    /// 1. Try stripping `_ClassNetCache` from current_group_path and appending
    ///    `:<function_name>` ??this works when the CNC group matches the
    ///    parameter group's class (e.g. DamageableComponent).
    /// 2. Search all registered groups for one whose path ends with
    ///    `:<function_name>`. If exactly one matches, use it (unique leaf match).
    ///    If multiple match, return None (ambiguous).
    ///
    /// The second strategy handles inheritance: `Wushu_PC_C_ClassNetCache` has
    /// `MulticastNotifyKilledEnemy` but the parameter group is under
    /// `ShooterCharacter:MulticastNotifyKilledEnemy`.
    /// Memoised wrapper around [`Self::compute_rpc_param_group_path`].
    ///
    /// The computation is a pure function of the current group path, the
    /// function name, and the set of declared group paths; only the third can
    /// change while a replay is being read, and `schema_generation` tracks
    /// exactly that. A generation change discards the whole memo, so a hit is
    /// indistinguishable from a recomputation.
    ///
    /// Strategy 2 below is O(groups) per call. Without this memo a replay pays
    /// 342,735 x 475 `ends_with` probes.
    fn find_rpc_param_group_path(&mut self, function_name: &str) -> Option<Arc<str>> {
        let Self {
            cache,
            channel_state,
            current_group_path,
            ..
        } = self;
        let memo = &mut channel_state.rpc_param_groups;
        let generation = cache.schema_generation();
        if memo.generation != generation {
            memo.by_group.clear();
            memo.generation = generation;
        }

        if let Some(by_function) = memo.by_group.get(current_group_path.as_str()) {
            if let Some(hit) = by_function.get(function_name) {
                return hit.clone();
            }
        }

        let resolved = Self::compute_rpc_param_group_path(cache, current_group_path, function_name);
        memo.by_group
            .entry(current_group_path.clone())
            .or_default()
            .insert(function_name.to_owned(), resolved.clone());
        resolved
    }

    fn compute_rpc_param_group_path(
        cache: &NetGuidCache,
        current_group_path: &str,
        function_name: &str,
    ) -> Option<Arc<str>> {
        // Strategy 1: direct path construction from CNC group.
        if let Some(base) = current_group_path.strip_suffix("_ClassNetCache") {
            let candidate = format!("{base}:{function_name}");
            if cache.get_group_by_path(&candidate).is_some() {
                return Some(Arc::from(candidate));
            }
        }

        // Strategy 2: search for unique group with `:<function_name>` suffix.
        let suffix = format!(":{function_name}");
        let mut found: Option<&str> = None;
        for group in cache.groups() {
            if group.path.ends_with(&suffix) && group.path.contains(':') {
                if found.is_some() {
                    // Ambiguous: multiple groups match this function name.
                    return None;
                }
                found = Some(&group.path);
            }
        }
        found.map(Arc::from)
    }
}

impl GuidPathSink for ExportSink<'_> {
    fn register_path(&mut self, guid: u32, path: &str, outer_guid: NetworkGuid) {
        let outer = if outer_guid.0 != 0 {
            Some(vrf_schema::NetworkGuid(outer_guid.0))
        } else {
            None
        };
        self.cache.set_net_guid_path(guid, path.to_string(), outer);
    }
}

/// The field-name prefix used for RPC parameters.
///
/// RPC parameters are emitted as `FunctionName.ParamName` (dot-separated).
/// This mirrors the existing `Rounds[0].Reports[1].X` convention for nested
/// TArray elements, giving downstream consumers a reliable prefix to split on
/// when distinguishing RPC parameters from ordinary replicated properties.
///
/// Why dot and not colon: colons appear in the *group path* (e.g.
/// `/Script/ShooterGame.ShooterCharacter:MulticastNotifyKilledEnemy`), so using
/// a dot in the field name avoids ambiguity with the path namespace. Downstream
/// can split on the first `.` in field_name to recover function vs parameter.
const _RPC_PARAM_NAMING_DOC: () = ();

impl FieldSink for ExportSink<'_> {
    fn on_field(&mut self, handle: u32, bit_count: u32, reader: BitReader<'_>) {
        let field_name = self.resolve_field_name(handle);

        let raw_bits = if bit_count > 0 {
            let byte_count = (bit_count as usize).div_ceil(8);
            let mut buf = vec![0u8; byte_count];
            let mut reader_copy = reader;
            let _ = reader_copy.copy_bits_to(&mut buf, bit_count as u64);
            Some(buf)
        } else {
            None
        };

        // Check if this field is a known DynamicArray that should be flattened.
        let is_array = self.is_known_array_field(field_name.as_deref());
        if is_array {
            if let Some(ref raw) = raw_bits {
                let schema = self.get_array_schema(field_name.as_deref());
                let flattened = decode_struct_array(raw, bit_count, schema, &mut self.stats.array);
                let parent_name = field_name.as_deref().unwrap_or("_array");
                for f in &flattened {
                    // Build full field name: "Rounds[0].RoundNumber" etc.
                    let full_name = format!("{parent_name}{}", f.path);

                    // Decode leaf fields using known handle?뭪ype mapping.
                    let (vi, vf, vb, vs) = decode_array_leaf(f.handle, &f.raw_bits, f.bit_count);

                    self.records.fields.push(FieldRecord {
                        time_ms: self.time_ms,
                        packet_id: self.packet_id,
                        channel_index: self.current_channel,
                        actor_net_guid: self.current_actor_guid,
                        object_net_guid: self.current_object_guid,
                        group_path: self.current_group_path.clone(),
                        handle: f.handle,
                        field_name: Some(full_name),
                        bit_count: f.bit_count,
                        raw_bits: Some(f.raw_bits.clone()),
                        value_i64: vi,
                        value_f64: vf,
                        value_bool: vb,
                        value_str: vs,
                    });
                    self.stats.fields_emitted += 1;
                }
            }
            // Also emit the parent array row (with raw bits) for completeness.
        }

        // Check if this field is a struct blob with a dedicated decoder
        // (RoundResults, TeamEconomy, RoundInfos). Decoded sub-fields are
        // additive: raw_bits parent row is still emitted below.
        if self.is_struct_blob_field(field_name.as_deref()) {
            if let Some(ref raw) = raw_bits {
                let fname = field_name.as_deref().unwrap_or("");
                self.decode_struct_blob(fname, raw, bit_count);
            }
        }

        // Apply the type overlay: decode raw_bits into a typed value if possible.
        let (value_i64, value_f64, value_bool, value_str) = match apply_overlay(
            &TABLE,
            &self.current_group_path,
            field_name.as_deref(),
            raw_bits.as_deref(),
            bit_count,
            &mut self.stats.overlay,
        ) {
            Some(result) => (
                result.value_i64,
                result.value_f64,
                result.value_bool,
                result.value_str,
            ),
            None => (None, None, None, None),
        };

        self.records.fields.push(FieldRecord {
            time_ms: self.time_ms,
            packet_id: self.packet_id,
            channel_index: self.current_channel,
            actor_net_guid: self.current_actor_guid,
            object_net_guid: self.current_object_guid,
            group_path: self.current_group_path.clone(),
            handle,
            field_name,
            bit_count,
            raw_bits,
            value_i64,
            value_f64,
            value_bool,
            value_str,
        });
        self.stats.fields_emitted += 1;
    }

    fn on_rpc(&mut self, handle: u32, bit_count: u32, reader: BitReader<'_>) {
        let field_name = self.resolve_field_name(handle);

        // Check if this is the movement RPC by matching the field name.
        let is_movement_rpc = field_name.as_deref()
            == Some("ReplaysClientReceiveRemoteCharacterUpdatesSingleArrayNoAutonomous");

        // Decode movement RPC payload if detected.
        if is_movement_rpc && bit_count > 0 {
            let mut rpc_reader = reader;
            let time_ms = self.time_ms;
            let packet_id = self.packet_id;
            let _ = vrf_movement::decode_movement_rpc(&mut rpc_reader, |mv| {
                self.records.movement.push(vrf_export::MovementRecord {
                    time_ms,
                    packet_id,
                    character_net_guid: mv.shooter_character_net_guid,
                    pos_x: mv.pos_x as f32,
                    pos_y: mv.pos_y as f32,
                    pos_z: mv.pos_z as f32,
                    yaw: mv.yaw as f32,
                    pitch: mv.pitch as f32,
                    vel_x: mv.vel_x as f32,
                    vel_y: mv.vel_y as f32,
                    vel_z: mv.vel_z as f32,
                });
            });
            // Don't store raw bits for movement RPCs (saves memory on 2.4M records).
            self.records.fields.push(FieldRecord {
                time_ms: self.time_ms,
                packet_id: self.packet_id,
                channel_index: self.current_channel,
                actor_net_guid: self.current_actor_guid,
                object_net_guid: self.current_object_guid,
                group_path: self.current_group_path.clone(),
                handle,
                field_name,
                bit_count,
                raw_bits: None,
                value_i64: None,
                value_f64: None,
                value_bool: None,
                value_str: None,
            });
        } else if bit_count > 0 {
            // Try to parse RPC parameters as a RepLayout field stream.
            // The parameter group path is `<ClassPath>:<FunctionName>` where
            // ClassPath = current_group_path minus `_ClassNetCache` suffix.
            //
            // We clone the reader before attempting parse so we can fall back
            // to raw_bits emission if parsing yields nothing.
            let fallback_reader = reader.clone();
            let parsed =
                self.try_parse_rpc_params(handle, bit_count, reader, field_name.as_deref());
            if !parsed {
                // Fallback: emit raw bits as a single row (no param group found).
                let raw_bits = {
                    let byte_count = (bit_count as usize).div_ceil(8);
                    let mut buf = vec![0u8; byte_count];
                    let mut rc = fallback_reader;
                    let _ = rc.copy_bits_to(&mut buf, bit_count as u64);
                    Some(buf)
                };
                self.records.fields.push(FieldRecord {
                    time_ms: self.time_ms,
                    packet_id: self.packet_id,
                    channel_index: self.current_channel,
                    actor_net_guid: self.current_actor_guid,
                    object_net_guid: self.current_object_guid,
                    group_path: self.current_group_path.clone(),
                    handle,
                    field_name,
                    bit_count,
                    raw_bits,
                    value_i64: None,
                    value_f64: None,
                    value_bool: None,
                    value_str: None,
                });
            }
        } else {
            // Zero-bit RPC ??just emit a marker row.
            self.records.fields.push(FieldRecord {
                time_ms: self.time_ms,
                packet_id: self.packet_id,
                channel_index: self.current_channel,
                actor_net_guid: self.current_actor_guid,
                object_net_guid: self.current_object_guid,
                group_path: self.current_group_path.clone(),
                handle,
                field_name,
                bit_count: 0,
                raw_bits: None,
                value_i64: None,
                value_f64: None,
                value_bool: None,
                value_str: None,
            });
        }
        self.stats.rpcs_emitted += 1;
    }
}

impl ReplicationSink for ExportSink<'_> {
    fn on_actor_open(&mut self, state: &ActorChannelState) {
        self.stats.actor_opens += 1;
        // Track archetype GUID per channel so ClassNetCache path resolution can
        // walk archetype -> outer path -> class name.
        if state.archetype_net_guid.is_valid() {
            self.channel_state
                .archetypes
                .insert(state.channel_index, state.archetype_net_guid);
        }

        // Resolve class_path: for dynamic actors the outer path of the
        // archetype GUID gives the class. For static actors the actor GUID
        // path itself is the class.
        let class_path = if state.archetype_net_guid.is_valid() {
            let outer = self
                .cache
                .get_outer_path(state.archetype_net_guid.0)
                .map(|s| s.to_owned());
            let arch_path = self
                .cache
                .get_path_by_guid(state.archetype_net_guid.0)
                .map(|s| s.to_owned());
            let combined = self.create_combined_candidate(outer.as_deref(), arch_path.as_deref());
            combined.or(outer)
        } else if state.actor_net_guid.is_valid() {
            self.cache
                .get_path_by_guid(state.actor_net_guid.0)
                .map(|s| s.to_owned())
        } else {
            None
        };

        // Resolve archetype_path from the archetype GUID.
        let archetype_path = if state.archetype_net_guid.is_valid() {
            self.cache
                .get_path_by_guid(state.archetype_net_guid.0)
                .map(|s| s.to_owned())
        } else {
            None
        };

        // Spawn location (only for dynamic actors that have it).
        let (spawn_x, spawn_y, spawn_z) = match state.spawn_location {
            Some(loc) => (Some(loc.x as f32), Some(loc.y as f32), Some(loc.z as f32)),
            None => (None, None, None),
        };

        // Spawn rotation.
        let (spawn_pitch, spawn_yaw, spawn_roll) = match state.spawn_rotation {
            Some(rot) => (Some(rot.pitch), Some(rot.yaw), Some(rot.roll)),
            None => (None, None, None),
        };

        self.records.actors.push(ActorRecord {
            time_ms: self.time_ms,
            packet_id: self.packet_id,
            channel_index: state.channel_index,
            actor_net_guid: state.actor_net_guid.0,
            event: "open",
            class_path,
            archetype_path,
            spawn_x,
            spawn_y,
            spawn_z,
            spawn_pitch,
            spawn_yaw,
            spawn_roll,
        });
    }

    fn on_actor_close(&mut self, channel_index: u32, actor_net_guid: NetworkGuid, _dormant: bool) {
        self.stats.actor_closes += 1;

        // Resolve class_path from the channel's archetype (same logic as open).
        let class_path = if let Some(&arch_guid) = self.channel_state.archetypes.get(&channel_index)
        {
            let outer = self.cache.get_outer_path(arch_guid.0).map(|s| s.to_owned());
            let arch_path = self
                .cache
                .get_path_by_guid(arch_guid.0)
                .map(|s| s.to_owned());
            let combined = self.create_combined_candidate(outer.as_deref(), arch_path.as_deref());
            combined.or(outer)
        } else if actor_net_guid.is_valid() {
            self.cache
                .get_path_by_guid(actor_net_guid.0)
                .map(|s| s.to_owned())
        } else {
            None
        };

        // Archetype path from channel state.
        let archetype_path = self
            .channel_state
            .archetypes
            .get(&channel_index)
            .and_then(|g| self.cache.get_path_by_guid(g.0))
            .map(|s| s.to_owned());

        self.records.actors.push(ActorRecord {
            time_ms: self.time_ms,
            packet_id: self.packet_id,
            channel_index,
            actor_net_guid: actor_net_guid.0,
            event: "close",
            class_path,
            archetype_path,
            spawn_x: None,
            spawn_y: None,
            spawn_z: None,
            spawn_pitch: None,
            spawn_yaw: None,
            spawn_roll: None,
        });
    }

    fn on_content_block(
        &mut self,
        channel_index: u32,
        actor_net_guid: NetworkGuid,
        header: &ContentBlockHeader,
    ) -> u32 {
        self.current_channel = channel_index;
        self.current_actor_guid = actor_net_guid.0;
        // Actor blocks describe the actor itself and carry no subobject GUID.
        // For subobject blocks it identifies *which* subobject, which is the
        // only way to tell one of a character's inventory item slots from
        // another; merging them makes a player look like they hold one item.
        //
        // A subobject GUID of 0 is kept as `Some(0)`, not folded to `None`. The
        // reference reads it unconditionally (`ContentBlockFramer.cs:436-437`)
        // and branches on `!header.ObjectNetGuid.IsValid`
        // (`ContentBlockPathResolver.cs:100`), so it treats the invalid GUID as
        // reachable rather than impossible. `None` is not a safe stand-in for
        // it: downstream `None` means "actor block", the adapter substitutes
        // the actor GUID, and the block collapses onto the actor -- the merge
        // cf97ecf existed to undo.
        self.current_object_guid = if header.is_actor {
            None
        } else {
            Some(header.object_net_guid.0)
        };
        self.current_group_path = self.resolve_group_path(channel_index, actor_net_guid.0, header);
        self.stats.content_blocks += 1;

        if header.has_rep_layout {
            0
        } else {
            self.resolve_function_count(header, channel_index)
        }
    }

    fn on_deleted_block(
        &mut self,
        _channel_index: u32,
        _actor_net_guid: NetworkGuid,
        _header: &ContentBlockHeader,
    ) {
        self.stats.content_blocks += 1;
    }

    /// Attach the resolved group path to a stream failure.
    ///
    /// The replication layer knows the bit offsets but not the names; this is the
    /// only place both are available, and the group path is what identifies the
    /// class to investigate. Note `function_count`: zero names an unresolved
    /// group, while a wrong non-zero count can still select the wrong handle
    /// width. Counts 1 and 2 both use the parser's required minimum of 2 and are
    /// therefore not distinguishable from this diagnostic alone.
    fn on_stream_failure(&mut self, failure: StreamFailure) {
        self.channel_state.push_stream_failure(format!(
            "{:?} actor={} bits={} function_count={} consumed={} skipped={} group={}",
            failure.kind,
            failure.actor_net_guid.0,
            failure.bit_count,
            failure.function_count,
            failure.consumed_bits,
            failure.remaining_bits,
            self.current_group_path,
        ));
    }
}

// ?? Free helper functions ????????????????????????????????????????????????????

/// Extract the class name from an archetype path by taking the leaf and
/// stripping a `Default__` prefix if present.
///
/// Example: `/Game/Characters/AggroBot/AggroBot_PC.Default__AggroBot_PC_C`
/// ??`AggroBot_PC_C`.
fn extract_class_name_from_archetype(archetype_path: &str) -> Option<&str> {
    if archetype_path.is_empty() {
        return None;
    }
    let leaf_start = archetype_path.rfind(['/', '.', ':']).map_or(0, |i| i + 1);
    let leaf = &archetype_path[leaf_start..];
    if leaf.is_empty() {
        return None;
    }
    Some(leaf.strip_prefix("Default__").unwrap_or(leaf))
}

/// Check whether `path` already ends with `.{class_name}` or `:{class_name}`.
fn ends_with_class_name(path: &str, class_name: &str) -> bool {
    let sep_index = path.len().wrapping_sub(class_name.len() + 1);
    if sep_index >= path.len() {
        return false;
    }
    let sep = path.as_bytes()[sep_index];
    (sep == b'.' || sep == b':') && path[sep_index + 1..] == *class_name
}

/// Check whether a path is a "Class Default Object" path (leaf starts with
/// `Default__`). Mirrors `ReplayPath.IsClassDefaultObjectPath`.
fn is_class_default_object_path(path: &str) -> bool {
    let leaf_start = path.rfind(['/', '.', ':']).map_or(0, |i| i + 1);
    path[leaf_start..].starts_with("Default__")
}

/// Look up a known subobject class path by the leaf name of the object path.
///
/// Some subobjects use "stably named" identifiers (no class_net_guid). The
/// replay only tells us the object name; this table provides the class path.
fn resolve_known_subobject_class_path(object_path: &str) -> Option<&'static str> {
    let leaf_start = object_path.rfind(['/', '.', ':']).map_or(0, |i| i + 1);
    let leaf = &object_path[leaf_start..];
    KNOWN_SUBOBJECT_CLASS_PATHS
        .iter()
        .find(|(name, _)| *name == leaf)
        .map(|(_, class_path)| *class_path)
}

/// Decode a leaf field from a CombatRoundReports array using the known
/// handle?뭪ype mapping.
///
/// Returns (value_i64, value_f64, value_bool, value_str). All None if the
/// handle is not recognized or decoding fails.
///
/// Handle?뭪ype mapping derived from `CombatRoundReportsDecoder`:
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
) -> (Option<i64>, Option<f64>, Option<bool>, Option<String>) {
    use vrf_decode::{DecodeError, FieldType, decode_field};

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

    match decode_field(field_type, raw, bit_count) {
        Ok(vrf_decode::DecodedValue::I64(v)) => (Some(v), None, None, None),
        Ok(vrf_decode::DecodedValue::F64(v)) => (None, Some(v), None, None),
        Ok(vrf_decode::DecodedValue::Bool(v)) => (None, None, Some(v), None),
        Ok(vrf_decode::DecodedValue::Str(v)) => (None, None, None, Some(v)),
        Err(DecodeError::RawOrSkip) | Err(_) => (None, None, None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run one content block through the sink and report the subobject GUID it
    /// recorded for the fields that would follow.
    fn object_guid_for(is_actor: bool, object_net_guid: u32) -> Option<u32> {
        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);

        let header = ContentBlockHeader {
            // RepLayout, so the block needs no ClassNetCache function count.
            has_rep_layout: true,
            is_actor,
            object_net_guid: NetworkGuid(object_net_guid),
            ..ContentBlockHeader::default()
        };
        sink.on_content_block(7, NetworkGuid(1234), &header);
        sink.current_object_guid
    }

    /// A subobject block whose object GUID is 0 must record `Some(0)`.
    ///
    /// The reference reads the field unconditionally
    /// (`ContentBlockFramer.cs:436-437`) and then branches on
    /// `!header.ObjectNetGuid.IsValid` in `ContentBlockPathResolver.cs:100`,
    /// so it treats an invalid object GUID as a state that occurs rather than
    /// one that cannot. Folding it to `None` here is not a no-op: `None` means
    /// "actor block, no subobject at all", the adapter substitutes the actor
    /// GUID for it, and every such block collapses back onto the actor -- the
    /// exact merge cf97ecf existed to undo. `FieldRecord::object_net_guid`
    /// documents the same distinction ("Kept distinct from `Some(0)`").
    #[test]
    fn a_subobject_block_keeps_a_zero_object_guid_distinct_from_none() {
        assert_eq!(
            object_guid_for(false, 0),
            Some(0),
            "zero is the invalid-GUID sentinel, not the absence of a subobject"
        );
    }

    /// The two cases that must keep working: an actor block carries no
    /// subobject GUID at all, and a real subobject GUID passes through.
    #[test]
    fn an_actor_block_has_no_object_guid_and_subobjects_keep_theirs() {
        assert_eq!(object_guid_for(true, 0), None, "actor block");
        assert_eq!(
            object_guid_for(true, 99),
            None,
            "an actor block ignores the GUID"
        );
        assert_eq!(object_guid_for(false, 99), Some(99), "subobject block");
    }
}
