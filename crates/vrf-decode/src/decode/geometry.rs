//! Vector, rotator, transform and replicated-movement readers.
//!
//! These all render into `value_str` -- there is no numeric column that can
//! hold three components -- so each one is a bit read followed by a `Display`
//! of the model type in [`crate::types`].

use vrf_bitio::BitReader;

use super::{DecodeError, DecodedValue, render};
use crate::types::{FQuat, FRepMovement, FRotator, FTransform, FVector, RotatorQuantization};

pub(super) fn decode_vector_float(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(render(read_float_vector(r)?))
}

pub(super) fn decode_vector_double(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(render(read_double_vector(r)?))
}

pub(super) fn decode_vector_net_quantize(
    r: &mut BitReader<'_>,
    scale: u32,
) -> Result<DecodedValue, DecodeError> {
    Ok(render(read_quantized_vector(r, scale)?))
}

pub(super) fn decode_vector_normal(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(render(read_fixed_vector_normal(r)?))
}

pub(super) fn decode_rotation_short(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(render(read_rotation_short(r)?))
}

pub(super) fn decode_rotation_byte(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(render(read_rotation_byte(r)?))
}

pub(super) fn decode_transform(r: &mut BitReader<'_>) -> Result<DecodedValue, DecodeError> {
    Ok(render(read_transform(r)?))
}

pub(super) fn decode_rep_movement(
    r: &mut BitReader<'_>,
    rotation: RotatorQuantization,
) -> Result<DecodedValue, DecodeError> {
    Ok(render(read_rep_movement(r, rotation)?))
}

// -- Shared vector/rotation reading functions ---------------------------------

fn read_float_vector(r: &mut BitReader<'_>) -> Result<FVector, vrf_bitio::BitError> {
    Ok(FVector {
        x: f64::from(r.read_f32()?),
        y: f64::from(r.read_f32()?),
        z: f64::from(r.read_f32()?),
    })
}

fn read_double_vector(r: &mut BitReader<'_>) -> Result<FVector, vrf_bitio::BitError> {
    Ok(FVector {
        x: r.read_f64()?,
        y: r.read_f64()?,
        z: r.read_f64()?,
    })
}

/// Read a quantized vector: 7-bit header encodes component bit count + extra info.
///
/// ```text
/// header = SerializedInt(128)
///   bits [5:0] = componentBitCount
///   bit  [6]   = extraInfo (1 = scaled integer, 0 = float/double fallback)
///
/// if componentBitCount > 0:
///   3 x componentBitCount bits, two's-complement, sign-extended from
///   componentBitCount bits (NOT sign-magnitude: e.g. all-ones reads as -1,
///   not -(max)). See read_packed_quantized_vector below.
///   if extraInfo: divide by scaleFactor
/// else:
///   if extraInfo == 0: 3 x f32 (float vector)
///   else:              3 x f64 (double vector)
/// ```
fn read_quantized_vector(
    r: &mut BitReader<'_>,
    scale_factor: u32,
) -> Result<FVector, vrf_bitio::BitError> {
    let header = r.read_serialized_int(1 << 7)?;
    let component_bit_count = header & 63;
    let extra_info = header >> 6;

    if component_bit_count > 0 {
        read_packed_quantized_vector(r, component_bit_count, extra_info, scale_factor)
    } else if extra_info == 0 {
        read_float_vector(r)
    } else {
        read_double_vector(r)
    }
}

fn read_packed_quantized_vector(
    r: &mut BitReader<'_>,
    component_bit_count: u32,
    extra_info: u32,
    scale_factor: u32,
) -> Result<FVector, vrf_bitio::BitError> {
    let x_raw = r.read_bits(component_bit_count)?;
    let y_raw = r.read_bits(component_bit_count)?;
    let z_raw = r.read_bits(component_bit_count)?;
    let sign_bit = 1u64 << (component_bit_count - 1);

    let fx = (x_raw ^ sign_bit) as i64 - sign_bit as i64;
    let fy = (y_raw ^ sign_bit) as i64 - sign_bit as i64;
    let fz = (z_raw ^ sign_bit) as i64 - sign_bit as i64;

    let (x, y, z) = if extra_info > 0 {
        let sf = f64::from(scale_factor);
        (fx as f64 / sf, fy as f64 / sf, fz as f64 / sf)
    } else {
        (fx as f64, fy as f64, fz as f64)
    };

    Ok(FVector { x, y, z })
}

