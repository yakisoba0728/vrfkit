//! Compare file and buffered Reader input with the same executable.

#![forbid(unsafe_code)]

use std::convert::Infallible;
use std::fmt::Write;

use vrfkit::{ReplayEvent, ReplayReader};

struct Fingerprint {
    hash: u64,
    bytes: u64,
}

impl Write for Fingerprint {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        for byte in value.bytes() {
            self.hash ^= u64::from(byte);
            self.hash = self.hash.wrapping_mul(0x100000001b3);
        }
        self.bytes += value.len() as u64;
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next().ok_or("expected file or buffered")?;
    let path = args.next().ok_or("expected replay path")?;
    let fingerprint = match args.next() {
        None => false,
        Some(flag) if flag == "--fingerprint" => true,
        _ => return Err("expected optional --fingerprint".into()),
    };
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let mut digest = Fingerprint {
        hash: 0xcbf29ce484222325,
        bytes: 0,
    };
    let mut rows = 0_u64;
    let mut raw_bytes = 0_u64;
    let mut on_event = |event: ReplayEvent<'_>| {
        if fingerprint {
            writeln!(&mut digest, "{event:?}").unwrap();
        }
        if let ReplayEvent::Records(batch) = event {
            rows += (batch.fields.len()
                + batch.movement.len()
                + batch.actors.len()
                + batch.partials.len()) as u64;
            for field in batch.fields {
                raw_bytes += field.raw_bits.as_ref().map_or(0, |bits| bits.len()) as u64;
                if fingerprint {
                    write!(&mut digest, "{:?}", field.value_f64.map(f64::to_bits)).unwrap();
                }
            }
            if fingerprint {
                // Debug renders finite floats exactly, but NaN payload bits
                // need an explicit representation for an input comparison.
                for row in batch.movement {
                    for value in [
                        row.pos_x, row.pos_y, row.pos_z, row.yaw, row.pitch, row.vel_x, row.vel_y,
                        row.vel_z,
                    ] {
                        write!(&mut digest, "{:08x}", value.to_bits()).unwrap();
                    }
                }
                for row in batch.actors {
                    for value in [
                        row.spawn_x,
                        row.spawn_y,
                        row.spawn_z,
                        row.spawn_pitch,
                        row.spawn_yaw,
                        row.spawn_roll,
                        row.spawn_vx,
                        row.spawn_vy,
                        row.spawn_vz,
                    ] {
                        write!(&mut digest, "{:?}", value.map(f32::to_bits)).unwrap();
                    }
                }
            }
        }
        Ok::<(), Infallible>(())
    };
    let summary = if mode == "file" {
        ReplayReader.read_path(path, &mut on_event)?
    } else if mode == "buffered" {
        let data = std::fs::read(path)?;
        ReplayReader.read(&data, &mut on_event)?
    } else {
        return Err("mode must be file or buffered".into());
    };
    if fingerprint {
        writeln!(&mut digest, "{summary:?}").unwrap();
        let header = &summary.metadata.header;
        for value in [
            header.min_record_hz,
            header.max_record_hz,
            header.frame_limit_in_ms,
            header.checkpoint_limit_in_ms,
        ] {
            write!(&mut digest, "{:08x}", value.to_bits()).unwrap();
        }
    }
    println!(
        "rows={rows} raw_bytes={raw_bytes} fields={} movement={} actors={} partials={} events={} skipped_checkpoints={} packets={} frames={} lost_blocks={} decode_errors={} fingerprint_bytes={} fingerprint_fnv1a64={:016x}",
        summary.field_rows,
        summary.movement_rows,
        summary.actor_rows,
        summary.partial_rows,
        summary.event_chunks,
        summary.skipped_checkpoint_chunks,
        summary.packets,
        summary.frames,
        summary.network.lost_content_blocks(),
        summary.decode_error_count,
        digest.bytes,
        digest.hash,
    );
    Ok(())
}
