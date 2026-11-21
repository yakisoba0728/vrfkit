use thiserror::Error;

/// A chunk malformed at the framing level; content-block errors live in
/// `vrf-net`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FrameError {
    #[error("bit-IO error during frame parsing: {0}")]
    Bit(String),

    /// A net-field export or export-GUID read failed. The typed source
    /// distinguishes a configured resource refusal from malformed input.
    #[error("schema error during frame parsing: {0}")]
    Schema(#[from] vrf_schema::SchemaError),

    #[error("negative packet size: {size}")]
    NegativePacketSize { size: i32 },

    #[error("packet size {size} exceeds maximum {max}")]
    PacketTooLarge { size: i32, max: i32 },

    #[error("frame time {seconds} s is outside the representable millisecond range")]
    TimeOutOfRange { seconds: f32 },

    #[error("{context}: needed {needed} bytes, only {available} available")]
    Truncated {
        context: &'static str,
        needed: usize,
        available: usize,
    },
}

// Ordinary frame bit failures retain the existing rendered-string contract.
impl From<vrf_bitio::BitError> for FrameError {
    fn from(e: vrf_bitio::BitError) -> Self {
        Self::Bit(e.to_string())
    }
}
