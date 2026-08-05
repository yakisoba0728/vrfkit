//! Sink implementation connecting `vrf-net` events to `vrf-export` records.
//!
//! # Design: no skipping
//!
//! Every field/RPC is emitted, even if we cannot resolve the group path or field
//! name. In that case we emit `group_path = "<unknown:{guid}>"` and
//! `field_name = None`. When an unresolved ClassNetCache function table makes
//! the block unsplittable, its whole payload is emitted as one explicitly
//! marked preservation row instead of fabricated fields. This keeps the
//! Parquet output a **lossless** representation of the stream.
//!
//! # Layout
//!
//! - [`intern`] -- the `Arc<str>` pool behind the two name columns.
//! - [`paths`] -- content-block group-path resolution and its memo.
//! - [`rpc`] -- the ClassNetCache RPC parameter walker.
//! - [`blobs`] -- the struct-blob and flattened-array decoders.
//! - [`stream`] -- the `vrf-net` trait impls that drive all of the above.
//!
//! This module holds what those five share: the sink, the per-packet record
//! buffers, and the state that must outlive a packet.
//!
//! # What the sink costs
//!
//! `vrfkit validate` runs this whole path and writes no file, so it measures
//! the sink alone. Five interleaved runs against the pre-rewrite binary on
//! 02d4d478 (46 MB, 530,401 packets, 608,020 content blocks): median 1.395 s
//! -> 1.062 s. That 333 ms is the group-path memo in [`paths`] plus the name
//! pool in [`intern`]; nothing else in the decode path changed.
//!
//! Peak working set for `validate` moved 64.5 MB -> 65.0 MB. The memo and the
//! pool are the only new state and together they are under a megabyte -- see
//! the measured entry counts in those two modules.

mod blobs;
mod intern;
mod paths;
mod rpc;
mod stream;

use std::sync::Arc;

use vrf_decode::{
    ArrayDecodeStats, GroupHashState, OVERLAY_HANDLE_TABLE, OVERLAY_TABLE, OverlayStats,
    OverlayTable, group_hash_state,
};
use vrf_export::{ActorRecord, FieldRecord, MovementRecord};
use vrf_net::net_guid::GuidPathSink;
use vrf_net::types::NetworkGuid;
use vrf_schema::{FxHashMap, NetGuidCache};

use intern::NameInterner;
use paths::BlockPathMemo;
use rpc::RpcParamGroupMemo;

/// Static overlay table built from C# descriptors.
static TABLE: OverlayTable = OverlayTable::with_handles(&OVERLAY_TABLE, &OVERLAY_HANDLE_TABLE);

/// How many stream-failure lines to retain. See [`ChannelState::stream_failures`].
const MAX_STREAM_FAILURE_RECORDS: usize = 32;

/// One BombPlayerState actor's identity, accumulated from its `Subject` and
/// `SpawnedCharacter` fields for the manifest `players` array.
///
/// `Subject` is the account UUID (a `String`); `SpawnedCharacter` is the
/// character actor NetGUID, which equals `movement.character_net_guid`. Together
/// they let every actor-keyed table join to a stable account identity -- the
/// one piece `playerLoadouts`' `characterId` cannot give when two players pick
/// the same agent.
#[derive(Debug, Clone, Default)]
pub struct PlayerIdentity {
    pub subject: Option<String>,
    pub character_net_guid: Option<u32>,
}

