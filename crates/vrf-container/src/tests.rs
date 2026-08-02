//! Unit and integration tests for the container parser.

use super::*;

// ===============================================================================
// Test helpers -- synthetic byte builders mirroring the C# test patterns
// ===============================================================================

mod helpers {
    pub fn add_u16(buf: &mut Vec<u8>, v: u16) {
        buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn add_u32(buf: &mut Vec<u8>, v: u32) {
        buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn add_i32(buf: &mut Vec<u8>, v: i32) {
        buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn add_i64(buf: &mut Vec<u8>, v: i64) {
        buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn add_f32(buf: &mut Vec<u8>, v: f32) {
        buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn add_fstring(buf: &mut Vec<u8>, s: &str) {
        let encoded: Vec<u8> = s.bytes().chain(std::iter::once(0u8)).collect();
        add_i32(buf, encoded.len() as i32);
        buf.extend_from_slice(&encoded);
    }

    pub fn add_guid(buf: &mut Vec<u8>, a: u32, b: u32, c: u32, d: u32) {
        add_u32(buf, a);
        add_u32(buf, b);
        add_u32(buf, c);
        add_u32(buf, d);
    }

    pub fn add_byte_array(buf: &mut Vec<u8>, data: &[u8]) {
        add_i32(buf, data.len() as i32);
        buf.extend_from_slice(data);
    }

    /// Build a minimal replay info section with the standard custom version.
    #[allow(clippy::too_many_arguments)]
    pub fn build_replay_info(
        magic: u32,
        file_version: u32,
        include_custom_version: bool,
        custom_version_value: i32,
        length_in_ms: i32,
        network_version: u32,
        changelist: u32,
        friendly_name: &str,
        is_live: bool,
        timestamp: i64,
        compressed: bool,
        encrypted: bool,
        encryption_key: &[u8],
    ) -> Vec<u8> {
        let mut buf = Vec::new();
        add_u32(&mut buf, magic);
        add_u32(&mut buf, file_version);

        if file_version >= 7 {
            if include_custom_version {
                add_i32(&mut buf, 1);
                add_guid(&mut buf, 0x95A4_F03E, 0x7E0B_49E4, 0xBA43_D356, 0x94FF_87D9);
                add_i32(&mut buf, custom_version_value);
            } else {
                add_i32(&mut buf, 0);
            }
        }

        add_i32(&mut buf, length_in_ms);
        add_u32(&mut buf, network_version);
        add_u32(&mut buf, changelist);
        add_fstring(&mut buf, friendly_name);
        add_u32(&mut buf, if is_live { 1 } else { 0 });
        add_i64(&mut buf, timestamp);
        add_u32(&mut buf, if compressed { 1 } else { 0 });
        add_u32(&mut buf, if encrypted { 1 } else { 0 });
        add_byte_array(&mut buf, encryption_key);

        buf
    }

    /// Default replay info: valid, uncompressed, unencrypted.
    pub fn default_replay_info() -> Vec<u8> {
        build_replay_info(
            0x43F4_EFDD,
            7,
            true,
            7,
            60000,
            19,
            1234,
            "Match",
            false,
            42,
            false,
            false,
            &[],
        )
    }

    /// Build a raw chunk (type + size + payload).
    pub fn build_chunk(chunk_type: u32, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        add_u32(&mut buf, chunk_type);
        add_i32(&mut buf, payload.len() as i32);
        buf.extend_from_slice(payload);
        buf
    }

    /// Build a valid header chunk payload.
    pub fn build_header_payload() -> Vec<u8> {
        build_header_payload_custom(0, &[3, 0, 0, 0, 49, 56, 0])
    }

    pub fn build_header_payload_custom(custom_version_count: i32, valorant_skip: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        add_u32(&mut buf, 0x2CF5_A13D); // NetworkMagic
        add_u32(&mut buf, 19); // NetworkVersion
        add_i32(&mut buf, custom_version_count);
        // Each custom version entry is 20 bytes
        for _ in 0..custom_version_count {
            buf.extend_from_slice(&[0u8; 20]);
        }
        add_u32(&mut buf, 0x1122_3344); // NetworkChecksum
        add_u32(&mut buf, 32); // EngineNetworkProtocolVersion
        add_u32(&mut buf, 0x5566_7788); // GameNetworkProtocolVersion
        add_guid(&mut buf, 0x0011_2233, 0x4455_6677, 0x8899_AABB, 0xCCDD_EEFF);

        // ReplayVersion
        add_u16(&mut buf, 12); // Major
        add_u16(&mut buf, 10); // Minor
        add_u16(&mut buf, 1); // Patch
        add_u32(&mut buf, 123456); // Changelist
        add_fstring(&mut buf, "++Ares-Core+release-12.10");

        // ValorantSkipByteCount + skip bytes
        buf.extend_from_slice(valorant_skip);

        // UE versions
        add_u32(&mut buf, 1001); // UE4Version
        add_u32(&mut buf, 1002); // UE5Version
        add_u32(&mut buf, 1003); // PackageVersionLicense

        // LevelNamesAndTimes: 1 entry
        add_i32(&mut buf, 1);
        add_fstring(&mut buf, "Ascent");
        add_u32(&mut buf, 42);

        // Flags
        add_u32(&mut buf, 0b1010); // HasStreamingFixes | GameSpecificFrameData

        // GameSpecificData: 2 entries
        add_i32(&mut buf, 2);
        add_fstring(&mut buf, "valorant");
        add_fstring(&mut buf, "competitive");

        // Recording params
        add_f32(&mut buf, 15.0);
        add_f32(&mut buf, 30.0);
        add_f32(&mut buf, 33.3);
        add_f32(&mut buf, 250.0);

        // Platform + build info
        add_fstring(&mut buf, "Windows");
        buf.push(7); // BuildConfig
        buf.push(3); // BuildTargetType (Client)

        buf
    }
}

// ===============================================================================
// ReplayInfo tests
// ===============================================================================

#[test]
fn info_empty_input_rejected() {
    let result = info::parse_replay_info(&[]);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::Truncated { .. }
    ));
}

#[test]
fn info_bad_magic_rejected() {
    let data = helpers::build_replay_info(
        0xDEAD_BEEF,
        7,
        true,
        7,
        60000,
        19,
        1234,
        "R",
        false,
        42,
        false,
        false,
        &[],
    );
    let result = info::parse_replay_info(&data);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::FileMagicMismatch {
            actual: 0xDEAD_BEEF
        }
    ));
}

#[test]
fn info_bad_file_version_rejected() {
    let mut data = Vec::new();
    helpers::add_u32(&mut data, 0x43F4_EFDD);
    helpers::add_u32(&mut data, 6); // wrong version
    // Don't need more -- should fail at version check
    // Add enough bytes for the parse to reach the check
    data.extend_from_slice(&[0u8; 100]);
    let result = info::parse_replay_info(&data);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::UnsupportedFileVersion { actual: 6 }
    ));
}

