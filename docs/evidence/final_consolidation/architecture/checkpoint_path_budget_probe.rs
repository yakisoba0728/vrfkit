use vrf_schema::{NetGuidCache, SchemaError};
use vrf_schema::read_checkpoint_tables;

fn packed(out: &mut Vec<u8>, mut value: u32) {
    loop {
        let mut byte = ((value & 0x7f) as u8) << 1;
        value >>= 7;
        if value != 0 { byte |= 1; }
        out.push(byte);
        if value == 0 { break; }
    }
}

fn main() {
    const PATH_BYTES: usize = 256 * 1024;
    const ENTRIES: u32 = 1025;
    const LIMIT: usize = 256 * 1024 * 1024;
    let mut archive = vec![0_u8; 16];
    archive.extend_from_slice(&ENTRIES.to_le_bytes());
    for guid in 1..=ENTRIES {
        packed(&mut archive, guid);
        packed(&mut archive, 0);
        if guid == 1 {
            archive.push(1);
            archive.extend_from_slice(&((PATH_BYTES + 1) as i32).to_le_bytes());
            archive.resize(archive.len() + PATH_BYTES, b'x');
            archive.push(0);
        } else {
            archive.push(0);
            packed(&mut archive, 0);
        }
        archive.push(3);
    }
    archive.extend_from_slice(&0_u32.to_le_bytes());
    let frame_offset = (archive.len() - 8) as u32;
    archive[..4].copy_from_slice(&frame_offset.to_le_bytes());
    let mut cache = NetGuidCache::new();
    let result = read_checkpoint_tables(&archive, &mut cache);
    let accepted_entries = cache.net_guid_entries();
    let retained_bytes: usize = accepted_entries.iter().map(|entry| entry.path.len()).sum();
    println!("input_bytes={} entries={} accepted_entries={} retained_path_bytes={}",
        archive.len(), ENTRIES, accepted_entries.len(), retained_bytes);
    match result {
        Err(SchemaError::ResourceLimit { resource, requested, limit })
            if resource == "checkpoint resolved GUID path bytes"
                && requested == LIMIT + PATH_BYTES && limit == LIMIT => {
            assert_eq!(accepted_entries.len(), (ENTRIES - 1) as usize);
            assert_eq!(retained_bytes, LIMIT);
            assert!(cache.get_path_by_guid(ENTRIES).is_none());
            println!("PASS: refused amplification before final cache insertion");
        }
        _ => {
            println!("FAIL: expected explicit checkpoint path budget refusal");
            std::process::exit(1);
        }
    }
}
