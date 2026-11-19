//! Read-only census of main-stream schema registrations on public fixtures.

#![forbid(unsafe_code)]

use vrf_container::{
    ChunkIterator, ChunkType, decompress_checkpoint, decompress_replay_data,
    parse_checkpoint_chunk, parse_preamble,
};
use vrf_schema::NetGuidCache;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os().nth(1).ok_or("expected replay path")?;
    let bytes = std::fs::read(path)?;
    let preamble = parse_preamble(&bytes)?;
    let mut chunks = ChunkIterator::new(&bytes, preamble.remaining_offset);
    let mut cache = NetGuidCache::new();
    let mut frames = 0_u64;
    let mut packets = 0_u64;
    let mut checkpoints = 0_u64;
    let mut checkpoint_declarations = 0_u64;
    let mut checkpoint_groups = 0_u64;
    let mut checkpoint_replacements = 0_u64;
    while let Some(chunk) = chunks.next_chunk()? {
        let payload = &bytes[chunk.data_offset..][..chunk.size_in_bytes as usize];
        if chunk.chunk_type == ChunkType::Checkpoint {
            let checkpoint = parse_checkpoint_chunk(payload)?;
            let data = decompress_checkpoint(
                checkpoint.archive,
                preamble.info.compressed,
                preamble.info.encrypted,
            )?;
            let mut checkpoint_cache = NetGuidCache::new();
            let tables = vrf_schema::read_checkpoint_tables(&data, &mut checkpoint_cache)?;
            let _ = vrf_frame::walk_demo_frames(
                &data[tables.frame_offset..],
                preamble.header.flags,
                &mut checkpoint_cache,
                |_, _| {},
            )?;
            checkpoints += 1;
            checkpoint_declarations += checkpoint_cache.schema_generation();
            checkpoint_groups += checkpoint_cache.group_count() as u64;
            checkpoint_replacements += checkpoint_cache.replaced_export_groups();
            continue;
        }
        if chunk.chunk_type != ChunkType::ReplayData {
            continue;
        }
        let data =
            decompress_replay_data(payload, preamble.info.compressed, preamble.info.encrypted)?;
        let walk =
            vrf_frame::walk_demo_frames(&data, preamble.header.flags, &mut cache, |_, _| {})?;
        frames += u64::from(walk.frames);
        packets += u64::from(walk.packets);
    }
    println!(
        "{{\"frames\":{},\"packets\":{},\"declarations\":{},\"retained_groups\":{},\"replacements\":{},\"declared_slots\":{},\"checkpoints\":{},\"checkpoint_declarations\":{},\"checkpoint_groups\":{},\"checkpoint_replacements\":{}}}",
        frames,
        packets,
        cache.schema_generation(),
        cache.group_count(),
        cache.replaced_export_groups(),
        cache.field_slot_count(),
        checkpoints,
        checkpoint_declarations,
        checkpoint_groups,
        checkpoint_replacements,
    );
    Ok(())
}