#[test]
fn info_missing_custom_version_rejected() {
    let data = helpers::build_replay_info(
        0x43F4_EFDD,
        7,
        false,
        7,
        60000,
        19,
        1234,
        "R",
        false,
        42,
        false,
        false,
        &[],
    );
    let result = info::parse_replay_info(&data);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::MissingLocalReplayVersion
    ));
}

#[test]
fn info_newer_custom_version_rejected() {
    let data = helpers::build_replay_info(
        0x43F4_EFDD,
        7,
        true,
        8,
        60000,
        19,
        1234,
        "R",
        false,
        42,
        false,
        false,
        &[],
    );
    let result = info::parse_replay_info(&data);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::UnsupportedLocalReplayVersion { actual: 8 }
    ));
}

#[test]
fn info_older_custom_version_rejected() {
    let data = helpers::build_replay_info(
        0x43F4_EFDD,
        7,
        true,
        6,
        60000,
        19,
        1234,
        "R",
        false,
        42,
        false,
        false,
        &[],
    );
    let result = info::parse_replay_info(&data);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::UnsupportedLocalReplayVersion { actual: 6 }
    ));
}

#[test]
fn info_valid_parses_summary() {
    let data = helpers::build_replay_info(
        0x43F4_EFDD,
        7,
        true,
        7,
        60000,
        19,
        1234,
        "Match  ",
        false,
        123456789,
        false,
        false,
        &[],
    );
    let (info, _offset) = info::parse_replay_info(&data).unwrap();
    assert_eq!(info.length_in_ms, 60000);
    assert_eq!(info.network_version, 19);
    assert_eq!(info.changelist, 1234);
    assert_eq!(info.friendly_name, "Match"); // trimmed
    assert!(!info.is_live);
    assert_eq!(info.timestamp, 123456789);
    assert!(!info.compressed);
    assert!(!info.encrypted);
    assert!(info.encryption_key.is_empty());
}

