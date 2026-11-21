use super::*;
use vrf_testkit::*;

#[test]
fn info_empty_input_rejected() {
    let result = info::parse_replay_info(&[]);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::Truncated { .. }
    ));
}

#[test]
fn info_bad_magic_rejected() {
    let data = replay_info(&Info {
        magic: 0xDEAD_BEEF,
        ..Default::default()
    });
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
    add_u32(&mut data, 0x43F4_EFDD);
    add_u32(&mut data, 6); // wrong version
    data.extend_from_slice(&[0u8; 100]);
    let result = info::parse_replay_info(&data);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::UnsupportedFileVersion { actual: 6 }
    ));
}

#[test]
fn info_missing_custom_version_rejected() {
    let data = replay_info(&Info {
        custom_versions: Vec::new(),
        ..Default::default()
    });
    let result = info::parse_replay_info(&data);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::MissingLocalReplayVersion
    ));
}

#[test]
fn info_newer_or_older_custom_version_rejected() {
    for version in [8, 6] {
        let data = replay_info(&Info {
            custom_versions: vec![(LOCAL_REPLAY, version)],
            ..Default::default()
        });
        let result = info::parse_replay_info(&data);
        assert!(matches!(
            result.unwrap_err(),
            ContainerError::UnsupportedLocalReplayVersion { actual } if actual == version
        ));
    }
}

/// An unknown custom-version GUID, whose version must not be validated, leaves
/// the replay readable; the reason is on the custom-version loop in `info.rs`.
#[test]
fn info_accepts_unknown_custom_version_guids() {
    let data = replay_info(&Info {
        custom_versions: vec![
            ([0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444], 999),
            (LOCAL_REPLAY, 7),
        ],
        ..Default::default()
    });
    let (info, _offset) = info::parse_replay_info(&data).unwrap();
    assert_eq!(info.length_in_ms, 60000);
    assert_eq!(info.friendly_name, "Match");
}

#[test]
fn info_valid_parses_summary() {
    let data = replay_info(&Info {
        friendly_name: "Match  ",
        timestamp: 123456789,
        ..Default::default()
    });
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
    let data = replay_info(&Info {
        encrypted: true,
        ..Default::default()
    });
    let result = info::parse_replay_info(&data);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::EncryptedWithoutKey
    ));
}

#[test]
fn info_truncated_input_rejected() {
    let data = 0x43F4_EFDDu32.to_le_bytes();
    let result = info::parse_replay_info(&data);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::Truncated {
            context: "file version",
            needed: 4,
            available: 0
        }
    ));
}

#[test]
fn header_valid_parses_all_fields() {
    let payload = header_payload();
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
    // Ends exactly where the layout does, so no residual is reported.
    assert_eq!(header.trailing_bytes, 0);
}

#[test]
fn header_trailing_bytes_are_reported_not_discarded() {
    let mut payload = header_payload();
    payload.extend_from_slice(&[0xAA; 5]);
    let header = header::parse_replay_header(&payload).unwrap();
    assert_eq!(header.trailing_bytes, 5);
}

#[test]
fn header_legacy_builds_have_no_valorant_skip_field() {
    for build in [
        "11.06", "11.07", "11.08", "11.09", "11.10", "11.11", "12.00", "12.01", "12.02", "12.03",
        "12.04", "12.05",
    ] {
        let branch = format!("++Ares-Core+release-{build}");
        let payload = header_payload_for_branch(&branch, 3, &[]);
        let parsed = header::parse_replay_header(&payload);
        assert!(parsed.is_ok(), "{build}: {parsed:?}");
        let header = parsed.unwrap();
        assert_eq!(header.replay_version.branch, branch);
        assert_eq!(header.ue4_version, 1001);
        assert_eq!(header.trailing_bytes, 0);
    }
}

