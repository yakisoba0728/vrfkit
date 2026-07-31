//! `inspect` subcommand — print replay info, header, and chunk summary.

use std::fs;

use vrf_container::{ChunkIterator, ChunkType, parse_preamble};

use crate::error::CliError;

pub fn run(path: &str) -> Result<(), CliError> {
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
    println!("  Friendly name:    {}", info.friendly_name);

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
    if unknown_count > 0 {
        println!("  Unknown:      {unknown_count:>6} chunks");
    }

    Ok(())
}
