//! Replay info section parser.
//!
//! # Wire layout
//!
//! | Offset | Type | Field |
//! |--------|------|-------|
//! | 0 | u32 | FileMagic (`0x43F4EFDD`) |
//! | 4 | u32 | LegacyFileVersion (must be 7) |
//! | 8 | i32 | CustomVersionCount |
//! | 12 | [20 x N] | CustomVersionEntries (GUID 16B + i32 version) |
//! | ... | i32 | LengthInMs |
//! | ... | u32 | NetworkVersion (NOT 19 here; unvalidated -- see the field doc) |
//! | ... | u32 | Changelist |
//! | ... | FString | FriendlyName |
//! | ... | u32 | IsLive (bool as u32) |
//! | ... | i64 | Timestamp (`FDateTime` ticks) |
//! | ... | u32 | Compressed (bool as u32) |
//! | ... | u32 | Encrypted (bool as u32) |
//! | ... | i32+[u8] | EncryptionKey (length-prefixed byte array) |

use vrf_bitio::BitReader;

use crate::error::ContainerError;
use crate::io::{read_fstring, read_guid, read_i32, read_i64, read_u32};
use crate::limits::{
    EXPECTED_FILE_VERSION, FILE_MAGIC, LOCAL_REPLAY_GUID, LOCAL_REPLAY_VERSION,
    MAX_CUSTOM_VERSION_COUNT, MAX_ENCRYPTION_KEY_BYTES, MAX_FRIENDLY_NAME_BYTES,
};

/// Parsed replay info from the file's leading section.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ReplayInfo {
    /// Match duration in milliseconds.
    pub length_in_ms: i32,
    /// Network version as the info section declares it: unvalidated, and not
    /// the 19 the Header chunk pins. `02d4d478` carries 480767974 here.
    pub network_version: u32,
    /// Disagrees with the header's changelist (`02d4d478`: 5090349 vs
    /// 2152573997), which `manifest.json` reports.
    pub changelist: u32,
    /// Human-readable name (trailing whitespace trimmed).
    pub friendly_name: String,
    pub is_live: bool,
    /// Unreal `FDateTime` ticks (100 ns since 0001-01-01), not a Windows FILETIME.
    /// No timezone is on the wire, so they are exported raw as `timestamp_ticks`.
    pub timestamp: i64,
    pub compressed: bool,
    pub encrypted: bool,
    pub encryption_key: Vec<u8>,
}

/// The replay info and the byte offset where chunks begin.
pub(crate) fn parse_replay_info(data: &[u8]) -> Result<(ReplayInfo, usize), ContainerError> {
    let mut reader = BitReader::new(data);

    let magic = read_u32(&mut reader, "file magic")?;
    if magic != FILE_MAGIC {
        return Err(ContainerError::FileMagicMismatch { actual: magic });
    }

    let file_version = read_u32(&mut reader, "file version")?;
    if file_version != EXPECTED_FILE_VERSION {
        return Err(ContainerError::UnsupportedFileVersion {
            actual: file_version,
        });
    }

    let custom_version_count = read_i32(&mut reader, "custom version count")?;
    if !(0..=MAX_CUSTOM_VERSION_COUNT).contains(&custom_version_count) {
        return Err(ContainerError::CountOverflow {
            field: "custom version count",
            count: custom_version_count,
            max: MAX_CUSTOM_VERSION_COUNT,
        });
    }

    let mut found_local_replay = false;
    let mut seen_guids: Vec<[u32; 4]> = Vec::new();

    for _ in 0..custom_version_count {
        let guid = read_guid(&mut reader)?;
        let version = read_i32(&mut reader, "custom version")?;

        if seen_guids.contains(&guid) {
            return Err(ContainerError::DuplicateCustomVersion);
        }
        seen_guids.push(guid);

        if guid == LOCAL_REPLAY_GUID {
            // The only pin. Other GUIDs are ignored, as Unreal readers do, so an
            // engine bump that adds one leaves every replay readable.
            if version != LOCAL_REPLAY_VERSION {
                return Err(ContainerError::UnsupportedLocalReplayVersion { actual: version });
            }
            found_local_replay = true;
        }
    }

    if !found_local_replay {
        return Err(ContainerError::MissingLocalReplayVersion);
    }

    let length_in_ms = read_i32(&mut reader, "length_in_ms")?;
    let network_version = read_u32(&mut reader, "network version")?;
    let changelist = read_u32(&mut reader, "changelist")?;
    let friendly_name_raw = read_fstring(&mut reader, "friendly name", MAX_FRIENDLY_NAME_BYTES)?;
    let friendly_name = friendly_name_raw.trim_end().to_string();

    let is_live = read_u32(&mut reader, "is_live")? != 0;
    let timestamp = read_i64(&mut reader, "timestamp")?;
    let compressed = read_u32(&mut reader, "compressed")? != 0;
    let encrypted = read_u32(&mut reader, "encrypted")? != 0;

    let key_len = read_i32(&mut reader, "encryption key length")?;
    if !(0..=MAX_ENCRYPTION_KEY_BYTES).contains(&key_len) {
        return Err(ContainerError::CountOverflow {
            field: "encryption key",
            count: key_len,
            max: MAX_ENCRYPTION_KEY_BYTES,
        });
    }
    let mut encryption_key = vec![0u8; key_len as usize];
    for byte in &mut encryption_key {
        *byte = reader
            .read_u8()
            .map_err(|e| ContainerError::BitIo(e.to_string()))?;
    }

    if !is_live && encrypted && encryption_key.is_empty() {
        return Err(ContainerError::EncryptedWithoutKey);
    }

    let bytes_consumed = (reader.position() / 8) as usize;

    Ok((
        ReplayInfo {
            length_in_ms,
            network_version,
            changelist,
            friendly_name,
            is_live,
            timestamp,
            compressed,
            encrypted,
            encryption_key,
        },
        bytes_consumed,
    ))
}
