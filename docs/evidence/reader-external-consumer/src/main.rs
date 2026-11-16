//! External path-dependency consumer of the documented main-stream Reader API.

#![forbid(unsafe_code)]

use std::convert::Infallible;

use vrfkit::{ReplayEvent, ReplayReader, ValidationVerdict};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("expected a public fixture path")?;
    let expected_checkpoints: u64 = args
        .next()
        .ok_or("expected the independently observed checkpoint count")?
        .to_str()
        .ok_or("checkpoint count is not UTF-8")?
        .parse()?;
    if args.next().is_some() {
        return Err("unexpected extra argument".into());
    }

    let mut unknown = 0_u64;
    let mut fields = 0_u64;
    let mut movement = 0_u64;
    let mut actors = 0_u64;
    let mut partials = 0_u64;
    let mut metadata_events = 0_u64;
    let mut container_events = 0_u64;
    let summary = {
        let reader = ReplayReader::new();
        reader.read_path(&path, |event| {
            match event {
                ReplayEvent::Metadata(_) => metadata_events += 1,
                ReplayEvent::Records(batch) => {
                    fields += batch.fields.len() as u64;
                    movement += batch.movement.len() as u64;
                    actors += batch.actors.len() as u64;
                    partials += batch.partials.len() as u64;
                    // This is the documented unknown-overlay example.
                    for field in batch.fields {
                        if field.value_i64.is_none()
                            && field.value_f64.is_none()
                            && field.value_bool.is_none()
                            && field.value_str.is_none()
                        {
                            unknown += 1;
                        }
                    }
                }
                ReplayEvent::ContainerEvent(_) => container_events += 1,
                _ => {}
            }
            Ok::<(), Infallible>(())
        })?
    };
    // Both the reader and its internal input buffer are gone. The summary
    // remains available, independently of the borrowed callback records.
    assert!(
        fields > 0,
        "the fixture must exercise decoded field callbacks"
    );
    assert_eq!(fields, summary.field_rows);
    assert_eq!(movement, summary.movement_rows);
    assert_eq!(actors, summary.actor_rows);
    assert_eq!(partials, summary.partial_rows);
    assert_eq!(container_events, summary.event_chunks);
    assert_eq!(metadata_events, 1);
    assert_eq!(summary.skipped_checkpoint_chunks, expected_checkpoints);
    assert_eq!(summary.block_verdict(), ValidationVerdict::Passed);
    assert_eq!(
        summary.metadata.header.replay_version.branch,
        "++Ares-Core+release-13.00"
    );

    println!(
        "{{\"callback_field_rows\":{},\"summary_field_rows\":{},\"movement_rows\":{},\"actor_rows\":{},\"partial_rows\":{},\"metadata_events\":{},\"container_events\":{},\"unknown_overlay_rows\":{},\"skipped_checkpoint_chunks\":{},\"expected_checkpoint_chunks\":{},\"packets\":{},\"frames\":{},\"decode_error_count\":{},\"block_verdict\":\"{:?}\"}}",
        fields,
        summary.field_rows,
        movement,
        actors,
        partials,
        metadata_events,
        container_events,
        unknown,
        summary.skipped_checkpoint_chunks,
        expected_checkpoints,
        summary.packets,
        summary.frames,
        summary.decode_error_count,
        summary.block_verdict(),
    );
    Ok(())
}
