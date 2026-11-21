//! Replay header chunk parser.
//!
//! # Wire layout (inside the Header chunk payload)
//!
//! | Offset | Type | Field |
//! |--------|------|-------|
//! | 0 | u32 | NetworkMagic (`0x2CF5A13D`) |
//! | 4 | u32 | NetworkVersion (19) |
//! | 8 | i32 | CustomVersionCount |
//! | 12 | [20 x N] | CustomVersionEntries (skipped) |
//! | ... | u32 | NetworkChecksum |
//! | ... | u32 | EngineNetworkProtocolVersion (32) |
//! | ... | u32 | GameNetworkProtocolVersion |
//! | ... | 16 bytes | GUID (4 x u32) |
//! | ... | u16 | ReplayVersion.Major |
//! | ... | u16 | ReplayVersion.Minor |
//! | ... | u16 | ReplayVersion.Patch |
//! | ... | u32 | ReplayVersion.Changelist |
//! | ... | FString | ReplayVersion.Branch |
//! | ... | u32 | ValorantSkipByteCount (release-12.06 onward) |
//! | ... | N bytes | Valorant-specific skip bytes (release-12.06 onward) |
//! | ... | u32 | UE4Version |
//! | ... | u32 | UE5Version |
//! | ... | u32 | PackageVersionLicense |
//! | ... | TupleArray | LevelNamesAndTimes |
//! | ... | u32 | Flags |
//! | ... | Array | GameSpecificData |
//! | ... | f32 | MinRecordHz |
//! | ... | f32 | MaxRecordHz |
//! | ... | f32 | FrameLimitInMs |
//! | ... | f32 | CheckpointLimitInMs |
//! | ... | FString | Platform |
//! | ... | u8 | BuildConfig |
//! | ... | u8 | BuildTargetType |

use vrf_bitio::BitReader;

use crate::error::ContainerError;
use crate::io::{read_f32, read_fstring, read_guid, read_i32, read_u16, read_u32};
use crate::limits::{
    CUSTOM_VERSION_ENTRY_BYTES, EXPECTED_ENGINE_NET_PROTO_VERSION, EXPECTED_NETWORK_VERSION,
    MAX_CUSTOM_VERSION_COUNT, MAX_FSTRING_BYTES, MAX_GAME_SPECIFIC_DATA, MAX_LEVEL_NAMES_AND_TIMES,
    NETWORK_MAGIC,
};

/// Replay version embedded in the header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReplayVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
    pub changelist: u32,
    /// The Unreal branch string, e.g. `"++Ares-Core+release-13.01"`. It selects
    /// the payload transform, and the legacy or modern header layout here.
    pub branch: String,
}

/// Parsed replay header from the first chunk.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ReplayHeader {
    /// Pinned at 19.
    pub network_version: u32,
    pub network_checksum: u32,
    /// Pinned at 32.
    pub engine_network_protocol_version: u32,
    pub game_network_protocol_version: u32,
    /// Session GUID stored as four u32 values (Unreal serialisation order).
    pub guid: [u32; 4],
    pub replay_version: ReplayVersion,
    pub ue4_version: u32,
    pub ue5_version: u32,
    pub package_version_license: u32,
    /// `(level name, time in milliseconds)` pairs.
    pub level_names_and_times: Vec<(String, u32)>,
    pub flags: u32,
    pub game_specific_data: Vec<String>,
    pub min_record_hz: f32,
    pub max_record_hz: f32,
    pub frame_limit_in_ms: f32,
    pub checkpoint_limit_in_ms: f32,
    pub platform: String,
    pub build_config: u8,
    pub build_target_type: u8,
    /// Payload bytes past `BuildTargetType`, reported uninterpreted: non-zero
    /// means the header grew. The manifest's `header_trailing_bytes`.
    pub trailing_bytes: usize,
}

