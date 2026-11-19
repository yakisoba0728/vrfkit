//! The values the decoder produces. The exporter builds Parquet columns
//! straight from [`MovementMove`], so its field names and units are a contract.

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

/// The wire location of one emitted move inside its RPC payload.
///
/// All ordinals are zero-based and count encountered occurrences, including
/// ones that emit no move. They do not collapse repeated array properties,
/// sparse update indices or component properties. A move is a decoded prefix;
/// a later error or replacement can still occur in the same RPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovementContext {
    /// Occurrence of the RemoteCharacterUpdates array property in the RPC.
    pub array_ordinal: u32,
    /// Arrival ordinal within this array, distinct from its sparse wire index.
    pub update_ordinal: u32,
    /// Sparse array index, decoded as `encodedIndex - 1`.
    pub update_index: u32,
    /// Occurrence of ComponentDataStream within this update.
    pub component_ordinal: u32,
    /// Decoded move ordinal within this component, independent of marker wrap.
    pub move_index: u32,
}

/// A character update descriptor. The decoder never builds one (moves go out
/// through its callback); kept for callers that construct it.
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
#[derive(Debug, Clone, Copy, Default)]
pub struct RpcDecodeResult {
    /// Total moves decoded across all updates.
    pub total_moves: u32,
    /// Declared array capacity, not the number of changed character updates.
    pub update_count: u32,
    /// Losses, per occurrence; nonzero keeps the batch's payload as a raw row.
    /// A failed component stream loses itself, a failed framing read the rest
    /// of its updates array. Also counted, as each would otherwise look like
    /// empty updates: an index past the declared count, a field longer than its
    /// window, a shooter-GUID field not 32 bits wide, a stream with no GUID,
    /// a missing array zero index, and bits after it other than one IntPacked byte.
    pub error_count: u32,
    /// Component decodes attempted with a known character GUID. Every attempt
    /// is either complete or an error; orphaned components count in error_count.
    pub component_streams: u32,
    /// Components whose complete logical payload was parsed, including empty ones.
    pub component_complete: u32,
    /// Component attempts that failed, also included in [`Self::error_count`].
    /// A decoded move prefix remains emitted and the whole RPC must stay raw.
    pub component_errors: u32,
    /// Complete native envelopes with a zero byte count and zero unused bits.
    pub component_empty: u32,
    /// Envelopes with valid length and unused-bit count, before inner parsing.
    pub component_envelopes: u32,
    /// Payload bytes within those envelopes, including the final unused bits.
    pub component_payload_bytes: u64,
    /// Unused high bits in the last payload byte, between zero and seven per envelope.
    pub component_unused_bits: u64,
    /// Recognized 0xA3 headers, including a header before an unparsed-body error.
    pub component_input_headers: u32,
    /// Recognized optional 0xA4 headers, including one before an unparsed-body error.
    pub component_discrepancy_headers: u32,
}
