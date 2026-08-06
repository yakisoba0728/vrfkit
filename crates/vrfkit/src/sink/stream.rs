//! The `vrf-net` callbacks: what the sink does with each decoded event.
//!
//! `FieldSink` receives replicated properties and RPCs; `ReplicationSink`
//! receives actor lifecycle, content-block framing and the two failure paths.
//! Everything these produce goes through `ExportSink::push_field`, so the nine
//! block-context columns are stamped in exactly one place.

use std::sync::Arc;

use smallvec::SmallVec;
use vrf_bitio::BitReader;
use vrf_decode::apply_overlay_with_checksum;
use vrf_decode::cnc::decode_cnc_payload;
use vrf_export::{ActorRecord, MovementRecord, UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME};
use vrf_net::content::ContentBlockHeader;
use vrf_net::field::FieldSink;
use vrf_net::pipeline::{ActorChannelState, ReplicationSink, StreamFailure};
use vrf_net::types::NetworkGuid;

use super::intern::put;
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
        self.resolve_field_name_and_checksum(handle).0
    }

    /// [`Self::resolve_field_name`] plus the handle's `compatible_checksum`.
    ///
    /// One schema walk yields both. The checksum feeds the overlay's last-resort
    /// lookup, and asking for it separately would double the cost of the hottest
    /// loop in the export.
    fn resolve_field_name_and_checksum(&mut self, handle: u32) -> (Option<Arc<str>>, Option<u32>) {
        // Destructured so the immutable borrow of `cache` that produces the
        // name and the mutable borrow of `channel_state` that pools it are
        // seen as the disjoint fields they are.
        let Self {
            cache,
            channel_state,
            current_group_path,
            ..
        } = self;
        // The replay's own export group names the handle when it can.
        if let Some(group) = cache.get_group_by_path(current_group_path) {
            if let Some(field) = group.get_field(handle) {
                return (
                    Some(channel_state.names.intern(field.name.as_str())),
                    Some(field.compatible_checksum),
                );
            }
        }
        // Some groups (e.g. `MagazineAmmo`) are declared without field names, so
        // a handle the wire leaves unnamed falls back to the overlay's handle
        // table -- without this the row keeps field_name None even though the
        // overlay resolved and typed it.
        let Some(name) = TABLE.lookup_handle(current_group_path, handle) else {
            return (None, None);
        };
        (Some(channel_state.names.intern(name)), None)
    }
}

