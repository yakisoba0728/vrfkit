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
//! | ... | [16B] | GUID (4 x u32) |
//! | ... | u16 | ReplayVersion.Major |
//! | ... | u16 | ReplayVersion.Minor |
//! | ... | u16 | ReplayVersion.Patch |
//! | ... | u32 | ReplayVersion.Changelist |
//! | ... | FString | ReplayVersion.Branch |
//! | ... | u32 | ValorantSkipByteCount |
//! | ... | [N] | Valorant-specific skip bytes |
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
use crate::limits::{
    CUSTOM_VERSION_ENTRY_BYTES, EXPECTED_ENGINE_NET_PROTO_VERSION, EXPECTED_NETWORK_VERSION,
    MAX_CUSTOM_VERSION_COUNT, MAX_FSTRING_BYTES, MAX_GAME_SPECIFIC_DATA, MAX_LEVEL_NAMES_AND_TIMES,
    NETWORK_MAGIC,
};

/// Replay version embedded in the header. The `branch` field (e.g.
/// `"++Ares-Core+release-13.01"`) determines which payload transform is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
    pub changelist: u32,
    /// The Unreal branch string, e.g. `"++Ares-Core+release-13.01"`.
    pub branch: String,
}

/// Parsed replay header from the first chunk.
#[derive(Debug, Clone)]
pub struct ReplayHeader {
    /// Network version (always 19).
    pub network_version: u32,
    /// Network checksum.
    pub network_checksum: u32,
    /// Engine network protocol version (always 32).
    pub engine_network_protocol_version: u32,
    /// Game network protocol version.
    pub game_network_protocol_version: u32,
    /// Session GUID stored as four u32 values (Unreal serialisation order).
    pub guid: [u32; 4],
    /// Replay version containing the branch string.
    pub replay_version: ReplayVersion,
    /// UE4 version number.
    pub ue4_version: u32,
    /// UE5 version number.
    pub ue5_version: u32,
    /// Package version license.
    pub package_version_license: u32,
    /// Level names paired with their time offsets in milliseconds.
    pub level_names_and_times: Vec<(String, u32)>,
    /// Header flags bitfield.
    pub flags: u32,
    /// Game-specific data strings.
    pub game_specific_data: Vec<String>,
    /// Minimum recording rate in Hz.
    pub min_record_hz: f32,
    /// Maximum recording rate in Hz.
    pub max_record_hz: f32,
    /// Frame time limit in milliseconds.
    pub frame_limit_in_ms: f32,
    /// Checkpoint time limit in milliseconds.
    pub checkpoint_limit_in_ms: f32,
    /// Platform identifier string.
    pub platform: String,
    /// Build configuration byte.
    pub build_config: u8,
    /// Build target type byte.
    pub build_target_type: u8,
    /// Bytes of the Header chunk payload this layout never reached.
    ///
    /// The parser stops at `BuildTargetType` and used to return success without
    /// asking whether anything followed, so a header extension -- exactly what a
    /// new engine build appends -- was skipped permanently with no error and no
    /// residual. The bytes are not interpreted, because nothing here knows their
    /// layout; their existence is reported instead, the way
    /// [`CheckpointChunk::trailing_bytes`] reports a checkpoint's.
    ///
    /// Expected to be zero. A non-zero value means the header grew.
    ///
    /// [`CheckpointChunk::trailing_bytes`]: crate::CheckpointChunk::trailing_bytes
    pub trailing_bytes: usize,
}

