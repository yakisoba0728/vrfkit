//! Decoder tests: build a full RPC payload bit by bit, then decode it.
//!
//! These live in their own module because they exercise the whole stack --
//! `rpc` framing down through `moves` into `primitives` -- so they belong to
//! none of those modules individually. The `BitWriter` below is the inverse of
//! the reader under test and is deliberately written out rather than shared
//! with `vrf-bitio`: a bug mirrored in both would cancel out.
use vrf_bitio::BitReader;

use crate::error::MovementError;
use crate::moves::{MOVEMENT_MAGIC, next_marker};
use crate::rpc::{
    COMPONENT_DATA_STREAM_HANDLE, REMOTE_CHARACTER_UPDATES_HANDLE,
    SHOOTER_CHARACTER_NET_GUID_HANDLE, decode_movement_rpc,
};

/// Helper: build a bit vector from individual bit values, then convert to bytes.
struct BitWriter {
    bits: Vec<bool>,
}

impl BitWriter {
    fn new() -> Self {
        Self { bits: Vec::new() }
    }

    fn write_bit(&mut self, v: bool) {
        self.bits.push(v);
    }

    fn write_bits_u64(&mut self, value: u64, count: u32) {
        for i in 0..count {
            self.bits.push((value >> i) & 1 != 0);
        }
    }

    fn write_u8(&mut self, v: u8) {
        self.write_bits_u64(u64::from(v), 8);
    }

    fn write_u16(&mut self, v: u16) {
        self.write_bits_u64(u64::from(v), 16);
    }

    fn write_u32(&mut self, v: u32) {
        self.write_bits_u64(u64::from(v), 32);
    }

    fn write_f32(&mut self, v: f32) {
        self.write_u32(v.to_bits());
    }

    fn write_int_packed(&mut self, mut value: u32) {
        loop {
            let mut next_byte = ((value & 0x7F) << 1) as u8;
            value >>= 7;
            if value != 0 {
                next_byte |= 1;
            }
            self.write_u8(next_byte);
            if value == 0 {
                break;
            }
        }
    }

    fn write_serialized_int(&mut self, value: u32, max_value: u32) {
        let mut written_value = 0u32;
        let mut mask = 1u32;
        while written_value.saturating_add(mask) < max_value {
            let bit = (value & mask) != 0;
            self.write_bit(bit);
            if bit {
                written_value |= mask;
            }
            mask <<= 1;
        }
    }

    fn write_other(&mut self, other: &BitWriter) {
        self.bits.extend_from_slice(&other.bits);
    }

    fn bit_count(&self) -> u32 {
        self.bits.len() as u32
    }

    fn to_bytes(&self) -> Vec<u8> {
        let byte_count = self.bits.len().div_ceil(8);
        let mut bytes = vec![0u8; byte_count];
        for (i, &bit) in self.bits.iter().enumerate() {
            if bit {
                bytes[i >> 3] |= 1 << (i & 7);
            }
        }
        bytes
    }
}

