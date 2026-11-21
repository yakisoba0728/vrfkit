//! Count optional gameplay projections without printing player identities.

use std::collections::BTreeMap;

use vrfkit::gameplay::{GameplayError, GameplayKind, RoundPhase, native_event, round_phase_event};
use vrfkit::{ReplayEvent, ReplayReader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let input = args.next().ok_or("usage: gameplay_events <replay.vrf>")?;
    if args.next().is_some() {
        return Err("usage: gameplay_events <replay.vrf>".into());
    }
    let mut counts = BTreeMap::from([
        ("native_round_started", 0_u64),
        ("native_character_death", 0),
        ("native_spike_planted", 0),
        ("native_spike_defused", 0),
        ("native_spike_exploded", 0),
        ("native_unprojected", 0),
        ("derived_phase_observations", 0),
        ("derived_unknown_phase", 0),
    ]);
    let summary = ReplayReader::new().read_path(input, |event| {
        match event {
            ReplayEvent::ContainerEvent(raw) => {
                let key = match native_event(&raw)?.map(|event| event.kind) {
                    Some(GameplayKind::RoundStarted { .. }) => "native_round_started",
                    Some(GameplayKind::CharacterDeath { .. }) => "native_character_death",
                    Some(GameplayKind::SpikePlanted) => "native_spike_planted",
                    Some(GameplayKind::SpikeDefused) => "native_spike_defused",
                    Some(GameplayKind::SpikeExploded) => "native_spike_exploded",
                    _ => "native_unprojected",
                };
                *counts.get_mut(key).expect("declared counter") += 1;
            }
            ReplayEvent::Records(batch) => {
                for field in batch.fields {
                    if let Some(projected) = round_phase_event(field)? {
                        *counts.get_mut("derived_phase_observations").unwrap() += 1;
                        if matches!(
                            projected.kind,
                            GameplayKind::RoundPhaseObserved(RoundPhase::Unknown(_))
                        ) {
                            *counts.get_mut("derived_unknown_phase").unwrap() += 1;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok::<(), GameplayError>(())
    })?;
    // Publish only after the complete read and every requested projection pass.
    for (key, count) in counts {
        println!("{key}: {count}");
    }
    println!("skipped_checkpoints: {}", summary.skipped_checkpoint_chunks);
    println!("block_verdict: {:?}", summary.block_verdict());
    Ok(())
}
