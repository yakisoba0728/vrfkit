//! The `vrf-net` callbacks: what the sink does with each decoded event.
//!
//! `FieldSink` receives replicated properties and RPCs; `ReplicationSink`
//! receives actor lifecycle, content-block framing and the two failure paths.
//! Everything these produce goes through `ExportSink::push_field`, so the nine
//! block-context columns are stamped in exactly one place.

use std::sync::Arc;

use vrf_bitio::BitReader;
use vrf_decode::apply_overlay_with_handle;
use vrf_export::{ActorRecord, MovementRecord, UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME};
use vrf_net::content::ContentBlockHeader;
use vrf_net::field::FieldSink;
use vrf_net::pipeline::{ActorChannelState, ReplicationSink, StreamFailure};
use vrf_net::types::NetworkGuid;

use super::paths::{channel_archetype, set_channel_archetype};
use super::rpc::copy_raw_bits;
use super::{ExportSink, FieldValues, TABLE};

/// The RPC whose payload is a movement batch rather than a parameter list.
const MOVEMENT_RPC: &str = "ReplaysClientReceiveRemoteCharacterUpdatesSingleArrayNoAutonomous";

impl ExportSink<'_> {
    /// Resolve a field or function name from the current block's group.
    ///
    /// Interned: 429,637 property rows and 342,735 RPC rows on the reference
    /// replay each used to clone the group's `String` name.
    fn resolve_field_name(&mut self, handle: u32) -> Option<Arc<str>> {
        // Destructured so the immutable borrow of `cache` that produces the
        // name and the mutable borrow of `channel_state` that pools it are
        // seen as the disjoint fields they are.
        let Self {
            cache,
            channel_state,
            current_group_path,
            ..
        } = self;
        let group = cache.get_group_by_path(current_group_path)?;
        let name = group.get_field(handle)?.name.as_str();
        Some(channel_state.names.intern(name))
    }
}

