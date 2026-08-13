//! `export` subcommand driver -- full pipeline from .vrf to Parquet.
//!
//! # Architecture
//!
//! DemoFrames and packets are processed in one wire-order pass. The frame
//! iterator applies one frame's ExportData, then lends that exact cache state
//! to the packet callback. Packet-side export mutations therefore precede the
//! next packet, while a later frame's schema cannot leak backward into an
//! earlier packet.
//!
//! # Layout
//!
//! - [`writers`] -- the two large tables' writers, running off the packet loop.
//! - [`checkpoints`] -- the optional full-state snapshot pass.
//! - [`summary`] -- the stderr report, whose every line a Python harness pins.

mod checkpoints;
mod publish;
mod summary;
mod totals;
mod writers;

use std::fs;
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::Instant;

use vrf_container::{
    ChunkIterator, ChunkType, decompress_replay_data_with_trailing, parse_event_chunk,
    parse_preamble,
};
use vrf_decode::OverlayErrorReport;
use vrf_export::{
    ActorWriter, EventRecord, EventWriter, FieldRecord, FieldWriter, MovementRecord,
    MovementWriter, NetGuidRecord, NetGuidWriter,
};
use vrf_frame::iter_demo_frames;
use vrf_net::pipeline::ReplicationReader;
use vrf_schema::NetGuidCache;

use crate::error::CliError;
use crate::manifest;
use crate::sink::{ChannelState, ExportSink, RecordBuffers};
use checkpoints::{CheckpointStats, ReplayContext};
use publish::OutputTransaction;
use summary::RunTotals;
use totals::SinkTotals;
use writers::WriterThread;

/// The first two payload words for an Event group that declares `word_count` of
/// them, or `None` when the payload does not fit that layout.
///
/// The payload is `[u32 tag][N x u32 words][FString name][f32 seconds]` and is
/// not self-describing: no count precedes the words. `N` was therefore assumed
/// per group and the words copied from fixed offsets with nothing checking that
/// the rest of the payload agreed. A build that changed `N` would not fail --
/// `characterDeath` claims two words, and on a payload carrying one the second
/// read lands exactly on the `FString`'s length prefix, which is a small
/// positive integer and reads in the exported column as an entirely plausible
/// killed NetGUID.
///
/// `N` is not readable forward, but it *is* checkable backward: the remaining
/// three parts have known widths, so the assumed layout must consume the
/// payload exactly. That is what this verifies. It is the same rule
/// `vrf_decode::decode_field` applies to every leaf -- refuse to return a
/// plausible wrong number when bits are left over -- applied to the container
/// the leaves sit in.
///
/// A group claiming no words is not checked. Those are `spikePlanted` and
/// friends plus every group this build does not recognise; they export no words
/// either way, and measuring them against a layout nobody established would
/// only manufacture alarms about payload shapes the project has never claimed
/// to know.
fn typed_event_words(payload: &[u8], word_count: u8) -> Option<(Option<u32>, Option<u32>)> {
    if word_count == 0 {
        return Some((None, None));
    }
    let words = usize::from(word_count);
    let word_at = |i: usize| -> Option<u32> {
        let start = 4 + i * 4;
        payload
            .get(start..start + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };

    // The FString sits immediately after the words: an i32 length, then that
    // many code units. Unreal counts the null terminator in the length, and a
    // negative length means UTF-16 (two bytes per unit).
    let name_at = 4 + words * 4;
    let raw = payload
        .get(name_at..name_at + 4)
        .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))?;
    let units = i64::from(raw).unsigned_abs();
    let name_bytes = if raw < 0 {
        units.checked_mul(2)?
    } else {
        units
    };
    // tag + words + length prefix + name + the trailing f32.
    let expected = (4 + words as u64 * 4) + 4 + name_bytes + 4;
    if expected != payload.len() as u64 {
        return None;
    }

    Some((word_at(0), if words >= 2 { word_at(1) } else { None }))
}

