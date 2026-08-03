//! Decoders for Valorant struct-array blobs that arrive as opaque raw bits.
//!
//! # Covered blobs
//!
//! | Field | Export group | Purpose | Module |
//! |-------|-------------|---------|--------|
//! | `RoundResults` | `BombGameState` | Per-round winning team and outcome | [`round_results`] |
//! | `TeamEconomy` | `BombGameState` | Team loadout value per round | [`team_economy`] |
//! | `RoundInfos` | `OwnerExclusivePlayerInfo` | Per-player per-round credit | [`round_infos`] |
//!
//! # Wire layout (established from C# reference parser + corpus validation)
//!
//! All three use the same UE RepLayout dynamic-array serialization, which
//! [`framing`] reads:
//!
//! ```text
//! [IntPacked: declared_count]
//! repeat {
//!     [IntPacked: encoded_index]  // 0 = end, otherwise index = encoded - 1
//!     repeat {
//!         [IntPacked: encoded_handle]  // 0 = end, otherwise handle = encoded - 1
//!         [IntPacked: bit_count]       // payload length in bits
//!         [bits: payload]              // field-specific content
//!     }
//! }
//! ```
//!
//! Field handles and payload types per blob:
//!
//! ## RoundResults (handles from `AresRoundResults.cs`)
//! - 93: WinningTeam (FName)
//! - 94: WinningTeamRole (enum byte, variable bit width)
//! - 95: RoundResult (enum byte, variable bit width)
//! - 96: EliminatedTeams (skipped - opaque nested array)
//!
//! ## TeamEconomy (handles from `AresTeamEconomy.cs`)
//! - 56: ReplicationId (IntPacked)
//! - 57: LoadoutValue (Int32)
//! - 58: AverageLoadoutValue (Int32)
//!
//! ## RoundInfos (handles from `OwnerExclusivePlayerInfoDescriptor.cs`)
//! - 40: RoundNumber (Int32)
//! - 41: StartOfRoundMoney (Int32)
//! - 42: StartOfRoundLoadoutValue (Int32)
//! - 43: EndOfRoundMoney (Int32)
//! - 44: EndOfRoundLoadoutValue (Int32)
//!
//! # FName wire format (from `FArchive.ReadFNameCore`)
//! ```text
//! [1 bit: is_hardcoded]
//! if hardcoded: [IntPacked: name_index]  -> returned as decimal string
//! else:        [FString: name] [Int32: number_suffix (ignored)]
//! ```
//!
//! # Volume
//!
//! These are per-round rows, not per-field ones: a replay carries a few dozen
//! of each. Nothing here is on a hot path, so it is written for the wire
//! format's clarity rather than for throughput.

mod framing;
mod round_infos;
mod round_results;
mod team_economy;

#[cfg(test)]
mod tests;

pub use round_infos::{PlayerRoundInfo, decode_round_infos};
pub use round_results::{AresRoundOutcome, AresTeamRole, RoundResult, decode_round_results};
pub use team_economy::{TeamEconomyUpdate, decode_team_economy};

/// Errors that can occur while decoding a struct-array blob.
#[derive(Debug, Clone, thiserror::Error)]
pub enum StructBlobError {
    /// The underlying bit reader hit EOF or produced a malformed primitive.
    #[error("bit read: {0}")]
    BitIo(#[from] vrf_bitio::BitError),

    /// The declared array element count exceeds a sane maximum.
    #[error("array count {count} exceeds maximum {max}")]
    ArrayCountTooLarge { count: u32, max: u32 },

    /// An element index is out of bounds relative to the declared count.
    #[error("element index {index} >= declared count {count}")]
    IndexOutOfBounds { index: u32, count: u32 },

    /// A field payload declared more bits than remain in the stream.
    #[error("field payload {bits} bits exceeds remaining {remaining}")]
    PayloadTooLarge { bits: u32, remaining: u64 },

    /// An unexpected field handle was encountered.
    #[error("unsupported field handle {handle} in {context}")]
    UnsupportedHandle { handle: u32, context: &'static str },

    /// Too many fields in a single element (guard against infinite loops).
    #[error("too many fields in element ({context})")]
    TooManyFields { context: &'static str },

    /// Bits remain after the blob should have been fully consumed.
    #[error("not fully consumed: {remaining} bits left")]
    NotFullyConsumed { remaining: u64 },
}

/// Convenience alias.
pub type Result<T> = core::result::Result<T, StructBlobError>;
