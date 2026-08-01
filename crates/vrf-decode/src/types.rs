//! Model types for decoded Unreal Engine replay primitives.
//!
//! These mirror the C# `Replay.Models.Unreal` types but are pure data structs
//! with no allocations. Display impls produce the compact string form written
//! to `value_str`.

use core::fmt;

/// Rotation quantization modes for [`FRepMovement`].
///
/// Determines how the rotation is serialized in `ReplicatedMovement`:
/// - `ByteComponents`: 1 flag bit + 8 data bits per axis (compact, ±1.4° precision)
/// - `ShortComponents`: 1 flag bit + 16 data bits per axis (precise, ±0.005°)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RotatorQuantization {
    ByteComponents,
    ShortComponents,
}

/// A 3D vector. Components are always `f64` regardless of wire format (float
/// values are widened on decode to avoid losing precision when mixing formats).
///
/// # Wire layouts
///
/// | Variant | Bits |
/// |---------|------|
/// | Float (3×f32) | 96 |
/// | Double (3×f64) | 192 |
/// | NetQuantize (packed header + N-bit signed × 3) | variable |
/// | NetQuantizeNormal (3 × SerializedInt(65536)) | ~48 |
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FVector {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl fmt::Display for FVector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({},{},{})", self.x, self.y, self.z)
    }
}

/// Euler rotation (degrees).
///
/// # Wire layouts
///
/// | Variant | Bits per axis |
/// |---------|---------------|
/// | Short | 1 flag + 16 | → `value * 360/65536` |
/// | Byte | 1 flag + 8 | → `value * 360/256` |
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FRotator {
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
}

impl fmt::Display for FRotator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rot({},{},{})", self.pitch, self.yaw, self.roll)
    }
}

/// Quaternion rotation (4 × f32).
///
/// # Wire layout
///
/// 128 bits = 4 × IEEE-754 single.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FQuat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl fmt::Display for FQuat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "quat({},{},{},{})", self.x, self.y, self.z, self.w)
    }
}

/// Transform = rotation (quat) + translation (vec3) + scale (vec3).
///
/// # Wire layout
///
/// 320 bits = FQuat(128) + FVector_float(96) + FVector_float(96).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FTransform {
    pub rotation: FQuat,
    pub translation: FVector,
    pub scale: FVector,
}

impl fmt::Display for FTransform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "xform({};{};{})",
            self.rotation, self.translation, self.scale
        )
    }
}

/// Replicated movement state.
///
/// # Wire layout
///
/// ```text
/// Bit 0: bSimulatedPhysicsSleep
/// Bit 1: bRepPhysics
/// Bit 2: bRepServerFrame
/// Bit 3: bRepServerHandle
/// [VectorNetQuantize100]: location
/// [RotationShort or RotationByte]: rotation
/// [VectorNetQuantize(1)]: linear velocity
/// if bRepPhysics: [VectorNetQuantize(1)]: angular velocity
/// if bRepServerFrame: IntPacked server frame
/// if bRepServerHandle: IntPacked server physics handle
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct FRepMovement {
    pub location: FVector,
    pub rotation: FRotator,
    pub linear_velocity: FVector,
    pub angular_velocity: Option<FVector>,
    pub simulated_physics_sleep: bool,
    pub rep_physics: bool,
    pub server_frame: u32,
    pub server_physics_handle: u32,
}

/// Writes an [`FVector`] as `{"x":..,"y":..,"z":..}`.
///
/// [`FVector`]'s own `Display` is the compact `(x,y,z)` form; a vector nested
/// inside a `ReplicatedMovement` object needs the named-member shape instead,
/// so this cannot reuse it.
fn write_vector_json(f: &mut fmt::Formatter<'_>, v: &FVector) -> fmt::Result {
    write!(f, "{{\"x\":{},\"y\":{},\"z\":{}}}", v.x, v.y, v.z)
}

/// `ReplicatedMovement` serializes as a JSON object, not the compact form the
/// other types use.
///
/// The compact form cannot carry the whole struct. `simulated_physics_sleep`
/// and `server_physics_handle` have nowhere to go in a `loc/rot/vel` triple,
/// and `value_str` is a single string column -- there is no struct column to
/// put them in. So they were dropped: 14,377 rows on 02d4d478 shipped a
/// human-readable string where the reference (ReplayJsonNormalizer.cs:255)
/// emits an eight-member object, and two of those members were simply gone.
///
/// Member names and order follow the reference exactly. Every component is
/// finite by construction -- vectors are an integer quotient of an integer
/// scale factor, rotator axes an integer multiple of 360/65536 or 360/256 --
/// so no component can render as `NaN` or `inf` and break the JSON.
impl fmt::Display for FRepMovement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("{\"linear_velocity\":")?;
        write_vector_json(f, &self.linear_velocity)?;
        f.write_str(",\"angular_velocity\":")?;
        match self.angular_velocity {
            Some(ref av) => write_vector_json(f, av)?,
            None => f.write_str("null")?,
        }
        f.write_str(",\"location\":")?;
        write_vector_json(f, &self.location)?;
        write!(
            f,
            ",\"rotation\":{{\"pitch\":{},\"yaw\":{},\"roll\":{}}}",
            self.rotation.pitch, self.rotation.yaw, self.rotation.roll
        )?;
        write!(
            f,
            ",\"simulated_physics_sleep\":{},\"rep_physics\":{}",
            self.simulated_physics_sleep, self.rep_physics
        )?;
        write!(
            f,
            ",\"server_frame\":{},\"server_physics_handle\":{}}}",
            self.server_frame, self.server_physics_handle
        )
    }
}
