//! Error types for the DemoFrame layer.

use thiserror::Error;

/// All error modes during DemoFrame iteration.
///
/// A frame error means the decompressed chunk is malformed at the framing level.
/// This is distinct from content-block errors (which live in `vrf-net`).
#[derive(Debug, Error)]
pub enum FrameError {
    /// A bit read failed (truncation or malformed primitive).
    #[error("bit-IO error during frame parsing: {0}")]
    Bit(String),

    /// A schema-reader error (net-field export or export-GUID parsing).
    #[error("schema error during frame parsing: {0}")]
    Schema(String),

    /// Packet size declared as negative.
    #[error("negative packet size: {size}")]
    NegativePacketSize { size: i32 },

    /// Packet size exceeds the protocol maximum (2 KiB).
    #[error("packet size {size} exceeds maximum {max}")]
    PacketTooLarge { size: i32, max: i32 },

    /// The data was truncated mid-frame.
    #[error("{context}: needed {needed} bytes, only {available} available")]
    Truncated {
        context: &'static str,
        needed: usize,
        available: usize,
    },
}

impl FrameError {
    /// Wrap a `BitError` into `FrameError::Bit`.
    pub(crate) fn bit(e: vrf_bitio::BitError) -> Self {
        Self::Bit(e.to_string())
    }

    /// Wrap a `SchemaError` into `FrameError::Schema`.
    pub(crate) fn schema(e: vrf_schema::SchemaError) -> Self {
        Self::Schema(e.to_string())
    }
}
