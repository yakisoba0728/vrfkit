//! Count optional current body relations without printing player identifiers.

use std::collections::BTreeMap;

use vrfkit::gameplay::{IdentityError, IdentityResolution, MainIdentityTimeline, spawned_character_observation};
use vrfkit::{ReplayEvent, ReplayReader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let input = args.next().ok_or("usage: gameplay_identity <replay.vrf>")?;
    if args.next().is_some() { return Err("usage: gameplay_identity <replay.vrf>".into()); }
    let mut timeline = MainIdentityTimeline::new();
    let mut counts = BTreeMap::from([
        ("spawned_character_declarations", 0_u64),
        ("null_body_declarations", 0),
        ("movement_resolved", 0),
        ("movement_unresolved", 0),
        ("movement_ambiguous", 0),
        ("movement_null", 0),
    ]);
    let summary = ReplayReader::new().read_path(input, |event| {
        if let ReplayEvent::Records(records) = event {
            timeline.observe_records(&records)?;
            for field in records.fields {
                if let Some(observation) = spawned_character_observation(field)? {
                    *counts.get_mut("spawned_character_declarations").unwrap() += 1;
                    if observation.character.net_guid == 0 {
                        *counts.get_mut("null_body_declarations").unwrap() += 1;
                    }
                }
            }
            for index in 0..records.movement.len() {
                let key = match timeline.movement_identity(&records, index)?.identity {
                    IdentityResolution::Resolved(_) => "movement_resolved",
                    IdentityResolution::Ambiguous { .. } => "movement_ambiguous",
                    IdentityResolution::Null => "movement_null",
                    _ => "movement_unresolved",
                };
                *counts.get_mut(key).expect("declared counter") += 1;
            }
        }
        Ok::<(), IdentityError>(())
    })?;
    // A later failure must not publish a successful-looking partial aggregate.
    for (key, count) in counts { println!("{key}: {count}"); }
    println!("skipped_checkpoints: {}", summary.skipped_checkpoint_chunks);
    println!("block_verdict: {:?}", summary.block_verdict());
    Ok(())
}
