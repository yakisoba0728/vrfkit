//! Vector, rotator and replicated-movement decoders, ported from the C#
//! reference's `PrimitiveDecodersVectorTests.cs`.
//!
//! The `write_*` helpers are the encoder side of each wire format, so a
//! test here specifies the layout in both directions.

use crate::decode::{DecodeError, DecodedValue, FieldType, decode_field};
use crate::types::RotatorQuantization;

/// Helper: build a quantized vector bitstream.
/// Format: SerializedInt(128) header + 3 x componentBitCount signed components.
fn write_quantized_vector(
    x: f64,
    y: f64,
    z: f64,
    scale_factor: u32,
    component_bit_count: u32,
) -> (Vec<u8>, u32) {
    let mut bits: Vec<bool> = Vec::new();
    // Header: info = componentBitCount | (1 << 6) -- indicates scaled integer
    let info = component_bit_count | (1 << 6);
    write_serialized_int(&mut bits, info, 1 << 7);
    // Components
    let xi = (x * f64::from(scale_factor)).round() as i64;
    let yi = (y * f64::from(scale_factor)).round() as i64;
    let zi = (z * f64::from(scale_factor)).round() as i64;
    write_signed_bits(&mut bits, xi, component_bit_count);
    write_signed_bits(&mut bits, yi, component_bit_count);
    write_signed_bits(&mut bits, zi, component_bit_count);
    bits_to_bytes(&bits)
}

fn write_serialized_int(bits: &mut Vec<bool>, value: u32, max: u32) {
    let mut written_value = 0u32;
    let mut mask = 1u32;
    while written_value + mask < max {
        let bit = (value & mask) != 0;
        bits.push(bit);
        if bit {
            written_value |= mask;
        }
        mask <<= 1;
    }
}

fn write_signed_bits(bits: &mut Vec<bool>, value: i64, count: u32) {
    let mask = if count == 64 {
        u64::MAX
    } else {
        (1u64 << count) - 1
    };
    let raw = (value as u64) & mask;
    for i in 0..count {
        bits.push((raw >> i) & 1 != 0);
    }
}

fn write_compressed_short_rotator_component(bits: &mut Vec<bool>, value: u16) {
    bits.push(value != 0);
    if value != 0 {
        for i in 0..16 {
            bits.push((value >> i) & 1 != 0);
        }
    }
}

fn write_compressed_byte_rotator_component(bits: &mut Vec<bool>, value: u8) {
    bits.push(value != 0);
    if value != 0 {
        for i in 0..8 {
            bits.push((value >> i) & 1 != 0);
        }
    }
}

fn write_int_packed(bits: &mut Vec<bool>, mut value: u32) {
    loop {
        let mut byte_val = ((value & 0x7F) << 1) as u8;
        value >>= 7;
        if value != 0 {
            byte_val |= 1;
        }
        for i in 0..8 {
            bits.push((byte_val >> i) & 1 != 0);
        }
        if value == 0 {
            break;
        }
    }
}

fn bits_to_bytes(bits: &[bool]) -> (Vec<u8>, u32) {
    let byte_count = bits.len().div_ceil(8);
    let mut bytes = vec![0u8; byte_count];
    for (i, &b) in bits.iter().enumerate() {
        if b {
            bytes[i / 8] |= 1 << (i % 8);
        }
    }
    (bytes, bits.len() as u32)
}

#[test]
fn vector_float_reads_three_floats() {
    let mut data = Vec::new();
    data.extend_from_slice(&1.25f32.to_le_bytes());
    data.extend_from_slice(&(-2.5f32).to_le_bytes());
    data.extend_from_slice(&3.75f32.to_le_bytes());
    let result = decode_field(FieldType::VectorFloat, &data, 96).unwrap();
    match result {
        DecodedValue::Str(s) => assert_eq!(s, "(1.25,-2.5,3.75)"),
        _ => panic!("expected Str"),
    }
}

