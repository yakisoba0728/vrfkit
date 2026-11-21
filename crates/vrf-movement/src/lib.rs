//! Decoder for the VALORANT remote-character-movement RPC
//! (`ReplaysClientReceiveRemoteCharacterUpdatesSingleArrayNoAutonomous`): a
//! batch of per-character updates, each carrying a `ComponentDataStream` of
//! movement "moves" (position, rotation, velocity). Unknown component bodies
//! count as errors so the caller can retain the complete raw RPC.
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
//! ComponentDataStream:
//!   byteCount         : i32 (nonnegative)
//!   payload           : exactly byteCount bytes
//!   unusedBits        : u8 (0..7; must be 0 for an empty payload)
//!   (nothing may follow; the payload's final unusedBits are outside its bit window)
//!
//! ComponentPayload (nonempty, byteCount*8 - unusedBits logical bits):
//!   MovementSection
//!   inputEndBit       : u16 (absolute bit position within ComponentPayload)
//!   inputHeader       : u8 (0xA3)
//!   inputBody         : through inputEndBit (only empty bodies are decoded)
//!   discrepancyHeader : u8 (0xA4), only when logical bits remain
//!   discrepancyBody   : through payload end (only empty bodies are decoded)
//!
//! MovementSection:
//!   magic             : u8 (must be 0x52)
//!   loop:
//!     marker          : 3 bits (0 -> break; else 1 to 7, wrapping to 1)
//!     MovementMove
//!   (the zero marker is required even when few bits remain)
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
//!   ELSE (variant 0, covered by synthetic tests):
//!     flagAndAngles   : 33 bits
//!       [0]      hasExternalCharacterRef
//!       [1..33]  variant0PackedAngles (u32)
//!     (if hasExternalCharacterRef -> error, not decoded)
//!   errorSentinel     : 1 bit  (must be false)
//! ```
//!
//! [`vrf_bitio::BitReader::read_quantized_vector`] reads each QuantizedVector.
//!
//! # Envelope evidence and preservation
//!
//! Six public replays spanning builds 12.05 through 13.00 contain 6,211,925
//! component streams. Every outer window matches the native byte count and
//! unused-bit byte, and every logical payload has an explicit movement zero
//! marker followed by a header-only input component. An optional header-only
//! discrepancy component appears in 5,764,165 streams.
//!
//! The observation pass reused the movement-value decoder: it establishes
//! envelope structure and movement counts, not independent numeric accuracy.
//! The method, source references and counter migration are recorded in
//! `docs/NATIVE_MOVEMENT_COMPONENTS.md` in the repository.
//!
//! No direct or u16-sized alternative is admitted without wire evidence.
//! An unknown header/body or invalid boundary increments `error_count` and
//! `component_errors`. Successfully decoded moves remain emitted; the caller
//! must preserve the complete RPC when errors are reported. Unused high bits
//! are excluded by their declared count, without assuming their values are zero.
//! The outer RPC length bounds reads; reference-parser policy limits on move
//! count and payload bytes are not imposed here.
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
