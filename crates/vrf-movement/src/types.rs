//! The values the decoder produces.
//!
//! Kept separate from the decoding logic because these cross the crate
//! boundary: the exporter builds Parquet columns directly from
//! [`MovementMove`]'s fields, so their names and units are part of the
//! contract, not an implementation detail.

/// A single decoded movement sample (one "move" from one character update).
#[derive(Debug, Clone, Copy)]
pub struct MovementMove {
    /// The character's network GUID (identifies which player/character).
    pub shooter_character_net_guid: u32,
    /// Position in Unreal world coordinates (cm).
    pub pos_x: f64,
    pub pos_y: f64,
    pub pos_z: f64,
    /// Yaw in degrees [0, 360).
    pub yaw: f64,
    /// Pitch in degrees [0, 360).
    pub pitch: f64,
    /// Velocity (cm/s). Only present for variant-1 moves; zero for variant-0.
    pub vel_x: f64,
    pub vel_y: f64,
    pub vel_z: f64,
    /// Server-assigned timestamp (VLQ-encoded tick).
    pub timestamp: u32,
    /// Movement state byte.
    pub movement_state: u8,
    /// Mode flags byte (same as movement_state in the wire format).
    pub mode_flags: u8,
    /// 0 = variant0, 1 = variant1.
    pub move_type: u8,
}

/// A single character update descriptor (carries moves).
#[derive(Debug, Clone)]
pub struct MovementUpdate {
    /// Index within the batch.
    pub index: u32,
    /// The character GUID this update belongs to.
    pub shooter_character_net_guid: Option<u32>,
    /// Number of moves decoded for this update.
    pub move_count: u32,
}

/// Result of decoding the full RPC payload.
#[derive(Debug, Clone, Copy)]
pub struct RpcDecodeResult {
    /// Total moves decoded across all updates.
    pub total_moves: u32,
    /// Number of character updates in the batch.
    pub update_count: u32,
    /// Number of updates that had parse errors.
    ///
    /// An update that fails mid-parse leaves the bit cursor at an
    /// indeterminate position, so the rest of its array is skipped rather than
    /// guessed at. The count is what makes that skip visible instead of silent.
    pub error_count: u32,
}
