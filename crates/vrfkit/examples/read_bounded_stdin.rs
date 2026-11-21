//! Decode finite stdin after EOF under a required encoded-byte cap.

use std::convert::Infallible;
use std::io;

use vrfkit::{ReadOptions, ReplayEvent, ReplayReader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let limit: usize = args
        .next()
        .ok_or("usage: read_bounded_stdin <max-input-bytes> [--checkpoints]")?
        .parse()?;
    let checkpoints = match args.next().as_deref() {
        None => false,
        Some("--checkpoints") => true,
        Some(_) => return Err("expected --checkpoints or no second argument".into()),
    };
    if args.next().is_some() {
        return Err("too many arguments".into());
    }

    let mut fields = 0_u64;
    let summary = ReplayReader.read_bounded(
        &mut io::stdin().lock(),
        limit,
        ReadOptions::default().with_checkpoints(checkpoints),
        |event| {
            // The entire encoded input has been buffered and EOF observed.
            // Parser callbacks still remain provisional until the call succeeds.
            if let ReplayEvent::Records(records) = event {
                fields += records.fields.len() as u64;
            }
            Ok::<(), Infallible>(())
        },
    )?;
    assert_eq!(fields, summary.field_rows);
    println!("main fields: {}", fields);
    println!("completed checkpoints: {}", summary.checkpoints.chunks);
    println!("skipped checkpoints: {}", summary.skipped_checkpoint_chunks);
    println!("checkpoint fields: {}", summary.checkpoints.field_rows);
    println!("block verdict: {:?}", summary.block_verdict());
    Ok(())
}