/// Persistent per-channel state that must survive across packets and chunks.
///
/// The replay pipeline creates a fresh `ExportSink` for every packet (to
/// satisfy borrow-checker constraints around `NetGuidCache` mutability). This
/// struct holds the state that *must* persist across those boundaries -- the
/// archetype GUID assigned when a channel is opened, which is needed later to
/// resolve ClassNetCache export groups when content blocks arrive, plus the two
/// memos and the name pool, none of which would ever warm up if they were
/// rebuilt half a million times.
#[derive(Debug, Clone, Default)]
pub struct ChannelState {
    /// channel_index -> archetype NetworkGuid.
    archetypes: FxHashMap<u32, NetworkGuid>,
    /// See [`RpcParamGroupMemo`].
    rpc_param_groups: RpcParamGroupMemo,
    /// See [`BlockPathMemo`].
    block_paths: BlockPathMemo,
    /// See [`NameInterner`].
    names: NameInterner,
    /// Bumped whenever an input to group-path resolution that
    /// `NetGuidCache::schema_generation` does NOT cover changes: the cache's
    /// GUID -> path and GUID -> outer maps, and this struct's archetype map.
    /// [`BlockPathMemo`] stamps itself with this and the schema generation
    /// together; between them they cover every input the resolution reads.
    resolution_generation: u64,
    /// One line per content block that framed and decoded but whose inner stream
    /// could not be walked.
    ///
    /// Lives here rather than on the sink because the sink is rebuilt for every
    /// packet, so anything recorded on it is lost immediately. Capped: a build
    /// whose transform is wrong would fail on essentially every block, and the
    /// first few dozen say everything the later million would.
    stream_failures: Vec<String>,
    /// BombPlayerState identity capture for the manifest `players` array. Keyed
    /// by the PlayerState actor's NetGUID; filled in `on_field` as `Subject`
    /// and `SpawnedCharacter` arrive, drained once at the end of the replay.
    players: FxHashMap<u32, PlayerIdentity>,
}

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

    /// Captured player identities (PlayerState actor NetGUID -> identity), for
    /// the manifest `players` array.
    #[must_use]
    pub fn players(&self) -> &FxHashMap<u32, PlayerIdentity> {
        &self.players
    }

    /// Declare that something group-path resolution reads has changed.
    ///
    /// Call sites are deliberately few -- the GUID registration in
    /// [`GuidPathSink::register_path`] and the archetype assignment in
    /// `on_actor_open` -- because every one of them is a place the memo could
    /// go stale. Adding a resolution input without a call here is silent byte
    /// movement, not a test failure.
    fn note_resolution_input_changed(&mut self) {
        self.resolution_generation = self.resolution_generation.wrapping_add(1);
    }
}

/// Counters the driver aggregates across packets.
#[derive(Debug, Clone, Default)]
pub struct ExportStats {
    pub fields_emitted: u64,
    pub rpcs_emitted: u64,
    pub actor_opens: u64,
    pub actor_closes: u64,
    pub content_blocks: u64,
    pub overlay: OverlayStats,
    pub array: ArrayDecodeStats,
    /// EffectContainer blobs turned into a `value_str` JSON array.
    ///
    /// Counted because nothing else moves when this decoder works. The overlay
    /// buckets are filled before the additive pass runs, so a successful effect
    /// decode leaves `decoded_ok`, `not_in_table` and the rest exactly where
    /// they were, and the only trace is a larger `fields.parquet`. A silent
    /// improvement is the same failure as a silent loss: the next session
    /// diffs two summaries, sees every counter identical, and concludes
    /// nothing changed. Failures already land in `overlay.decoded_err`.
    pub effect_blobs_decoded: u64,

    /// Struct-blob (`RoundResults`, `TeamEconomy`, `RoundInfos`) parent rows
    /// whose dedicated decoder produced elements.
    pub struct_blobs_decoded: u64,

    /// Struct-blob decodes that returned an error.
    ///
    /// These used to be `let Ok(..) else { return false }` -- discarded with no
    /// counter and no line. That is how build 13.02 moving `RoundResults` from
    /// handle 93 to 81 read as a completely clean export: every counter on the
    /// summary was identical to a good run and the match score simply was not
    /// in the Parquet. The decoders are additive, so a failure still costs no
    /// rows and no bits; it must not also cost the operator the knowledge that
    /// it happened.
    pub struct_blobs_failed: u64,

    /// The first failure verbatim, so the summary can name the member and the
    /// handle instead of only admitting that something went wrong.
    pub struct_blob_first_error: Option<String>,

    /// Movement-decode problems: per-update soft errors
    /// (`RpcDecodeResult.error_count`) plus hard `Err` failures, summed.
    /// `decode_movement_rpc` used to drop its `Result` wholesale, so a build
    /// that changed the movement section format would silently shorten
    /// `movement.parquet` with every other counter reading clean.
    pub movement_rpc_errors: u64,

    /// The first movement-decode problem verbatim, for the summary to name.
    pub movement_first_error: Option<String>,

