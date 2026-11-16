use std::collections::BTreeMap;
use vrfkit::{ReplayEvent, ReplayReader};
use vrfkit::gameplay::{GameplayError, GameplayKind, GameplaySource, native_event, round_phase_event};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os().nth(1).ok_or("missing replay")?;
    let mut rows = Vec::new();
    let mut ignored = BTreeMap::<String, u64>::new();
    let summary = ReplayReader::new().read_path(path, |event| {
        match event {
            ReplayEvent::ContainerEvent(raw) => {
                if let Some(event) = native_event(&raw)? {
                    let GameplaySource::NativeContainer(source) = event.source else { panic!() };
                    let (word0, word1) = match event.kind {
                        GameplayKind::RoundStarted { round_number } => (Some(round_number), None),
                        GameplayKind::CharacterDeath { killer, victim } => (Some(killer.net_guid), Some(victim.net_guid)),
                        _ => (None, None),
                    };
                    rows.push(format!("N\t{}\t{}\t{}\t{}\t{}\t{}", source.group, source.time1_ms,
                        source.time2_ms, source.payload_seconds.to_bits(),
                        word0.map(|x|x.to_string()).unwrap_or_else(||"null".into()),
                        word1.map(|x|x.to_string()).unwrap_or_else(||"null".into())));
                } else {
                    *ignored.entry(raw.group.clone()).or_default() += 1;
                }
            },
            ReplayEvent::Records(batch) => {
                for field in batch.fields {
                    if let Some(event) = round_phase_event(field)? {
                        let GameplayKind::RoundPhaseObserved(phase) = event.kind else { panic!() };
                        rows.push(format!("P\t{}\t{}\t{}\t{}\t{}", event.time_ms(), field.packet_id,
                            field.channel_index, field.actor_net_guid, phase.wire_value()));
                    }
                }
            },
            _ => {},
        }
        Ok::<(), GameplayError>(())
    })?;
    if summary.block_verdict() != vrfkit::ValidationVerdict::Passed
        || summary.decode_error_count != 0
        || summary.event_trailing_bytes != 0
        || summary.unknown_chunks != 0
        || summary.non_finite_frame_times != 0
        || summary.network.lost_content_blocks() != 0
        || summary.network.partial_errors != 0
        || summary.network.unfinished_partials != 0
        || summary.decode_counters.get("movement_rpc_errors") != Some(&0)
        || summary.decode_counters.get("movement_component_errors") != Some(&0)
    {
        return Err("Reader quality counters did not satisfy the public-fixture guard".into());
    }
    for row in rows { println!("{row}"); }
    for (group, count) in ignored { println!("U\t{group}\t{count}"); }
    println!("S\t{}\t{}", summary.event_chunks, summary.skipped_checkpoint_chunks);
    Ok(())
}
