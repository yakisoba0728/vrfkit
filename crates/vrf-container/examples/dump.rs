//! Print the parsed preamble and chunk index for one replay.

use std::error::Error;
use std::path::PathBuf;

use vrf_container::{ChunkIterator, ChunkType, parse_preamble};

const DEFAULT_REPLAY: &str = "02d4d478-1dfb-4412-9a77-29ca29105a9d.vrf";

fn resolve_replay() -> Result<PathBuf, &'static str> {
    if let Some(arg) = std::env::args_os().nth(1) {
        return Ok(PathBuf::from(arg));
    }
    if let Some(dir) = std::env::var_os("VRFKIT_CORPUS_DIR") {
        return Ok(PathBuf::from(dir).join(DEFAULT_REPLAY));
    }
    Err("usage: dump <path-to-replay.vrf> (or set VRFKIT_CORPUS_DIR)")
}

fn main() -> Result<(), Box<dyn Error>> {
    let path = resolve_replay().map_err(std::io::Error::other)?;
    let data = std::fs::read(&path)?;
    let preamble = parse_preamble(&data)?;

    println!("file: {}", path.display());
    println!("bytes: {}", data.len());
    println!("friendly name: {}", preamble.info.friendly_name);
    println!("duration ms: {}", preamble.info.length_in_ms);
    println!("compressed: {}", preamble.info.compressed);
    println!("encrypted: {}", preamble.info.encrypted);
    println!("branch: {}", preamble.header.replay_version.branch);
    println!("network version: {}", preamble.header.network_version);
    println!(
        "engine protocol: {}",
        preamble.header.engine_network_protocol_version
    );
    println!("flags: 0x{:08X}", preamble.header.flags);
    println!("platform: {}", preamble.header.platform);
    println!("header trailing bytes: {}", preamble.header.trailing_bytes);

    let mut counts = [0u64; 5];
    let mut chunks = ChunkIterator::new(&data, preamble.remaining_offset);
    while let Some(chunk) = chunks.next_chunk()? {
        let slot = match chunk.chunk_type {
            ChunkType::Header => 0,
            ChunkType::ReplayData => 1,
            ChunkType::Checkpoint => 2,
            ChunkType::Event => 3,
            ChunkType::Unknown(_) => 4,
        };
        counts[slot] += 1;
    }

    println!("replay-data chunks: {}", counts[1]);
    println!("checkpoint chunks: {}", counts[2]);
    println!("event chunks: {}", counts[3]);
    println!("additional header chunks: {}", counts[0]);
    println!("unknown chunks: {}", counts[4]);
    Ok(())
}
