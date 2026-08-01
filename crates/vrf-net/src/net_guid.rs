//! Net GUID loading -- `InternalLoadObject` recursive reader.
//!
//! This is the Unreal mechanism for serializing object references on the wire.
//! A GUID is read, and if it carries export flags with a path, the path (and
//! possibly an outer GUID, recursively) is also consumed from the stream.
//!
//! # Callback
//!
//! Path registration is *not* handled here. Instead the caller provides a
//! [`GuidPathSink`] that receives `(guid, path, outer_guid)` tuples.
//! This keeps the NetGuidCache (a large HashMap) in the caller's domain.

use vrf_bitio::BitReader;

use crate::error::{NetError, Result};
use crate::types::{ExportFlags, MAX_NET_GUID_RECURSION, NetworkGuid};

/// Callback invoked when a net GUID's path is decoded from the stream.
///
/// Implementors should store `(guid, path, outer_guid)` in their cache.
pub trait GuidPathSink {
    /// A GUID path was read from the wire.
    fn register_path(&mut self, guid: u32, path: &str, outer_guid: NetworkGuid);
}

/// Read a net GUID reference (and any associated export data) from the stream.
///
/// ```text
/// Wire layout:
///   net_guid           : IntPacked (u32)
///   if guid == default || is_exporting:
///     export_flags     : u8
///   if HasPath in export_flags:
///     outer_guid       : InternalLoadObject (recursive)
///     path_name        : FString
///     if HasNetworkChecksum:
///       checksum       : u32
/// ```
///
/// `is_exporting` is true when called from a package-map export bunch (the
/// entire bunch is declaring paths). In normal content-block headers it is
/// false, so only "default" GUIDs (value == 1) carry inline path data.
pub fn internal_load_object(
    reader: &mut BitReader<'_>,
    is_exporting: bool,
    depth: u32,
    sink: &mut dyn GuidPathSink,
) -> Result<NetworkGuid> {
    if depth >= MAX_NET_GUID_RECURSION {
        return Err(NetError::GuidRecursionLimit { depth });
    }

    let guid = NetworkGuid(reader.read_int_packed()?);
    if !guid.is_valid() {
        return Ok(guid);
    }

    // Export flags are present when:
    // 1. The GUID is the "default" object (value == 1), OR
    // 2. We are inside a package-map export bunch.
    let flags = if guid.is_default() || is_exporting {
        ExportFlags(reader.read_u8()?)
    } else {
        ExportFlags(0)
    };

    if !flags.has_path() {
        return Ok(guid);
    }

    // Recursive: the path's outer object is itself a net GUID reference.
    let outer_guid = internal_load_object(reader, is_exporting, depth + 1, sink)?;

    // FString path. Cap at 4096 bytes to reject corrupt lengths early.
    let path = reader.read_fstring(4096)?;

    if flags.has_network_checksum() {
        let _checksum = reader.read_u32()?;
    }

    sink.register_path(guid.0, &path, outer_guid);
    Ok(guid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct VecSink(Vec<(u32, String, NetworkGuid)>);

    impl GuidPathSink for VecSink {
        fn register_path(&mut self, guid: u32, path: &str, outer: NetworkGuid) {
            self.0.push((guid, path.to_owned(), outer));
        }
    }

    /// Build a minimal InternalLoadObject payload for a non-exporting read.
    fn build_simple_guid(guid: u32) -> Vec<u8> {
        let mut bits: Vec<bool> = Vec::new();
        write_int_packed_bits(&mut bits, guid);
        bits_to_bytes(&bits)
    }

    /// Build an InternalLoadObject with path export.
    fn build_export_guid(guid: u32, path: &str, outer_guid: u32) -> Vec<u8> {
        let mut bits: Vec<bool> = Vec::new();
        write_int_packed_bits(&mut bits, guid);
        // export flags = HasPath (0x01)
        write_byte_bits(&mut bits, 0x01);
        // outer guid (simple, no path)
        write_int_packed_bits(&mut bits, outer_guid);
        // FString: length (i32) + bytes + null
        let path_bytes = format!("{}\0", path);
        let len = path_bytes.len() as i32;
        for b in len.to_le_bytes() {
            write_byte_bits(&mut bits, b);
        }
        for b in path_bytes.bytes() {
            write_byte_bits(&mut bits, b);
        }
        bits_to_bytes(&bits)
    }

    #[test]
    fn zero_guid_is_invalid_and_consumed() {
        let data = build_simple_guid(0);
        let mut reader = BitReader::new(&data);
        let mut sink = VecSink::default();
        let guid = internal_load_object(&mut reader, false, 0, &mut sink).unwrap();
        assert!(!guid.is_valid());
        assert!(sink.0.is_empty());
    }

    #[test]
    fn simple_guid_no_path() {
        let data = build_simple_guid(42);
        let mut reader = BitReader::new(&data);
        let mut sink = VecSink::default();
        let guid = internal_load_object(&mut reader, false, 0, &mut sink).unwrap();
        assert_eq!(guid.0, 42);
        assert!(sink.0.is_empty()); // No path since not default and not exporting
    }

    #[test]
    fn exporting_guid_with_path() {
        let data = build_export_guid(18, "/Game/Test.Test_C", 0);
        let mut reader = BitReader::new(&data);
        let mut sink = VecSink::default();
        let guid = internal_load_object(&mut reader, true, 0, &mut sink).unwrap();
        assert_eq!(guid.0, 18);
        assert_eq!(sink.0.len(), 1);
        assert_eq!(sink.0[0].0, 18);
        assert_eq!(sink.0[0].1, "/Game/Test.Test_C");
        assert_eq!(sink.0[0].2, NetworkGuid(0));
    }

    // --- helpers ---

    fn write_int_packed_bits(bits: &mut Vec<bool>, mut value: u32) {
        loop {
            let mut next_byte = ((value & 0x7F) << 1) as u8;
            value >>= 7;
            if value != 0 {
                next_byte |= 1;
            }
            for i in 0..8 {
                bits.push((next_byte & (1 << i)) != 0);
            }
            if value == 0 {
                break;
            }
        }
    }

    fn write_byte_bits(bits: &mut Vec<bool>, byte: u8) {
        for i in 0..8 {
            bits.push((byte & (1 << i)) != 0);
        }
    }

    fn bits_to_bytes(bits: &[bool]) -> Vec<u8> {
        let byte_count = bits.len().div_ceil(8);
        let mut bytes = vec![0u8; byte_count];
        for (i, &bit) in bits.iter().enumerate() {
            if bit {
                bytes[i >> 3] |= 1 << (i & 7);
            }
        }
        bytes
    }
}
