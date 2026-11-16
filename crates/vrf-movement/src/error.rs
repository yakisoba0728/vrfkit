use vrf_bitio::BitError;

/// A movement RPC decode failure. Exhaustive on purpose: adding
/// `#[non_exhaustive]` or removing a variant would break downstream matches.
#[derive(Debug, Clone, thiserror::Error)]
pub enum MovementError {
    #[error("bit read error: {0}")]
    Bit(#[from] BitError),

    #[error("invalid movement magic: 0x{0:02X} (expected 0x52)")]
    InvalidMagic(u8),

    #[error("movement marker mismatch: expected {expected}, got {actual}")]
    MarkerMismatch { expected: u8, actual: u8 },

    #[error("movement error sentinel was set")]
    ErrorSentinel,

    #[error("variant-0 external character reference is not supported")]
    Variant0ExternalCharRef,

    #[error("update count too large: {0}")]
    TooManyUpdates(u32),

    #[error("movement component byte count needs 32 bits, only {available_bits} remain")]
    TruncatedComponentHeader { available_bits: u64 },

    #[error("movement component byte count is negative: {count}")]
    NegativeComponentByteCount { count: i32 },

    #[error(
        "movement component declares {byte_count} payload bytes plus one unused-bit byte, but {available_bits} bits remain"
    )]
    ComponentLengthMismatch {
        byte_count: u32,
        available_bits: u64,
    },

    #[error("movement component has {unused_bits} unused bits for {byte_count} payload bytes")]
    InvalidUnusedBitCount { byte_count: u32, unused_bits: u8 },

    #[error("movement input boundary {end_bit} is outside [{minimum}, {payload_bits}]")]
    InvalidInputBoundary {
        end_bit: u16,
        minimum: u64,
        payload_bits: u64,
    },

    /// An unrecognized component stays unresolved; its containing RPC must
    /// retain its full raw payload rather than guessing the new layout.
    #[error("unknown {component} component magic 0x{actual:02X}")]
    UnknownComponentMagic { component: &'static str, actual: u8 },

    /// Header-only components are the observed replay form. Extra data has
    /// no verified interpretation and makes the RPC a raw-preservation case.
    #[error("{component} component has {remaining_bits} unparsed body bits")]
    UnparsedComponentBody {
        component: &'static str,
        remaining_bits: u64,
    },

    /// Never emitted: an index past the declared count is counted in
    /// `RpcDecodeResult::error_count` instead. Kept for API compatibility.
    #[error("update index {index} out of range (count={count})")]
    UpdateIndexOutOfRange { index: u32, count: u32 },
}