    /// RPC payloads whose RepLayout parameter loop broke on a malformed read
    /// before the terminating zero handle.
    ///
    /// `try_parse_rpc_params` keeps whatever rows it already parsed and returns
    /// `true`, so a truncated RPC reads as success: fewer parameter rows than
    /// declared, no other counter moves, and `rpcs_emitted` ticks up exactly as
    /// it does for a clean parse. This is the one signal that distinguishes
    /// "completed" from "abandoned mid-stream". Zero on valid replays; a non-zero
    /// value means the wire declared more parameters than the bits could carry.
    pub truncated_rpcs: u64,
}

impl ExportStats {
    /// Record a movement-RPC decode outcome so a silent failure cannot read
    /// as success. Soft per-update errors (caught and counted by the decoder
    /// in `RpcDecodeResult.error_count`) and hard `Err`s both land here; the
    /// first is kept verbatim for the summary.
    pub fn record_movement_decode(
        &mut self,
        result: Result<&vrf_movement::RpcDecodeResult, &vrf_movement::MovementError>,
    ) {
        match result {
            Ok(r) if r.error_count > 0 => {
                self.movement_rpc_errors = self
                    .movement_rpc_errors
                    .saturating_add(u64::from(r.error_count));
                self.movement_first_error.get_or_insert(format!(
                    "{} movement update(s) skipped mid-decode",
                    r.error_count
                ));
            }
            Ok(_) => {}
            Err(e) => {
                self.movement_rpc_errors = self.movement_rpc_errors.saturating_add(1);
                self.movement_first_error
                    .get_or_insert_with(|| e.to_string());
            }
        }
    }
}

#[cfg(test)]
mod movement_stats_tests {
    use super::ExportStats;
    use vrf_movement::{MovementError, RpcDecodeResult};

    fn ok(
        total_moves: u32,
        update_count: u32,
        error_count: u32,
    ) -> Result<RpcDecodeResult, MovementError> {
        Ok(RpcDecodeResult {
            total_moves,
            update_count,
            error_count,
        })
    }

    #[test]
    fn a_clean_decode_records_nothing() {
        let mut s = ExportStats::default();
        s.record_movement_decode(ok(5, 1, 0).as_ref());
        assert_eq!(s.movement_rpc_errors, 0);
        assert!(s.movement_first_error.is_none());
    }

    #[test]
    fn soft_errors_are_counted_and_first_error_is_kept() {
        let mut s = ExportStats::default();
        s.record_movement_decode(ok(2, 5, 3).as_ref());
        assert_eq!(s.movement_rpc_errors, 3);
        assert!(s.movement_first_error.is_some());
        // A later hard failure adds to the count but must not overwrite the
        // first error.
        let first = s.movement_first_error.clone();
        s.record_movement_decode(Err(MovementError::InvalidMagic(0x00)).as_ref());
        assert_eq!(s.movement_rpc_errors, 4);
        assert_eq!(s.movement_first_error, first);
    }

    #[test]
    fn a_hard_error_records_its_display() {
        let mut s = ExportStats::default();
        s.record_movement_decode(Err(MovementError::ErrorSentinel).as_ref());
        assert_eq!(s.movement_rpc_errors, 1);
        let msg = s.movement_first_error.expect("first error recorded");
        assert!(msg.contains("sentinel"), "got: {msg}");
    }
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
/// export bunches that declare new GUID->path mappings inline).
pub struct ExportSink<'a> {
    /// Schema cache -- mutable because in-packet path registrations need it.
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

