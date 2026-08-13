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
//! # Members are selected by DECLARED NAME, not by handle number
//!
//! Handle numbers are a per-build layout detail. Build 13.02 removed
//! `TeamEconomy` and `TeamComponents` from `BombGameState` and added
//! `TeamStates`, shifting every later handle down by eight; `RoundResults`
//! moved from 92 and its members from 93..=96 to 80 and 81..=84. Decoders
//! pinned to the old numbers produced NOTHING on that build -- not a wrong
//! value, no value -- and because the failure was discarded without a counter
//! it read as a clean parse for a whole build. See [`framing::member_name`]
//! for why resolution runs handle -> name and never the reverse.
//!
//! Members and payload types per blob, with the handles each build happens to
//! use written down as a reading aid ONLY. Nothing matches on them:
//!
//! ## RoundResults (`BombGameState`)      13.01: 93..=96   13.02: 81..=84
//! - `WinningTeam` (FName)
//! - `WinningTeamRole` (enum byte, variable bit width)
//! - `RoundResult` (enum byte, variable bit width)
//! - `EliminatedTeams` (skipped - opaque nested array). Declared at TWO
//!   consecutive handles in both builds; matching on the name covers both
//!   without either being written down.
//!
//! ## RoundInfos (`OwnerExclusivePlayerInfo`)   40..=44 in both builds
//! - `RoundNumber`, `StartOfRoundMoney`, `StartOfRoundLoadoutValue`,
//!   `EndOfRoundMoney`, `EndOfRoundLoadoutValue` (all Int32)
//!
//! ## TeamEconomy (`BombGameState`, 13.01 only) -- HANDLES, deliberately
//! - 56: ReplicationId (IntPacked), 57: LoadoutValue (Int32),
//!   58: AverageLoadoutValue (Int32)
//!
//! This one keeps the numbers because it has no choice: the replay declares
//! handle 56 as `"241"`, a hardcoded FName index rather than a name, so there
//! is nothing to match on. Generalising it is also pointless -- the property
//! does not exist in 13.02, where team economy moved into a separately
//! replicated `/Script/ShooterGame.BaseTeamState` actor. Nothing in THIS module
//! reads that one and nothing needs to: it replicates plain scalars, so the
//! overlay table types them directly (`LoadoutValue` and `AverageLoadoutValue`,
//! Int32, from `ADDITIONS` in `apply_type_corrections.py`) and the field stream
//! writes them like any other property. The failure counter covers this decoder
//! if 13.01's numbers ever move.
//!
//! # FName wire format (from `FArchive.ReadFNameCore`)
//! ```text
//! [1 bit: is_hardcoded]
//! if hardcoded: [IntPacked: name_index]  -> returned as decimal string
//! else:        [FString: name] [Int32: number_suffix]
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

    /// A primitive nested in the blob was structurally invalid.
    #[error("field decode: {0}")]
    Decode(#[from] crate::DecodeError),

    /// The declared array element count exceeds a sane maximum.
    #[error("array count {count} exceeds maximum {max}")]
    ArrayCountTooLarge { count: u32, max: u32 },

    /// An element index is out of bounds relative to the declared count.
    #[error("element index {index} >= declared count {count}")]
    IndexOutOfBounds { index: u32, count: u32 },

    /// A field payload declared more bits than remain in the stream.
    #[error("field payload {bits} bits exceeds remaining {remaining}")]
    PayloadTooLarge { bits: u32, remaining: u64 },

    /// An unexpected field handle was encountered. Only [`team_economy`] can
    /// raise this; the other two select members by declared name.
    #[error("unsupported field handle {handle} in {context}")]
    UnsupportedHandle { handle: u32, context: &'static str },

    /// The replay declares no name for a handle the blob carries, so there is
    /// nothing to select a member with.
    #[error("undeclared field handle {handle} in {context}")]
    UndeclaredHandle { handle: u32, context: &'static str },

    /// The handle is declared, under a name this decoder has no arm for --
    /// the shape this takes when a build renames or adds a member.
    #[error("unsupported member {name} (handle {handle}) in {context}")]
    UnsupportedMember {
        name: String,
        handle: u32,
        context: &'static str,
    },

    /// Too many fields in a single element (guard against infinite loops).
    #[error("too many fields in element ({context})")]
    TooManyFields { context: &'static str },

    /// A byte enum carried a value this decoder has no variant for.
    ///
    /// The `from_byte` conversions used to return `None` here, and `None` is
    /// also how these members spell "the wire did not send this field" -- so an
    /// unrecognised value became an ABSENT field with nothing counted. That is
    /// exactly the shape a game patch adding an enum variant takes: the column
    /// quietly starts going null on the new rows while every counter stays
    /// clean. Reported instead, so the first one is loud.
    #[error("{enum_name} has no variant for value {value} in {context}")]
    UnknownEnumValue {
        enum_name: &'static str,
        value: u8,
        context: &'static str,
    },

    /// An enum member declared no bits or more than a byte of payload.
    #[error("{name} enum width {bits} is invalid in {context}")]
    InvalidEnumWidth {
        name: String,
        bits: u64,
        context: &'static str,
    },

    /// Bits remain after the blob should have been fully consumed.
    #[error("not fully consumed: {remaining} bits left")]
    NotFullyConsumed { remaining: u64 },

    /// A member did not consume the field window the wire declared for it.
    ///
    /// Each field gets a `sub_reader` of its declared width, and that reader
    /// advances the PARENT past the whole window whatever the member does with
    /// it -- so the blob stays aligned, the remaining members decode, and
    /// `ensure_consumed` at the end is satisfied. The leftover was invisible.
    ///
    /// Concretely: a 64-bit `EndOfRoundMoney` whose first 32 bits read 1900
    /// exported 1900 and dropped the other 32, counted as decoded rather than
    /// failed. Whether 1900 was the value or half of one, nothing could say --
    /// which is the situation this crate treats as worse than a failure.
    #[error("{name} (handle {handle}) left {remaining} of its {declared} bits unread in {context}")]
    MemberNotFullyConsumed {
        name: String,
        handle: u32,
        declared: u32,
        remaining: u64,
        context: &'static str,
    },
}

/// Convenience alias.
pub type Result<T> = core::result::Result<T, StructBlobError>;
