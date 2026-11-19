//! Stream exact local identity evidence to an independent comparator.
//! Contains actor identities. Publish only the comparator's aggregate receipt.

#![forbid(unsafe_code)]

use std::error::Error;
use std::io::{self, BufWriter, Write};

use vrfkit::gameplay::{IdentityResolution, MainIdentityTimeline, spawned_character_observation};
use vrfkit::{FieldRecord, ReplayEvent, ReplayReader, ValidationVerdict};

fn raw_hex(row: &FieldRecord) -> String {
    row.raw_bits
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|value| format!("{value:02x}"))
        .collect()
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("missing replay")?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let mut timeline = MainIdentityTimeline::new();
    let mut moves = 0_u64;
    let mut declarations = 0_u64;
    let summary = ReplayReader::new().read_path(path, |event| {
        if let ReplayEvent::Records(records) = event {
            timeline.observe_records(&records)?;
            for row in records.fields {
                if let Some(observed) = spawned_character_observation(row)? {
                    assert!(std::ptr::eq(observed.source, row));
                    writeln!(
                        out,
                        "D\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                        row.packet_id,
                        row.time_ms,
                        row.channel_index,
                        observed.player_state_net_guid,
                        observed.character.net_guid,
                        row.bit_count,
                        row.handle,
                        row.compatible_checksum.ok_or("declaration checksum")?,
                        row.group_path,
                        raw_hex(row),
                    )?;
                    declarations += 1;
                }
            }
            for index in 0..records.movement.len() {
                let linked = timeline.movement_identity(&records, index)?;
                assert!(std::ptr::eq(linked.movement, &records.movement[index]));
                assert!(std::ptr::eq(
                    linked.provenance,
                    &records.movement_provenance[index]
                ));
                let row = linked.movement;
                let provenance = linked.provenance;
                let source = provenance.source.ok_or("missing provenance")?;
                write!(
                    out,
                    "M\t{moves}\t{}\t{}\t{}\t{}\t{}\t{}",
                    row.time_ms,
                    row.packet_id,
                    row.character_net_guid,
                    row.timestamp,
                    row.movement_state,
                    row.move_type
                )?;
                for value in [
                    row.pos_x, row.pos_y, row.pos_z, row.yaw, row.pitch, row.vel_x, row.vel_y,
                    row.vel_z,
                ] {
                    write!(out, "\t{}", value.to_bits())?;
                }
                write!(
                    out,
                    "\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    source.initial_packet_id,
                    source.completion_packet_id,
                    source.channel_index,
                    source.actor_net_guid,
                    source
                        .object_net_guid
                        .map_or_else(|| "null".into(), |x| x.to_string()),
                    source.bunch_index,
                    source.block_index,
                    source.rpc_index,
                    source.rpc_handle,
                    provenance.array_ordinal,
                    provenance.update_ordinal,
                    provenance.update_index,
                    provenance.component_ordinal,
                    provenance.move_index
                )?;
                match linked.identity {
                    IdentityResolution::Null => write!(out, "\tnull")?,
                    IdentityResolution::Unresolved(reason) => {
                        write!(out, "\tunresolved\t{reason:?}")?
                    }
                    IdentityResolution::Ambiguous { declarations } => {
                        write!(out, "\tambiguous")?;
                        for row in declarations {
                            write!(
                                out,
                                "\t{}:{}:{}:{}",
                                row.actor_net_guid,
                                row.packet_id,
                                row.time_ms,
                                raw_hex(row)
                            )?;
                        }
                    }
                    IdentityResolution::Resolved(body) => {
                        assert_eq!(body.character_net_guid, row.character_net_guid);
                        let declaration = body.declaration;
                        write!(
                            out,
                            "\tresolved\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                            body.player_state_net_guid,
                            declaration.packet_id,
                            declaration.time_ms,
                            declaration.channel_index,
                            declaration.handle,
                            declaration.bit_count,
                            raw_hex(declaration),
                            body.player_state_open.packet_id,
                            body.player_state_open.time_ms,
                            body.player_state_open.channel_index,
                            body.character_open.packet_id,
                            body.character_open.time_ms,
                            body.character_open.channel_index
                        )?;
                    }
                    _ => return Err("unrecognized identity resolution".into()),
                }
                out.write_all(b"\n")?;
                moves += 1;
            }
        }
        Ok::<(), Box<dyn Error>>(())
    })?;
    assert_eq!(summary.movement_rows, moves);
    assert_eq!(summary.decode_error_count, 0);
    assert_eq!(summary.block_verdict(), ValidationVerdict::Passed);
    writeln!(
        out,
        "S\t{moves}\t{declarations}\t{}",
        summary.skipped_checkpoint_chunks
    )?;
    out.flush()?;
    Ok(())
}