pub(crate) fn parse_replay_header(payload: &[u8]) -> Result<ReplayHeader, ContainerError> {
    let mut reader = BitReader::new(payload);

    let net_magic = read_u32(&mut reader, "network magic")?;
    if net_magic != NETWORK_MAGIC {
        return Err(ContainerError::NetworkMagicMismatch { actual: net_magic });
    }

    let network_version = read_u32(&mut reader, "network version")?;
    if network_version != EXPECTED_NETWORK_VERSION {
        return Err(ContainerError::UnexpectedNetworkVersion {
            actual: network_version,
        });
    }

    let custom_version_count = read_i32(&mut reader, "custom version count")?;
    if !(0..=MAX_CUSTOM_VERSION_COUNT).contains(&custom_version_count) {
        return Err(ContainerError::CountOverflow {
            field: "header custom version count",
            count: custom_version_count,
            max: MAX_CUSTOM_VERSION_COUNT,
        });
    }
    let skip_bits =
        u64::from(custom_version_count as u32) * u64::from(CUSTOM_VERSION_ENTRY_BYTES) * 8;
    reader
        .skip_bits(skip_bits)
        .map_err(|e| ContainerError::BitIo(e.to_string()))?;

    let network_checksum = read_u32(&mut reader, "network checksum")?;

    let engine_network_protocol_version = read_u32(&mut reader, "engine net proto version")?;
    if engine_network_protocol_version != EXPECTED_ENGINE_NET_PROTO_VERSION {
        return Err(ContainerError::UnexpectedEngineNetProtoVersion {
            actual: engine_network_protocol_version,
        });
    }

    let game_network_protocol_version = read_u32(&mut reader, "game net proto version")?;

    let guid = read_guid(&mut reader)?;

    let major = read_u16(&mut reader, "replay version major")?;
    let minor = read_u16(&mut reader, "replay version minor")?;
    let patch = read_u16(&mut reader, "replay version patch")?;
    let changelist = read_u32(&mut reader, "replay version changelist")?;
    let branch = read_fstring(&mut reader, "replay version branch", MAX_FSTRING_BYTES)?;

    let replay_version = ReplayVersion {
        major,
        minor,
        patch,
        changelist,
        branch,
    };

    // 11.06-12.05 put UE4Version (522) right after Branch; the length-prefixed
    // extension starts at 12.06. A malformed modern header is not retried as
    // legacy, which would read its length as a package version; a legacy build
    // missing here fails on the modern path (BitIo EOF or CountOverflow).
    let legacy_header = matches!(
        replay_version.branch.as_str(),
        "++Ares-Core+release-11.06"
            | "++Ares-Core+release-11.07"
            | "++Ares-Core+release-11.08"
            | "++Ares-Core+release-11.09"
            | "++Ares-Core+release-11.10"
            | "++Ares-Core+release-11.11"
            | "++Ares-Core+release-12.00"
            | "++Ares-Core+release-12.01"
            | "++Ares-Core+release-12.02"
            | "++Ares-Core+release-12.03"
            | "++Ares-Core+release-12.04"
            | "++Ares-Core+release-12.05"
    );
    if !legacy_header {
        let valorant_skip_count = read_u32(&mut reader, "valorant skip byte count")?;
        reader
            .skip_bits(u64::from(valorant_skip_count) * 8)
            .map_err(|e| ContainerError::BitIo(e.to_string()))?;
    }

    let ue4_version = read_u32(&mut reader, "UE4 version")?;
    let ue5_version = read_u32(&mut reader, "UE5 version")?;
    let package_version_license = read_u32(&mut reader, "package version license")?;

    let level_count = read_i32(&mut reader, "level names count")?;
    if !(0..=MAX_LEVEL_NAMES_AND_TIMES).contains(&level_count) {
        return Err(ContainerError::CountOverflow {
            field: "level names and times",
            count: level_count,
            max: MAX_LEVEL_NAMES_AND_TIMES,
        });
    }
    let mut level_names_and_times = Vec::with_capacity(level_count as usize);
    for _ in 0..level_count {
        let name = read_fstring(&mut reader, "level name", MAX_FSTRING_BYTES)?;
        let time = read_u32(&mut reader, "level time")?;
        level_names_and_times.push((name, time));
    }

    let flags = read_u32(&mut reader, "flags")?;

    let gsd_count = read_i32(&mut reader, "game specific data count")?;
    if !(0..=MAX_GAME_SPECIFIC_DATA).contains(&gsd_count) {
        return Err(ContainerError::CountOverflow {
            field: "game specific data",
            count: gsd_count,
            max: MAX_GAME_SPECIFIC_DATA,
        });
    }
    let mut game_specific_data = Vec::with_capacity(gsd_count as usize);
    for _ in 0..gsd_count {
        game_specific_data.push(read_fstring(
            &mut reader,
            "game specific data entry",
            MAX_FSTRING_BYTES,
        )?);
    }

    let min_record_hz = read_f32(&mut reader, "min record hz")?;
    let max_record_hz = read_f32(&mut reader, "max record hz")?;
    let frame_limit_in_ms = read_f32(&mut reader, "frame limit")?;
    let checkpoint_limit_in_ms = read_f32(&mut reader, "checkpoint limit")?;

    let platform = read_fstring(&mut reader, "platform", MAX_FSTRING_BYTES)?;
    let build_config = reader
        .read_u8()
        .map_err(|e| ContainerError::BitIo(e.to_string()))?;
    let build_target_type = reader
        .read_u8()
        .map_err(|e| ContainerError::BitIo(e.to_string()))?;

    // Every read above is byte-granular, so this division loses nothing.
    let trailing_bytes = (reader.bits_remaining() / 8) as usize;

    Ok(ReplayHeader {
        network_version,
        network_checksum,
        engine_network_protocol_version,
        game_network_protocol_version,
        guid,
        replay_version,
        ue4_version,
        ue5_version,
        package_version_license,
        level_names_and_times,
        flags,
        game_specific_data,
        min_record_hz,
        max_record_hz,
        frame_limit_in_ms,
        checkpoint_limit_in_ms,
        platform,
        build_config,
        build_target_type,
        trailing_bytes,
    })
}
