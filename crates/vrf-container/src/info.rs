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
//! | ... | u32 | NetworkVersion (must be 19) |
//! | ... | u32 | Changelist |
//! | ... | FString | FriendlyName |
//! | ... | u32 | IsLive (bool as u32) |
//! | ... | i64 | Timestamp (Windows ticks) |
//! | ... | u32 | Compressed (bool as u32) |
//! | ... | u32 | Encrypted (bool as u32) |
//! | ... | i32+[u8] | EncryptionKey (length-prefixed byte array) |

use vrf_bitio::BitReader;

use crate::error::ContainerError;
use crate::limits::{
    EXPECTED_FILE_VERSION, FILE_MAGIC, LOCAL_REPLAY_GUID, LOCAL_REPLAY_VERSION,
    MAX_CUSTOM_VERSION_COUNT, MAX_ENCRYPTION_KEY_BYTES, MAX_FRIENDLY_NAME_BYTES,
};

/// Parsed replay info from the file's leading section.
///
/// Contains the metadata needed to decide whether the replay is supported and
/// how to decompress its payload chunks.
#[derive(Debug, Clone)]
pub struct ReplayInfo {
    /// Match duration in milliseconds.
    pub length_in_ms: i32,
    /// Network version as the info section declares it.
    ///
    /// NOT the 19 that [`crate::EXPECTED_NETWORK_VERSION`] pins, and nothing
    /// here validates it: `02d4d478` carries 480767974. The 19 is checked in
    /// the *header* chunk, which is a different field in a different section.
    /// This doc comment used to say "always 19 for supported replays", which
    /// was never true of any replay in the corpus.
    pub network_version: u32,
    /// Build changelist number, as the info section declares it.
    ///
    /// A replay carries two: this one and `ReplayHeader::replay_version
    /// .changelist`, and they disagree -- 5090349 here against 2152573997 in
    /// the header on `02d4d478`. Downstream (`manifest.json`, and the valplay
    /// adapter through it) reports the header's.
    pub changelist: u32,
    /// Human-readable name (trailing whitespace trimmed).
    pub friendly_name: String,
    /// Whether the replay was recorded as a live session.
    pub is_live: bool,
    /// Timestamp in Unreal `FDateTime` ticks: 100 ns units since 0001-01-01.
    ///
    /// NOT a Windows FILETIME, which this said until 2026-08-04. The two
    /// differ by the 1601-year epoch offset, so reading `02d4d478`'s
    /// 639205853799940000 as FILETIME dates the match to 3626 instead of
    /// 2026-07-25. No timezone is on the wire, so a calendar rendering would
    /// be asserting UTC-vs-local that one replay cannot settle; the ticks are
    /// exported raw as `timestamp_ticks`.
    pub timestamp: i64,
    /// Whether chunk payloads are Oodle-compressed.
    pub compressed: bool,
    /// Whether the replay is encrypted.
    pub encrypted: bool,
    /// Encryption key bytes (empty if unencrypted).
    pub encryption_key: Vec<u8>,
}

/// Parse the replay info section from the start of the buffer.
///
/// Returns the parsed info and the byte offset where chunks begin.
pub(crate) fn parse_replay_info(data: &[u8]) -> Result<(ReplayInfo, usize), ContainerError> {
    if data.is_empty() {
        return Err(ContainerError::Truncated {
            context: "replay info",
            needed: 4,
            available: 0,
        });
    }

    let mut reader = BitReader::new(data);

    // --- Magic ------------------------------------------------------------
    let magic = read_u32(&mut reader, "file magic")?;
    if magic != FILE_MAGIC {
        return Err(ContainerError::FileMagicMismatch { actual: magic });
    }

    // --- Legacy file version ----------------------------------------------
    let file_version = read_u32(&mut reader, "file version")?;
    if file_version != EXPECTED_FILE_VERSION {
        return Err(ContainerError::UnsupportedFileVersion {
            actual: file_version,
        });
    }

    // --- Custom version container -----------------------------------------
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

        // Duplicate check
        if seen_guids.contains(&guid) {
            return Err(ContainerError::DuplicateCustomVersion);
        }
        seen_guids.push(guid);

        if guid == LOCAL_REPLAY_GUID {
            // The one custom version this parser pins. Its version is validated;
            // every other GUID is ignored so an engine/game bump that adds an
            // entry does not make every replay unreadable. Unreal readers
            // conventionally iterate the list and pick out the GUIDs they care
            // about.
            if version != LOCAL_REPLAY_VERSION {
                return Err(ContainerError::UnsupportedLocalReplayVersion { actual: version });
            }
            found_local_replay = true;
        }
    }

    if !found_local_replay {
        return Err(ContainerError::MissingLocalReplayVersion);
    }

    // --- Summary ----------------------------------------------------------
    let length_in_ms = read_i32(&mut reader, "length_in_ms")?;
    let network_version = read_u32(&mut reader, "network version")?;
    let changelist = read_u32(&mut reader, "changelist")?;
    let friendly_name_raw = read_fstring(&mut reader, "friendly name", MAX_FRIENDLY_NAME_BYTES)?;
    let friendly_name = friendly_name_raw.trim_end().to_string();

    let is_live = read_u32(&mut reader, "is_live")? != 0;
    let timestamp = read_i64(&mut reader, "timestamp")?;
    let compressed = read_u32(&mut reader, "compressed")? != 0;
    let encrypted = read_u32(&mut reader, "encrypted")? != 0;

    // Encryption key: length-prefixed byte array
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

    // Validate: completed encrypted replay must have a key.
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

fn read_i64(reader: &mut BitReader<'_>, context: &'static str) -> Result<i64, ContainerError> {
    let lo = reader.read_u32().map_err(|_| ContainerError::Truncated {
        context,
        needed: 8,
        available: (reader.bits_remaining() / 8) as usize,
    })?;
    let hi = reader.read_u32().map_err(|_| ContainerError::Truncated {
        context,
        needed: 4,
        available: (reader.bits_remaining() / 8) as usize,
    })?;
    Ok(i64::from(lo) | (i64::from(hi) << 32))
}

fn read_fstring(
    reader: &mut BitReader<'_>,
    context: &'static str,
    max_bytes: i64,
) -> Result<String, ContainerError> {
    reader
        .read_fstring(max_bytes)
        .map_err(|_| ContainerError::Truncated {
            context,
            needed: 4,
            available: (reader.bits_remaining() / 8) as usize,
        })
}

/// Read an Unreal GUID: four u32 values stored as 16 little-endian bytes.
fn read_guid(reader: &mut BitReader<'_>) -> Result<[u32; 4], ContainerError> {
    let a = read_u32(reader, "guid")?;
    let b = read_u32(reader, "guid")?;
    let c = read_u32(reader, "guid")?;
    let d = read_u32(reader, "guid")?;
    Ok([a, b, c, d])
}
