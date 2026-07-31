//! Shared types used across the replication layer.

/// Maximum payload size in bits for a single packet (2 KB = 16 384 bits).
///
/// Used as the upper bound for `read_serialized_int` when reading the bunch
/// payload bit count. Matches Unreal's `MAX_PACKET_SIZE * 8`.
pub const MAX_PACKET_SIZE_BITS: u32 = 2 * 1024 * 8;

/// Maximum recursion depth for `InternalLoadObject`.
pub const MAX_NET_GUID_RECURSION: u32 = 16;

/// Maximum number of GUIDs in a single package-map export bunch.
pub const MAX_GUID_COUNT: u32 = 2048;

/// Reason a channel was closed by the server.
///
/// ```text
/// Bit layout: read_serialized_int(MAX = 15)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum ChannelCloseReason {
    /// Actor was destroyed.
    #[default]
    Destroyed = 0,
    /// Actor entered dormancy (still logically alive).
    Dormancy = 1,
}

impl ChannelCloseReason {
    /// The `MAX` enum sentinel used by `read_serialized_int`.
    pub const MAX: u32 = 15;

    /// Convert from raw wire value.
    #[must_use]
    pub fn from_raw(v: u32) -> Self {
        match v {
            1 => Self::Dormancy,
            _ => Self::Destroyed,
        }
    }
}

/// A network GUID as transmitted on the wire.
///
/// - `0` means invalid / not present.
/// - `1` means "default object" (triggers export-flags read).
/// - Odd values are static (level-placed) actors.
/// - Even non-zero values are dynamic (spawned) actors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct NetworkGuid(pub u32);

impl NetworkGuid {
    /// A GUID of zero means "no object".
    #[must_use]
    #[inline]
    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }

    /// GUID == 1 is the default object (always triggers export flags).
    #[must_use]
    #[inline]
    pub const fn is_default(self) -> bool {
        self.0 == 1
    }

    /// Dynamic actors have even, non-zero GUIDs.
    #[must_use]
    #[inline]
    pub const fn is_dynamic(self) -> bool {
        self.is_valid() && (self.0 & 1) == 0
    }
}

/// Flags read after a net GUID when exporting path information.
///
/// ```text
/// Bit layout: 1 byte (8 bits), only low 3 meaningful
///   bit 0 — HasPath
///   bit 1 — NoLoad
///   bit 2 — HasNetworkChecksum
/// ```
#[derive(Debug, Clone, Copy)]
pub struct ExportFlags(pub u8);

impl ExportFlags {
    pub const HAS_PATH: u8 = 1 << 0;
    #[allow(dead_code)]
    pub const NO_LOAD: u8 = 1 << 1;
    pub const HAS_NETWORK_CHECKSUM: u8 = 1 << 2;

    #[must_use]
    #[inline]
    pub const fn has_path(self) -> bool {
        self.0 & Self::HAS_PATH != 0
    }

    #[must_use]
    #[inline]
    pub const fn has_network_checksum(self) -> bool {
        self.0 & Self::HAS_NETWORK_CHECKSUM != 0
    }
}

/// 3D vector as decoded from the spawn data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FVector {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Rotation as decoded from compressed short format.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FRotator {
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
}
