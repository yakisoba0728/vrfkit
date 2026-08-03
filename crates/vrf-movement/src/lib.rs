//! Decoder for the VALORANT remote-character-movement RPC payload.
//!
//! # Wire format overview
//!
//! The `ReplaysClientReceiveRemoteCharacterUpdatesSingleArrayNoAutonomous` RPC
//! carries a batch of per-character movement updates. Each update contains a
//! `ComponentDataStream` -- a nested binary protocol encoding one or more
//! movement "moves" (position, rotation, velocity snapshots).
//!
//! ## RPC top-level structure
//!
//! ```text
//! +-------------------------------------------------------------------------+
//! | skip_bit            : 1 bit  (if false -> empty payload)                |
//! | loop (property-style framing):                                          |
//! |   encodedHandle     : IntPacked  (0 -> break)                           |
//! |   handle = encodedHandle - 1                                            |
//! |   payloadBits       : IntPacked                                         |
//! |   payload           : [payloadBits] bits                                |
//! |   * handle 1 = RemoteCharacterUpdates array                             |
//! |   * other handles -> skip                                               |
//! +-------------------------------------------------------------------------+
//! ```
//!
//! ## RemoteCharacterUpdates array (handle 1)
//!
//! ```text
//! +-------------------------------------------------------------------------+
//! | updateCount         : IntPacked                                         |
//! | loop:                                                                   |
//! |   encodedIndex      : IntPacked  (0 -> break)                           |
//! |   index = encodedIndex - 1                                              |
//! |   update = ReadRemoteCharacterUpdate(...)                               |
//! | (trailing: if exactly 8 bits remain, consume IntPacked padding)         |
//! +-------------------------------------------------------------------------+
//! ```
//!
//! ## RemoteCharacterUpdate (per-character)
//!
//! ```text
//! +-------------------------------------------------------------------------+
//! | loop (property-style framing):                                          |
//! |   encodedHandle     : IntPacked  (0 -> break)                           |
//! |   handle = encodedHandle - 1                                            |
//! |   payloadBits       : IntPacked                                         |
//! |   payload           : [payloadBits] bits                                |
//! |   * handle 2 = ShooterCharacterNetGuidValue (u32)                       |
//! |   * handle 3 = ComponentDataStream                                      |
//! |   * other handles -> skip                                               |
//! +-------------------------------------------------------------------------+
//! ```
//!
//! ## ComponentDataStream
//!
//! ```text
//! +-------------------------------------------------------------------------+
//! | Option A: byte-wrapped envelope                                         |
//! |   byteCount       : u16  (> 0, and fits in remaining bits)              |
//! |   [byteCount*8 bits]: inner ComponentPayload                            |
//! |                                                                         |
//! | Option B: direct                                                        |
//! |   (falls through to ComponentPayload)                                   |
//! +-------------------------------------------------------------------------+
//!
//! ComponentPayload:
//! +-------------------------------------------------------------------------+
//! | movementBitCount  : u16                                                 |
//! | * if movementBitCount == 0 || > remaining -> movement uses all remaining|
//! | * else -> movement uses exactly movementBitCount bits                   |
//! | MovementSection   : [movementBitCount or remaining] bits                |
//! +-------------------------------------------------------------------------+
//!
//! MovementSection:
//! +-------------------------------------------------------------------------+
//! | magic             : u8  (must be 0x52)                                  |
//! | loop:                                                                   |
//! |   marker          : 3 bits  (0 -> break)                                |
//! |   MovementMove                                                          |
//! |   (if remaining <= 31 bits -> end)                                      |
//! |   nextMarker expected = NextMarker(prev)                                |
//! +-------------------------------------------------------------------------+
//! ```
//!
//! ## MovementMove (single move record)
//!
//! ```text
//! +-------------------------------------------------------------------------+
//! | header            : 25 bits packed:                                     |
//! |   [0]     moveType (bool: 0=variant0, 1=variant1)                       |
//! |   [1..9]  rotationYawMultiplier (u8)                                    |
//! |   [9..17] movementState (u8)                                            |
//! |   [17..25] unusedByte (u8)                                              |
//! | rotationInput     : FixedVector (3 x u16 -> signed x 1/65536)           |
//! | timestamp         : VLQ (u32)                                           |
//! | position          : QuantizedVector (scaleFactor=100)                   |
//! | hasOptionalByte   : 1 bit                                               |
//! |   [if true] optionalByte : u8                                           |
//! | flagAndPackedAngles : 33 bits                                           |
//! |   [0]     flag48 (bool)                                                 |
//! |   [1..33] packedAngles: pitch=[0..16], yaw=[16..32]                     |
//! | IF moveType == 1 (variant 1):                                           |
//! |   variant1Flag    : 1 bit                                               |
//! |   velocity        : QuantizedVector (scaleFactor=10)                    |
//! | ELSE (variant 0):                                                       |
//! |   flagAndAngles   : 33 bits                                             |
//! |     [0]     hasExternalCharacterRef                                     |
//! |     [1..33] variant0PackedAngles (u32)                                  |
//! |   (if hasExternalCharacterRef -> error, not decoded)                    |
//! | errorSentinel     : 1 bit  (must be false)                              |
//! +-------------------------------------------------------------------------+
//! ```
//!
//! ## QuantizedVector
//!
//! ```text
//! +-------------------------------------------------------------------------+
//! | componentBitCountAndExtraInfo : SerializedInt(128)  [7 bits]            |
//! |   componentBits = value & 63                                            |
//! |   extraInfo = value >> 6                                                |
//! |                                                                         |
//! | IF componentBits > 0:                                                   |
//! |   Read 3 signed components of `componentBits` each                      |
//! |   IF extraInfo > 0: divide by scaleFactor                               |
//! | ELIF extraInfo == 0:                                                    |
//! |   3 x f32                                                               |
//! | ELSE:                                                                   |
//! |   3 x f64                                                               |
//! +-------------------------------------------------------------------------+
//! ```

//! # Module map
//!
//! The nesting above is mirrored by the module layout, outermost first:
//!
//! | Module | Layer |
//! |--------|-------|
//! | `rpc` | Batch, updates array, one update, the component data stream |
//! | `moves` | The movement section and one move record |
//! | `primitives` | FixedVector, QuantizedVector, sign extension, VLQ |
//! | `types` | [`MovementMove`], [`MovementUpdate`], [`RpcDecodeResult`] |
//! | `error` | [`MovementError`] |
//!
//! # Accuracy
//!
//! The decoder is validated against the C# reference to **zero** error on yaw,
//! pitch and velocity, and a maximum of 0.0005 on position. The 25-bit move
//! header, the VLQ timestamp and every scale constant are wire format, not
//! style: an equivalent-looking rewrite of the arithmetic changes the numbers.
//!
//! # Cargo features
//!
//! None. This crate decodes exactly one RPC, and every layer above is required
//! to locate the bits of the layer below -- there is no sub-part of it a
//! consumer could decline and still get a move out.

#![forbid(unsafe_code)]

mod error;
mod moves;
mod primitives;
mod rpc;
mod types;

pub use error::MovementError;
pub use rpc::decode_movement_rpc;
pub use types::{MovementMove, MovementUpdate, RpcDecodeResult};

#[cfg(test)]
mod tests;