/// Build a single move payload (variant 0 or variant 1).
fn build_move(variant1: bool, timestamp: u32, x: f32, y: f32, z: f32) -> BitWriter {
    let mut w = BitWriter::new();

    // 25-bit header: moveType(1) + rotationYawMultiplier(8) + movementState(8) + unused(8)
    w.write_bit(variant1); // moveType
    w.write_u8(2); // rotationYawMultiplier
    w.write_u8(3); // movementState
    w.write_u8(0); // unused

    // FixedVector rotationInput: 3 x u16 = 48 bits (all zero = center)
    w.write_serialized_int(0x8000, 0x10000);
    w.write_serialized_int(0x8000, 0x10000);
    w.write_serialized_int(0x8000, 0x10000);

    // Timestamp VLQ (= IntPacked)
    w.write_int_packed(timestamp);

    // Position: QuantizedVector(scaleFactor=100)
    // Use componentBits=0, extraInfo=0 -> 3 x f32
    w.write_serialized_int(0, 128); // info = 0 -> componentBits=0, extraInfo=0
    w.write_f32(x);
    w.write_f32(y);
    w.write_f32(z);

    // hasOptionalByte = false
    w.write_bit(false);

    // 33-bit flag+packedAngles: flag48(1) + packedAngles(32)
    w.write_bit(false); // flag48
    w.write_u32(0); // packedAngles (pitch=0, yaw=0)

    if variant1 {
        // variant1Flag + quantized velocity
        w.write_bit(true);
        // QuantizedVector(scaleFactor=10): componentBits=10, extraInfo=1
        let info = 10u32 | (1 << 6); // componentBits=10, extraInfo=1
        w.write_serialized_int(info, 128);
        // 3 signed components of 10 bits each = 30 bits total
        // velocity = (4.0, 5.0, 6.0) -> scaled by 10 = (40, 50, 60)
        let vx = 40i64 as u64 & 0x3FF;
        let vy = 50i64 as u64 & 0x3FF;
        let vz = 60i64 as u64 & 0x3FF;
        let packed = vx | (vy << 10) | (vz << 20);
        w.write_bits_u64(packed, 30);
    } else {
        // variant0: 33-bit flag+angles
        w.write_bit(false); // hasExternalCharacterRef = false
        w.write_u32(0); // variant0PackedAngles
    }

    // errorSentinel = false
    w.write_bit(false);

    w
}

/// Build a ComponentDataStream payload (direct, not byte-wrapped).
fn build_component_data_stream(moves: &[BitWriter]) -> BitWriter {
    let mut movement = BitWriter::new();
    movement.write_u8(MOVEMENT_MAGIC);

    let mut marker: u8 = 1;
    for (i, mv) in moves.iter().enumerate() {
        movement.write_bits_u64(u64::from(marker), 3);
        movement.write_other(mv);
        if i + 1 < moves.len() {
            marker = next_marker(marker);
        }
    }
    // Terminal marker = 0 (only if we haven't hit padding)
    if !moves.is_empty() {
        movement.write_bits_u64(0, 3);
    }

    let mut payload = BitWriter::new();
    payload.write_u16(movement.bit_count() as u16); // movementBitCount
    payload.write_other(&movement);
    payload
}

/// Build a full RPC payload with one character update.
fn build_rpc_payload(shooter_guid: u32, component_stream: &BitWriter) -> BitWriter {
    // Build the single update's property stream
    let mut update = BitWriter::new();
    // handle 2 (ShooterCharacterNetGuidValue): encodedHandle=3, payload=32 bits
    update.write_int_packed(SHOOTER_CHARACTER_NET_GUID_HANDLE + 1);
    update.write_int_packed(32);
    update.write_u32(shooter_guid);
    // handle 3 (ComponentDataStream): encodedHandle=4, payload=stream bits
    update.write_int_packed(COMPONENT_DATA_STREAM_HANDLE + 1);
    update.write_int_packed(component_stream.bit_count());
    update.write_other(component_stream);
    // terminator
    update.write_int_packed(0);

    // Build the updates array
    let mut array = BitWriter::new();
    array.write_int_packed(1); // updateCount = 1
    array.write_int_packed(1); // encodedIndex = 1 -> index 0
    array.write_other(&update);
    array.write_int_packed(0); // array terminator

    // Build the RPC wrapper
    let mut rpc = BitWriter::new();
    rpc.write_bit(false); // first bit (consumed, value discarded per C# TryReadBit(out _))
    // Property-style: handle 1 (RemoteCharacterUpdates)
    rpc.write_int_packed(REMOTE_CHARACTER_UPDATES_HANDLE + 1); // encodedHandle = 2
    rpc.write_int_packed(array.bit_count()); // payload bits
    rpc.write_other(&array);
    rpc.write_int_packed(0); // terminator

    rpc
}

