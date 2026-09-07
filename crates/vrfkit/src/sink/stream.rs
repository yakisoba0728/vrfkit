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
use vrf_net::pipeline::{
    ActorChannelState, RepLayoutTailOutcome, ReplicationSink, StreamFailure, StreamFailureCause,
};
use vrf_net::types::NetworkGuid;

use super::intern::put;
use super::paths::{channel_archetype, retire_channel_archetype, set_channel_archetype};
use super::rpc::copy_raw_bits;
use super::{ExportSink, FieldValues, TABLE};

/// The RPC whose payload is a movement batch rather than a parameter list.
const MOVEMENT_RPC: &str = "ReplaysClientReceiveRemoteCharacterUpdatesSingleArrayNoAutonomous";
const ABILITIES_AND_BUFFS_COMPONENT: &str = "AbilitiesAndBuffsComponent";
const CHAINED_CNC_H1_FIELD_NAME: &str = "__vrfkit_chained_cnc_h1__";
const UNPARSED_REP_LAYOUT_TAIL_FIELD_NAME: &str = "__vrfkit_unparsed_rep_layout_tail__";

fn copy_exact_raw_bits(mut reader: BitReader<'_>, bit_count: u32) -> Option<SmallVec<[u8; 16]>> {
    if bit_count == 0 {
        return None;
    }
    let mut raw = SmallVec::with_capacity((bit_count as usize).div_ceil(8));
    raw.resize((bit_count as usize).div_ceil(8), 0);
    reader.copy_bits_to(&mut raw, u64::from(bit_count)).ok()?;
    Some(raw)
}

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
            compatible_checksum: field_checksum,
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
            let fallback_reader = reader.clone();
            let failed = self.decode_movement_rpc(reader);
            // Clean batches are represented row-for-row in movement.parquet.
            // A failed or partial batch is different: its missing rows cannot
            // reproduce the input, so retain the entire RPC payload here.
            self.push_field(FieldValues {
                handle,
                field_name,
                bit_count,
                raw_bits: failed
                    .then(|| copy_raw_bits(fallback_reader, bit_count))
                    .flatten(),
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
    fn decode_movement_rpc(&mut self, reader: BitReader<'_>) -> bool {
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
        let failed = match &result {
            Ok(decoded) => decoded.error_count != 0,
            Err(_) => true,
        };
        self.stats.record_movement_decode(result.as_ref());
        failed
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
        // Through `canonical_group`, not a bare comparison: Swiftplay replicates
        // these same fields under `Swiftplay_EoRCredits_PlayerState_C`, and the
        // overlay already treats that as the Bomb class. Comparing the raw path
        // left `manifest.players` empty on 4 of 64 demo replays whose `Subject`
        // was present on all ten actors.
        if vrf_decode::canonical_group(&self.current_group_path) != BOMB_PLAYER_STATE {
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
            // Last *non-zero* write wins, not last write. The field is
            // replicated again as 0 when the player disconnects, and plain
            // last-write-wins threw the real GUID away -- 9 players across 5
            // of 69 demos. 0 is not a NetGUID, so there is nothing to lose by
            // ignoring it.
            "SpawnedCharacter" => {
                if let Some(c) = character.filter(|c| *c != 0) {
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
                let mut reader = vrf_bitio::BitReader::with_bit_len(payload, total_len).ok()?;
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
        // Stamped with the actor it belongs to: channel numbers are recycled,
        // and an entry left behind by the previous occupant would otherwise be
        // read as this actor's. See `paths::ChannelArchetype`.
        if state.archetype_net_guid.is_valid() {
            set_channel_archetype(
                self.channel_state,
                state.channel_index,
                state.actor_net_guid,
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

    fn on_actor_close(&mut self, channel_index: u32, actor_net_guid: NetworkGuid, dormant: bool) {
        self.stats.actor_closes += 1;

        // Resolve class_path from the channel's archetype (same logic as
        // open, and the same absence for a static actor: no archetype means
        // no class_path, full stop). The actor's own GUID path used to fill
        // this gap, but that path is the level's instance name, not a class --
        // exactly the fallback `on_actor_open` above dropped, for the same
        // reason. Keeping it here meant the open row for a static actor
        // shipped `class_path = NULL` while its close row shipped an instance
        // name in the same column.
        let archetype = channel_archetype(self.channel_state, channel_index, actor_net_guid);
        let class_path = self.actor_class_path(archetype);

        // Archetype path from channel state.
        let archetype_path =
            archetype.and_then(|g| self.cache.get_path_by_guid(g.0).map(str::to_owned));

        // `ChannelCloseReason::Dormancy` means the server stopped replicating an
        // actor that is still alive; every other reason is the actor going
        // away. Both were written as "close", so a persistent effect settling
        // into dormancy was exported as a despawn -- a lifetime that ends
        // early, followed by a wake-up re-open that reads as a second spawn of
        // the same object. The flag was already on the wire and already parsed
        // (`packet.rs` sets `b_dormant` from the close reason); it just did not
        // reach the row.
        //
        // Both events are still emitted, so no row and no timestamp is lost --
        // only the label changes, and only for the closes that were never
        // despawns.
        let event = if dormant { "dormant" } else { "close" };

        self.records.actors.push(ActorRecord {
            time_ms: self.time_ms,
            packet_id: self.packet_id,
            channel_index,
            actor_net_guid: actor_net_guid.0,
            event,
            class_path,
            archetype_path,
            spawn_x: None,
            spawn_y: None,
            spawn_z: None,
            spawn_pitch: None,
            spawn_yaw: None,
            spawn_roll: None,
        });
        if !dormant {
            retire_channel_archetype(self.channel_state, channel_index);
        }
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
        self.current_is_abilities_and_buffs = !header.is_actor
            && self.cache.get_path_by_guid(header.object_net_guid.0)
                == Some(ABILITIES_AND_BUFFS_COMPONENT);
        self.stats.content_blocks += 1;
        self.resolve_block(channel_index, actor_net_guid, header)
    }

    fn on_rep_layout_tail(
        &mut self,
        _actor_net_guid: NetworkGuid,
        bit_count: u32,
        reader: BitReader<'_>,
    ) -> RepLayoutTailOutcome {
        let Some(raw_tail) = copy_exact_raw_bits(reader, bit_count) else {
            return RepLayoutTailOutcome::Unpreserved {
                cause: StreamFailureCause::ReadError,
            };
        };

        if self.current_is_abilities_and_buffs {
            // 34 is the minimum compatible capacity in the measured 34..=65
            // band, not a declared function count. It is safe only with the
            // direct pre-remap component identity above and the strict shape
            // checks below: one handle-1 RPC, exact end, set body flag.
            if let Some(rpcs) = decode_cnc_payload(&raw_tail, bit_count, ABILITIES_AND_BUFFS_FC) {
                if let [rpc] = rpcs.as_slice() {
                    let raw_body = (|| {
                        if rpc.handle != 1 || rpc.payload_bits == 0 {
                            return None;
                        }
                        let mut body =
                            BitReader::with_bit_len(&raw_tail, u64::from(bit_count)).ok()?;
                        body.skip_bits(rpc.payload_offset).ok()?;
                        let body = body.sub_reader(u64::from(rpc.payload_bits)).ok()?;
                        let mut flag = body.clone();
                        if !flag.read_bit().ok()? {
                            return None;
                        }
                        copy_exact_raw_bits(body, rpc.payload_bits)
                    })();
                    if let Some(raw_body) = raw_body {
                        let field_name = self.channel_state.names.intern(CHAINED_CNC_H1_FIELD_NAME);
                        self.push_field(FieldValues {
                            handle: rpc.handle,
                            field_name: Some(field_name),
                            bit_count: rpc.payload_bits,
                            raw_bits: Some(raw_body),
                            ..FieldValues::default()
                        });
                        self.stats.fields_emitted += 1;
                        self.stats.rpcs_emitted += 1;
                        self.stats.cnc_rpcs_emitted += 1;
                        self.stats.rep_layout_cnc_tails_decoded += 1;
                        return RepLayoutTailOutcome::Decoded { rpc_count: 1 };
                    }
                }
            }
        }

        let field_name = self
            .channel_state
            .names
            .intern(UNPARSED_REP_LAYOUT_TAIL_FIELD_NAME);
        self.push_field(FieldValues {
            field_name: Some(field_name),
            bit_count,
            raw_bits: Some(raw_tail),
            ..FieldValues::default()
        });
        self.stats.fields_emitted += 1;
        self.stats.rep_layout_cnc_tails_preserved += 1;
        RepLayoutTailOutcome::Preserved {
            cause: StreamFailureCause::UnverifiedRepLayoutTail,
        }
    }

    fn on_rep_layout_tail_failure_payload(
        &mut self,
        failure: StreamFailure,
        reader: BitReader<'_>,
    ) {
        let Some(raw) = copy_exact_raw_bits(reader, failure.bit_count) else {
            return;
        };
        if let Some(failures) = self.channel_state.failures.as_mut() {
            failures.note_payload(&failure, Arc::clone(&self.current_group_path), &raw);
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

    fn on_unresolved_class_net_cache_payload(&mut self, failure: StreamFailure, payload: &[u8]) {
        if let Some(failures) = self.channel_state.failures.as_mut() {
            failures.note_payload(&failure, Arc::clone(&self.current_group_path), payload);
        }

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

    /// Sample the decoded bytes of a block whose inner stream failed to walk.
    ///
    /// Framing calls this right after `on_stream_failure` for the same block
    /// whenever the decoded bytes exist, so the aggregate's samples for the
    /// real-loss shapes carry the payload that actually failed -- the
    /// evidence a cause hypothesis needs. Bounded like every sample: the
    /// first few per cell, payloads truncated.
    fn on_stream_failure_payload(&mut self, failure: StreamFailure, payload: &[u8]) {
        if let Some(failures) = self.channel_state.failures.as_mut() {
            failures.note_payload(&failure, Arc::clone(&self.current_group_path), payload);
        }
    }

    fn wants_stream_failure_details(&self) -> bool {
        self.channel_state.failure_aggregate_enabled()
    }

    /// Attach the resolved group path to a stream failure.
    ///
    /// The replication layer knows the bit offsets but not the names; this is the
    /// only place both are available, and the group path is what identifies the
    /// class to investigate. Note `function_count`: zero names an unresolved
    /// group, while a wrong non-zero count can still select the wrong handle
    /// width. Counts 1 and 2 both use the parser's required minimum of 2 and are
    /// therefore not distinguishable from this diagnostic alone.
    ///
    /// When diagnostics are enabled, the failure is also recorded into the
    /// bounded [`FailureAggregate`](super::failure_stats::FailureAggregate), which makes population counts available
    /// per group. `failure.payload_preserved` is the authoritative flag for
    /// whether the failed stream reached a whole-payload raw row, including
    /// unresolved ClassNetCache blocks and unparsed post-RepLayout tails.
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
        if let Some(failures) = self.channel_state.failures.as_mut() {
            failures.note_failure(&failure, Arc::clone(&self.current_group_path));
        }
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

    #[test]
    fn on_field_keeps_exact_parent_raw_bits_for_unknown_and_typed_failures() {
        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
        let payload = [0b1110_1101];

        sink.on_field(
            999,
            5,
            BitReader::with_bit_len(&payload, 5).expect("bounded field reader"),
        );

        sink.set_current_group_path(Arc::from(
            "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Base",
        ));
        sink.on_field(
            0,
            5,
            BitReader::with_bit_len(&payload, 5).expect("bounded field reader"),
        );

        assert_eq!(sink.records.fields.len(), 2);
        for row in &sink.records.fields {
            assert_eq!(row.bit_count, 5);
            assert_eq!(row.raw_bits.as_deref(), Some(&[0b0000_1101][..]));
        }
        assert_eq!(sink.records.fields[0].field_name, None);
        assert_eq!(
            sink.records.fields[1].field_name.as_deref(),
            Some("DamageTaken")
        );
        assert!(sink.records.fields[1].value_f64.is_none());
        assert_eq!(sink.stats.overlay.decoded_err, 1);
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
            cause: vrf_net::pipeline::StreamFailureCause::UnresolvedFunctionCount,
            record_handle: None,
            record_offset: Some(0),
            payload_preserved: true,
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

    fn one_h1_cnc_tail(body: &[bool]) -> Vec<bool> {
        let mut bits = Vec::new();
        write_serialized_int(&mut bits, 1, ABILITIES_AND_BUFFS_FC);
        write_int_packed(&mut bits, body.len() as u32);
        bits.extend_from_slice(body);
        bits
    }

    #[test]
    fn verified_abilities_tail_emits_one_raw_structural_h1_row() {
        let body = [true, false, true, false, true, false, true, false, true];
        let tail = one_h1_cnc_tail(&body);
        let tail_bytes = bits_to_bytes(&tail);
        let mut cache = NetGuidCache::new();
        cache.set_net_guid_path(144, ABILITIES_AND_BUFFS_COMPONENT.to_owned(), None);
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
        let header = ContentBlockHeader {
            has_rep_layout: true,
            is_actor: false,
            object_net_guid: NetworkGuid(144),
            ..ContentBlockHeader::default()
        };
        sink.on_content_block(3, NetworkGuid(89), &header);

        let outcome = sink.on_rep_layout_tail(
            NetworkGuid(89),
            tail.len() as u32,
            BitReader::with_bit_len(&tail_bytes, tail.len() as u64).unwrap(),
        );

        assert_eq!(outcome, RepLayoutTailOutcome::Decoded { rpc_count: 1 });
        assert_eq!(sink.records.fields.len(), 1);
        let row = &sink.records.fields[0];
        assert_eq!(row.handle, 1);
        assert_eq!(row.field_name.as_deref(), Some(CHAINED_CNC_H1_FIELD_NAME));
        assert_eq!(row.bit_count, body.len() as u32);
        assert_eq!(
            row.raw_bits.as_deref(),
            Some(bits_to_bytes(&body).as_slice())
        );
        assert!(row.compatible_checksum.is_none());
        assert!(row.value_i64.is_none());
        assert!(row.value_f64.is_none());
        assert!(row.value_bool.is_none());
        assert!(row.value_str.is_none());
        assert_eq!(sink.stats.rep_layout_cnc_tails_decoded, 1);
        assert_eq!(sink.stats.rep_layout_cnc_tails_preserved, 0);
    }

    #[test]
    fn matching_tail_shape_without_raw_component_provenance_stays_whole_and_raw() {
        let body = [true, false, true, false, true, false, true, false, true];
        let tail = one_h1_cnc_tail(&body);
        let tail_bytes = bits_to_bytes(&tail);
        let mut cache = NetGuidCache::new();
        cache.set_net_guid_path(145, ABILITIES_AND_BUFFS_COMPONENT.to_owned(), None);
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
        let known_header = ContentBlockHeader {
            has_rep_layout: true,
            is_actor: false,
            object_net_guid: NetworkGuid(145),
            ..ContentBlockHeader::default()
        };
        sink.on_content_block(3, NetworkGuid(89), &known_header);
        assert!(sink.current_is_abilities_and_buffs);

        let header = ContentBlockHeader {
            has_rep_layout: true,
            is_actor: false,
            object_net_guid: NetworkGuid(144),
            ..ContentBlockHeader::default()
        };
        sink.on_content_block(3, NetworkGuid(89), &header);
        assert!(
            !sink.current_is_abilities_and_buffs,
            "a following unresolved block must clear prior provenance"
        );

        let outcome = sink.on_rep_layout_tail(
            NetworkGuid(89),
            tail.len() as u32,
            BitReader::with_bit_len(&tail_bytes, tail.len() as u64).unwrap(),
        );

        assert_eq!(
            outcome,
            RepLayoutTailOutcome::Preserved {
                cause: StreamFailureCause::UnverifiedRepLayoutTail,
            }
        );
        assert_eq!(sink.records.fields.len(), 1);
        let row = &sink.records.fields[0];
        assert_eq!(
            row.field_name.as_deref(),
            Some(UNPARSED_REP_LAYOUT_TAIL_FIELD_NAME)
        );
        assert_eq!(row.bit_count, tail.len() as u32);
        assert_eq!(row.raw_bits.as_deref(), Some(tail_bytes.as_slice()));
        assert!(row.compatible_checksum.is_none());
        assert!(row.value_i64.is_none());
        assert!(row.value_f64.is_none());
        assert!(row.value_bool.is_none());
        assert!(row.value_str.is_none());
        assert_eq!(sink.stats.rep_layout_cnc_tails_decoded, 0);
        assert_eq!(sink.stats.rep_layout_cnc_tails_preserved, 1);
    }

    #[test]
    fn verified_component_preserves_exact_but_unverified_tail_shapes_whole() {
        let first = one_h1_cnc_tail(&[true, false, true]);
        let mut two_rpcs = first.clone();
        two_rpcs.extend(one_h1_cnc_tail(&[true, true, false]));
        let false_flag = one_h1_cnc_tail(&[false, true, true]);

        let mut cache = NetGuidCache::new();
        cache.set_net_guid_path(144, ABILITIES_AND_BUFFS_COMPONENT.to_owned(), None);
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
        let header = ContentBlockHeader {
            has_rep_layout: true,
            is_actor: false,
            object_net_guid: NetworkGuid(144),
            ..ContentBlockHeader::default()
        };
        sink.on_content_block(3, NetworkGuid(89), &header);

        for tail in [&two_rpcs, &false_flag] {
            let bytes = bits_to_bytes(tail);
            let outcome = sink.on_rep_layout_tail(
                NetworkGuid(89),
                tail.len() as u32,
                BitReader::with_bit_len(&bytes, tail.len() as u64).unwrap(),
            );
            assert_eq!(
                outcome,
                RepLayoutTailOutcome::Preserved {
                    cause: StreamFailureCause::UnverifiedRepLayoutTail,
                }
            );
            let row = sink.records.fields.last().unwrap();
            assert_eq!(
                row.field_name.as_deref(),
                Some(UNPARSED_REP_LAYOUT_TAIL_FIELD_NAME)
            );
            assert_eq!(row.bit_count, tail.len() as u32);
            assert_eq!(row.raw_bits.as_deref(), Some(bytes.as_slice()));
        }
        assert_eq!(sink.stats.rep_layout_cnc_tails_decoded, 0);
        assert_eq!(sink.stats.rep_layout_cnc_tails_preserved, 2);
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
        let reader = BitReader::with_bit_len(&data, bits.len() as u64).unwrap();

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
        let reader = BitReader::with_bit_len(&data, bits.len() as u64).unwrap();

        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);

        let emitted = sink.try_parse_rpc_params(7, reader, Some("SomeFunction"));
        assert!(emitted, "one parameter row is emitted");
        assert_eq!(sink.stats.truncated_rpcs, 0);
    }

    /// Build an RPC payload of one parameter, the zero-handle terminator, and
    /// `suffix_bits` bits of whatever follows it.
    fn rpc_payload_with_suffix(suffix_bits: usize) -> Vec<bool> {
        let mut bits = Vec::new();
        bits.push(false); // property checksum
        write_int_packed(&mut bits, 1); // encodedHandle = 1 -> handle 0
        write_int_packed(&mut bits, 8); // payload_bits = 8
        bits.extend(std::iter::repeat_n(false, 8)); // payload
        write_int_packed(&mut bits, 0); // terminator handle
        bits.extend(std::iter::repeat_n(true, suffix_bits));
        bits
    }

    /// Bits left over after the zero-handle terminator must be counted.
    ///
    /// The terminator ended the walk without asking what was still in the
    /// payload. Because a parameter had already been emitted, the caller's
    /// whole-payload fallback row was suppressed too -- so the suffix reached
    /// no row, no `truncated_rpcs`, and not even `skipped_bits`. Every leaf's
    /// payload in this crate is checked for full consumption; the container's
    /// was not.
    ///
    /// This counts rather than rejects. The rows already parsed are good, and
    /// discarding them to punish a tail nobody has yet seen would lose data to
    /// make a point. The counter is what turns "this cannot happen" into a
    /// measurement.
    #[test]
    fn bits_after_the_rpc_terminator_are_counted_not_discarded() {
        let bits = rpc_payload_with_suffix(16);
        let data = bits_to_bytes(&bits);
        let reader = BitReader::with_bit_len(&data, bits.len() as u64).unwrap();

        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);

        let emitted = sink.try_parse_rpc_params(7, reader, Some("SomeFunction"));
        assert!(emitted, "the parameter that did parse is still emitted");
        assert_eq!(
            sink.stats.rpc_suffix_bits_dropped, 16,
            "the bits after the terminator must reach a counter"
        );
        // Not a truncation: the walk ended where the wire told it to.
        assert_eq!(sink.stats.truncated_rpcs, 0);
        drop(sink);
        assert_eq!(records.fields.len(), 2, "parameter plus whole raw fallback");
        let fallback = records.fields.last().unwrap();
        assert_eq!(fallback.bit_count, bits.len() as u32);
        assert_eq!(fallback.raw_bits.as_deref(), Some(data.as_slice()));
    }

    #[test]
    fn a_partially_parsed_truncated_rpc_retains_the_whole_payload() {
        let mut bits = Vec::new();
        bits.push(false); // property checksum
        write_int_packed(&mut bits, 1);
        write_int_packed(&mut bits, 8);
        bits.extend(std::iter::repeat_n(false, 8)); // one complete parameter
        write_int_packed(&mut bits, 2);
        write_int_packed(&mut bits, 100); // second parameter overruns
        bits.extend(std::iter::repeat_n(true, 8));
        let data = bits_to_bytes(&bits);
        let reader = BitReader::with_bit_len(&data, bits.len() as u64).unwrap();
        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);

        assert!(sink.try_parse_rpc_params(7, reader, Some("SomeFunction")));
        assert_eq!(sink.stats.truncated_rpcs, 1);
        drop(sink);
        assert_eq!(records.fields.len(), 2, "parameter plus whole raw fallback");
        let fallback = records.fields.last().unwrap();
        assert_eq!(fallback.bit_count, bits.len() as u32);
        assert_eq!(fallback.raw_bits.as_deref(), Some(data.as_slice()));
    }

    #[test]
    fn a_failed_movement_decode_retains_the_whole_rpc_payload() {
        let mut update = Vec::new();
        write_int_packed(&mut update, 3); // shooter GUID handle 2
        write_int_packed(&mut update, 32);
        for bit in 0..32 {
            update.push((4321u32 & (1 << bit)) != 0);
        }
        write_int_packed(&mut update, 4); // component stream handle 3
        write_int_packed(&mut update, 8);
        update.extend(std::iter::repeat_n(false, 8)); // short u16 header
        write_int_packed(&mut update, 0);

        let mut array = Vec::new();
        write_int_packed(&mut array, 1);
        write_int_packed(&mut array, 1);
        array.extend(update);
        write_int_packed(&mut array, 0);

        let mut bits = vec![false]; // top-level ignored bit
        write_int_packed(&mut bits, 2); // updates-array handle 1
        write_int_packed(&mut bits, array.len() as u32);
        bits.extend(array);
        write_int_packed(&mut bits, 0);
        let data = bits_to_bytes(&bits);

        let path = "/Script/Test.Movement_ClassNetCache";
        let mut cache = NetGuidCache::new();
        cache
            .add_export_group(vrf_schema::NetFieldExportGroup::new(path.into(), 7, 1))
            .unwrap();
        assert!(cache.set_field_on_group(
            7,
            vrf_schema::NetFieldExport {
                handle: 0,
                compatible_checksum: 0,
                name: MOVEMENT_RPC.into(),
            },
        ));
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
        sink.set_current_group_path(Arc::from(path));
        let reader = BitReader::with_bit_len(&data, bits.len() as u64).unwrap();

        sink.on_rpc(0, bits.len() as u32, reader);

        assert_eq!(sink.stats.movement_rpc_errors, 1);
        drop(sink);
        assert_eq!(records.fields.len(), 1);
        assert_eq!(records.fields[0].bit_count, bits.len() as u32);
        assert_eq!(records.fields[0].raw_bits.as_deref(), Some(data.as_slice()));
    }

    /// The one permitted leftover stays silent.
    ///
    /// `FunctionParameters` grammar allows a single trailing alignment bit
    /// (`BitsRemaining == 1 -> SkipBits(1)`). Counting that would make the new
    /// counter fire on well-formed payloads, which is how a real signal gets
    /// ignored.
    #[test]
    fn a_single_alignment_bit_after_the_rpc_terminator_is_not_a_drop() {
        for suffix in [0, 1] {
            let bits = rpc_payload_with_suffix(suffix);
            let data = bits_to_bytes(&bits);
            let reader = BitReader::with_bit_len(&data, bits.len() as u64).unwrap();

            let mut cache = NetGuidCache::new();
            let mut channel_state = ChannelState::new();
            let mut records = RecordBuffers::default();
            let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);

            sink.try_parse_rpc_params(7, reader, Some("SomeFunction"));
            assert_eq!(
                sink.stats.rpc_suffix_bits_dropped, 0,
                "{suffix} trailing bit(s) is within the grammar"
            );
        }
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
            cause: vrf_net::pipeline::StreamFailureCause::UnresolvedFunctionCount,
            record_handle: None,
            record_offset: Some(0),
            payload_preserved: true,
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
    ///
    /// The payload is the exact one
    /// `unresolved_abilities_and_buffs_emits_cnc_rpc_row` proves walks cleanly
    /// under fc=34 -- deliberately, not `[0xFF; 8]` (which
    /// `decode_cnc_payload(&[0xFF; 8], 64, 34)` returns `None` for, i.e. it
    /// does not walk at all). With a non-walking payload this test would stay
    /// green even if the `current_group_path.contains("AbilitiesAndBuffsComponent")`
    /// guard above were deleted, because `decode_cnc_payload` alone would
    /// still refuse it -- so it would not be testing the gate.
    #[test]
    fn unresolved_payload_for_other_group_emits_no_cnc_rows() {
        // Same construction as the fc=34 walking test: handle=1, 6 bits;
        // payload_bits=32; 32 bits of 1s.
        let mut bits = Vec::new();
        write_serialized_int(&mut bits, 1, 34);
        write_int_packed(&mut bits, 32);
        bits.extend(std::iter::repeat_n(true, 32));
        let data = bits_to_bytes(&bits);
        let bit_count = bits.len() as u32;

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
            bit_count,
            function_count: 0,
            consumed_bits: 0,
            remaining_bits: u64::from(bit_count),
            cause: vrf_net::pipeline::StreamFailureCause::UnresolvedFunctionCount,
            record_handle: None,
            record_offset: Some(0),
            payload_preserved: true,
        };
        sink.on_unresolved_class_net_cache_payload(failure, &data);

        // Only the preservation row, no CNC rows: the payload walks (proven
        // above), so only the group-path gate can be what stops it here.
        assert_eq!(sink.records.fields.len(), 1);
        assert_eq!(sink.stats.cnc_rpcs_emitted, 0);
    }
    /// A dormancy close is not a despawn, and must not be exported as one.
    ///
    /// `vrf-net` reads the channel close reason and hands the sink a `dormant`
    /// flag; the sink took the flag as `_dormant` and wrote `"close"` for every
    /// close there is. Dormancy means the server stopped replicating an actor
    /// that is still alive -- exactly what a persistent effect does when it
    /// settles -- so `actors.parquet` reported a despawn that never happened.
    /// A consumer pairing `open` with `close` for lifetime ends the effect
    /// early, and the wake-up re-open then reads as a second spawn of the same
    /// thing.
    ///
    /// The fix is the flag reaching the row, not a filter: both events are
    /// still emitted, so nothing is lost and the row count is unchanged. Only
    /// the label stops lying.
    #[test]
    fn a_dormancy_close_is_not_recorded_as_a_despawn() {
        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);

        sink.on_actor_close(3, NetworkGuid(42), false);
        sink.on_actor_close(4, NetworkGuid(43), true);

        assert_eq!(sink.records.actors.len(), 2, "both closes are still rows");
        assert_eq!(
            sink.records.actors[0].event, "close",
            "a real despawn keeps the name every consumer already reads"
        );
        assert_eq!(
            sink.records.actors[1].event, "dormant",
            "a dormancy close must be distinguishable from a despawn"
        );
        // Both still count as closes: the actor channel did close.
        assert_eq!(sink.stats.actor_closes, 2);
    }

    /// A static actor (no archetype) must not get its class_path filled in
    /// from its own GUID path on close, the same way `on_actor_open` already
    /// refuses to: that path is the level's instance name, not a class, and
    /// filling it in only on close made the open and close rows for the same
    /// static actor disagree.
    #[test]
    fn a_static_actors_close_row_does_not_fabricate_a_class_path_from_its_own_guid() {
        let mut cache = NetGuidCache::new();
        // The actor's own GUID path -- an instance name, e.g. what a level
        // placement looks like on the wire -- must not read back as a class.
        cache.set_net_guid_path(42, "WindowShieldA1".to_owned(), None);
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);

        // No archetype: `NetworkGuid(0)` is invalid, so `on_actor_open` never
        // registers a channel archetype for it.
        sink.on_actor_open(&ActorChannelState {
            channel_index: 3,
            is_open: true,
            is_dormant: false,
            actor_net_guid: NetworkGuid(42),
            archetype_net_guid: NetworkGuid(0),
            level_guid: NetworkGuid(0),
            spawn_location: None,
            spawn_rotation: None,
            spawn_scale: None,
            spawn_velocity: None,
            open_packet_id: 0,
        });
        sink.on_actor_close(3, NetworkGuid(42), false);

        assert_eq!(sink.records.actors[0].class_path, None, "open row");
        assert_eq!(
            sink.records.actors[1].class_path, None,
            "close row must agree with the open row, not fabricate a class \
             from the actor's own instance-name path"
        );
    }

    #[test]
    fn destroyed_channel_archetypes_are_retired_but_dormant_ones_survive() {
        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
        let opened = |channel_index, actor, archetype| ActorChannelState {
            channel_index,
            is_open: true,
            is_dormant: false,
            actor_net_guid: NetworkGuid(actor),
            archetype_net_guid: NetworkGuid(archetype),
            level_guid: NetworkGuid(0),
            spawn_location: None,
            spawn_rotation: None,
            spawn_scale: None,
            spawn_velocity: None,
            open_packet_id: 0,
        };

        sink.on_actor_open(&opened(3, 42, 8));
        sink.on_actor_open(&opened(4, 43, 9));
        sink.on_actor_close(3, NetworkGuid(42), false);
        sink.on_actor_close(4, NetworkGuid(43), true);

        assert!(
            channel_archetype(sink.channel_state, 3, NetworkGuid(42)).is_none(),
            "destroyed channels must not accumulate sink-side archetypes"
        );
        assert_eq!(
            channel_archetype(sink.channel_state, 4, NetworkGuid(43)),
            Some(NetworkGuid(9)),
            "dormancy preserves the class needed when the same actor wakes"
        );
    }

    /// Player identity has to survive a game mode that is not Bomb.
    ///
    /// Swiftplay replicates the same fields under
    /// `Swiftplay_EoRCredits_PlayerState_C`. The overlay already handles that
    /// through `GROUP_ALIASES`, but this capture compared the raw path against
    /// `BombPlayerState` and so recorded nothing -- 4 of 64 demo replays came
    /// out with an empty `manifest.players` while their `Subject` field was
    /// present on all ten actors.
    #[test]
    fn player_identity_is_captured_on_a_swiftplay_player_state() {
        const SWIFT: &str = "/Game/GameModes/_Development/Swiftplay_EndOfRoundCredits/Swiftplay_EoRCredits_PlayerState.Swiftplay_EoRCredits_PlayerState_C";
        for path in [BOMB_PLAYER_STATE, SWIFT] {
            let mut cache = NetGuidCache::new();
            let mut channel_state = ChannelState::new();
            let mut records = RecordBuffers::default();
            let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
            sink.current_group_path = Arc::from(path);
            sink.current_actor_guid = 42;

            sink.record_player_identity(Some("Subject"), Some("uuid-here"), None);
            sink.record_player_identity(Some("SpawnedCharacter"), None, Some(576));

            let players = sink.channel_state.players.clone();
            let entry = players
                .get(&42)
                .unwrap_or_else(|| panic!("nothing for {path}"));
            assert_eq!(entry.subject.as_deref(), Some("uuid-here"), "{path}");
            assert_eq!(entry.character_net_guid, Some(576), "{path}");
        }
    }

    /// A disconnect must not erase the character link.
    ///
    /// `SpawnedCharacter` is replicated twice: the real GUID about 60 ms in,
    /// and then 0 when the player leaves. Last-write-wins kept the 0, so
    /// `manifest.players.character_net_guid` was 0 for 9 players across 5 of
    /// 69 demo replays -- and every one of those was a character that *did*
    /// spawn, still reachable through its pawn's `PlayerState`. The cost was
    /// paid downstream: spike custody went `unknown`, two planters went
    /// unattributed, and the worst replay attributed only 73.2% of its
    /// movement rows to a player.
    #[test]
    fn a_disconnect_does_not_erase_the_character_link() {
        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
        sink.current_group_path = Arc::from(BOMB_PLAYER_STATE);
        sink.current_actor_guid = 42;

        sink.record_player_identity(Some("SpawnedCharacter"), None, Some(1368));
        sink.record_player_identity(Some("SpawnedCharacter"), None, Some(0));

        let players = sink.channel_state.players.clone();
        assert_eq!(players.get(&42).unwrap().character_net_guid, Some(1368));
    }

    /// The 32-line failure window is a display buffer, and the aggregate must
    /// not inherit its cap: a replay that fails 100 blocks keeps all 100 in
    /// the aggregate while the line list stops at the usual 32. This is the
    /// property the 2026-09-07 corpus lacked -- 714 saturated windows, no
    /// population counts.
    #[test]
    fn failures_past_the_line_cap_are_all_aggregated() {
        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        channel_state.enable_failure_aggregate(false);
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
        sink.set_current_group_path(Arc::from("/Script/ShooterGame.AresAbilitySystemComponent"));
        let failure = |consumed: u64| StreamFailure {
            kind: vrf_net::pipeline::StreamKind::RepLayout,
            actor_net_guid: NetworkGuid(7),
            bit_count: 200,
            function_count: 0,
            consumed_bits: consumed,
            remaining_bits: 200 - consumed,
            cause: vrf_net::pipeline::StreamFailureCause::AbandonedTail,
            record_handle: Some(3),
            record_offset: Some(consumed),
            payload_preserved: false,
        };

        for i in 0..100 {
            sink.on_stream_failure(failure(i % 2));
        }

        assert_eq!(
            sink.channel_state.stream_failures().len(),
            32,
            "the line window stays capped"
        );
        let agg = sink.channel_state.failures.as_ref().unwrap();
        assert_eq!(agg.total_failures(), 100, "the aggregate never caps");
        assert_eq!(agg.real_loss(), 100);
        // Two cells: consumed_bits is a key dimension, so the 50 failures that
        // stopped at 0 and the 50 that stopped at 1 stay apart, and each cell
        // still counts its whole population.
        let cells = agg.cells_sorted();
        assert_eq!(cells.len(), 2, "two distinct consumed values, two cells");
        let counted: u64 = cells.iter().map(|(_, cell)| cell.count).sum();
        assert_eq!(counted, 100, "the cells together hold every failure");
        for (key, cell) in &cells {
            assert_eq!(cell.count, 50, "{:?}", key.consumed_bits);
        }
        assert_eq!(
            cells[0].0.cause,
            vrf_net::pipeline::StreamFailureCause::AbandonedTail
        );
        assert_eq!(cells[0].0.record_handle, Some(3));
    }

    /// A preserved unresolved RPC failure and a genuinely lost RepLayout
    /// failure must land in separate aggregate buckets, so real loss is never
    /// inflated by payloads that are on disk as preservation rows.
    #[test]
    fn preserved_unresolved_failures_are_separated_from_real_loss() {
        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        channel_state.enable_failure_aggregate(true);
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
        sink.set_current_group_path(Arc::from("AbilitiesAndBuffsComponent"));

        // The framing layer's exact sequence for an unresolved block:
        // on_unresolved_class_net_cache_payload, then on_stream_failure.
        let unresolved = StreamFailure {
            kind: vrf_net::pipeline::StreamKind::Rpc,
            actor_net_guid: NetworkGuid(9),
            bit_count: 64,
            function_count: 0,
            consumed_bits: 0,
            remaining_bits: 64,
            cause: vrf_net::pipeline::StreamFailureCause::UnresolvedFunctionCount,
            record_handle: None,
            record_offset: Some(0),
            payload_preserved: true,
        };
        for i in 0..40 {
            let _ = i;
            sink.on_unresolved_class_net_cache_payload(unresolved, &[0xDE, 0xAD]);
            sink.on_stream_failure(unresolved);
        }

        // ...plus a real RepLayout loss.
        sink.set_current_group_path(Arc::from("/Script/ShooterGame.AresAbilitySystemComponent"));
        sink.on_stream_failure(StreamFailure {
            kind: vrf_net::pipeline::StreamKind::RepLayout,
            actor_net_guid: NetworkGuid(7),
            bit_count: 200,
            function_count: 0,
            consumed_bits: 185,
            remaining_bits: 15,
            cause: vrf_net::pipeline::StreamFailureCause::AbandonedTail,
            record_handle: Some(3),
            record_offset: Some(185),
            payload_preserved: false,
        });

        let agg = sink.channel_state.failures.as_ref().unwrap();
        assert_eq!(agg.total_failures(), 41);
        assert_eq!(agg.preserved_unresolved(), 40);
        assert_eq!(agg.real_loss(), 1, "only the RepLayout block is loss");
        let cells = agg.cells_sorted();
        assert_eq!(cells.len(), 2);
        // Sorted by count: the preserved cell first, its samples carrying the
        // real payload bytes recorded by on_unresolved.
        assert_eq!(
            cells[0].0.cause,
            vrf_net::pipeline::StreamFailureCause::UnresolvedFunctionCount
        );
        assert_eq!(cells[0].1.samples.len(), 3, "sample cap, not 40");
        assert_eq!(cells[0].1.samples[0].payload_hex.as_deref(), Some("dead"));
        assert_eq!(cells[1].0.kind, vrf_net::pipeline::StreamKind::RepLayout);
        assert_eq!(
            cells[1].0.group_path.as_ref(),
            "/Script/ShooterGame.AresAbilitySystemComponent"
        );
    }

    /// Taking the aggregate drains it, so a checkpoint pass that creates one
    /// channel state per chunk cannot double-count a chunk's failures into
    /// the caller's totals.
    #[test]
    fn taking_the_aggregate_drains_it() {
        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        channel_state.enable_failure_aggregate(false);
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
        sink.set_current_group_path(Arc::from("SomeGroup"));
        sink.on_stream_failure(StreamFailure {
            kind: vrf_net::pipeline::StreamKind::RepLayout,
            actor_net_guid: NetworkGuid(1),
            bit_count: 16,
            function_count: 0,
            consumed_bits: 8,
            remaining_bits: 8,
            cause: vrf_net::pipeline::StreamFailureCause::ReadError,
            record_handle: Some(0),
            record_offset: Some(1),
            payload_preserved: false,
        });

        let taken = sink.channel_state.take_failure_aggregate();
        assert_eq!(taken.total_failures(), 1);
        assert!(
            sink.channel_state.failures.is_none(),
            "taking disables the drained aggregate"
        );
    }

    /// ...but a character that never spawned still reports nothing, rather
    /// than a 0 that reads like a NetGUID.
    #[test]
    fn a_character_that_never_spawned_stays_none() {
        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
        sink.current_group_path = Arc::from(BOMB_PLAYER_STATE);
        sink.current_actor_guid = 7;

        sink.record_player_identity(Some("SpawnedCharacter"), None, Some(0));

        let players = sink.channel_state.players.clone();
        assert_eq!(players.get(&7).unwrap().character_net_guid, None);
    }
}