impl FieldSink for ExportSink<'_> {
    fn on_field(&mut self, handle: u32, bit_count: u32, reader: BitReader<'_>) {
        let field_name = self.resolve_field_name(handle);
        let raw_bits = copy_raw_bits(reader, bit_count);

        // Additive pass 1: a known DynamicArray is flattened into one row per
        // leaf. The parent row with the whole payload is still emitted below.
        if self.is_known_array_field(field_name.as_deref()) {
            if let Some(ref raw) = raw_bits {
                self.emit_flattened_array(field_name.as_deref(), raw, bit_count);
            }
        }

        // Additive pass 2: a struct blob with a dedicated decoder
        // (RoundResults, TeamEconomy, RoundInfos). Its sub-fields are extra
        // rows; the raw_bits parent row is still emitted below.
        if self.is_struct_blob_field(field_name.as_deref()) {
            // The `Arc` is cloned, not the string: `decode_struct_blob` takes
            // `&mut self` and the name would otherwise still be borrowed from
            // the local it lives in.
            if let (Some(raw), Some(name)) = (raw_bits.as_deref(), field_name.clone()) {
                self.decode_struct_blob(&name, raw, bit_count);
            }
        }

        // Apply the type overlay: decode raw_bits into a typed value if possible.
        let (value_i64, value_f64, value_bool, value_str) = match apply_overlay_with_handle(
            &TABLE,
            &self.current_group_path,
            field_name.as_deref(),
            handle,
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

        self.push_field(FieldValues {
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

        if field_name.as_deref() == Some(MOVEMENT_RPC) && bit_count > 0 {
            self.decode_movement_rpc(reader);
            // Raw bits are deliberately not stored for movement RPCs: the
            // payload is already in movement.parquet, row for row, and keeping
            // it here as well would add a blob column entry per batch.
            self.push_field(FieldValues {
                handle,
                field_name,
                bit_count,
                ..FieldValues::default()
            });
        } else if bit_count > 0 {
            // Try to parse RPC parameters as a RepLayout field stream.
            // The parameter group path is `<ClassPath>:<FunctionName>` where
            // ClassPath = current_group_path minus `_ClassNetCache` suffix.
            //
            // We clone the reader before attempting the parse so we can fall
            // back to raw_bits emission if parsing yields nothing.
            let fallback_reader = reader.clone();
            let parsed = self.try_parse_rpc_params(handle, reader, field_name.as_deref());
            if !parsed {
                // Fallback: emit raw bits as a single row (no param group found).
                self.push_field(FieldValues {
                    handle,
                    field_name,
                    bit_count,
                    raw_bits: copy_raw_bits(fallback_reader, bit_count),
                    ..FieldValues::default()
                });
            }
        } else {
            // Zero-bit RPC -- just emit a marker row.
            self.push_field(FieldValues {
                handle,
                field_name,
                ..FieldValues::default()
            });
        }
        self.stats.rpcs_emitted += 1;
    }
}

impl ExportSink<'_> {
    /// Decode a movement RPC payload into `movement.parquet` rows.
    fn decode_movement_rpc(&mut self, reader: BitReader<'_>) {
        let mut rpc_reader = reader;
        let time_ms = self.time_ms;
        let packet_id = self.packet_id;
        let movement = &mut self.records.movement;
        let result = vrf_movement::decode_movement_rpc(&mut rpc_reader, |mv| {
            movement.push(MovementRecord {
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
                timestamp: mv.timestamp,
                movement_state: mv.movement_state,
                // mv.mode_flags is intentionally not carried: the decoder
                // assigns it from the same local as movement_state, so it
                // can never hold a different value.
                move_type: mv.move_type,
            });
        });
        self.stats.record_movement_decode(result.as_ref());
    }

    /// Resolve the class path an actor channel should be labelled with.
    ///
    /// Shared by open and close so the two cannot drift: a channel that opened
    /// as one class and closed as another would be a join key that silently
    /// does not join.
    fn actor_class_path(&self, archetype: Option<NetworkGuid>) -> Option<String> {
        let archetype = archetype.filter(|g| g.is_valid())?;
        let outer = self.cache.get_outer_path(archetype.0).map(str::to_owned);
        let arch_path = self.cache.get_path_by_guid(archetype.0).map(str::to_owned);
        let combined = self.create_combined_candidate(outer.as_deref(), arch_path.as_deref());
        combined.or(outer)
    }
}

impl ReplicationSink for ExportSink<'_> {
    fn on_actor_open(&mut self, state: &ActorChannelState) {
        self.stats.actor_opens += 1;
        // Track archetype GUID per channel so ClassNetCache path resolution can
        // walk archetype -> outer path -> class name.
        if state.archetype_net_guid.is_valid() {
            set_channel_archetype(
                self.channel_state,
                state.channel_index,
                state.archetype_net_guid,
            );
        }

        // Resolve class_path from the archetype GUID's outer path.
        //
        // A static actor has no archetype: NewActorSerializer.cs:29 returns
        // before reading the spawn block for anything that is not dynamic, so
        // the reference leaves both ReplicationClassPath and ArchetypePath
        // null. This used to fall back to the actor GUID's own path, on the
        // stated premise that "for static actors the actor GUID path itself is
        // the class". It is not -- that path is the level's instance name.
        // 27 opens on 02d4d478 shipped `Ascent_C_0`, `AresWorldSettings`,
        // `WindowShieldA1` and the like as replication class paths.
        //
        // Nothing is lost by dropping it: all 27 paths are byte-identical to
        // the `path` column net_guids.parquet already carries for the same
        // GUID, so a consumer that wants the instance name can join for it.
        let class_path = self.actor_class_path(Some(state.archetype_net_guid));

        // Resolve archetype_path from the archetype GUID.
        let archetype_path = if state.archetype_net_guid.is_valid() {
            self.cache
                .get_path_by_guid(state.archetype_net_guid.0)
                .map(str::to_owned)
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
        let archetype = channel_archetype(self.channel_state, channel_index);
        let class_path = match self.actor_class_path(archetype) {
            Some(path) => Some(path),
            // No archetype on this channel: a close still names the actor, and
            // its own GUID path is the only label left.
            None if archetype.is_none() && actor_net_guid.is_valid() => self
                .cache
                .get_path_by_guid(actor_net_guid.0)
                .map(str::to_owned),
            None => None,
        };

        // Archetype path from channel state.
        let archetype_path =
            archetype.and_then(|g| self.cache.get_path_by_guid(g.0).map(str::to_owned));

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
        self.stats.content_blocks += 1;
        self.resolve_block(channel_index, actor_net_guid, header)
    }

    fn on_deleted_block(
        &mut self,
        _channel_index: u32,
        _actor_net_guid: NetworkGuid,
        _header: &ContentBlockHeader,
    ) {
        self.stats.content_blocks += 1;
    }

    fn on_unresolved_class_net_cache_payload(&mut self, failure: StreamFailure, payload: &[u8]) {
        let field_name = self
            .channel_state
            .names
            .intern(UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME);
        self.push_field(FieldValues {
            handle: u32::MAX,
            field_name: Some(field_name),
            bit_count: failure.bit_count,
            raw_bits: Some(payload.to_vec()),
            ..FieldValues::default()
        });
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
        let line = format!(
            "{:?} actor={} bits={} function_count={} consumed={} skipped={} group={}",
            failure.kind,
            failure.actor_net_guid.0,
            failure.bit_count,
            failure.function_count,
            failure.consumed_bits,
            failure.remaining_bits,
            self.current_group_path,
        );
        self.channel_state.push_stream_failure(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::{ChannelState, RecordBuffers};
    use vrf_schema::NetGuidCache;

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

    /// A whole unresolved block is one preservation row, not an RPC or a set
    /// of invented fields. The reserved field name is its sole discriminator.
    #[test]
    fn unresolved_class_net_cache_payload_emits_one_distinguished_row() {
        let mut cache = NetGuidCache::new();
        cache.set_net_guid_path(144, "AbilitiesAndBuffsComponent".to_owned(), None);
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
        sink.time_ms = 1234;
        sink.packet_id = 56;

        let header = ContentBlockHeader {
            has_rep_layout: false,
            is_actor: false,
            object_net_guid: NetworkGuid(144),
            is_stably_named: true,
            ..ContentBlockHeader::default()
        };
        let function_count = sink.on_content_block(7, NetworkGuid(89), &header);
        assert_eq!(function_count, 0);

        let failure = StreamFailure {
            kind: vrf_net::pipeline::StreamKind::Rpc,
            actor_net_guid: NetworkGuid(89),
            bit_count: 7,
            function_count: 0,
            consumed_bits: 0,
            remaining_bits: 7,
        };
        sink.on_unresolved_class_net_cache_payload(failure, &[0x66]);

        assert_eq!(sink.records.fields.len(), 1);
        let row = &sink.records.fields[0];
        assert_eq!(row.time_ms, 1234);
        assert_eq!(row.packet_id, 56);
        assert_eq!(row.channel_index, 7);
        assert_eq!(row.actor_net_guid, 89);
        assert_eq!(row.object_net_guid, Some(144));
        assert_eq!(&*row.group_path, "AbilitiesAndBuffsComponent");
        assert_eq!(row.handle, u32::MAX);
        assert_eq!(
            row.field_name.as_deref(),
            Some(UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME)
        );
        assert_eq!(row.bit_count, 7);
        assert_eq!(row.raw_bits.as_deref(), Some(&[0x66][..]));
        assert!(row.value_i64.is_none());
        assert!(row.value_f64.is_none());
        assert!(row.value_bool.is_none());
        assert!(row.value_str.is_none());
        assert_eq!(sink.stats.fields_emitted, 0);
        assert_eq!(sink.stats.rpcs_emitted, 0);
        assert_eq!(sink.stats.overlay.decoded_ok, 0);
        assert_eq!(sink.stats.overlay.decoded_err, 0);
        assert_eq!(sink.stats.overlay.raw_or_skip, 0);
        assert_eq!(sink.stats.overlay.not_in_table, 0);
        assert_eq!(sink.stats.overlay.no_field_name, 0);
    }
}