#[test]
fn decodes_single_variant0_move() {
    let mv = build_move(false, 42, 1.25, 2.5, 3.75);
    let stream = build_component_data_stream(&[mv]);
    let rpc = build_rpc_payload(1234, &stream);
    let bytes = rpc.to_bytes();
    let mut reader = BitReader::with_bit_len(&bytes, rpc.bit_count() as u64);

    let mut moves = Vec::new();
    let result = decode_movement_rpc(&mut reader, |m| moves.push(m)).unwrap();

    assert_eq!(result.total_moves, 1);
    assert_eq!(result.update_count, 1);
    assert_eq!(result.error_count, 0);
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].shooter_character_net_guid, 1234);
    assert_eq!(moves[0].move_type, 0);
    assert_eq!(moves[0].timestamp, 42);
    assert!((moves[0].pos_x - 1.25).abs() < 0.001);
    assert!((moves[0].pos_y - 2.5).abs() < 0.001);
    assert!((moves[0].pos_z - 3.75).abs() < 0.001);
    assert_eq!(moves[0].vel_x, 0.0);
    assert_eq!(moves[0].vel_y, 0.0);
    assert_eq!(moves[0].vel_z, 0.0);
}

#[test]
fn decodes_single_variant1_move_with_velocity() {
    let mv = build_move(true, 42, 1.25, 2.5, 3.75);
    let stream = build_component_data_stream(&[mv]);
    let rpc = build_rpc_payload(5678, &stream);
    let bytes = rpc.to_bytes();
    let mut reader = BitReader::with_bit_len(&bytes, rpc.bit_count() as u64);

    let mut moves = Vec::new();
    let result = decode_movement_rpc(&mut reader, |m| moves.push(m)).unwrap();

    assert_eq!(result.total_moves, 1);
    assert_eq!(moves[0].move_type, 1);
    assert!((moves[0].vel_x - 4.0).abs() < 0.001);
    assert!((moves[0].vel_y - 5.0).abs() < 0.001);
    assert!((moves[0].vel_z - 6.0).abs() < 0.001);
}

#[test]
fn decodes_two_moves_in_one_update() {
    let mv1 = build_move(false, 42, 1.0, 2.0, 3.0);
    let mv2 = build_move(false, 84, 10.0, 11.0, 12.0);
    let stream = build_component_data_stream(&[mv1, mv2]);
    let rpc = build_rpc_payload(9999, &stream);
    let bytes = rpc.to_bytes();
    let mut reader = BitReader::with_bit_len(&bytes, rpc.bit_count() as u64);

    let mut moves = Vec::new();
    let result = decode_movement_rpc(&mut reader, |m| moves.push(m)).unwrap();

    assert_eq!(result.total_moves, 2);
    assert_eq!(moves.len(), 2);
    assert_eq!(moves[0].timestamp, 42);
    assert!((moves[0].pos_x - 1.0).abs() < 0.001);
    assert_eq!(moves[1].timestamp, 84);
    assert!((moves[1].pos_x - 10.0).abs() < 0.001);
}

#[test]
fn empty_rpc_returns_zero() {
    // Zero bits -> empty
    let data = [0u8; 0];
    let mut reader = BitReader::with_bit_len(&data, 0);

    let mut moves = Vec::new();
    let result = decode_movement_rpc(&mut reader, |m| moves.push(m)).unwrap();
    assert_eq!(result.total_moves, 0);
    assert!(moves.is_empty());
}

#[test]
fn invalid_magic_returns_error() {
    // Build a stream with wrong magic
    let mut movement = BitWriter::new();
    movement.write_u8(0x00); // wrong magic

    let mut payload = BitWriter::new();
    payload.write_u16(movement.bit_count() as u16);
    payload.write_other(&movement);

    let rpc = build_rpc_payload(1234, &payload);
    let bytes = rpc.to_bytes();
    let mut reader = BitReader::with_bit_len(&bytes, rpc.bit_count() as u64);

    let mut moves = Vec::new();
    let result = decode_movement_rpc(&mut reader, |m| moves.push(m));
    // The error should be caught at the update level, incrementing error_count.
    // Since we catch errors in decode_single_update, it returns Ok with error_count > 0.
    match result {
        Ok(r) => assert_eq!(r.error_count, 1),
        Err(MovementError::InvalidMagic(0x00)) => {} // also acceptable
        Err(e) => panic!("unexpected error: {e}"),
    }
}