#[test]
fn vector_double_reads_three_doubles() {
    let mut data = Vec::new();
    data.extend_from_slice(&1.25f64.to_le_bytes());
    data.extend_from_slice(&(-2.5f64).to_le_bytes());
    data.extend_from_slice(&3.75f64.to_le_bytes());
    let result = decode_field(FieldType::VectorDouble, &data, 192).unwrap();
    match result {
        DecodedValue::Str(s) => assert_eq!(s, "(1.25,-2.5,3.75)"),
        _ => panic!("expected Str"),
    }
}

#[test]
fn quantized_vector_scale1() {
    let (data, bit_count) = write_quantized_vector(10.0, -2.0, 3.0, 1, 6);
    let result = decode_field(FieldType::VectorNetQuantize { scale: 1 }, &data, bit_count).unwrap();
    match result {
        DecodedValue::Str(s) => assert_eq!(s, "(10,-2,3)"),
        _ => panic!("expected Str"),
    }
}

#[test]
fn quantized_vector_scale10() {
    let (data, bit_count) = write_quantized_vector(1.2, -3.4, 5.6, 10, 7);
    let result =
        decode_field(FieldType::VectorNetQuantize { scale: 10 }, &data, bit_count).unwrap();
    match result {
        DecodedValue::Str(s) => {
            // Parse back to check within tolerance
            let nums: Vec<f64> = s
                .trim_matches(|c| c == '(' || c == ')')
                .split(',')
                .map(|n| n.parse::<f64>().unwrap())
                .collect();
            assert!((nums[0] - 1.2).abs() < 1e-9, "x={}", nums[0]);
            assert!((nums[1] - (-3.4)).abs() < 1e-9, "y={}", nums[1]);
            assert!((nums[2] - 5.6).abs() < 1e-9, "z={}", nums[2]);
        }
        _ => panic!("expected Str"),
    }
}

#[test]
fn quantized_vector_scale100() {
    let (data, bit_count) = write_quantized_vector(1.23, -4.56, 7.89, 100, 11);
    let result = decode_field(
        FieldType::VectorNetQuantize { scale: 100 },
        &data,
        bit_count,
    )
    .unwrap();
    match result {
        DecodedValue::Str(s) => {
            let nums: Vec<f64> = s
                .trim_matches(|c| c == '(' || c == ')')
                .split(',')
                .map(|n| n.parse::<f64>().unwrap())
                .collect();
            assert!((nums[0] - 1.23).abs() < 1e-9);
            assert!((nums[1] - (-4.56)).abs() < 1e-9);
            assert!((nums[2] - 7.89).abs() < 1e-9);
        }
        _ => panic!("expected Str"),
    }
}

#[test]
fn quantized_vector_rejects_a_zero_scale() {
    let (data, bit_count) = write_quantized_vector(1.0, -2.0, 3.0, 1, 4);
    let err = decode_field(FieldType::VectorNetQuantize { scale: 0 }, &data, bit_count)
        .expect_err("a zero divisor must not decode to infinite components");
    assert!(
        matches!(err, DecodeError::InvalidQuantizationScale { scale: 0 }),
        "got {err:?}"
    );
}