/// Fixed-point normal vector: 3 x SerializedInt(65536), bias = 32768, scale = 32767.
fn read_fixed_vector_normal(r: &mut BitReader<'_>) -> Result<FVector, vrf_bitio::BitError> {
    const BIAS: i32 = 1 << 15;
    const SCALE: f64 = (BIAS - 1) as f64;
    const MAX: u32 = 1 << 16;

    let dx = r.read_serialized_int(MAX)?;
    let dy = r.read_serialized_int(MAX)?;
    let dz = r.read_serialized_int(MAX)?;

    Ok(FVector {
        x: (dx as i32 - BIAS) as f64 / SCALE,
        y: (dy as i32 - BIAS) as f64 / SCALE,
        z: (dz as i32 - BIAS) as f64 / SCALE,
    })
}

fn read_rotation_short(r: &mut BitReader<'_>) -> Result<FRotator, vrf_bitio::BitError> {
    let pitch = read_compressed_short_component(r)?;
    let yaw = read_compressed_short_component(r)?;
    let roll = read_compressed_short_component(r)?;
    Ok(FRotator { pitch, yaw, roll })
}

fn read_rotation_byte(r: &mut BitReader<'_>) -> Result<FRotator, vrf_bitio::BitError> {
    let pitch = read_compressed_byte_component(r)?;
    let yaw = read_compressed_byte_component(r)?;
    let roll = read_compressed_byte_component(r)?;
    Ok(FRotator { pitch, yaw, roll })
}

fn read_compressed_short_component(r: &mut BitReader<'_>) -> Result<f32, vrf_bitio::BitError> {
    if r.read_bit()? {
        let v = r.read_u16()?;
        Ok(f32::from(v) * (360.0 / 65536.0))
    } else {
        Ok(0.0)
    }
}

fn read_compressed_byte_component(r: &mut BitReader<'_>) -> Result<f32, vrf_bitio::BitError> {
    if r.read_bit()? {
        let v = r.read_u8()?;
        Ok(f32::from(v) * (360.0 / 256.0))
    } else {
        Ok(0.0)
    }
}

fn read_quaternion(r: &mut BitReader<'_>) -> Result<FQuat, vrf_bitio::BitError> {
    Ok(FQuat {
        x: r.read_f32()?,
        y: r.read_f32()?,
        z: r.read_f32()?,
        w: r.read_f32()?,
    })
}

fn read_transform(r: &mut BitReader<'_>) -> Result<FTransform, vrf_bitio::BitError> {
    let rotation = read_quaternion(r)?;
    let translation = read_float_vector(r)?;
    let scale = read_float_vector(r)?;
    Ok(FTransform {
        rotation,
        translation,
        scale,
    })
}

fn read_rep_movement(
    r: &mut BitReader<'_>,
    rotation_quant: RotatorQuantization,
) -> Result<FRepMovement, vrf_bitio::BitError> {
    let simulated_physics_sleep = r.read_bit()?;
    let rep_physics = r.read_bit()?;
    let rep_server_frame = r.read_bit()?;
    let rep_server_handle = r.read_bit()?;

    let location = read_quantized_vector(r, 100)?;
    let rotation = match rotation_quant {
        RotatorQuantization::ByteComponents => read_rotation_byte(r)?,
        RotatorQuantization::ShortComponents => read_rotation_short(r)?,
    };
    let linear_velocity = read_quantized_vector(r, 1)?;

    let angular_velocity = if rep_physics {
        Some(read_quantized_vector(r, 1)?)
    } else {
        None
    };

    let server_frame = if rep_server_frame {
        r.read_int_packed()?
    } else {
        0
    };

    let server_physics_handle = if rep_server_handle {
        r.read_int_packed()?
    } else {
        0
    };

    Ok(FRepMovement {
        location,
        rotation,
        linear_velocity,
        angular_velocity,
        simulated_physics_sleep,
        rep_physics,
        server_frame,
        server_physics_handle,
    })
}