#[test]
fn info_completed_encrypted_without_key_rejected() {
    let data = helpers::build_replay_info(
        0x43F4_EFDD,
        7,
        true,
        7,
        60000,
        19,
        1234,
        "R",
        false,
        42,
        false,
        true,
        &[],
    );
    let result = info::parse_replay_info(&data);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::EncryptedWithoutKey
    ));
}

#[test]
fn info_truncated_input_rejected() {
    // Just the magic, then cut off
    let data = 0x43F4_EFDDu32.to_le_bytes();
    let result = info::parse_replay_info(&data);
    assert!(result.is_err());
}

// ===============================================================================
// ReplayHeader tests
// ===============================================================================

#[test]
fn header_valid_parses_all_fields() {
    let payload = helpers::build_header_payload();
    let header = header::parse_replay_header(&payload).unwrap();

    assert_eq!(header.network_version, 19);
    assert_eq!(header.network_checksum, 0x1122_3344);
    assert_eq!(header.engine_network_protocol_version, 32);
    assert_eq!(header.game_network_protocol_version, 0x5566_7788);
    assert_eq!(
        header.guid,
        [0x0011_2233, 0x4455_6677, 0x8899_AABB, 0xCCDD_EEFF]
    );
    assert_eq!(header.replay_version.major, 12);
    assert_eq!(header.replay_version.minor, 10);
    assert_eq!(header.replay_version.patch, 1);
    assert_eq!(header.replay_version.changelist, 123456);
    assert_eq!(header.replay_version.branch, "++Ares-Core+release-12.10");
    assert_eq!(header.ue4_version, 1001);
    assert_eq!(header.ue5_version, 1002);
    assert_eq!(header.package_version_license, 1003);
    assert_eq!(
        header.level_names_and_times,
        vec![("Ascent".to_string(), 42)]
    );
    assert_eq!(header.flags, 0b1010);
    assert_eq!(
        header.game_specific_data,
        vec!["valorant".to_string(), "competitive".to_string()]
    );
    assert_eq!(header.min_record_hz, 15.0);
    assert_eq!(header.max_record_hz, 30.0);
    assert!((header.frame_limit_in_ms - 33.3).abs() < 0.01);
    assert_eq!(header.checkpoint_limit_in_ms, 250.0);
    assert_eq!(header.platform, "Windows");
    assert_eq!(header.build_config, 7);
    assert_eq!(header.build_target_type, 3);
}

#[test]
fn header_alternate_valorant_skip_bytes() {
    // 12.11 style: [2, 0, 0, 0, 57, 0]
    let payload = helpers::build_header_payload_custom(0, &[2, 0, 0, 0, 57, 0]);
    let header = header::parse_replay_header(&payload).unwrap();
    assert_eq!(header.replay_version.branch, "++Ares-Core+release-12.10");
    assert_eq!(header.ue4_version, 1001);
}

#[test]
fn header_bad_network_magic_rejected() {
    let mut payload = helpers::build_header_payload();
    // Overwrite first 4 bytes (network magic)
    payload[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
    let result = header::parse_replay_header(&payload);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::NetworkMagicMismatch {
            actual: 0xDEAD_BEEF
        }
    ));
}

#[test]
fn header_negative_custom_version_count_rejected() {
    let mut payload = helpers::build_header_payload();
    // Overwrite custom version count at bytes 8..12
    payload[8..12].copy_from_slice(&(-1i32).to_le_bytes());
    let result = header::parse_replay_header(&payload);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::CountOverflow { .. }
    ));
}

