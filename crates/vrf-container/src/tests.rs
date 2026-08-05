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

    /// Serialise an FString in its UTF-16 form: a negative length counting
    /// code units (terminator included), then little-endian UTF-16.
    ///
    /// Only the Event chunk tests need this form, so it is unused when the
    /// `event` feature is off rather than genuinely dead.
    #[cfg_attr(not(feature = "event"), allow(dead_code))]
    pub fn add_fstring_utf16(buf: &mut Vec<u8>, s: &str) {
        let units: Vec<u16> = s.encode_utf16().chain(std::iter::once(0u16)).collect();
        add_i32(buf, -(units.len() as i32));
        for unit in units {
            add_u16(buf, unit);
        }
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

/// A custom-version list carrying a GUID the parser does not recognise must
/// not make the replay unreadable. Unreal readers iterate the list and pick out
/// the GUIDs they care about, ignoring the rest; a future engine bump that adds
/// an engine/game custom-version entry would otherwise break every replay.
///
/// This builds a list with an unknown GUID (carrying a version the parser must
/// NOT validate) followed by the required `LocalFileReplay` GUID at version 7,
/// and asserts the info still parses.
#[test]
fn info_accepts_unknown_custom_version_guids() {
    let mut buf = Vec::new();
    helpers::add_u32(&mut buf, 0x43F4_EFDD); // magic
    helpers::add_u32(&mut buf, 7); // file version
    // Two custom versions: an unknown one, then LOCAL_REPLAY.
    helpers::add_i32(&mut buf, 2);
    // Unknown GUID with an arbitrary version that must be ignored.
    helpers::add_guid(&mut buf, 0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444);
    helpers::add_i32(&mut buf, 999);
    // The required LocalFileReplay GUID at its required version.
    helpers::add_guid(&mut buf, 0x95A4_F03E, 0x7E0B_49E4, 0xBA43_D356, 0x94FF_87D9);
    helpers::add_i32(&mut buf, 7);
    // Summary fields.
    helpers::add_i32(&mut buf, 60000); // length_in_ms
    helpers::add_u32(&mut buf, 19); // network version
    helpers::add_u32(&mut buf, 1234); // changelist
    helpers::add_fstring(&mut buf, "Match");
    helpers::add_u32(&mut buf, 0); // is_live
    helpers::add_i64(&mut buf, 42); // timestamp
    helpers::add_u32(&mut buf, 0); // compressed
    helpers::add_u32(&mut buf, 0); // encrypted
    helpers::add_byte_array(&mut buf, &[]); // encryption key

    let (info, _offset) = info::parse_replay_info(&buf).unwrap();
    assert_eq!(info.length_in_ms, 60000);
    assert_eq!(info.friendly_name, "Match");
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

// ===============================================================================
// Event chunk parsing tests
// ===============================================================================

// Gated with the parser they exercise, so `--no-default-features` still
// compiles this file rather than dropping the whole suite.
#[cfg(feature = "event")]
mod event_chunks {
    use super::*;

    /// The inner payload of the first `roundStarted` event in the reference replay
    /// `02d4d478-1dfb-4412-9a77-29ca29105a9d.vrf`, copied byte for byte.
    ///
    /// Using real bytes rather than a synthetic blob keeps the test honest about
    /// what the parser must survive: a 46-byte payload the parser deliberately does
    /// not interpret, whose length must still be respected exactly.
    const REFERENCE_ROUND_START_PAYLOAD: [u8; 46] = [
        0x02, 0x00, 0x00, 0x00, // group tag (RoundStart)
        0x00, 0x00, 0x00, 0x00, // one group-dependent word
        0x1E, 0x00, 0x00, 0x00, // FString length: 30
        b'E', b'R', b'e', b'p', b'l', b'a', b'y', b'E', b'v', b'e', b'n', b't', b'G', b'r', b'o',
        b'u', b'p', b':', b':', b'R', b'o', b'u', b'n', b'd', b'S', b't', b'a', b'r', b't', 0x00,
        0x22, 0xC0, 0x7F, 0x3D, // f32 seconds
    ];

    /// Build an Event chunk payload from its six header fields.
    fn build_event_chunk(
        id: &str,
        group: &str,
        metadata: &str,
        time1: u32,
        time2: u32,
        body: &[u8],
    ) -> Vec<u8> {
        let mut buf = Vec::new();
        helpers::add_fstring(&mut buf, id);
        helpers::add_fstring(&mut buf, group);
        helpers::add_fstring(&mut buf, metadata);
        helpers::add_u32(&mut buf, time1);
        helpers::add_u32(&mut buf, time2);
        helpers::add_i32(&mut buf, body.len() as i32);
        buf.extend_from_slice(body);
        buf
    }

    #[test]
    fn event_chunk_reference_round_start() {
        let payload = build_event_chunk(
            "02d4d478-1dfb-4412-9a77-29ca29105a9d_DC4D6C49E0C640FD814D88134F0A8642",
            "roundStarted",
            "0",
            62,
            62,
            &REFERENCE_ROUND_START_PAYLOAD,
        );

        let event = parse_event_chunk(&payload).unwrap();
        assert_eq!(
            event.id,
            "02d4d478-1dfb-4412-9a77-29ca29105a9d_DC4D6C49E0C640FD814D88134F0A8642"
        );
        assert_eq!(event.group, "roundStarted");
        assert_eq!(event.metadata, "0");
        assert_eq!(event.time1, 62);
        assert_eq!(event.time2, 62);
        assert_eq!(event.size_in_bytes, 46);
        // The payload is handed back untouched -- not reinterpreted, not truncated.
        assert_eq!(event.payload, &REFERENCE_ROUND_START_PAYLOAD);
        assert_eq!(event.trailing_bytes, 0);
    }

    #[test]
    fn event_chunk_empty_metadata_is_empty_not_missing() {
        // `characterDeath` carries no metadata. An empty FString is a real value;
        // the parser must not turn it into anything else.
        let payload = build_event_chunk("id", "characterDeath", "", 50402, 50402, &[0xAA, 0xBB]);
        let event = parse_event_chunk(&payload).unwrap();
        assert_eq!(event.metadata, "");
        assert_eq!(event.payload, &[0xAA, 0xBB]);
    }

    #[test]
    fn event_chunk_zero_length_payload_accepted() {
        let payload = build_event_chunk("id", "group", "meta", 1, 2, &[]);
        let event = parse_event_chunk(&payload).unwrap();
        assert_eq!(event.size_in_bytes, 0);
        assert!(event.payload.is_empty());
        assert_eq!(event.trailing_bytes, 0);
    }

    #[test]
    fn event_chunk_utf16_strings_decoded() {
        // FString allows a UTF-16 encoding via a negative length. No corpus file
        // uses it for these fields, but the format permits it and a parser that
        // silently mis-read one would produce a wrong group name, not an error.
        let mut payload = Vec::new();
        helpers::add_fstring_utf16(&mut payload, "utf16id");
        helpers::add_fstring_utf16(&mut payload, "roundStarted");
        helpers::add_fstring(&mut payload, "7");
        helpers::add_u32(&mut payload, 11);
        helpers::add_u32(&mut payload, 11);
        helpers::add_i32(&mut payload, 1);
        payload.push(0x5A);

        let event = parse_event_chunk(&payload).unwrap();
        assert_eq!(event.id, "utf16id");
        assert_eq!(event.group, "roundStarted");
        assert_eq!(event.metadata, "7");
        assert_eq!(event.payload, &[0x5A]);
    }

    #[test]
    fn event_chunk_negative_payload_size_rejected() {
        let mut payload = Vec::new();
        helpers::add_fstring(&mut payload, "id");
        helpers::add_fstring(&mut payload, "group");
        helpers::add_fstring(&mut payload, "");
        helpers::add_u32(&mut payload, 1);
        helpers::add_u32(&mut payload, 1);
        helpers::add_i32(&mut payload, -1);

        assert!(matches!(
            parse_event_chunk(&payload).unwrap_err(),
            ContainerError::InvalidEventPayloadSize { size: -1 }
        ));
    }

    #[test]
    fn event_chunk_payload_shorter_than_declared_rejected() {
        // Declares 64 bytes, supplies 4. Reading the short slice as if it were the
        // whole payload is the silent-truncation failure this must not do.
        let mut payload = build_event_chunk("id", "group", "", 1, 1, &[0u8; 64]);
        payload.truncate(payload.len() - 60);

        let err = parse_event_chunk(&payload).unwrap_err();
        assert!(
            matches!(
                err,
                ContainerError::Truncated {
                    context: "event payload",
                    needed: 64,
                    available: 4
                }
            ),
            "expected a truncated-payload error, got: {err}"
        );
    }

    #[test]
    fn event_chunk_truncated_header_rejected() {
        // The header itself runs off the end: an id, then nothing.
        let mut payload = Vec::new();
        helpers::add_fstring(&mut payload, "id");
        assert!(matches!(
            parse_event_chunk(&payload).unwrap_err(),
            ContainerError::Truncated { .. }
        ));
    }

    #[test]
    fn event_chunk_trailing_bytes_are_counted_not_dropped() {
        // Bytes past the declared payload are what a format change would look like.
        // They must be reported, not quietly ignored.
        let mut payload = build_event_chunk("id", "group", "", 1, 1, &[0x01, 0x02]);
        payload.extend_from_slice(&[0xFF; 3]);

        let event = parse_event_chunk(&payload).unwrap();
        assert_eq!(event.payload, &[0x01, 0x02]);
        assert_eq!(event.trailing_bytes, 3);
    }

    #[test]
    fn event_chunk_found_by_the_chunk_iterator() {
        // End to end through the iterator: an Event chunk sitting between two
        // others must be located by type and its payload sliced correctly.
        let body = [0xDE, 0xAD, 0xBE, 0xEF];
        let event_payload = build_event_chunk("id", "spikePlanted", "", 69118, 69118, &body);

        let mut data = Vec::new();
        helpers::add_u32(&mut data, ChunkType::Checkpoint.to_raw());
        helpers::add_i32(&mut data, 2);
        data.extend_from_slice(&[0u8; 2]);
        helpers::add_u32(&mut data, ChunkType::Event.to_raw());
        helpers::add_i32(&mut data, event_payload.len() as i32);
        data.extend_from_slice(&event_payload);

        let mut iter = ChunkIterator::new(&data, 0);
        let mut found = 0;
        while let Some(chunk) = iter.next_chunk().unwrap() {
            if chunk.chunk_type != ChunkType::Event {
                continue;
            }
            let slice = &data[chunk.data_offset..chunk.data_offset + chunk.size_in_bytes as usize];
            let event = parse_event_chunk(slice).unwrap();
            assert_eq!(event.group, "spikePlanted");
            assert_eq!(event.time1, 69118);
            assert_eq!(event.payload, &body);
            found += 1;
        }
        assert_eq!(found, 1);
    }
} // mod event_chunks