impl FieldSink for ExportSink<'_> {
    fn on_field(&mut self, handle: u32, bit_count: u32, reader: BitReader<'_>) {
        let (field_name, field_checksum) = self.resolve_field_name_and_checksum(handle);
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

        // Additive pass 3: `MultiItemSlot.MultiContents` -- a dynamic array of
        // item actor references. Each decoded NetGUID is an extra row; the
        // raw_bits parent row is still emitted below.
        if self.is_multi_contents_field(field_name.as_deref()) {
            if let Some(raw) = raw_bits.as_deref() {
                self.emit_multi_contents(raw, bit_count);
            }
        }

        // Apply the type overlay: decode raw_bits into a typed value if possible.
        let (value_i64, value_f64, value_bool, value_str) = match apply_overlay_with_checksum(
            &TABLE,
            &self.current_group_path,
            self.current_group_hash,
            field_name.as_deref(),
            handle,
            field_checksum,
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

        self.record_player_identity(field_name.as_deref(), value_str.as_deref(), value_i64);

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

/// Bomb-mode PlayerState. Its `Subject` (account UUID, FString) and
/// `SpawnedCharacter` (character actor NetGUID == movement.character_net_guid)
/// are captured per actor into the manifest `players` array.
const BOMB_PLAYER_STATE: &str = "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C";

/// The ClassNetCache function_count for `AbilitiesAndBuffsComponent`.
///
/// This group's `_ClassNetCache` export is never declared in VALORANT replays,
/// so the handle width for the RPC stream is unknown at decode time. The value
/// was determined by brute-forcing fc 2-256 against 9,274 payloads from a
/// reference replay: fc=34 is the minimum that walks every payload cleanly
/// (9274/9274). See [`ExportSink::emit_brute_forced_cnc_rpcs`].
const ABILITIES_AND_BUFFS_FC: u32 = 34;

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

    /// Capture BombPlayerState identity for the manifest `players` array.
    /// `Subject` is the account UUID; `SpawnedCharacter` is the character actor
    /// NetGUID, equal to `movement.character_net_guid`. Together they let any
    /// actor-keyed table join to a stable account identity -- the link
    /// `playerLoadouts`' `characterId` cannot provide when two players share an
    /// agent.
    fn record_player_identity(
        &mut self,
        field_name: Option<&str>,
        subject: Option<&str>,
        character: Option<i64>,
    ) {
        if self.current_group_path.as_ref() != BOMB_PLAYER_STATE {
            return;
        }
        let Some(name) = field_name else {
            return;
        };
        let entry = self
            .channel_state
            .players
            .entry(self.current_actor_guid)
            .or_default();
        match name {
            "Subject" => {
                if let Some(s) = subject {
                    entry.subject = Some(s.to_owned());
                }
            }
            "SpawnedCharacter" => {
                if let Some(c) = character {
                    entry.character_net_guid = Some(c as u32);
                }
            }
            _ => {}
        }
    }

    /// Attempt to decode the ClassNetCache RPC stream for an unresolved
    /// `AbilitiesAndBuffsComponent` payload and emit one row per RPC.
    ///
    /// Gated on `AbilitiesAndBuffsComponent`, whose `_ClassNetCache` export
    /// group is never declared in VALORANT replays. The function_count was
    /// determined empirically by brute-forcing fc 2-256 across 9,274 payloads
    /// from a reference replay: fc=34 is the minimum that walks **every**
    /// payload cleanly, and each payload contains exactly one RPC at handle 1.
    /// The inner payload is not standard RepLayout `FunctionParameters`, but
    /// it is not opaque either: it is a deterministic flag bit followed by a
    /// little-endian `u32` stream (see `decode_abilities_and_buffs_inner`). It
    /// is the GAS state-sync stream, not one row per ability cast, so the RPC's
    /// raw bits are preserved as a row without further typed extraction.
    ///
    /// A per-payload brute-force (trying each fc independently) was rejected
    /// because simple payloads can walk cleanly under smaller fc values,
    /// producing garbage handles. Using a single constant fc avoids that: every
    /// payload gets the same handle width, and the 9274/9274 clean-walk rate
    /// confirms the fc is correct for this group. If a game update changes the
    /// function table, the walk will start failing and the preservation row
    /// will be the only record -- the failure is visible, not silent.
    ///
    /// Several adjacent fc values (34-65) produce the same 6-bit handle width
    /// for handle 1 and therefore identical walks. The constant is the minimum
    /// of that range.
    fn emit_brute_forced_cnc_rpcs(&mut self, payload: &[u8], bit_count: u32) {
        if !self
            .current_group_path
            .contains("AbilitiesAndBuffsComponent")
        {
            return;
        }

        let Some(rpcs) = decode_cnc_payload(payload, bit_count, ABILITIES_AND_BUFFS_FC) else {
            return;
        };

        let total_len = u64::from(bit_count);
        for rpc in &rpcs {
            // Extract the RPC's payload bits from the buffer. The brute-force
            // already validated that each payload fits, so the read cannot
            // fail on well-formed input; on a malformed tail the payload is
            // dropped (the preservation row still carries the full blob).
            let raw_bits = (|| {
                let mut reader = vrf_bitio::BitReader::with_bit_len(payload, total_len);
                reader.skip_bits(rpc.payload_offset).ok()?;
                let byte_count = (rpc.payload_bits as usize).div_ceil(8);
                let mut buf = SmallVec::with_capacity(byte_count);
                buf.resize(byte_count, 0u8);
                reader
                    .copy_bits_to(&mut buf, u64::from(rpc.payload_bits))
                    .ok()?;
                Some(buf)
            })();

            let field_name = self.channel_state.names.intern_fmt(|out| {
                put(out, format_args!("_cnc_h{}", rpc.handle));
            });

            self.push_field(FieldValues {
                handle: rpc.handle,
                field_name: Some(field_name),
                bit_count: rpc.payload_bits,
                raw_bits,
                ..FieldValues::default()
            });
            self.stats.fields_emitted += 1;
            self.stats.cnc_rpcs_emitted += 1;
        }
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
            raw_bits: Some(SmallVec::from_slice(payload)),
            ..FieldValues::default()
        });

        // Additive pass: brute-force the ClassNetCache function_count for
        // groups whose RPC stream is well-formed but whose export group is
        // never declared. The preservation row above stays regardless; each
        // decoded RPC is an extra row.
        self.emit_brute_forced_cnc_rpcs(payload, failure.bit_count);
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

    /// Encode `value` as Unreal's `IntPacked` into `bits`, LSB-first.
    /// Mirrors the reference wire encoder in `vrf-net`'s field tests.
    fn write_int_packed(bits: &mut Vec<bool>, mut value: u32) {
        loop {
            let mut next_byte = ((value & 0x7F) << 1) as u8;
            value >>= 7;
            if value != 0 {
                next_byte |= 1;
            }
            for i in 0..8 {
                bits.push((next_byte & (1 << i)) != 0);
            }
            if value == 0 {
                break;
            }
        }
    }

    /// Pack a LSB-first bit list into bytes.
    fn bits_to_bytes(bits: &[bool]) -> Vec<u8> {
        let byte_count = bits.len().div_ceil(8);
        let mut bytes = vec![0u8; byte_count];
        for (i, &bit) in bits.iter().enumerate() {
            if bit {
                bytes[i >> 3] |= 1 << (i & 7);
            }
        }
        bytes
    }

    /// A truncated RPC payload -- the first parameter declares more bits than
    /// the stream carries -- must bump `truncated_rpcs`. No parameter row lands
    /// (the break fires before the field push), so the caller's raw_bits
    /// fallback still fires; the counter is the only thing that distinguishes
    /// this from a payload that simply had no parameters.
    #[test]
    fn a_truncated_rpc_payload_increments_truncated_rpcs() {
        let mut bits = Vec::new();
        bits.push(false); // property checksum
        write_int_packed(&mut bits, 1); // encodedHandle = 1 -> handle 0
        write_int_packed(&mut bits, 100); // payload_bits = 100 (exceeds remaining)
        // No payload data follows: the walker breaks here.
        let data = bits_to_bytes(&bits);
        let reader = BitReader::with_bit_len(&data, bits.len() as u64);

        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);

        let emitted = sink.try_parse_rpc_params(7, reader, Some("SomeFunction"));
        assert!(!emitted, "no parameter rows are emitted before the break");
        assert_eq!(sink.stats.truncated_rpcs, 1);
    }

    /// A well-formed RPC payload -- one parameter then the zero-handle
    /// terminator -- must leave `truncated_rpcs` at zero. This is the
    /// byte-identical-output invariant on valid input.
    #[test]
    fn a_completed_rpc_payload_leaves_truncated_rpcs_at_zero() {
        let mut bits = Vec::new();
        bits.push(false); // property checksum
        write_int_packed(&mut bits, 1); // encodedHandle = 1 -> handle 0
        write_int_packed(&mut bits, 8); // payload_bits = 8
        bits.extend(std::iter::repeat_n(false, 8)); // 8 bits of payload data
        write_int_packed(&mut bits, 0); // terminator handle
        let data = bits_to_bytes(&bits);
        let reader = BitReader::with_bit_len(&data, bits.len() as u64);

        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);

        let emitted = sink.try_parse_rpc_params(7, reader, Some("SomeFunction"));
        assert!(emitted, "one parameter row is emitted");
        assert_eq!(sink.stats.truncated_rpcs, 0);
    }

    /// Write a SerializedInt value with a given max (same encoding as
    /// `vrf-bitio`'s `read_serialized_int`).
    fn write_serialized_int(bits: &mut Vec<bool>, value: u32, max: u32) {
        let mut written = 0u32;
        let mut mask = 1u32;
        while written.saturating_add(mask) < max {
            let bit = (value & mask) != 0;
            bits.push(bit);
            if bit {
                written |= mask;
            }
            mask <<= 1;
        }
    }

    /// `AbilityCastsThisRound` must be recognised as a flattenable array
    /// under the `AbilityStatisticsReplicator` group, and NOT under other
    /// groups (where handle 2 means something else).
    #[test]
    fn ability_casts_this_round_is_known_array_under_correct_group() {
        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);

        // Under the correct group: is_known_array_field returns true.
        sink.set_current_group_path(Arc::from(
            "/Game/Characters/_Core/Comp_AbilityStatisticsReplicator.Comp_AbilityStatisticsReplicator_C",
        ));
        assert!(
            sink.is_known_array_field(Some("AbilityCastsThisRound")),
            "should be known under AbilityStatisticsReplicator"
        );

        // Under an unrelated group: returns false.
        sink.set_current_group_path(Arc::from("/Script/ShooterGame.SomeOtherComponent"));
        assert!(
            !sink.is_known_array_field(Some("AbilityCastsThisRound")),
            "should NOT be known under an unrelated group"
        );
    }

    /// An unresolved `AbilitiesAndBuffsComponent` payload that walks cleanly
    /// under fc=34 must emit one additive `_cnc_h1` row alongside the
    /// preservation row. The RPC handle and payload bits must be correct.
    #[test]
    fn unresolved_abilities_and_buffs_emits_cnc_rpc_row() {
        // Build a minimal CNC stream with fc=34, handle=1, 32-bit payload
        // of all 1s (to prevent false-positive walks at lower fc values).
        let mut bits = Vec::new();
        write_serialized_int(&mut bits, 1, 34); // handle=1, 6 bits
        write_int_packed(&mut bits, 32); // payload_bits=32
        bits.extend(std::iter::repeat_n(true, 32)); // 32 bits of 1s payload

        let data = bits_to_bytes(&bits);
        let bit_count = bits.len() as u32;

        let mut cache = NetGuidCache::new();
        cache.set_net_guid_path(144, "AbilitiesAndBuffsComponent".to_owned(), None);
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
        sink.time_ms = 100;
        sink.packet_id = 7;

        let header = ContentBlockHeader {
            has_rep_layout: false,
            is_actor: false,
            object_net_guid: NetworkGuid(144),
            is_stably_named: true,
            ..ContentBlockHeader::default()
        };
        sink.on_content_block(3, NetworkGuid(89), &header);

        let failure = StreamFailure {
            kind: vrf_net::pipeline::StreamKind::Rpc,
            actor_net_guid: NetworkGuid(89),
            bit_count,
            function_count: 0,
            consumed_bits: 0,
            remaining_bits: u64::from(bit_count),
        };
        sink.on_unresolved_class_net_cache_payload(failure, &data);

        // Two rows: the preservation row + one additive CNC RPC row.
        assert_eq!(
            sink.records.fields.len(),
            2,
            "preservation row + one CNC RPC row"
        );

        // Row 0: preservation.
        let pres = &sink.records.fields[0];
        assert_eq!(
            pres.field_name.as_deref(),
            Some(UNRESOLVED_CLASS_NET_CACHE_PAYLOAD_FIELD_NAME)
        );

        // Row 1: the CNC RPC.
        let rpc = &sink.records.fields[1];
        assert_eq!(rpc.handle, 1, "handle should be 1");
        assert_eq!(rpc.bit_count, 32, "payload_bits should be 32");
        assert_eq!(
            rpc.field_name.as_deref(),
            Some("_cnc_h1"),
            "field name should identify the RPC handle"
        );
        assert!(rpc.raw_bits.is_some(), "raw bits should be extracted");
        assert_eq!(sink.stats.cnc_rpcs_emitted, 1);
    }

    /// An unresolved payload for a group OTHER than AbilitiesAndBuffsComponent
    /// must not produce CNC rows -- the brute-force is gated.
    #[test]
    fn unresolved_payload_for_other_group_emits_no_cnc_rows() {
        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);

        let header = ContentBlockHeader {
            has_rep_layout: false,
            is_actor: false,
            object_net_guid: NetworkGuid(200),
            is_stably_named: true,
            ..ContentBlockHeader::default()
        };
        sink.on_content_block(3, NetworkGuid(89), &header);
        // current_group_path resolves to a bare name that is NOT
        // AbilitiesAndBuffsComponent.

        let failure = StreamFailure {
            kind: vrf_net::pipeline::StreamKind::Rpc,
            actor_net_guid: NetworkGuid(89),
            bit_count: 64,
            function_count: 0,
            consumed_bits: 0,
            remaining_bits: 64,
        };
        // A random payload that happens to be walkable.
        sink.on_unresolved_class_net_cache_payload(failure, &[0xFF; 8]);

        // Only the preservation row, no CNC rows.
        assert_eq!(sink.records.fields.len(), 1);
        assert_eq!(sink.stats.cnc_rpcs_emitted, 0);
    }
}
