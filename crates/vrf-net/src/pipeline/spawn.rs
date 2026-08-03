//! Dynamic-actor spawn data: archetype, level, transform and velocity.
//!
//! This is the block Unreal writes immediately after the actor GUID when a
//! channel opens for a *dynamic* (even, non-zero GUID) actor. It is small and
//! rare -- 2 028 opens on the reference replay against 530 401 bunches -- but
//! its bit width is load-bearing for everything after it in the same bunch, so
//! the reasoning below is kept next to the reads rather than in a design doc.

use vrf_bitio::BitReader;

use crate::error::Result;
use crate::net_guid::{self, GuidPathSink};
use crate::types::{FRotator, FVector};

use super::ActorChannelState;

/// Default spawn location and velocity when the wire omits them.
const ORIGIN: FVector = FVector {
    x: 0.0,
    y: 0.0,
    z: 0.0,
};

/// Default spawn scale when the wire omits it.
const UNIT_SCALE: FVector = FVector {
    x: 1.0,
    y: 1.0,
    z: 1.0,
};

/// Quantization divisor Unreal uses for the spawn transform components.
const SPAWN_SCALE_FACTOR: i32 = 10;

/// Read the spawn block for a dynamic actor into `state`.
pub(super) fn read_dynamic_spawn_data(
    payload: &mut BitReader<'_>,
    state: &mut ActorChannelState,
    sink: &mut dyn GuidPathSink,
) -> Result<()> {
    // Archetype. Its path is what identifies the replay controller later, so
    // this read must happen before the net-player-index check in `channel.rs`.
    state.archetype_net_guid = net_guid::internal_load_object(payload, false, 0, sink)?;
    // Level
    state.level_guid = net_guid::internal_load_object(payload, false, 0, sink)?;
    // Location -- defaults to the origin, not to absent.
    state.spawn_location = read_optional_quantized_vector(payload, SPAWN_SCALE_FACTOR, ORIGIN)?;
    // Rotation
    if payload.read_bit()? {
        state.spawn_rotation = Some(read_rotation_short(payload)?);
    }
    // Scale -- defaults to unit scale, not to the origin.
    state.spawn_scale = read_optional_quantized_vector(payload, SPAWN_SCALE_FACTOR, UNIT_SCALE)?;
    // Velocity -- unconditional, exactly as NewActorSerializer.cs:69-72 reads
    // it.
    //
    // This used to be gated on the actor being a PlayerController, on the
    // stated premise that "PlayerController actors set bReplicateMovement ==
    // false, so their spawn data omits velocity entirely". That premise was
    // invented; nothing in the reference or the wire supports it. The bit is
    // present with value 0, which is exactly why the reference reports a zero
    // velocity rather than none.
    //
    // Skipping it cost one bit at the head of the controller's opening bunch.
    // See PROJECT_STATUS 17-A for why one bit was invisible and why this must
    // be fixed together with the net-player-index byte.
    state.spawn_velocity = read_optional_quantized_vector(payload, SPAWN_SCALE_FACTOR, ORIGIN)?;
    Ok(())
}

/// Read an optional, optionally-quantized vector.
///
/// ```text
/// Bit layout:
///   hasValue         : 1 bit
///   [if !hasValue -> return the default]
///   isQuantized      : 1 bit
///   [if isQuantized]
///     componentInfo  : SerializedInt(128)
///     componentBitCount = info & 63
///     extraInfo = info >> 6
///     [if componentBitCount > 0] -> packed quantized
///     [else if extraInfo == 0]   -> 3 x f32
///     [else]                     -> 3 x f64
///   [else] -> 3 x f64
/// ```
///
/// A clear leading bit does not mean "absent" -- it means "take the default".
/// `ArchiveVectorReaders.ReadOptionalQuantizedVector` returns `defaultVector`
/// there, and `NewActorSerializer.cs:56-72` passes (0,0,0) for location and
/// velocity and (1,1,1) for scale.
///
/// Returning `None` instead collapsed that case into the genuinely-absent one:
/// a static actor never enters the spawn block at all, so its location is
/// unknown, while a dynamic actor with the bit clear has a known location of
/// exactly (0,0,0). On 02d4d478 that is 66 actors -- game state, player state,
/// vote and mission actors, which really do sit at the origin -- reported as
/// having no location alongside the 27 that truly have none.
fn read_optional_quantized_vector(
    reader: &mut BitReader<'_>,
    scale_factor: i32,
    default: FVector,
) -> Result<Option<FVector>> {
    if !reader.read_bit()? {
        return Ok(Some(default));
    }

    if !reader.read_bit()? {
        // Unquantized: 3x f64.
        return Ok(Some(read_f64_vector(reader)?));
    }

    let info = reader.read_serialized_int(128)?;
    let component_bit_count = info & 63;
    let extra_info = info >> 6;

    if component_bit_count == 0 {
        return Ok(Some(if extra_info == 0 {
            let x = f64::from(reader.read_f32()?);
            let y = f64::from(reader.read_f32()?);
            let z = f64::from(reader.read_f32()?);
            FVector { x, y, z }
        } else {
            read_f64_vector(reader)?
        }));
    }

    let x = reader.read_bits(component_bit_count)?;
    let y = reader.read_bits(component_bit_count)?;
    let z = reader.read_bits(component_bit_count)?;

    let sign_bit = 1u64 << (component_bit_count - 1);
    let sign_bias = sign_bit as i64;
    let fx = (x ^ sign_bit) as i64 - sign_bias;
    let fy = (y ^ sign_bit) as i64 - sign_bias;
    let fz = (z ^ sign_bit) as i64 - sign_bias;

    // `extra_info == 0` means the components are already whole units; anything
    // else means they were multiplied by the scale factor before quantizing.
    // The two arms stay separate rather than dividing by a 1.0 divisor: these
    // values reach Parquet unrounded, and "the compiler surely folds it" is not
    // the standard this crate's output is held to.
    Ok(Some(if extra_info > 0 {
        let divisor = f64::from(scale_factor);
        FVector {
            x: fx as f64 / divisor,
            y: fy as f64 / divisor,
            z: fz as f64 / divisor,
        }
    } else {
        FVector {
            x: fx as f64,
            y: fy as f64,
            z: fz as f64,
        }
    }))
}