#[test]
fn header_skip_field_starts_at_12_06() {
    for build in ["12.06", "12.07", "12.08", "12.09", "12.10", "13.06"] {
        let branch = format!("++Ares-Core+release-{build}");
        // The middle shape is the one measured on 12.11.
        for skip in [
            &[0, 0, 0, 0][..],
            &[2, 0, 0, 0, 57, 0][..],
            &[3, 0, 0, 0, 49, 56, 0][..],
        ] {
            let payload = header_payload_for_branch(&branch, 3, skip);
            let header = header::parse_replay_header(&payload).unwrap();
            assert_eq!(header.ue4_version, 1001, "{build}");
            assert_eq!(header.platform, "Windows", "{build}");
            assert_eq!(header.trailing_bytes, 0, "{build}");
        }
        let missing = header_payload_for_branch(&branch, 3, &[]);
        assert!(header::parse_replay_header(&missing).is_err(), "{build}");
    }
}

#[test]
fn header_bad_network_magic_rejected() {
    let mut payload = header_payload();
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
    let mut payload = header_payload();
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
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::Truncated {
            context: "network version",
            needed: 4,
            available: 0
        }
    ));
}

#[test]
fn chunk_iter_multiple_chunks() {
    let mut data = Vec::new();
    data.extend_from_slice(&chunk(0, &[0x11])); // Header
    data.extend_from_slice(&chunk(1, &[0x22, 0x33])); // ReplayData
    data.extend_from_slice(&chunk(3, &[])); // Event (empty)

    let mut iter = ChunkIterator::new(&data, 0);

    let c0 = iter.next_chunk().unwrap().unwrap();
    assert_eq!(c0.chunk_type, ChunkType::Header);
    assert_eq!(c0.size_in_bytes, 1);
    assert_eq!(c0.data_offset, 8);

    let c1 = iter.next_chunk().unwrap().unwrap();
    assert_eq!(c1.chunk_type, ChunkType::ReplayData);
    assert_eq!(c1.size_in_bytes, 2);
    assert_eq!(c1.data_offset, 17);

    let c2 = iter.next_chunk().unwrap().unwrap();
    assert_eq!(c2.chunk_type, ChunkType::Event);
    assert_eq!(c2.size_in_bytes, 0);
    assert_eq!(c2.data_offset, 27);

    assert!(iter.next_chunk().unwrap().is_none());
}

#[test]
fn chunk_iter_truncated_header_rejected() {
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
    let mut data = Vec::new();
    add_u32(&mut data, 1); // type
    add_i32(&mut data, 100); // size
    data.extend_from_slice(&[0u8; 2]);
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
    add_u32(&mut data, 0);
    add_i32(&mut data, -1);
    let mut iter = ChunkIterator::new(&data, 0);
    assert!(matches!(
        iter.next_chunk().unwrap_err(),
        ContainerError::InvalidChunkSize { size: -1 }
    ));
}

#[test]
fn preamble_valid_file() {
    let mut data = replay_info(&Info::default());
    data.extend_from_slice(&chunk(0, &header_payload()));
    let after_header = data.len();
    data.extend_from_slice(&chunk(1, &[0xDE; 16]));

    let preamble = parse_preamble(&data).unwrap();
    assert_eq!(preamble.info.length_in_ms, 60000);
    assert_eq!(
        preamble.header.replay_version.branch,
        "++Ares-Core+release-12.10"
    );
    assert_eq!(preamble.remaining_offset, after_header);
}

#[test]
fn preamble_data_before_header_rejected() {
    let mut data = replay_info(&Info::default());
    data.extend_from_slice(&chunk(1, &[0xDE; 16]));

    let result = parse_preamble(&data);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::DataBeforeHeader
    ));
}

#[test]
fn preamble_other_chunks_before_the_header_are_rejected() {
    for chunk_type in [
        ChunkType::Checkpoint,
        ChunkType::Event,
        ChunkType::Unknown(0xFFFF_FFFF),
    ] {
        let mut data = replay_info(&Info::default());
        data.extend_from_slice(&chunk(chunk_type.to_raw(), &[0xAB]));
        data.extend_from_slice(&chunk(ChunkType::Header.to_raw(), &header_payload()));

        assert!(matches!(
            parse_preamble(&data).unwrap_err(),
            ContainerError::ChunkBeforeHeader { chunk_type: actual }
                if actual == chunk_type
        ));
    }
}

