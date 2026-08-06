//! Export wiring: one effect blob in, one JSON array out.

use super::elements::{
    decode_effect_floats_at, decode_effect_objects_at, decode_effect_vectors_at,
};
use super::framing::{new_blob_reader, scan_element_handles};
use super::{EffectArrayKind, EffectBlobError, EffectHandles, Result};

/// Decode one effect-array blob and render it as a JSON array.
///
/// Each element becomes `{"tag":<u32|null>,"value":<value|null>}`, where the
/// value is a number for [`EffectArrayKind::Float`] and
/// [`EffectArrayKind::Object`] and `{"x":..,"y":..,"z":..}` for
/// [`EffectArrayKind::Vector`]. A sparse array's unpopulated slots keep their
/// position and render both members as `null`, so an element's index in the
/// JSON is its index on the wire.
///
/// # Arguments
/// - `raw`: the parameter payload, as `fields.parquet` stores it.
/// - `bit_count`: the payload's exact bit length. This is the RPC parameter's
///   declared `payload_bits`, **not** `raw.len() * 8` -- the last byte is
///   padded, and feeding the padding in as data is a latent bug the audit in
///   `docs/archive/PROJECT_STATUS.md` 12-D calls out on the Python side.
///
/// # Errors
/// Returns [`EffectBlobError`] if the payload is not a well-formed array of
/// this kind, does not consume its window, or contains a float that JSON
/// cannot represent. The caller keeps the raw bits and counts the failure; it
/// must not substitute a partial or plausible-looking structure.
pub fn decode_effect_blob_json(
    kind: EffectArrayKind,
    raw: &[u8],
    bit_count: u32,
) -> Result<String> {
    // First pass: which handles does *this* function put the element's members
    // under. `None` means no element carries a field, so the pair is both
    // underivable and unused -- any pair decodes such a blob identically.
    let handles = scan_element_handles(raw, bit_count)?.unwrap_or(EffectHandles::from_base(0));
    let mut reader = new_blob_reader(raw, bit_count)?;

    // Grown on demand rather than pre-reserved: a reservation sized from the
    // element count was tried and measured no faster end to end, while
    // costing resident memory -- these strings sit in the export's row
    // buffer until the Parquet write.
    let mut out = String::new();
    match kind {
        EffectArrayKind::Float => {
            let elements = decode_effect_floats_at(&mut reader, handles)?;
            out.push('[');
            for (index, elem) in elements.iter().enumerate() {
                push_separator(&mut out, index);
                push_tag(&mut out, elem.tag_index);
                match elem.value {
                    Some(v) if v.is_finite() => push_json_f64(&mut out, f64::from(v)),
                    Some(_) => return Err(EffectBlobError::NonFiniteFloat { index }),
                    None => out.push_str("null"),
                }
                out.push('}');
            }
        }
        EffectArrayKind::Object => {
            let elements = decode_effect_objects_at(&mut reader, handles)?;
            out.push('[');
            for (index, elem) in elements.iter().enumerate() {
                push_separator(&mut out, index);
                push_tag(&mut out, elem.tag_index);
                match elem.value {
                    Some(v) => push_u32(&mut out, v),
                    None => out.push_str("null"),
                }
                out.push('}');
            }
        }
        EffectArrayKind::Vector => {
            let elements = decode_effect_vectors_at(&mut reader, handles)?;
            out.push('[');
            for (index, elem) in elements.iter().enumerate() {
                push_separator(&mut out, index);
                push_tag(&mut out, elem.tag_index);
                match elem.value {
                    Some(v) => {
                        if !(v.x.is_finite() && v.y.is_finite() && v.z.is_finite()) {
                            return Err(EffectBlobError::NonFiniteFloat { index });
                        }
                        out.push_str("{\"x\":");
                        push_json_f64(&mut out, v.x);
                        out.push_str(",\"y\":");
                        push_json_f64(&mut out, v.y);
                        out.push_str(",\"z\":");
                        push_json_f64(&mut out, v.z);
                        out.push('}');
                    }
                    None => out.push_str("null"),
                }
                out.push('}');
            }
        }
    }
    out.push(']');

    // Checked after the decode, not during: the decoders stop at the array
    // terminator by design, so "did it consume the window" is only answerable
    // once they have returned.
    //
    // Every remaining bit counts, including a sub-byte tail. This used to
    // tolerate 1-7 bits on the grounds that byte padding cannot carry an
    // element -- but `bit_count` here is the RPC parameter's exact declared
    // payload length, not `raw.len() * 8`, so the storage padding was already
    // excluded before this function saw the blob (see this function's own
    // `bit_count` argument note). Anything left inside the window is declared
    // payload that nothing accounted for, which is the same evidence of a wrong
    // read at four bits as at forty.
    let remaining = reader.bits_remaining();
    if remaining > 0 {
        return Err(EffectBlobError::ResidualBits { remaining });
    }

    Ok(out)
}

/// Elements are comma-separated; the first one is not preceded by anything.
fn push_separator(out: &mut String, index: usize) {
    if index > 0 {
        out.push(',');
    }
}

/// Open one element object and write its `tag` member.
///
/// A hand-written `u32`-to-decimal writer was tried here and for the object
/// value below, on the reasoning that `write!` builds a `format_args` and
/// dispatches through `Display` for each of a replay's ~128,000 tags. It
/// measured neutral in an interleaved A/B of the whole export -- the effect
/// path's time is in `push_json_f64`, not here -- so the twenty lines went
/// away again and `write!` stayed.
fn push_tag(out: &mut String, tag_index: Option<u32>) {
    use core::fmt::Write as _;
    match tag_index {
        Some(t) => {
            let _ = write!(out, "{{\"tag\":{t},\"value\":");
        }
        None => out.push_str("{\"tag\":null,\"value\":"),
    }
}

/// Append a `u32` in decimal.
fn push_u32(out: &mut String, value: u32) {
    use core::fmt::Write as _;
    let _ = write!(out, "{value}");
}

/// Append a JSON number for a finite `f64`.
///
/// Rust's `Display` for floats is the shortest representation that round-trips,
/// which is always a valid JSON number for a finite value. `1.0` renders as
/// `1`; that is a JSON number too, so no consumer sees a type it cannot read.
///
/// Deliberately still `write!`: a dedicated shortest-float printer would be
/// faster, but the ones available render large magnitudes in exponent form
/// (`1E20` where Rust writes `100000000000000000000`), and this output is
/// pinned byte-for-byte by the export oracle.
fn push_json_f64(out: &mut String, v: f64) {
    use core::fmt::Write as _;
    // Writing into a String is infallible; the Result exists only to satisfy
    // the `fmt::Write` signature.
    let _ = write!(out, "{v}");
}
