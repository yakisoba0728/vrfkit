//! Copy into crates/vrfkit/examples/phase_a_reader_memory.rs to build.
//! The caller supplies the generated sparse fixture and an OS address limit.

#![forbid(unsafe_code)]

use std::convert::Infallible;
use vrfkit::{ReplayEvent, ReplayReader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next().ok_or("missing input mode")?;
    let path = args.next().ok_or("missing sparse fixture")?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let mut callbacks = 0;
    let mut events = 0;
    let mut consumer = |event: ReplayEvent<'_>| {
        callbacks += 1;
        events += usize::from(matches!(event, ReplayEvent::ContainerEvent(_)));
        Ok::<(), Infallible>(())
    };
    let summary = if mode == "file" {
        ReplayReader.read_path(path, &mut consumer)?
    } else if mode == "buffered" {
        let data = std::fs::read(path)?;
        ReplayReader.read(&data, &mut consumer)?
    } else {
        return Err("expected file or buffered".into());
    };
    assert_eq!(summary.unknown_chunks, 1);
    assert_eq!(summary.skipped_checkpoint_chunks, 1);
    assert_eq!(summary.event_chunks, 1);
    assert_eq!(summary.frames, 0);
    assert_eq!((callbacks, events), (2, 1));
    println!("callbacks=2 events=1 unknown_chunks=1 skipped_checkpoints=1 frames=0");
    Ok(())
}