#[test]
fn header_truncated_rejected() {
    let result = header::parse_replay_header(&[0x3D, 0xA1, 0xF5, 0x2C]);
    assert!(result.is_err());
}

// ===============================================================================
// ChunkIterator tests
// ===============================================================================

#[test]
fn chunk_iter_empty_returns_none() {
    let mut iter = ChunkIterator::new(&[], 0);
    assert!(iter.next_chunk().unwrap().is_none());
}

#[test]
fn chunk_iter_single_chunk() {
    let payload = [0xAAu8, 0xBB];
    let chunk_data = helpers::build_chunk(1, &payload); // ReplayData
    let mut iter = ChunkIterator::new(&chunk_data, 0);
    let chunk = iter.next_chunk().unwrap().unwrap();
    assert_eq!(chunk.chunk_type, ChunkType::ReplayData);
    assert_eq!(chunk.size_in_bytes, 2);
    assert_eq!(chunk.data_offset, 8);
    assert!(iter.next_chunk().unwrap().is_none());
}

#[test]
fn chunk_iter_multiple_chunks() {
    let mut data = Vec::new();
    data.extend_from_slice(&helpers::build_chunk(0, &[0x11])); // Header
    data.extend_from_slice(&helpers::build_chunk(1, &[0x22, 0x33])); // ReplayData
    data.extend_from_slice(&helpers::build_chunk(3, &[])); // Event (empty)

    let mut iter = ChunkIterator::new(&data, 0);

    let c0 = iter.next_chunk().unwrap().unwrap();
    assert_eq!(c0.chunk_type, ChunkType::Header);
    assert_eq!(c0.size_in_bytes, 1);

    let c1 = iter.next_chunk().unwrap().unwrap();
    assert_eq!(c1.chunk_type, ChunkType::ReplayData);
    assert_eq!(c1.size_in_bytes, 2);

    let c2 = iter.next_chunk().unwrap().unwrap();
    assert_eq!(c2.chunk_type, ChunkType::Event);
    assert_eq!(c2.size_in_bytes, 0);

    assert!(iter.next_chunk().unwrap().is_none());
}

#[test]
fn chunk_iter_truncated_header_rejected() {
    // Only 6 bytes -- not enough for the 8-byte chunk header
    let data = [0u8; 6];
    let mut iter = ChunkIterator::new(&data, 0);
    assert!(matches!(
        iter.next_chunk().unwrap_err(),
        ContainerError::Truncated {
            context: "chunk header",
            ..
        }
    ));
}

#[test]
fn chunk_iter_truncated_payload_rejected() {
    // Chunk header says 100 bytes payload but only 2 are available
    let mut data = Vec::new();
    helpers::add_u32(&mut data, 1); // type
    helpers::add_i32(&mut data, 100); // size = 100
    data.extend_from_slice(&[0u8; 2]); // only 2 bytes
    let mut iter = ChunkIterator::new(&data, 0);
    assert!(matches!(
        iter.next_chunk().unwrap_err(),
        ContainerError::Truncated {
            context: "chunk payload",
            ..
        }
    ));
}

#[test]
fn chunk_iter_negative_size_rejected() {
    let mut data = Vec::new();
    helpers::add_u32(&mut data, 0);
    helpers::add_i32(&mut data, -1);
    let mut iter = ChunkIterator::new(&data, 0);
    assert!(matches!(
        iter.next_chunk().unwrap_err(),
        ContainerError::InvalidChunkSize { size: -1 }
    ));
}

#[test]
fn chunk_type_unknown_preserved() {
    let data = helpers::build_chunk(0xFFFF_FFFF, &[0x42]);
    let mut iter = ChunkIterator::new(&data, 0);
    let chunk = iter.next_chunk().unwrap().unwrap();
    assert_eq!(chunk.chunk_type, ChunkType::Unknown(0xFFFF_FFFF));
}

// ===============================================================================
// Preamble tests
// ===============================================================================

