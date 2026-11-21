//! Count independent checkpoint callbacks without writing identifiers or rows.

use std::convert::Infallible;

use vrfkit::{CheckpointEvent, ReadOptions, ReplayEvent, ReplayReader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::args_os()
        .nth(1)
        .ok_or("usage: stream_checkpoints <replay.vrf>")?;
    let mut completed = 0_u64;
    let summary = ReplayReader.read_path_with_options(
        input,
        ReadOptions::default().with_checkpoints(true),
        |event| {
            if let ReplayEvent::Checkpoint(CheckpointEvent::Summary(_)) = event {
                completed += 1;
            }
            Ok::<(), Infallible>(())
        },
    )?;
    assert_eq!(completed, summary.checkpoints.chunks);
    println!("main fields: {}", summary.field_rows);
    println!("checkpoints: {}", completed);
    println!("checkpoint fields: {}", summary.checkpoints.field_rows);
    println!("checkpoint actors: {}", summary.checkpoints.actor_rows);
    println!("checkpoint movement: {}", summary.checkpoints.movement_rows);
    println!("checkpoint partials: {}", summary.checkpoints.partial_rows);
    Ok(())
}
