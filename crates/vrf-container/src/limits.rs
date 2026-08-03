//! Magic numbers, version pins and bounds, each traced to its C# source.
//!
//! These are gathered in one module because they are the parser's entire
//! contract with the file format: every one of them is a value the reference
//! implementation states outright, and none may be adjusted to make a
//! particular file parse. A bound that has to move is a format discovery, and
//! the citation is what lets the next reader check that claim against the
//! reference rather than against this crate's behaviour.

/// File-level magic number.
///
/// Source: `ReplayInfoReader.cs` -- `private const uint FileMagic = 0x43F4EFDD`.
/// Present at byte offset 0 of every `.vrf` file.
pub(crate) const FILE_MAGIC: u32 = 0x43F4_EFDD;

/// Network-level magic number inside the Header chunk.
///
/// Source: `Constants.cs` -- `public const uint NetworkMagic = 0x2CF5A13D`.
pub(crate) const NETWORK_MAGIC: u32 = 0x2CF5_A13D;

/// Expected legacy file version in the replay info section.
///
/// Source: `LocalFileReplayCustomVersions.cs` -- version 7 is the only supported
/// value; older versions used a different serialisation layout.
pub(crate) const EXPECTED_FILE_VERSION: u32 = 7;

/// Expected network version in both the info section and the header chunk.
///
/// Source: `Constants.cs` -- `public const uint ExpectedNetworkVersion = 19`.
pub(crate) const EXPECTED_NETWORK_VERSION: u32 = 19;

/// Expected engine network protocol version inside the Header chunk.
///
/// Source: `Constants.cs` --
/// `public const uint ExpectedEngineNetworkProtocolVersion = 32`.
pub(crate) const EXPECTED_ENGINE_NET_PROTO_VERSION: u32 = 32;

/// Maximum sane custom version count (guard against corrupt length fields).
///
/// Source: `Constants.cs` -- `public const int MaxCustomVersionCount = 1024`.
pub(crate) const MAX_CUSTOM_VERSION_COUNT: i32 = 1024;

/// Maximum byte count for a serialised FString (friendly name, branch, etc).
///
/// Source: `Constants.cs` -- `public const int MaxFStringSerializedBytes = 1024 * 1024`.
/// The info's FriendlyName uses a tighter 64 KiB limit (see info module).
pub(crate) const MAX_FSTRING_BYTES: i64 = 1024 * 1024;

/// Maximum byte count for a serialised encryption key.
///
/// Source: `ReplayInfoReader.cs` -- `MaxEncryptionKeySizeBytes = 4096`.
pub(crate) const MAX_ENCRYPTION_KEY_BYTES: i32 = 4096;

/// Maximum byte count for the replay info's FriendlyName.
///
/// Source: `ReplayInfoReader.cs` -- `MaxFriendlyNameSerializedBytes = 64 * 1024`.
/// This is tighter than the general FString limit because the friendly name
/// is user-controlled metadata.
pub(crate) const MAX_FRIENDLY_NAME_BYTES: i64 = 64 * 1024;

/// Maximum number of level name entries in the header.
///
/// Source: `ReplayHeaderReader.cs` -- `MaxLevelNamesAndTimes = 1024`.
pub(crate) const MAX_LEVEL_NAMES_AND_TIMES: i32 = 1024;

/// Maximum number of game-specific data strings in the header.
///
/// Source: `ReplayHeaderReader.cs` -- `MaxGameSpecificDataEntries = 128`.
pub(crate) const MAX_GAME_SPECIFIC_DATA: i32 = 128;

/// Maximum decompressed chunk size (256 MiB). Guards against corrupt size fields
/// causing unbounded allocation.
///
/// Source: `ReplayDataChunkPayloadReader.cs` -- `MaxChunkSize = 1024 * 1024 * 256`.
pub(crate) const MAX_CHUNK_SIZE: i32 = 256 * 1024 * 1024;

/// Each custom version entry is a 16-byte GUID + 4-byte version number = 20 bytes.
///
/// Source: `ReplayHeaderReader.cs` -- `CustomVersionEntryByteCount = 20`.
pub(crate) const CUSTOM_VERSION_ENTRY_BYTES: u32 = 20;

/// The expected GUID for the `LocalFileReplay` custom version.
///
/// Source: `LocalFileReplayCustomVersions.cs` --
/// `Guid.Parse("95A4F03E-7E0B-49E4-BA43-D35694FF87D9")`.
///
/// Stored as four little-endian u32 in Unreal's GUID serialisation order.
pub(crate) const LOCAL_REPLAY_GUID: [u32; 4] = [0x95A4_F03E, 0x7E0B_49E4, 0xBA43_D356, 0x94FF_87D9];

/// Expected `LocalFileReplay` custom version number.
///
/// Source: `LocalFileReplayCustomVersions.cs` -- `public const int CustomVersions = 7`.
pub(crate) const LOCAL_REPLAY_VERSION: i32 = 7;