/// Parse the header chunk payload.
pub(crate) fn parse_replay_header(payload: &[u8]) -> Result<ReplayHeader, ContainerError> {
    let mut reader = BitReader::new(payload);

    // --- Network magic ----------------------------------------------------
    let net_magic = read_u32(&mut reader, "network magic")?;
    if net_magic != NETWORK_MAGIC {
        return Err(ContainerError::NetworkMagicMismatch { actual: net_magic });
    }

    // --- Network version --------------------------------------------------
    let network_version = read_u32(&mut reader, "network version")?;
    if network_version != EXPECTED_NETWORK_VERSION {
        return Err(ContainerError::UnexpectedNetworkVersion {
            actual: network_version,
        });
    }

    // --- Custom versions (skip) -------------------------------------------
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

    // --- Network checksum -------------------------------------------------
    let network_checksum = read_u32(&mut reader, "network checksum")?;

    // --- Engine network protocol version ----------------------------------
    let engine_network_protocol_version = read_u32(&mut reader, "engine net proto version")?;
    if engine_network_protocol_version != EXPECTED_ENGINE_NET_PROTO_VERSION {
        return Err(ContainerError::UnexpectedEngineNetProtoVersion {
            actual: engine_network_protocol_version,
        });
    }

    // --- Game network protocol version ------------------------------------
    let game_network_protocol_version = read_u32(&mut reader, "game net proto version")?;

    // --- GUID -------------------------------------------------------------
    let guid = [
        read_u32(&mut reader, "guid")?,
        read_u32(&mut reader, "guid")?,
        read_u32(&mut reader, "guid")?,
        read_u32(&mut reader, "guid")?,
    ];

    // --- Replay version ---------------------------------------------------
    let major = read_u16(&mut reader, "replay version major")?;
    let minor = read_u16(&mut reader, "replay version minor")?;
    let patch = read_u16(&mut reader, "replay version patch")?;
    let changelist = read_u32(&mut reader, "replay version changelist")?;
    let branch = read_fstring(&mut reader, "replay version branch")?;

    let replay_version = ReplayVersion {
        major,
        minor,
        patch,
        changelist,
        branch,
    };

    // --- Valorant-specific skip bytes -------------------------------------
    let valorant_skip_count = read_u32(&mut reader, "valorant skip byte count")?;
    let skip_bits_val = u64::from(valorant_skip_count) * 8;
    reader
        .skip_bits(skip_bits_val)
        .map_err(|e| ContainerError::BitIo(e.to_string()))?;

    // --- UE versions ------------------------------------------------------
    let ue4_version = read_u32(&mut reader, "UE4 version")?;
    let ue5_version = read_u32(&mut reader, "UE5 version")?;
    let package_version_license = read_u32(&mut reader, "package version license")?;

    // --- Level names and times (TupleArray<FString, u32>) -----------------
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
        let name = read_fstring(&mut reader, "level name")?;
        let time = read_u32(&mut reader, "level time")?;
        level_names_and_times.push((name, time));
    }

    // --- Flags ------------------------------------------------------------
    let flags = read_u32(&mut reader, "flags")?;

    // --- Game-specific data (Array<FString>) ------------------------------
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
        game_specific_data.push(read_fstring(&mut reader, "game specific data entry")?);
    }

    // --- Recording parameters ---------------------------------------------
    let min_record_hz = read_f32(&mut reader, "min record hz")?;
    let max_record_hz = read_f32(&mut reader, "max record hz")?;
    let frame_limit_in_ms = read_f32(&mut reader, "frame limit")?;
    let checkpoint_limit_in_ms = read_f32(&mut reader, "checkpoint limit")?;

    // --- Platform and build info ------------------------------------------
    let platform = read_fstring(&mut reader, "platform")?;
    let build_config = reader
        .read_u8()
        .map_err(|e| ContainerError::BitIo(e.to_string()))?;
    let build_target_type = reader
        .read_u8()
        .map_err(|e| ContainerError::BitIo(e.to_string()))?;

    // Every read above is byte-granular, so the reader sits on a byte boundary
    // and this division loses nothing.
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

// --- Helpers ------------------------------------------------------------------

fn read_u32(reader: &mut BitReader<'_>, context: &'static str) -> Result<u32, ContainerError> {
    reader.read_u32().map_err(|_| ContainerError::Truncated {
        context,
        needed: 4,
        available: (reader.bits_remaining() / 8) as usize,
    })
}

fn read_i32(reader: &mut BitReader<'_>, context: &'static str) -> Result<i32, ContainerError> {
    reader.read_i32().map_err(|_| ContainerError::Truncated {
        context,
        needed: 4,
        available: (reader.bits_remaining() / 8) as usize,
    })
}

fn read_u16(reader: &mut BitReader<'_>, context: &'static str) -> Result<u16, ContainerError> {
    reader.read_u16().map_err(|_| ContainerError::Truncated {
        context,
        needed: 2,
        available: (reader.bits_remaining() / 8) as usize,
    })
}

fn read_f32(reader: &mut BitReader<'_>, context: &'static str) -> Result<f32, ContainerError> {
    reader.read_f32().map_err(|_| ContainerError::Truncated {
        context,
        needed: 4,
        available: (reader.bits_remaining() / 8) as usize,
    })
}

fn read_fstring(
    reader: &mut BitReader<'_>,
    context: &'static str,
) -> Result<String, ContainerError> {
    reader
        .read_fstring(MAX_FSTRING_BYTES)
        .map_err(|_| ContainerError::Truncated {
            context,
            needed: 4,
            available: (reader.bits_remaining() / 8) as usize,
        })
}