#[inline]
fn read_f64_vector(reader: &mut BitReader<'_>) -> Result<FVector> {
    let x = reader.read_f64()?;
    let y = reader.read_f64()?;
    let z = reader.read_f64()?;
    Ok(FVector { x, y, z })
}

/// Read a compressed short rotator (3 components, each optionally present).
///
/// ```text
/// For each of pitch, yaw, roll:
///   hasComponent : 1 bit
///   [if hasComponent]
///     value      : u16 (16 bits)
///     degrees = value * (360.0 / 65536.0)
/// ```
fn read_rotation_short(reader: &mut BitReader<'_>) -> Result<FRotator> {
    let pitch = read_compressed_short_component(reader)?;
    let yaw = read_compressed_short_component(reader)?;
    let roll = read_compressed_short_component(reader)?;
    Ok(FRotator { pitch, yaw, roll })
}

#[inline]
fn read_compressed_short_component(reader: &mut BitReader<'_>) -> Result<f32> {
    if reader.read_bit()? {
        let raw = reader.read_u16()?;
        Ok(f32::from(raw) * (360.0 / 65536.0))
    } else {
        Ok(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits_to_bytes(bits: &[bool]) -> Vec<u8> {
        let mut bytes = vec![0u8; bits.len().div_ceil(8)];
        for (i, &bit) in bits.iter().enumerate() {
            if bit {
                bytes[i >> 3] |= 1 << (i & 7);
            }
        }
        bytes
    }

    /// A clear leading bit yields the caller's default, never `None`. The two
    /// defaults differ (origin vs unit scale), which is the whole point.
    #[test]
    fn absent_vector_takes_the_callers_default() {
        let data = bits_to_bytes(&[false]);
        let mut reader = BitReader::with_bit_len(&data, 1);
        assert_eq!(
            read_optional_quantized_vector(&mut reader, SPAWN_SCALE_FACTOR, UNIT_SCALE).unwrap(),
            Some(UNIT_SCALE)
        );
        assert_eq!(reader.position(), 1, "exactly one bit is consumed");
    }

    /// The quantized path sign-extends each component and divides by the scale
    /// factor only when `extra_info` is non-zero.
    #[test]
    fn quantized_vector_sign_extends_and_scales() {
        // hasValue=1, isQuantized=1, info = 8 | (1 << 6) = 72 -> 8-bit
        // components with extra_info = 1, so each is divided by 10.
        let mut bits = vec![true, true];
        // SerializedInt(72, max=128): 7 value bits, LSB first.
        for i in 0..7 {
            bits.push((72u32 >> i) & 1 != 0);
        }
        for byte in [0xFFu8, 0x01, 0x80] {
            for i in 0..8 {
                bits.push((byte >> i) & 1 != 0);
            }
        }
        let data = bits_to_bytes(&bits);
        let mut reader = BitReader::with_bit_len(&data, bits.len() as u64);
        let v = read_optional_quantized_vector(&mut reader, SPAWN_SCALE_FACTOR, ORIGIN)
            .unwrap()
            .unwrap();
        assert_eq!(v.x, -0.1, "0xFF as i8 is -1");
        assert_eq!(v.y, 0.1);
        assert_eq!(v.z, -12.8, "0x80 as i8 is -128");
    }

    /// A missing rotator component reads as zero degrees and consumes one bit.
    #[test]
    fn rotation_short_skips_absent_components() {
        let mut bits = vec![true];
        for i in 0..16 {
            bits.push((16384u32 >> i) & 1 != 0);
        }
        bits.push(false);
        bits.push(false);
        let data = bits_to_bytes(&bits);
        let mut reader = BitReader::with_bit_len(&data, bits.len() as u64);
        let r = read_rotation_short(&mut reader).unwrap();
        assert_eq!(r.pitch, 90.0);
        assert_eq!(r.yaw, 0.0);
        assert_eq!(r.roll, 0.0);
        assert_eq!(reader.position(), 19);
    }
}
