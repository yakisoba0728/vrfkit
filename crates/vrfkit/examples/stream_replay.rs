//! Print aggregate counts without retaining decoded rows or writing Parquet.

#![forbid(unsafe_code)]

use std::convert::Infallible;

use vrfkit::{ReplayEvent, ReplayReader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: stream_replay <file.vrf>")?;
    let mut fields = 0_u64;
    let mut raw_bytes = 0_u64;
    let summary = ReplayReader::new().read_path(path, |event| {
        if let ReplayEvent::Records(batch) = event {
            fields += batch.fields.len() as u64;
            raw_bytes += batch
                .fields
                .iter()
                .filter_map(|field| field.raw_bits.as_ref())
                .map(|bits| bits.len() as u64)
                .sum::<u64>();
        }
        Ok::<(), Infallible>(())
    })?;
    assert_eq!(fields, summary.field_rows);
    println!(
        "packets={} fields={} movement={} actors={} events={} partials={} raw_field_bytes={} skipped_checkpoints={} block_verdict={:?}",
        summary.packets,
        fields,
        summary.movement_rows,
        summary.actor_rows,
        summary.event_chunks,
        summary.partial_rows,
        raw_bytes,
        summary.skipped_checkpoint_chunks,
        summary.block_verdict(),
    );
    println!(
        "decode_errors={} movement_errors={} partial_rejections={} lost_blocks={}",
        summary.decode_error_count,
        summary.decode_counters["movement_rpc_errors"],
        summary.network.partial_errors,
        summary.network.lost_content_blocks(),
    );
    Ok(())
}
