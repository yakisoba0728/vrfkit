//! Stream movement plus exact decode context as versioned JSONL on stdout.
//! Rows are provisional until the final Reader summary is written. This local
//! diagnostic output contains character identities; do not publish raw rows.

#![forbid(unsafe_code)]

use std::io::{self, BufWriter, Write};

use vrfkit::{MovementProvenance, MovementRecord, ReplayEvent, ReplayReader};

fn write_move(
    out: &mut impl Write,
    movement: &MovementRecord,
    context: &MovementProvenance,
) -> io::Result<()> {
    let source = context.source.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "movement has no transport source",
        )
    })?;
    if source.completion_packet_id != movement.packet_id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "movement and source completion packets differ",
        ));
    }
    let values = [
        movement.pos_x,
        movement.pos_y,
        movement.pos_z,
        movement.yaw,
        movement.pitch,
        movement.vel_x,
        movement.vel_y,
        movement.vel_z,
    ];
    if !values.iter().all(|value| value.is_finite()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "movement contains a non-finite value",
        ));
    }
    write!(
        out,
        concat!(
            "{{\"type\":\"move\",\"initial_packet_id\":{},\"completion_packet_id\":{},",
            "\"channel_index\":{},\"actor_net_guid\":{},\"object_net_guid\":"
        ),
        source.initial_packet_id,
        source.completion_packet_id,
        source.channel_index,
        source.actor_net_guid,
    )?;
    match source.object_net_guid {
        Some(guid) => write!(out, "{guid}")?,
        None => out.write_all(b"null")?,
    }
    write!(
        out,
        concat!(
            ",\"bunch_index\":{},\"block_index\":{},\"rpc_index\":{},\"rpc_handle\":{},",
            "\"array_ordinal\":{},\"update_ordinal\":{},\"update_index\":{},",
            "\"component_ordinal\":{},\"move_index\":{},\"character_net_guid\":{},",
            "\"timestamp\":{},\"movement_state\":{},\"move_type\":{}"
        ),
        source.bunch_index,
        source.block_index,
        source.rpc_index,
        source.rpc_handle,
        context.array_ordinal,
        context.update_ordinal,
        context.update_index,
        context.component_ordinal,
        context.move_index,
        movement.character_net_guid,
        movement.timestamp,
        movement.movement_state,
        movement.move_type,
    )?;
    for (name, value) in [
        "pos_x", "pos_y", "pos_z", "yaw", "pitch", "vel_x", "vel_y", "vel_z",
    ]
    .iter()
    .zip(values)
    {
        // Expanding the existing f32 exactly to f64 lets JSON readers recover
        // its stored value without confusing short f32 formatting with f64.
        write!(out, ",\"{name}\":{:?}", f64::from(value))?;
    }
    out.write_all(b"}\n")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("usage: movement_provenance <file.vrf>")?;
    if args.next().is_some() {
        return Err("usage: movement_provenance <file.vrf>".into());
    }
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    out.write_all(
        b"{\"type\":\"movement_provenance_header\",\"schema_version\":1,\"float_precision\":\"f32\"}\n",
    )?;
    let summary = ReplayReader::new().read_path(path, |event| {
        if let ReplayEvent::Records(batch) = event {
            if batch.movement.len() != batch.movement_provenance.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "movement and provenance row counts differ",
                ));
            }
            for (movement, context) in batch.movement.iter().zip(batch.movement_provenance) {
                write_move(&mut out, movement, context)?;
            }
        }
        Ok::<(), io::Error>(())
    })?;
    writeln!(
        out,
        concat!(
            "{{\"type\":\"movement_provenance_summary\",\"movement_rows\":{},",
            "\"movement_rpc_errors\":{},\"lost_content_blocks\":{},",
            "\"partial_errors\":{},\"unfinished_partials\":{},",
            "\"block_verdict\":\"{:?}\",\"skipped_checkpoint_chunks\":{}}}"
        ),
        summary.movement_rows,
        summary.decode_counters["movement_rpc_errors"],
        summary.network.lost_content_blocks(),
        summary.network.partial_errors,
        summary.network.unfinished_partials,
        summary.block_verdict(),
        summary.skipped_checkpoint_chunks,
    )?;
    out.flush()?;
    Ok(())
}