#[test]
fn rep_movement_decodes_required_fields() {
    let mut bits: Vec<bool> = Vec::new();
    // 4 flag bits: all false
    bits.extend([false, false, false, false]);
    // Location: VectorNetQuantize100 (scale=100, componentBitCount=11)
    let info = 11u32 | (1 << 6);
    write_serialized_int(&mut bits, info, 1 << 7);
    let xi = (1.23_f64 * 100.0).round() as i64;
    let yi = (-4.56_f64 * 100.0).round() as i64;
    let zi = (7.89_f64 * 100.0).round() as i64;
    write_signed_bits(&mut bits, xi, 11);
    write_signed_bits(&mut bits, yi, 11);
    write_signed_bits(&mut bits, zi, 11);
    // Rotation short: all zero (3 bits, all false = no rotation data)
    write_compressed_short_rotator_component(&mut bits, 0);
    write_compressed_short_rotator_component(&mut bits, 0);
    write_compressed_short_rotator_component(&mut bits, 0);
    // Linear velocity: VectorNetQuantize(1), scale=1, componentBitCount=6
    let linfo = 6u32 | (1 << 6);
    write_serialized_int(&mut bits, linfo, 1 << 7);
    write_signed_bits(&mut bits, 10, 6);
    write_signed_bits(&mut bits, -2, 6);
    write_signed_bits(&mut bits, 3, 6);

    let (data, bit_count) = bits_to_bytes(&bits);
    let result = decode_field(
        FieldType::RepMovement {
            rotation: RotatorQuantization::ShortComponents,
        },
        &data,
        bit_count,
    )
    .unwrap();
    // Assert the whole string, not substrings: the members that carry no
    // data here (angular_velocity, the two counters) are exactly the ones a
    // substring check cannot notice going missing.
    match result {
        DecodedValue::Str(s) => assert_eq!(
            s,
            concat!(
                r#"{"linear_velocity":{"x":10,"y":-2,"z":3},"#,
                r#""angular_velocity":null,"#,
                r#""location":{"x":1.23,"y":-4.56,"z":7.89},"#,
                r#""rotation":{"pitch":0,"yaw":0,"roll":0},"#,
                r#""simulated_physics_sleep":false,"rep_physics":false,"#,
                r#""server_frame":0,"server_physics_handle":0}"#
            )
        ),
        _ => panic!("expected Str"),
    }
}

#[test]
fn rep_movement_decodes_optional_fields() {
    let mut bits: Vec<bool> = Vec::new();
    // 4 flag bits: all true
    bits.extend([true, true, true, true]);
    // Location
    let info = 11u32 | (1 << 6);
    write_serialized_int(&mut bits, info, 1 << 7);
    let xi = (1.23_f64 * 100.0).round() as i64;
    let yi = (-4.56_f64 * 100.0).round() as i64;
    let zi = (7.89_f64 * 100.0).round() as i64;
    write_signed_bits(&mut bits, xi, 11);
    write_signed_bits(&mut bits, yi, 11);
    write_signed_bits(&mut bits, zi, 11);
    // Rotation short: pitch=90deg(16384), yaw=180deg(32768), roll=270deg(49152)
    write_compressed_short_rotator_component(&mut bits, 16384);
    write_compressed_short_rotator_component(&mut bits, 32768);
    write_compressed_short_rotator_component(&mut bits, 49152);
    // Linear velocity
    let linfo = 6u32 | (1 << 6);
    write_serialized_int(&mut bits, linfo, 1 << 7);
    write_signed_bits(&mut bits, 10, 6);
    write_signed_bits(&mut bits, -2, 6);
    write_signed_bits(&mut bits, 3, 6);
    // Angular velocity (bRepPhysics=true)
    let ainfo = 5u32 | (1 << 6);
    write_serialized_int(&mut bits, ainfo, 1 << 7);
    write_signed_bits(&mut bits, -4, 5);
    write_signed_bits(&mut bits, 5, 5);
    write_signed_bits(&mut bits, -6, 5);
    // Server frame (bRepServerFrame=true)
    write_int_packed(&mut bits, 123);
    // Server physics handle (bRepServerHandle=true)
    write_int_packed(&mut bits, 456);

    let (data, bit_count) = bits_to_bytes(&bits);
    let result = decode_field(
        FieldType::RepMovement {
            rotation: RotatorQuantization::ShortComponents,
        },
        &data,
        bit_count,
    )
    .unwrap();
    // server_physics_handle=456 is asserted here for the same reason: the
    // old compact form had no slot for it at all, so no test could see it.
    match result {
        DecodedValue::Str(s) => assert_eq!(
            s,
            concat!(
                r#"{"linear_velocity":{"x":10,"y":-2,"z":3},"#,
                r#""angular_velocity":{"x":-4,"y":5,"z":-6},"#,
                r#""location":{"x":1.23,"y":-4.56,"z":7.89},"#,
                r#""rotation":{"pitch":90,"yaw":180,"roll":270},"#,
                r#""simulated_physics_sleep":true,"rep_physics":true,"#,
                r#""server_frame":123,"server_physics_handle":456}"#
            )
        ),
        _ => panic!("expected Str"),
    }
}

