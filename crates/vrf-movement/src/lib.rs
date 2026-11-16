//! Decoder for the VALORANT remote-character-movement RPC
//! (`ReplaysClientReceiveRemoteCharacterUpdatesSingleArrayNoAutonomous`): a
//! batch of per-character updates, each carrying a `ComponentDataStream` of
//! movement "moves" (position, rotation, velocity). "Seen" and "never seen"
//! refer to the sample in "Measured on real replays".
//!
//! # Wire format
//!
//! The RPC and each update are property loops: `encodedHandle : IntPacked`
//! (0 -> break, handle = encodedHandle - 1), `payloadBits : IntPacked`, then
//! that many payload bits. A handle not listed below is skipped by its length.
//!
//! ```text
//! RPC:
//!   firstBit          : 1 bit, value discarded (0 in every RPC seen); none -> empty
//!   properties        : handle 1 = RemoteCharacterUpdates array
//!
//! RemoteCharacterUpdates array:
//!   updateCount       : IntPacked
//!   loop:
//!     encodedIndex    : IntPacked  (0 -> break)
//!     index = encodedIndex - 1
//!     update = RemoteCharacterUpdate (below)
//!   (trailing: if exactly 8 bits remain, consume IntPacked padding; never seen)
//!
//! RemoteCharacterUpdate:
//!   properties        : handle 2 = ShooterCharacterNetGuidValue (u32)
//!                       handle 3 = ComponentDataStream
//!
//! ComponentDataStream, byte-wrapped (every stream seen):
//!   byteCount         : u16  (> 0, and fits in remaining bits)
//!   [byteCount*8 bits]: ComponentPayload
//!   trailer           : the rest of the stream, unread (24 bits in every one)
//!                       and tallied (`RpcDecodeResult::envelope_trailer_bits`)
//! ComponentDataStream, direct (never seen): ComponentPayload
//!
//! ComponentPayload:
//!   movementBitCount  : u16  (0 in every stream seen)
//!   MovementSection   : movementBitCount bits (never seen), or all remaining
//!                       when it is 0 or larger than what remains
//!
//! MovementSection:
//!   magic             : u8  (must be 0x52)
//!   loop:
//!     marker          : 3 bits  (0 -> break; else 1 to 7, wrapping to 1)
//!     MovementMove
//!     (if remaining <= 31 bits -> end; the bits stay unread, see below)
//!
//! MovementMove:
//!   header            : 25 bits packed:
//!     [0]      moveType (bool: 0=variant0, 1=variant1)
//!     [1..9]   rotationYawMultiplier (u8)
//!     [9..17]  movementState (u8)
//!     [17..25] unusedByte (u8)
//!   rotationInput     : FixedVector (3 x u16 -> signed x 1/65536)
//!   timestamp         : VLQ (u32)
//!   position          : QuantizedVector (scaleFactor=100)
//!   hasOptionalByte   : 1 bit
//!     [if true] optionalByte : u8
//!   flagAndPackedAngles : 33 bits
//!     [0]      flag48 (bool)
//!     [1..33]  packedAngles: pitch=[0..16], yaw=[16..32]
//!   IF moveType == 1 (variant 1):
//!     variant1Flag    : 1 bit
//!     velocity        : QuantizedVector (scaleFactor=10)
//!   ELSE (variant 0, never seen):
//!     flagAndAngles   : 33 bits
//!       [0]      hasExternalCharacterRef
//!       [1..33]  variant0PackedAngles (u32)
//!     (if hasExternalCharacterRef -> error, not decoded)
//!   errorSentinel     : 1 bit  (must be false)
//! ```
//!
//! [`vrf_bitio::BitReader::read_quantized_vector`] reads each QuantizedVector.
//!
//! # Measured on real replays
//!
//! 80 replays (78 distinct) from 24 builds, 11.06 to 13.06: 18,488,787 RPCs,
//! 156,407,150 component streams, 157,457,629 moves.
//!
//! - Every stream is byte-wrapped and every inner `movementBitCount` is 0, so
//!   every movement window runs to the end of its envelope.
//! - The `<= 31` bits a section ends with after its last move are not padding:
//!   a `000` where the next marker would sit, then 8 to 23 bits that are not
//!   all zero. Nothing reads them (`MAX_MOVEMENT_PADDING_BITS`).
//! - Exactly 24 bits follow the envelope in every stream (3,753,771,600 bits in
//!   all), skipped unread and tallied per stream; what they carry is unknown.
//! - Implemented after an independent parser's grammar but never seen: the
//!   direct form, a sized movement window, the updates array's trailing 8-bit
//!   IntPacked, variant-0 moves, and the f32 and f64 QuantizedVector forms.
//!
//! No Cargo features: every layer is needed to locate the bits of the next.

#![forbid(unsafe_code)]

mod error;
mod moves;
mod rpc;
mod types;

pub use error::MovementError;
pub use rpc::{decode_movement_rpc, decode_movement_rpc_with_context};
pub use types::{MovementContext, MovementMove, MovementUpdate, RpcDecodeResult};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod boundary_tests;