    // -- per-content-block context (set by on_content_block) ----------------
    current_channel: u32,
    current_actor_guid: u32,
    /// Subobject GUID of the block being walked; `None` for actor blocks.
    current_object_guid: Option<u32>,
    /// Interned, so a block's rows share one allocation instead of each
    /// carrying its own copy of the path. See [`intern`].
    current_group_path: Arc<str>,
    /// The half-finished overlay key hash for [`current_group_path`](Self::current_group_path).
    ///
    /// A content block probes the overlay ~2M times per replay with the same
    /// group path for every field in it, and the group path is long
    /// (`/Game/Characters/.../AggroBot_PC.AggroBot_PC_C`) while the field names
    /// are short. Caching the group-path fold and finishing only the field-name
    /// half per probe is the saving. Refreshed by [`set_current_group_path`].
    ///
    /// A stale value is a performance and typing regression, not a wrong-value
    /// bug: the slot tag and the full string equality check still reject a
    /// mismatching key, so the field degrades to `raw_bits` instead of decoding
    /// to a wrong type.
    current_group_hash: GroupHashState,
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
        let current_group_path = empty_group_path();
        let current_group_hash = group_hash_state(&current_group_path);
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
            current_group_path,
            current_group_hash,
        }
    }

    /// Set `current_group_path` and refresh its cached overlay hash in one step.
    ///
    /// Every assignment to [`current_group_path`](Self::current_group_path) must
    /// go through here, or the cached [`current_group_hash`](Self::current_group_hash)
    /// goes stale. The three sites are all in [`paths`]: the memo-hit return, the
    /// fresh resolution, and the bare-instance-name ClassNetCache replacement.
    /// The hash is a common-subexpression optimisation: a stale value turns
    /// overlay hits into misses (fields degrade to `raw_bits`), never a wrong
    /// value -- the slot tag plus full string equality still guard every hit.
    fn set_current_group_path(&mut self, path: Arc<str>) {
        self.current_group_hash = group_hash_state(&path);
        self.current_group_path = path;
    }

    /// Push one field row, stamped with the current block context.
    ///
    /// Every `FieldRecord` this crate produces is built here. Nine of the
    /// fourteen columns are block context that no call site should be able to
    /// get wrong, and before this they were spelled out at seven of them.
    fn push_field(&mut self, row: FieldValues) {
        self.records.fields.push(FieldRecord {
            time_ms: self.time_ms,
            packet_id: self.packet_id,
            channel_index: self.current_channel,
            actor_net_guid: self.current_actor_guid,
            object_net_guid: self.current_object_guid,
            // A refcount bump, not a copy: this is the 1.25-million-row column
            // the interning exists for.
            group_path: Arc::clone(&self.current_group_path),
            handle: row.handle,
            field_name: row.field_name,
            bit_count: row.bit_count,
            raw_bits: row.raw_bits,
            value_i64: row.value_i64,
            value_f64: row.value_f64,
            value_bool: row.value_bool,
            value_str: row.value_str,
        });
    }
}

/// The part of a field row that is not block context. See
/// [`ExportSink::push_field`].
#[derive(Debug, Default)]
struct FieldValues {
    handle: u32,
    field_name: Option<Arc<str>>,
    bit_count: u32,
    raw_bits: Option<Vec<u8>>,
    value_i64: Option<i64>,
    value_f64: Option<f64>,
    value_bool: Option<bool>,
    value_str: Option<String>,
}

/// The group path a sink starts with, before any content block has been seen.
///
/// A fresh `Arc` per sink would be 530,401 allocations of an empty string over a
/// replay, so this hands out clones of one process-wide value. It is only ever
/// observable if a field arrives before its content block, which the framer
/// does not do.
fn empty_group_path() -> Arc<str> {
    use std::sync::OnceLock;
    static EMPTY: OnceLock<Arc<str>> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::from("")))
}

impl GuidPathSink for ExportSink<'_> {
    /// Record a GUID -> path mapping the wire declared inline.
    ///
    /// The write is skipped when it would change nothing. That is not only an
    /// allocation saving: [`BlockPathMemo`] keys on a generation counter that
    /// must move whenever a resolution input moves, and the cache's GUID -> path
    /// and GUID -> outer maps are two of those inputs. Deciding "did this
    /// change" here is what lets the memo stay exactly equivalent to
    /// recomputing, instead of being invalidated by every re-declaration of a
    /// mapping the cache already held.
    ///
    /// Both halves of the state are compared, not just the path. A repeat call
    /// carrying the same path but an invalid outer *removes* the outer in
    /// `set_net_guid_path`; skipping that on a path match alone would preserve a
    /// stale outer, which changes resolved group paths and the `outer_net_guid`
    /// column of `net_guids.parquet`.
    fn register_path(&mut self, guid: u32, path: &str, outer_guid: NetworkGuid) {
        let outer = if outer_guid.0 != 0 {
            Some(vrf_schema::NetworkGuid(outer_guid.0))
        } else {
            None
        };
        if self.cache.get_path_by_guid(guid) == Some(path)
            && self.cache.get_outer_guid(guid) == outer
        {
            return;
        }
        self.cache.set_net_guid_path(guid, path.to_string(), outer);
        self.channel_state.note_resolution_input_changed();
    }

    fn path_for_guid(&self, guid: u32) -> Option<&str> {
        self.cache.get_path_by_guid(guid)
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