#[test]
fn fstring_length_and_encoding_errors_keep_their_typed_source() {
    let valid = replay_info(&Info::default());
    let name_offset = valid
        .windows(b"Match\0".len())
        .position(|window| window == b"Match\0")
        .expect("friendly name fixture");

    let mut oversized = valid.clone();
    oversized[name_offset - 4..name_offset].copy_from_slice(&i32::MAX.to_le_bytes());
    assert!(matches!(
        info::parse_replay_info(&oversized).unwrap_err(),
        ContainerError::FString {
            context: "friendly name",
            source: vrf_bitio::BitError::InvalidLength {
                length: value,
                ..
            },
        } if value == i64::from(i32::MAX)
    ));

    let mut invalid_utf8 = valid;
    invalid_utf8[name_offset] = 0xFF;
    assert!(matches!(
        info::parse_replay_info(&invalid_utf8).unwrap_err(),
        ContainerError::FString {
            context: "friendly name",
            source: vrf_bitio::BitError::InvalidString { .. },
        }
    ));
}

/// A ReplayData payload: Time1 and Time2 zero, SizeInBytes, MemorySizeInBytes,
/// then `body`.
fn replay_data(size: i32, memory: i32, body: &[u8]) -> Vec<u8> {
    let mut payload = vec![0; 8];
    add_i32(&mut payload, size);
    add_i32(&mut payload, memory);
    payload.extend_from_slice(body);
    payload
}

/// A compressed ReplayData chunk whose archive declares 64 bytes of output
/// (decompressed_size 64, compressed_size 4) and carries four bytes that are
/// not a codec stream.
fn compressed_replay_data_needing_64_bytes() -> Vec<u8> {
    replay_data(12, 64, &[64, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0])
}

/// Without the decoder a compressed archive is refused by name, not emptied.
#[cfg(not(feature = "oodle"))]
#[test]
fn a_compressed_archive_without_the_decoder_is_refused() {
    let payload = compressed_replay_data_needing_64_bytes();
    assert!(matches!(
        decompress_replay_data(&payload, true, false),
        Err(ContainerError::OodleUnsupported { needed: 64 })
    ));
}

/// With the decoder the same archive reaches the codec, which rejects it.
#[cfg(feature = "oodle")]
#[test]
fn a_compressed_archive_with_the_decoder_reaches_the_codec() {
    let payload = compressed_replay_data_needing_64_bytes();
    assert!(matches!(
        decompress_replay_data(&payload, true, false),
        Err(ContainerError::OodleDecompression(_))
    ));
}

/// A stream the codec panics on is an error, not a panic, on both archive
/// paths; the caught panic's message is kept.
#[cfg(feature = "oodle")]
#[test]
fn a_codec_panic_is_reported_as_a_decompression_error() {
    let archive = archive_that_panics_the_codec();
    let payload = replay_data(archive.len() as i32, 64, &archive);
    let result = std::panic::catch_unwind(|| decompress_replay_data(&payload, true, false));
    let Ok(Err(ContainerError::OodleDecompression(message))) = result else {
        panic!("expected a decompression error, got {result:?}");
    };
    assert!(message.starts_with("decoder panicked: "), "{message}");

    #[cfg(feature = "checkpoint")]
    {
        let result = std::panic::catch_unwind(|| decompress_checkpoint(&archive, true, false));
        assert!(
            matches!(result, Ok(Err(ContainerError::OodleDecompression(ref m))) if m.starts_with("decoder panicked: ")),
            "{result:?}"
        );
    }
}

#[test]
fn preamble_no_header_chunk_rejected() {
    let data = replay_info(&Info::default());
    let result = parse_preamble(&data);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::MissingHeaderChunk
    ));
}