#[test]
fn preamble_valid_file() {
    let mut data = helpers::default_replay_info();
    let header_payload = helpers::build_header_payload();
    data.extend_from_slice(&helpers::build_chunk(0, &header_payload));
    // Add a ReplayData chunk after
    data.extend_from_slice(&helpers::build_chunk(1, &[0xDE; 16]));

    let preamble = parse_preamble(&data).unwrap();
    assert_eq!(preamble.info.length_in_ms, 60000);
    assert_eq!(
        preamble.header.replay_version.branch,
        "++Ares-Core+release-12.10"
    );
    assert!(preamble.remaining_offset > 0);
}

#[test]
fn preamble_unknown_chunk_before_header_skipped() {
    let mut data = helpers::default_replay_info();
    // Unknown chunk first
    data.extend_from_slice(&helpers::build_chunk(0xFFFF_FFFF, &[0x01, 0x02]));
    // Then the real header
    let header_payload = helpers::build_header_payload();
    data.extend_from_slice(&helpers::build_chunk(0, &header_payload));

    let preamble = parse_preamble(&data).unwrap();
    assert_eq!(
        preamble.header.replay_version.branch,
        "++Ares-Core+release-12.10"
    );
}

#[test]
fn preamble_data_before_header_rejected() {
    let mut data = helpers::default_replay_info();
    // ReplayData chunk before Header
    data.extend_from_slice(&helpers::build_chunk(1, &[0xDE; 16]));

    let result = parse_preamble(&data);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::DataBeforeHeader
    ));
}

#[test]
fn preamble_no_header_chunk_rejected() {
    let data = helpers::default_replay_info();
    // No chunks at all after info
    let result = parse_preamble(&data);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::MissingHeaderChunk
    ));
}

// ===============================================================================
// ReplayData meta parsing tests
// ===============================================================================

#[test]
fn replay_data_meta_valid() {
    let mut payload = Vec::new();
    helpers::add_u32(&mut payload, 1000); // time1
    helpers::add_u32(&mut payload, 2000); // time2
    helpers::add_i32(&mut payload, 64); // size_in_bytes
    helpers::add_i32(&mut payload, 128); // memory_size_in_bytes
    payload.extend_from_slice(&[0u8; 64]); // data placeholder

    let meta = parse_replay_data_meta(&payload).unwrap();
    assert_eq!(meta.time1, 1000);
    assert_eq!(meta.time2, 2000);
    assert_eq!(meta.size_in_bytes, 64);
    assert_eq!(meta.memory_size_in_bytes, 128);
}

#[test]
fn replay_data_meta_truncated() {
    let payload = [0u8; 12]; // needs 16
    let result = parse_replay_data_meta(&payload);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::Truncated { .. }
    ));
}

#[test]
fn replay_data_meta_negative_memory_size_rejected() {
    let mut payload = Vec::new();
    helpers::add_u32(&mut payload, 0);
    helpers::add_u32(&mut payload, 0);
    helpers::add_i32(&mut payload, 10);
    helpers::add_i32(&mut payload, -1); // negative
    let result = parse_replay_data_meta(&payload);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::InvalidMemorySize { size: -1 }
    ));
}

#[test]
fn decompress_uncompressed_valid() {
    let mut payload = Vec::new();
    helpers::add_u32(&mut payload, 100); // time1
    helpers::add_u32(&mut payload, 200); // time2
    helpers::add_i32(&mut payload, 4); // size = memory_size
    helpers::add_i32(&mut payload, 4); // memory_size
    payload.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

    let result = decompress_replay_data(&payload, false, false).unwrap();
    assert_eq!(result, vec![0xDE, 0xAD, 0xBE, 0xEF]);
}

#[test]
fn decompress_uncompressed_size_mismatch_rejected() {
    let mut payload = Vec::new();
    helpers::add_u32(&mut payload, 0);
    helpers::add_u32(&mut payload, 0);
    helpers::add_i32(&mut payload, 10); // size != memory_size
    helpers::add_i32(&mut payload, 20);
    payload.extend_from_slice(&[0u8; 20]);

    let result = decompress_replay_data(&payload, false, false);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::SizeMismatch { .. }
    ));
}

#[test]
fn decompress_encrypted_rejected() {
    let payload = [0u8; 32];
    let result = decompress_replay_data(&payload, false, true);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::EncryptedNotSupported
    ));
}