pub fn run(vrf_path: &str, out_dir: &str, with_checkpoints: bool) -> Result<(), CliError> {
    let start = Instant::now();

    // -- Read file ---------------------------------------------------------
    eprintln!("reading {vrf_path}...");
    let data = fs::read(vrf_path)?;
    let file_size = data.len();

    // -- Parse preamble ----------------------------------------------------
    let preamble = parse_preamble(&data)?;
    let ctx = ReplayContext {
        branch: &preamble.header.replay_version.branch,
        flags: preamble.header.flags,
        compressed: preamble.info.compressed,
        encrypted: preamble.info.encrypted,
    };

    eprintln!(
        "branch: {}, flags: 0x{:04X}, compressed: {}, duration: {} ms",
        ctx.branch, ctx.flags, ctx.compressed, preamble.info.length_in_ms
    );

    // -- Setup output ------------------------------------------------------
    let destination = PathBuf::from(out_dir);
    let output = OutputTransaction::begin(&destination)?;
    let out_path = output.path();

    let create = |name: &str| -> Result<BufWriter<fs::File>, CliError> {
        Ok(BufWriter::new(fs::File::create(out_path.join(name))?))
    };

    let mut field_writer = FieldWriter::new(create("fields.parquet")?)?;
    let mut movement_writer = MovementWriter::new(create("movement.parquet")?)?;
    let mut actor_writer = ActorWriter::new(create("actors.parquet")?)?;
    // Event chunks are a couple of hundred rows and are written inline for the
    // same reason `actors` is: the encoding cost is far below a thread's worth.
    let mut event_writer = EventWriter::new(create("events.parquet")?)?;
    let mut checkpoint_writer = if with_checkpoints {
        Some(FieldWriter::new(create("checkpoint_fields.parquet")?)?)
    } else {
        None
    };

    let mut fields = WriterThread::<FieldRecord>::spawn("fields", move |rx| {
        for batch in rx {
            field_writer.push_batch(batch)?;
        }
        field_writer.finish()
    });
    let mut movement = WriterThread::<MovementRecord>::spawn("movement", move |rx| {
        for batch in rx {
            movement_writer.push_batch(batch)?;
        }
        movement_writer.finish()
    });

    // -- Setup replication reader and schema cache --------------------------
    let mut cache = NetGuidCache::new();
    let mut repl_reader = ReplicationReader::new(ctx.branch)
        .map_err(|e| CliError::Usage(format!("unsupported branch: {e}")))?;

    // -- Iterate chunks ----------------------------------------------------
    let mut chunk_iter = ChunkIterator::new(&data, preamble.remaining_offset);
    let mut chunks_processed = 0u32;
    let mut total_packets: u32 = 0;
    let mut channel_state = ChannelState::new();

    // Reusable per-packet record buffers; see `RecordBuffers`.
    let mut buffers = RecordBuffers::default();
    let mut movement_rows: u64 = 0;
    let mut event_rows: u64 = 0;
    let mut event_trailing_bytes: u64 = 0;
    let mut replay_data_trailing_bytes: u64 = 0;
    let mut error_report = OverlayErrorReport::default();
    // Every sink-derived counter, in one place. See `totals`.
    let mut sink_totals = SinkTotals::default();
    let mut event_layout_mismatches: u64 = 0;
    let mut event_first_layout_mismatch: Option<String> = None;
    let mut cp_stats = CheckpointStats::default();

    while let Some(chunk) = chunk_iter.next_chunk()? {
        // Sliced once for all three chunk kinds. Safe for the kinds this loop
        // ignores too: `next_chunk` refuses a chunk whose declared size runs
        // past the file, so the range is in bounds before it is returned.
        let payload = &data[chunk.data_offset..chunk.data_offset + chunk.size_in_bytes as usize];

        // Event chunks carry the server's own labelled timeline. They are
        // uncompressed and independent of the replication pass, so they are
        // read here and written straight out.
        if chunk.chunk_type == ChunkType::Event {
            let event = parse_event_chunk(payload)?;
            event_trailing_bytes += event.trailing_bytes as u64;
            // Typed payload words for groups whose word count is structurally
            // fixed. Payload layout: [u32 tag][N x u32 words][FString][f32].
            // `typed_event_words` checks that the assumed N consumes the
            // payload exactly before naming anything; a payload that disagrees
            // yields no words and is counted, never guessed at.
            // `raw_payload` still keeps every byte either way.
            let word_count: u8 = match event.group.as_str() {
                "characterDeath" => 2,
                "characterUltimateUsed" | "roundStarted" | "switchTeams" => 1,
                // spikePlanted/Defused/Exploded carry no words; any unknown
                // group claims none rather than guessing.
                _ => 0,
            };
            let (word0, word1) = match typed_event_words(event.payload, word_count) {
                Some(words) => words,
                None => {
                    event_layout_mismatches += 1;
                    event_first_layout_mismatch.get_or_insert_with(|| {
                        format!(
                            "{} declared {word_count} word(s) but its {}-byte payload does not fit that layout",
                            event.group,
                            event.payload.len()
                        )
                    });
                    (None, None)
                }
            };
            event_writer.push(EventRecord {
                id: event.id,
                group: event.group,
                metadata: event.metadata,
                time1: event.time1,
                time2: event.time2,
                payload_size: event.size_in_bytes,
                raw_payload: event.payload.to_vec(),
                word0,
                word1,
            })?;
            event_rows += 1;
            continue;
        }
        if chunk.chunk_type == ChunkType::Checkpoint {
            if let Some(writer) = checkpoint_writer.as_mut() {
                checkpoints::process_chunk(
                    payload,
                    &ctx,
                    writer,
                    &mut cp_stats,
                    &mut error_report,
                )?;
            }
            continue;
        }
        if chunk.chunk_type != ChunkType::ReplayData {
            continue;
        }

        // `_with_trailing` rather than the plain call: the outer chunk can be
        // larger than the inner SizeInBytes, and the plain signature drops that
        // excess with nothing to show for it. Counted, not rejected -- no replay
        // has ever been measured carrying any, so failing on it would be
        // guessing at a format we have not seen.
        let (decompressed, trailing) =
            decompress_replay_data_with_trailing(payload, ctx.compressed, ctx.encrypted)?;
        replay_data_trailing_bytes += trailing as u64;

        // Process each packet before the iterator advances to later ExportData.
        // The callback cannot return a writer error through `FrameError`, so it
        // records the first one and makes later callbacks no-ops until the
        // frame walk finishes and the error can be returned here.
        let mut packet_error = None;
        iter_demo_frames(&decompressed, ctx.flags, &mut cache, |pkt, packet_cache| {
            if packet_error.is_some() {
                return;
            }
            let pkt_id = total_packets;
            total_packets += 1;

            // Scoped so the sink's borrow of `buffers` ends before they are
            // drained. The buffers outlive the sink; that is the point.
            {
                let mut sink = ExportSink::new(packet_cache, &mut channel_state, &mut buffers);
                sink.time_ms = pkt.time_ms;
                sink.packet_id = pkt_id;

                repl_reader.process_packet(pkt.data, pkt_id as i32, &mut sink);

                // The sink is dropped at the end of this scope, so a counter
                // not read here is a counter that never existed. All of them
                // go through one function; see `totals`.
                sink_totals.absorb(&mut sink.stats, &mut error_report);
            }

            // Hand field and movement records to their writer threads.
            let result = (|| -> Result<(), CliError> {
                fields.append(&mut buffers.fields)?;
                movement_rows += buffers.movement.len() as u64;
                movement.append(&mut buffers.movement)?;
                // Drain actor lifecycle records to the inline writer.
                for record in buffers.actors.drain(..) {
                    actor_writer.push(record)?;
                }
                Ok(())
            })();
            if let Err(error) = result {
                packet_error = Some(error);
            }
        })?;
        if let Some(error) = packet_error {
            return Err(error);
        }

        chunks_processed += 1;

        if chunks_processed % 100 == 0 {
            eprintln!(
                "  chunk {chunks_processed}: {total_packets} packets, {} groups",
                cache.group_count()
            );
        }
    }

    // -- Finish writers ----------------------------------------------------
    //
    // The two offloaded writers are joined here, before the elapsed time is
    // taken and before any file size is read, so both files are complete and
    // both results are checked.
    fields.finish()?;
    movement.finish()?;
    actor_writer.finish()?;
    event_writer.finish()?;
    if let Some(w) = checkpoint_writer.take() {
        w.finish()?;
    }

    // -- Write the NetGUID registry ----------------------------------------
    //
    // Written after the replication pass because the cache accumulates over the
    // whole replay: a GUID's outer may be declared in a later chunk than the
    // one that first referenced it. Sorted so the file is byte-reproducible
    // across runs (the cache is HashMap-backed and iterates in arbitrary order).
    let mut net_guid_writer = NetGuidWriter::new(create("net_guids.parquet")?)?;
    let mut guid_entries = cache.net_guid_entries();
    guid_entries.sort_unstable_by_key(|e| e.net_guid);
    let net_guid_rows = guid_entries.len();
    for entry in guid_entries {
        net_guid_writer.push(NetGuidRecord {
            net_guid: entry.net_guid,
            path: entry.path.to_owned(),
            outer_net_guid: entry.outer_net_guid,
        })?;
    }
    net_guid_writer.finish()?;

    // Drain fragments that never got their final piece. Without this the
    // accumulator is simply dropped and a partial bunch lost at EOF is
    // indistinguishable from one still legitimately in flight -- the counters
    // it feeds only exist if someone asks for them.
    repl_reader.finish();

    let net_stats = repl_reader.stats();
    let elapsed = start.elapsed();

    // -- Write manifest ----------------------------------------------------
    //
    // Before the summary so the path the summary prints names a file that
    // exists by the time it is read.
    let staged_manifest_path = out_path.join("manifest.json");
    // Drain per-PlayerState identity (Subject + SpawnedCharacter) captured
    // during the walk into a sorted players list for the manifest.
    let mut players: Vec<(u32, Option<String>, Option<u32>)> = channel_state
        .players()
        .iter()
        .filter(|(_, id)| id.subject.is_some())
        .map(|(&g, id)| (g, id.subject.clone(), id.character_net_guid))
        .collect();
    players.sort_unstable_by_key(|(g, _, _)| *g);
    manifest::write_manifest(
        &staged_manifest_path,
        vrf_path,
        file_size,
        &preamble,
        &cache,
        net_stats,
        total_packets,
        elapsed,
        &players,
    )?;

    // No handle remains open in staging at this point. Replace the destination
    // only after every table and the manifest are complete; a failed run before
    // here drops the guard and removes staging without touching the prior run.
    output.publish()?;
    let manifest_path = destination.join("manifest.json");

    summary::print(
        &destination,
        net_stats,
        &RunTotals {
            chunks_processed,
            total_packets,
            export_groups: cache.group_count(),
            movement_rows,
            net_guid_rows,
            event_rows,
            event_trailing_bytes,
            replay_data_trailing_bytes,
            elapsed,
            event_layout_mismatches,
            event_first_layout_mismatch,
            sink: sink_totals,
        },
        &error_report,
        with_checkpoints.then_some(&cp_stats),
        &manifest_path,
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{SinkTotals, typed_event_words};
    use vrf_decode::OverlayErrorReport;

    /// Every counter a packet's sink produced must survive the sink.
    ///
    /// `ExportSink` is rebuilt for each of a replay's ~530,000 packets, so a
    /// counter the driver does not read is dropped 530,000 times and reads as a
    /// permanent zero. `cnc_rpcs_emitted` was exactly that: the only evidence
    /// the AbilitiesAndBuffs brute-force produced RPC structure at all, never
    /// aggregated, so a build that stopped reaching that decoder would leave
    /// "Decode errors: 0" and every other line on the summary untouched.
    ///
    /// This is one accumulation point shared by the ReplayData pass and the
    /// checkpoint pass, so the same omission cannot be made twice.
    #[test]
    fn absorbing_a_packets_stats_keeps_every_counter() {
        let mut report = OverlayErrorReport::default();
        let mut totals = SinkTotals::default();

        for _ in 0..2 {
            let mut stats = crate::sink::ExportStats {
                effect_blobs_decoded: 1,
                struct_blobs_decoded: 2,
                struct_blobs_failed: 3,
                struct_blob_first_error: Some("blob boom".to_owned()),
                multi_contents_items_emitted: 4,
                movement_rpc_errors: 5,
                movement_first_error: Some("movement boom".to_owned()),
                truncated_rpcs: 6,
                rpc_suffix_bits_dropped: 7,
                cnc_rpcs_emitted: 8,
                array_leaf_decode_errors: 22,
                ..crate::sink::ExportStats::default()
            };
            stats.overlay.decoded_ok = 9;
            stats.overlay.decoded_err = 10;
            stats.overlay.raw_or_skip = 11;
            stats.overlay.not_in_table = 12;
            stats.overlay.no_field_name = 13;
            stats.overlay.handle_conflicts_refused = 14;
            stats.array.elements_decoded = 15;
            stats.array.fields_emitted = 16;
            stats.array.truncations = 17;
            stats.array.errors = 18;
            stats.array.unconsumed_nested_bits = 19;
            stats.array.implicit_terminations = 20;
            stats.array.unconsumed_root_bits = 21;
            totals.absorb(&mut stats, &mut report);
        }

        assert_eq!(totals.effect_blobs_decoded, 2);
        assert_eq!(totals.struct_blobs_decoded, 4);
        assert_eq!(totals.struct_blobs_failed, 6);
        assert_eq!(totals.multi_contents_items_emitted, 8);
        assert_eq!(totals.movement_rpc_errors, 10);
        assert_eq!(totals.truncated_rpcs, 12);
        assert_eq!(totals.rpc_suffix_bits_dropped, 14);
        assert_eq!(totals.cnc_rpcs_emitted, 16);
        assert_eq!(totals.array_leaf_decode_errors, 44);
        assert_eq!(totals.overlay.decoded_ok, 18);
        assert_eq!(totals.overlay.decoded_err, 20);
        assert_eq!(totals.overlay.raw_or_skip, 22);
        assert_eq!(totals.overlay.not_in_table, 24);
        assert_eq!(totals.overlay.no_field_name, 26);
        assert_eq!(totals.overlay.handle_conflicts_refused, 28);
        assert_eq!(totals.array.elements_decoded, 30);
        assert_eq!(totals.array.fields_emitted, 32);
        assert_eq!(totals.array.truncations, 34);
        assert_eq!(totals.array.errors, 36);
        assert_eq!(totals.array.unconsumed_nested_bits, 38);
        assert_eq!(totals.array.implicit_terminations, 40);
        assert_eq!(totals.array.unconsumed_root_bits, 42);
        // First error wins, so a later packet cannot overwrite the one that
        // names the build change.
        assert_eq!(totals.struct_blob_first_error.as_deref(), Some("blob boom"));
        assert_eq!(
            totals.movement_first_error.as_deref(),
            Some("movement boom")
        );
    }

    /// Build an Event payload: `[u32 tag][words][FString][f32 seconds]`.
    fn payload(tag: u32, words: &[u32], name: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&tag.to_le_bytes());
        for w in words {
            out.extend_from_slice(&w.to_le_bytes());
        }
        // Unreal's FString length counts the null terminator.
        let len = i32::try_from(name.len() + 1).expect("test name fits");
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        out.extend_from_slice(&1.5f32.to_le_bytes());
        out
    }

    /// A payload that matches the assumed word count yields its words.
    #[test]
    fn a_payload_matching_the_assumed_layout_yields_its_words() {
        let p = payload(
            3,
            &[0x1111_1111, 0x2222_2222],
            "EReplayEventGroup::CharacterDeath",
        );
        assert_eq!(
            typed_event_words(&p, 2),
            Some((Some(0x1111_1111), Some(0x2222_2222)))
        );
        let p1 = payload(3, &[0x3333_3333], "EReplayEventGroup::RoundStart");
        assert_eq!(typed_event_words(&p1, 1), Some((Some(0x3333_3333), None)));
    }

    /// A build that changes the word count must not export a plausible NetGUID
    /// read out of the following `FString`.
    ///
    /// The words were copied from fixed offsets with nothing checking that the
    /// rest of the payload agreed. `characterDeath` claims two words; a payload
    /// carrying one puts the FString's length prefix exactly where `word1` is
    /// read, and a length prefix is a small positive integer -- indistinguishable
    /// from a NetGUID in the exported column. No counter moved and no error was
    /// raised, because nothing had asked whether the layout still held.
    #[test]
    fn a_payload_that_does_not_fit_the_assumed_word_count_yields_nothing() {
        // One real word, but `characterDeath`'s assumed count is two.
        let p = payload(3, &[0x1111_1111], "EReplayEventGroup::CharacterDeath");
        assert_eq!(
            typed_event_words(&p, 2),
            None,
            "a payload one word short must refuse to name word1"
        );
        // ...and the reverse: three words where two were assumed.
        let p3 = payload(3, &[1, 2, 3], "EReplayEventGroup::CharacterDeath");
        assert_eq!(typed_event_words(&p3, 2), None);
    }

    /// A group that claims no words is not measured against a layout nobody
    /// established. `spikePlanted` and every unrecognised group land here, and
    /// they already export no words; validating them would only invent alarms
    /// about payload shapes this project has never claimed to know.
    #[test]
    fn a_group_claiming_no_words_is_not_checked() {
        assert_eq!(typed_event_words(&[], 0), Some((None, None)));
        assert_eq!(typed_event_words(&[0xAB; 3], 0), Some((None, None)));
    }

    /// A truncated payload cannot be verified, so it yields nothing rather than
    /// whatever `get()` happens to return.
    #[test]
    fn a_truncated_payload_yields_nothing() {
        assert_eq!(typed_event_words(&[0u8; 6], 1), None);
    }
}
