//! `inspect` subcommand -- print replay info, header, and chunk summary.

use std::fs;

use vrf_container::{ChunkIterator, ChunkType, parse_preamble};

use crate::error::CliError;

fn friendly_name(value: &str, redact_identifiers: bool) -> &str {
    if redact_identifiers {
        "[redacted]"
    } else {
        value
    }
}

pub fn run(path: &str, redact_identifiers: bool) -> Result<(), CliError> {
    let data = fs::read(path)?;
    let preamble = parse_preamble(&data)?;

    let info = &preamble.info;
    let header = &preamble.header;
    let ver = &header.replay_version;

    println!("=== Replay Info ===");
    println!("  File size:        {} bytes", data.len());
    println!("  Duration:         {} ms", info.length_in_ms);
    println!("  Compressed:       {}", info.compressed);
    println!("  Encrypted:        {}", info.encrypted);
    println!(
        "  Friendly name:    {}",
        friendly_name(&info.friendly_name, redact_identifiers)
    );

    println!();
    println!("=== Header ===");
    println!(
        "  Replay version:   {}.{}.{} (changelist {})",
        ver.major, ver.minor, ver.patch, ver.changelist
    );
    println!("  Branch:           {}", ver.branch);
    println!("  Network version:  {}", header.network_version);
    println!(
        "  Engine net proto: {}",
        header.engine_network_protocol_version
    );
    println!(
        "  Game net proto:   {}",
        header.game_network_protocol_version
    );
    println!("  Flags:            0x{:04X}", header.flags);
    println!(
        "    HasStreamingFixes:      {}",
        header.flags & vrf_frame::FLAG_HAS_STREAMING_FIXES != 0
    );
    println!(
        "    GameSpecificFrameData:  {}",
        header.flags & vrf_frame::FLAG_GAME_SPECIFIC_FRAME_DATA != 0
    );
    println!("  Platform:         {}", header.platform);
    println!("  Levels:           {}", header.level_names_and_times.len());

    // Chunk summary
    println!();
    println!("=== Chunks ===");
    let mut iter = ChunkIterator::new(&data, preamble.remaining_offset);
    let mut replay_data_count = 0u32;
    let mut checkpoint_count = 0u32;
    let mut event_count = 0u32;
    let mut unknown_count = 0u32;
    let mut total_replay_data_bytes: u64 = 0;

    while let Some(chunk) = iter.next_chunk()? {
        match chunk.chunk_type {
            ChunkType::ReplayData => {
                replay_data_count += 1;
                total_replay_data_bytes += chunk.size_in_bytes as u64;
            }
            ChunkType::Checkpoint => checkpoint_count += 1,
            ChunkType::Event => event_count += 1,
            ChunkType::Unknown(_) => unknown_count += 1,
            ChunkType::Header => {} // already consumed
        }
    }

    println!("  ReplayData:   {replay_data_count:>6} chunks ({total_replay_data_bytes} bytes)");
    println!("  Checkpoint:   {checkpoint_count:>6} chunks");
    println!("  Event:        {event_count:>6} chunks");
    // Unconditional, zero included: this was the only chunk line that could
    // vanish, and "no line" is indistinguishable from "zero unknown chunks"
    // -- exactly the ambiguity a regression in `ChunkType::from_raw` needs to
    // hide behind.
    println!("  Unknown:      {unknown_count:>6} chunks");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::friendly_name;

    #[test]
    fn redaction_never_returns_the_replay_value() {
        let private = "identity-bearing replay label";
        let shown = friendly_name(private, true);
        assert_eq!(shown, "[redacted]");
        assert!(!shown.contains(private));
    }

    #[test]
    fn default_output_remains_backward_compatible() {
        assert_eq!(friendly_name("ordinary label", false), "ordinary label");
    }
}