#[test]
fn rep_movement_byte_quantized_rotation() {
    let mut bits: Vec<bool> = Vec::new();
    // 4 flag bits: all false
    bits.extend([false, false, false, false]);
    // Location
    let info = 11u32 | (1 << 6);
    write_serialized_int(&mut bits, info, 1 << 7);
    let xi = (1.23_f64 * 100.0).round() as i64;
    let yi = (-4.56_f64 * 100.0).round() as i64;
    let zi = (7.89_f64 * 100.0).round() as i64;
    write_signed_bits(&mut bits, xi, 11);
    write_signed_bits(&mut bits, yi, 11);
    write_signed_bits(&mut bits, zi, 11);
    // Rotation byte: 64->90deg, 128->180deg, 192->270deg
    write_compressed_byte_rotator_component(&mut bits, 64);
    write_compressed_byte_rotator_component(&mut bits, 128);
    write_compressed_byte_rotator_component(&mut bits, 192);
    // Linear velocity
    let linfo = 6u32 | (1 << 6);
    write_serialized_int(&mut bits, linfo, 1 << 7);
    write_signed_bits(&mut bits, 10, 6);
    write_signed_bits(&mut bits, -2, 6);
    write_signed_bits(&mut bits, 3, 6);

    let (data, bit_count) = bits_to_bytes(&bits);
    let result = decode_field(
        FieldType::RepMovement {
            rotation: RotatorQuantization::ByteComponents,
        },
        &data,
        bit_count,
    )
    .unwrap();
    match result {
        DecodedValue::Str(s) => {
            assert!(
                s.contains(r#""rotation":{"pitch":90,"yaw":180,"roll":270}"#),
                "got: {s}"
            );
        }
        _ => panic!("expected Str"),
    }
}

/// A `ReplicatedMovement` whose quantized vector takes the raw-`f32` fallback
/// (`componentBitCount == 0`, `extraInfo == 0`) can carry any bit pattern,
/// including a NaN. `FRepMovement`'s `Display` renders a JSON object, and
/// `NaN` is not a JSON literal -- so a payload like this used to emit
/// `"x":NaN` into `value_str` while every decode counter reported success.
///
/// The doc comment at `types.rs` asserted "every component is finite by
/// construction"; that reasoning covers only the quantized path and not this
/// fallback.
#[test]
fn rep_movement_with_a_non_finite_component_is_rejected() {
    let mut bits: Vec<bool> = Vec::new();
    // Four leading flags, all clear: no physics, no server frame, no handle.
    bits.extend(std::iter::repeat_n(false, 4));
    // Location: header 0 -> componentBitCount 0, extraInfo 0 -> three raw f32.
    write_serialized_int(&mut bits, 0, 1 << 7);
    for word in [0x7fc0_0000u32, 1.0f32.to_bits(), 2.0f32.to_bits()] {
        for i in 0..32 {
            bits.push((word >> i) & 1 != 0);
        }
    }
    // Rotation (byte-quantized): three cleared presence flags.
    bits.extend(std::iter::repeat_n(false, 3));
    // Linear velocity: componentBitCount 1, extraInfo 1, three 1-bit values.
    write_serialized_int(&mut bits, 1 | (1 << 6), 1 << 7);
    bits.extend(std::iter::repeat_n(false, 3));

    let (data, bit_count) = bits_to_bytes(&bits);
    let result = decode_field(
        FieldType::RepMovement {
            rotation: RotatorQuantization::ByteComponents,
        },
        &data,
        bit_count,
    );

    let err = result.expect_err("a NaN component must not decode as success");
    assert!(
        matches!(err, DecodeError::NonFiniteComponent { .. }),
        "expected NonFiniteComponent, got {err:?}"
    );
}