#[test]
fn replay_data_meta_valid() {
    let mut payload = Vec::new();
    add_u32(&mut payload, 1000); // time1
    add_u32(&mut payload, 2000); // time2
    add_i32(&mut payload, 64); // size_in_bytes
    add_i32(&mut payload, 128); // memory_size_in_bytes
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
    let result = parse_replay_data_meta(&replay_data(10, -1, &[]));
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::InvalidMemorySize { size: -1 }
    ));
}

#[test]
fn decompress_uncompressed_size_mismatch_rejected() {
    let payload = replay_data(10, 20, &[0; 20]);
    let result = decompress_replay_data(&payload, false, false);
    assert!(matches!(
        result.unwrap_err(),
        ContainerError::SizeMismatch { .. }
    ));
}

/// Payload bytes past `SizeInBytes` are counted, not cut away with the slice:
/// the uncompressed path here, the Oodle path in the codec-unread test below.
#[test]
fn replay_data_trailing_bytes_are_reported_not_discarded() {
    // Three bytes past the declared archive.
    let payload = replay_data(4, 4, &[0xDE, 0xAD, 0xBE, 0xEF, 0x11, 0x22, 0x33]);

    let meta = parse_replay_data_meta(&payload).unwrap();
    assert_eq!(meta.trailing_bytes, 3);

    let (plain, trailing) = decompress_replay_data_with_trailing(&payload, false, false).unwrap();
    assert_eq!(plain, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(trailing, 3, "the excess must be counted, not discarded");
}

/// An exactly-sized payload reports zero, so the counter cannot be confused
/// with ordinary framing.
#[test]
fn replay_data_exact_payload_reports_no_trailing_bytes() {
    let payload = replay_data(4, 4, &[1, 2, 3, 4]);
    assert_eq!(parse_replay_data_meta(&payload).unwrap().trailing_bytes, 0);
    let (plain, trailing) = decompress_replay_data_with_trailing(&payload, false, false).unwrap();
    assert_eq!(plain, [1, 2, 3, 4]);
    assert_eq!(trailing, 0);
    let wrapped = decompress_replay_data(&payload, false, false).unwrap();
    assert_eq!(wrapped, plain, "the public wrapper returns the same bytes");
}

/// A `SizeInBytes` larger than the payload is truncation, not a negative
/// residual: the count must saturate at zero rather than wrap.
#[test]
fn replay_data_short_payload_reports_no_trailing_bytes() {
    let payload = replay_data(64, 64, &[1, 2, 3, 4]);
    assert_eq!(parse_replay_data_meta(&payload).unwrap().trailing_bytes, 0);
    assert!(matches!(
        decompress_replay_data(&payload, false, false),
        Err(ContainerError::Truncated { .. })
    ));
}

/// Archive bytes no block reads are counted together with the framing residual
/// past `SizeInBytes`, and an archive read to the end reports zero.
#[cfg(feature = "oodle")]
#[test]
fn replay_data_input_the_codec_never_reads_is_counted() {
    for (unread, past_archive) in [(7, 0), (0, 0), (7, 3)] {
        let archive = archive_with_unread_input(&[1, 2, 3, 4, 5], unread);
        let mut payload = replay_data(archive.len() as i32, 5, &archive);
        payload.extend(std::iter::repeat_n(0xCD, past_archive));

        let (plain, count) = decompress_replay_data_with_trailing(&payload, true, false).unwrap();
        assert_eq!(plain, [1, 2, 3, 4, 5]);
        assert_eq!(
            count,
            unread + past_archive,
            "every payload byte no reader consumed must be counted"
        );
    }
}

/// The same residual in a checkpoint archive, which has no framing residual
/// of its own: the archive slice is exactly its declared size.
#[cfg(all(feature = "oodle", feature = "checkpoint"))]
#[test]
fn checkpoint_input_the_codec_never_reads_is_counted() {
    for unread in [7, 0] {
        let archive = archive_with_unread_input(&[1, 2, 3, 4, 5], unread);
        let (plain, count) = decompress_checkpoint_with_trailing(&archive, true, false).unwrap();
        assert_eq!(plain, [1, 2, 3, 4, 5]);
        assert_eq!(count, unread, "input the codec never read must be counted");
        assert_eq!(decompress_checkpoint(&archive, true, false).unwrap(), plain);
    }
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

// Gated with the parser it exercises, so a build without `event` still compiles.
#[cfg(feature = "event")]
mod event_chunks {
    use super::*;

    /// The inner payload of the first `roundStarted` event in 02d4d478, byte for
    /// byte: real bytes, whose 46-byte length the parser must respect exactly.
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
        add_fstring(&mut buf, id);
        add_fstring(&mut buf, group);
        add_fstring(&mut buf, metadata);
        add_u32(&mut buf, time1);
        add_u32(&mut buf, time2);
        add_i32(&mut buf, body.len() as i32);
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
    fn event_payload_reference_round_start_exposes_every_structural_value() {
        let payload = parse_event_payload(&REFERENCE_ROUND_START_PAYLOAD, 1)
            .expect("the measured one-word layout must consume the payload exactly");

        assert_eq!(payload.tag, 2);
        assert_eq!(payload.words, vec![0]);
        assert_eq!(payload.name, "EReplayEventGroup::RoundStart");
        assert_eq!(payload.seconds.to_bits(), 0x3D7F_C022);
    }

    #[test]
    fn event_payload_refuses_a_word_count_that_does_not_consume_exactly() {
        assert!(parse_event_payload(&REFERENCE_ROUND_START_PAYLOAD, 0).is_none());
        assert!(parse_event_payload(&REFERENCE_ROUND_START_PAYLOAD, 2).is_none());
    }

    #[test]
    fn known_event_payload_requires_the_measured_name_and_tag() {
        let parsed = parse_known_event_payload("roundStarted", &REFERENCE_ROUND_START_PAYLOAD)
            .expect("the reference enum name is the measured public constant");
        assert_eq!(parsed.name, "EReplayEventGroup::RoundStart");

        assert!(parse_known_event_payload("futureGroup", &REFERENCE_ROUND_START_PAYLOAD).is_none());

        let mut renamed = REFERENCE_ROUND_START_PAYLOAD.to_vec();
        renamed[12] = b'X';
        assert!(parse_known_event_payload("roundStarted", &renamed).is_none());

        let mut retagged = REFERENCE_ROUND_START_PAYLOAD.to_vec();
        retagged[..4].copy_from_slice(&99u32.to_le_bytes());
        assert!(parse_known_event_payload("roundStarted", &retagged).is_none());
    }

    /// Every entry must come back out of all three accessors, and a group listed
    /// twice would leave the second unreachable. The values themselves are
    /// pinned against the adapter's allowlists in vrfkit's adapter_contract.
    #[test]
    fn every_known_event_group_round_trips_through_its_accessors() {
        let mut seen = std::collections::BTreeSet::new();
        for known in KNOWN_EVENT_GROUPS {
            assert!(seen.insert(known.group), "{} is listed twice", known.group);
            assert_eq!(known_event_word_count(known.group), Some(known.word_count));
            assert_eq!(
                known_event_payload_name(known.group),
                Some(known.payload_name)
            );
            assert_eq!(
                known_event_payload_tag(known.group),
                Some(known.payload_tag)
            );
        }
    }

    /// The const lookup compares bytes by hand; a prefix, an extension or a
    /// case change must not match.
    #[test]
    fn known_event_lookup_matches_only_the_whole_group_name() {
        for near_miss in [
            "",
            "characterDeat",
            "characterDeathX",
            "CharacterDeath",
            "spikePlanted ",
            "spike",
        ] {
            assert_eq!(known_event_word_count(near_miss), None, "{near_miss:?}");
            assert_eq!(known_event_payload_name(near_miss), None, "{near_miss:?}");
            assert_eq!(known_event_payload_tag(near_miss), None, "{near_miss:?}");
        }
    }

    #[test]
    fn event_payload_seconds_matches_the_chunk_millisecond_time() {
        let seconds = f32::from_bits(0x3D7F_C022);
        assert!(event_payload_seconds_matches_time(62, seconds));
        assert!(event_payload_seconds_matches_time(100, 0.1));
        assert!(!event_payload_seconds_matches_time(103, 0.1));
        assert!(!event_payload_seconds_matches_time(0, f32::NAN));
        assert!(!event_payload_seconds_matches_time(0, f32::INFINITY));
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
    fn event_chunk_negative_payload_size_rejected() {
        let mut payload = Vec::new();
        add_fstring(&mut payload, "id");
        add_fstring(&mut payload, "group");
        add_fstring(&mut payload, "");
        add_u32(&mut payload, 1);
        add_u32(&mut payload, 1);
        add_i32(&mut payload, -1);

        assert!(matches!(
            parse_event_chunk(&payload).unwrap_err(),
            ContainerError::InvalidEventPayloadSize { size: -1 }
        ));
    }

    #[test]
    fn event_chunk_payload_shorter_than_declared_rejected() {
        // Reading the short slice as the whole payload is the silent truncation
        // this must not do.
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
        add_fstring(&mut payload, "id");
        assert!(matches!(
            parse_event_chunk(&payload).unwrap_err(),
            ContainerError::FString {
                context: "event group",
                source: vrf_bitio::BitError::Eof { .. },
            }
        ));
    }

    #[test]
    fn event_chunk_trailing_bytes_are_counted_not_dropped() {
        let mut payload = build_event_chunk("id", "group", "", 1, 1, &[0x01, 0x02]);
        payload.extend_from_slice(&[0xFF; 3]);

        let event = parse_event_chunk(&payload).unwrap();
        // An empty FString, as `characterDeath` metadata is, is a real value.
        assert_eq!(event.metadata, "");
        assert_eq!(event.payload, &[0x01, 0x02]);
        assert_eq!(event.trailing_bytes, 3);
    }

    /// `[u32 tag][words][FString name][f32 seconds]`, seconds fixed at 1.5.
    fn event_payload(tag: u32, words: &[u32], name: &str) -> Vec<u8> {
        let mut out = Vec::new();
        add_u32(&mut out, tag);
        for &word in words {
            add_u32(&mut out, word);
        }
        add_fstring(&mut out, name);
        add_f32(&mut out, 1.5);
        out
    }

    /// A measured zero-word group is still checked through its tag, FString
    /// and trailing f32: zero is an established arity, not an absent claim.
    #[test]
    fn event_payload_accepts_a_zero_word_layout_that_consumes_exactly() {
        let payload = event_payload(4, &[], "EReplayEventGroup::SpikePlanted");
        let parsed = parse_event_payload(&payload, 0).expect("zero-word layout");
        assert_eq!(parsed.tag, 4);
        assert!(parsed.words.is_empty());
        assert_eq!(parsed.name, "EReplayEventGroup::SpikePlanted");
        assert_eq!(parsed.seconds, 1.5);
    }

    /// characterDeath is the only multi-word group. The one-word reference's
    /// word is 0, so only distinct nonzero words show a word swapped, dropped
    /// or zeroed on its way to `events.parquet`'s word0 and word1.
    #[test]
    fn event_payload_yields_multi_word_values_in_order() {
        let payload = event_payload(
            8,
            &[0x1111_1111, 0x2222_2222],
            "EReplayEventGroup::CharacterDeath",
        );
        let parsed = parse_event_payload(&payload, 2).expect("two-word layout");
        assert_eq!(parsed.tag, 8);
        assert_eq!(parsed.words, [0x1111_1111, 0x2222_2222]);
        assert_eq!(parsed.name, "EReplayEventGroup::CharacterDeath");
        assert_eq!(parsed.seconds, 1.5);
    }

    /// A payload shorter than the layout's fixed parts cannot be verified, so
    /// it yields nothing rather than whatever a partial read returns.
    #[test]
    fn event_payload_refuses_a_payload_shorter_than_its_fixed_parts() {
        assert!(parse_event_payload(&[0xAB; 3], 0).is_none());
        assert!(parse_event_payload(&[0u8; 6], 1).is_none());
    }
}
