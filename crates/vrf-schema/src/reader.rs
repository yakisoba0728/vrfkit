//! Wire-format readers for net field exports and export GUIDs.
//!
//! These functions consume bytes from a [`vrf_bitio::BitReader`] using the exact
//! layout that Unreal Engine's `ExportDataReader` writes:
//!
//! ## `ReadNetFieldExports` byte layout (byte-aligned, not bit-aligned)
//!
//! ```text
//! numLayoutCmdExports: IntPacked
//! for each export:
//!   pathNameIndex:  IntPacked
//!   isExported:    IntPacked (1 = new group, 0 = reference existing)
//!   if isExported:
//!     pathName:    FString (i32 length + UTF-8/UTF-16 bytes)
//!     numExports:  IntPacked (declared field-slot count)
//!   isFieldExported: u8 boolean (1 byte, not 1 bit -- this is FBinaryArchive)
//!   if isFieldExported:
//!     handle:             IntPacked
//!     compatibleChecksum: u32 (little-endian, 4 bytes)
//!     name:               FName (isHardcoded: u8 bool, then FString + i32 number)
//! ```
//!
//! ## `ReadExportGuids` byte layout
//!
//! ```text
//! numGuids: IntPacked
//! for each guid:
//!   payloadSize: i32 (little-endian)
//!   payload[payloadSize]:
//!     netGuid:     IntPacked
//!     exportFlags: u8 (if guid is default or isExportingNetGuidBunch)
//!     if HasPath:
//!       outerGuid:        (recursive InternalLoadObject)
//!       pathName:         FString
//!       if HasNetworkChecksum: u32
//! ```

use vrf_bitio::BitReader;

use crate::cache::NetGuidCache;
use crate::error::{Result, SchemaError};
use crate::export::{NetFieldExport, NetFieldExportGroup, render_fname};
use crate::guid::{ExportFlags, NetworkGuid};

/// Maximum string size allowed when reading path names (guard against corrupt
/// length prefixes allocating unbounded memory).
const MAX_FSTRING_BYTES: i64 = 1024 * 1024; // 1 MiB

/// Maximum recursion depth for nested NetGUID objects.
const MAX_NET_GUID_RECURSION: u32 = 16;

/// Maximum slots in a live export group. Checkpoint archives carry the same
/// `NumNetFieldExports` concept and are already bounded at 65,536; no corpus
/// group reaches that ceiling. Keeping both wire forms at the same limit stops
/// a five-byte IntPacked count from becoming an attacker-sized allocation.
const MAX_FIELDS_PER_GROUP: u32 = 65_536;

/// Read an FName from a **byte-aligned** archive.
///
/// FName on the wire (FBinaryArchive variant):
///   isHardcoded: u8 (1 byte boolean)
///   if hardcoded: nameIndex = IntPacked -> returned as decimal string
///   else: name = FString, number = i32 -> rendered by [`render_fname`]
///
/// The number is part of the name's identity, not decoration: see
/// [`render_fname`] for what dropping it cost.
fn read_fname(reader: &mut BitReader<'_>) -> Result<String> {
    let is_hardcoded = reader.read_u8()? != 0;
    if is_hardcoded {
        let name_index = reader.read_int_packed()?;
        Ok(name_index.to_string())
    } else {
        let name = reader.read_fstring(MAX_FSTRING_BYTES)?;
        let number = reader.read_i32()?;
        Ok(render_fname(name, number))
    }
}

/// Read all net-field export commands from the stream and populate the cache.
///
/// This is the primary schema-reception entry point. Each frame of the replay
/// may carry zero or more export commands that declare new groups or add fields
/// to existing ones. The cache accumulates state across frames.
///
/// Returns the number of layout-command exports processed.
pub fn read_net_field_exports(reader: &mut BitReader<'_>, cache: &mut NetGuidCache) -> Result<u32> {
    let num_exports = reader.read_int_packed()?;

    for _ in 0..num_exports {
        let path_name_index = reader.read_int_packed()?;
        let is_exported = reader.read_int_packed()? == 1;

        if is_exported {
            // New or re-exported group: read path + capacity, register it.
            let path_name = reader.read_fstring(MAX_FSTRING_BYTES)?;
            let num_fields = reader.read_int_packed()?;

            if num_fields > MAX_FIELDS_PER_GROUP {
                return Err(SchemaError::FieldCountOverflow {
                    count: num_fields,
                    max: MAX_FIELDS_PER_GROUP,
                });
            }

            let group = NetFieldExportGroup::try_new(path_name, path_name_index, num_fields)?;
            cache.add_export_group(group)?;
        } else {
            // Reference to an existing group by index -- it must already be known.
            if cache.get_group_by_index(path_name_index).is_none() {
                return Err(SchemaError::UnknownPathIndex {
                    index: path_name_index,
                });
            }
        }

        // Read the optional field export that follows.
        let is_field_exported = reader.read_u8()? != 0;
        if !is_field_exported {
            continue;
        }

        let handle = reader.read_int_packed()?;
        let compatible_checksum = reader.read_u32()?;
        let name = read_fname(reader)?;

        let field = NetFieldExport {
            handle,
            compatible_checksum,
            name,
        };

        // The C# code silently drops fields whose handle exceeds the group length.
        cache.set_field_on_group(path_name_index, field);
    }

    Ok(num_exports)
}

/// Read exported NetGUID payloads from the stream and populate the cache.
///
/// Each payload maps a `NetworkGuid` to an object path (and optionally an outer
/// GUID forming the containment hierarchy). The payloads are length-prefixed so
/// they can be individually validated for complete consumption.
///
/// Returns the number of GUID payloads processed.
pub fn read_export_guids(reader: &mut BitReader<'_>, cache: &mut NetGuidCache) -> Result<u32> {
    let num_guids = reader.read_int_packed()?;

    for _ in 0..num_guids {
        let size = reader.read_i32()?;
        if size < 0 {
            return Err(SchemaError::NegativePayloadSize { size });
        }
        let byte_count = size as u64;

        // Carve out a sub-reader for exactly `size` bytes so we can verify
        // complete consumption (matching the C# EnsureFullyConsumed check).
        let mut payload = reader.sub_reader(byte_count * 8)?;

        internal_load_object(&mut payload, cache, true, 0)?;

        if payload.bits_remaining() >= 8 {
            return Err(SchemaError::TrailingPayloadData {
                remaining: (payload.bits_remaining() / 8) as usize,
            });
        }
    }

    Ok(num_guids)
}

/// Recursively read a NetGUID object reference and register it in the cache.
///
/// This mirrors `NetGuidObjectReader.InternalLoadObject`. Each object may
/// reference an outer (containing) object via another nested NetGUID.
fn internal_load_object(
    reader: &mut BitReader<'_>,
    cache: &mut NetGuidCache,
    is_exporting: bool,
    depth: u32,
) -> Result<NetworkGuid> {
    if depth >= MAX_NET_GUID_RECURSION {
        return Err(SchemaError::RecursionLimitExceeded {
            limit: MAX_NET_GUID_RECURSION,
        });
    }

    let net_guid = NetworkGuid(reader.read_int_packed()?);
    if !net_guid.is_valid() {
        return Ok(net_guid);
    }

    // Export flags are present when this is either the default object or we are
    // inside an export-GUID bunch.
    let flags = if net_guid.is_default() || is_exporting {
        ExportFlags(reader.read_u8()?)
    } else {
        ExportFlags::NONE
    };

    if !flags.contains(ExportFlags::HAS_PATH) {
        return Ok(net_guid);
    }

    // Outer object (recursive).
    let outer_guid = internal_load_object(reader, cache, is_exporting, depth + 1)?;

    // Path name.
    let path_name = reader.read_fstring(MAX_FSTRING_BYTES)?;

    // Optional network checksum (discarded).
    if flags.contains(ExportFlags::HAS_NETWORK_CHECKSUM) {
        let _checksum = reader.read_u32()?;
    }

    let outer = if outer_guid.is_valid() {
        Some(outer_guid)
    } else {
        None
    };
    cache.set_net_guid_path(net_guid.0, path_name, outer);

    Ok(net_guid)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Test helpers mirroring the C# test's byte-building functions ----------

    /// Encode a u32 as Unreal's IntPacked format (7 payload bits per byte,
    /// continuation in low bit).
    fn encode_int_packed(mut value: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        loop {
            let mut next_byte = ((value & 0x7F) << 1) as u8;
            value >>= 7;
            if value != 0 {
                next_byte |= 1; // continuation flag
            }
            bytes.push(next_byte);
            if value == 0 {
                break;
            }
        }
        bytes
    }

    /// Encode an FString: i32 length (including null) + UTF-8 bytes + null.
    fn encode_fstring(s: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        let len = (s.len() + 1) as i32; // +1 for null terminator
        bytes.extend_from_slice(&len.to_le_bytes());
        bytes.extend_from_slice(s.as_bytes());
        bytes.push(0);
        bytes
    }

    /// Encode an FName (non-hardcoded): isHardcoded=0, FString, number=0.
    fn encode_fname(s: &str) -> Vec<u8> {
        encode_fname_numbered(s, 0)
    }

    /// Encode an FName with an explicit instance number.
    fn encode_fname_numbered(s: &str, number: i32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(0); // isHardcoded = false
        bytes.extend(encode_fstring(s));
        bytes.extend_from_slice(&number.to_le_bytes());
        bytes
    }

    /// Encode a u32 as 4 little-endian bytes.
    fn encode_u32(v: u32) -> Vec<u8> {
        v.to_le_bytes().to_vec()
    }

    /// Encode an i32 as 4 little-endian bytes.
    fn encode_i32(v: i32) -> Vec<u8> {
        v.to_le_bytes().to_vec()
    }

    /// Build a full net-field export command for a new group with an optional field.
    fn build_new_group(
        path_name_index: u32,
        path: &str,
        num_fields: u32,
        field: Option<(u32, &str)>,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(encode_int_packed(path_name_index));
        bytes.extend(encode_int_packed(1)); // isExported = true
        bytes.extend(encode_fstring(path));
        bytes.extend(encode_int_packed(num_fields));
        if let Some((handle, name)) = field {
            bytes.push(1); // isFieldExported = true
            bytes.extend(encode_int_packed(handle));
            bytes.extend(encode_u32(0xAABBCCDD));
            bytes.extend(encode_fname(name));
        } else {
            bytes.push(0); // isFieldExported = false
        }
        bytes
    }

    /// Build a reference to an existing group + add a field.
    fn build_existing_group_field(path_name_index: u32, handle: u32, name: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(encode_int_packed(path_name_index));
        bytes.extend(encode_int_packed(0)); // isExported = false (reference)
        bytes.push(1); // isFieldExported = true
        bytes.extend(encode_int_packed(handle));
        bytes.extend(encode_u32(0xAABBCCDD));
        bytes.extend(encode_fname(name));
        bytes
    }

    /// Build an export-GUID payload (single object, no outer, with HasPath).
    fn build_guid_payload(net_guid: u32, path: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(encode_int_packed(net_guid));
        bytes.push(ExportFlags::HAS_PATH.0); // flags
        bytes.extend(encode_int_packed(0)); // outer guid = 0 (invalid, terminates recursion)
        bytes.extend(encode_fstring(path));
        bytes
    }

    // -- ReadNetFieldExports tests (ported from ExportDataReaderTests.cs) -----

    #[test]
    fn registers_exported_group() {
        let mut data = Vec::new();
        data.extend(encode_int_packed(1)); // 1 export
        data.extend(build_new_group(11, "/Game/Test.Test_C", 3, None));

        let mut reader = BitReader::new(&data);
        let mut cache = NetGuidCache::new();
        read_net_field_exports(&mut reader, &mut cache).unwrap();

        let group = cache.get_group_by_index(11).unwrap();
        assert_eq!(group.path, "/Game/Test.Test_C");
        assert_eq!(group.path_name_index, 11);
        assert_eq!(group.len(), 3);
        assert!(cache.get_group_by_path("/Game/Test.Test_C").is_some());
    }

    #[test]
    fn live_group_rejects_field_count_above_checkpoint_protocol_limit() {
        let mut data = Vec::new();
        data.extend(encode_int_packed(1));
        data.extend(build_new_group(11, "/Game/Test.Test_C", 65_537, None));

        let mut reader = BitReader::new(&data);
        let mut cache = NetGuidCache::new();
        assert!(matches!(
            read_net_field_exports(&mut reader, &mut cache).unwrap_err(),
            SchemaError::FieldCountOverflow {
                count: 65_537,
                max: 65_536
            }
        ));
        assert_eq!(cache.group_count(), 0);
    }

    #[test]
    fn stores_export_by_handle() {
        let mut data = Vec::new();
        data.extend(encode_int_packed(1));
        data.extend(build_new_group(
            11,
            "/Game/Test.Test_C",
            3,
            Some((2, "FieldName")),
        ));

        let mut reader = BitReader::new(&data);
        let mut cache = NetGuidCache::new();
        read_net_field_exports(&mut reader, &mut cache).unwrap();

        let group = cache.get_group_by_index(11).unwrap();
        let field = group.get_field(2).unwrap();
        assert_eq!(field.handle, 2);
        assert_eq!(field.compatible_checksum, 0xAABBCCDD);
        assert_eq!(field.name, "FieldName");
    }

    #[test]
    fn existing_path_index_updates_group() {
        let mut data = Vec::new();
        data.extend(encode_int_packed(2)); // 2 exports
        data.extend(build_new_group(11, "/Game/Test.Test_C", 3, None));
        data.extend(build_existing_group_field(11, 1, "LaterField"));

        let mut reader = BitReader::new(&data);
        let mut cache = NetGuidCache::new();
        read_net_field_exports(&mut reader, &mut cache).unwrap();

        let group = cache.get_group_by_index(11).unwrap();
        assert_eq!(group.get_field(1).unwrap().name, "LaterField");
    }

    #[test]
    fn re_exported_group_expands_without_losing_existing_fields() {
        // First export: group with capacity 2, field at handle 1.
        // Second export: same path re-exported with capacity 4, field at handle 3.
        let mut data = Vec::new();
        data.extend(encode_int_packed(2)); // 2 exports

        // First: group(capacity=2) + field at handle 1
        data.extend(encode_int_packed(11)); // pathNameIndex
        data.extend(encode_int_packed(1)); // isExported
        data.extend(encode_fstring("/Game/Test.Test_C"));
        data.extend(encode_int_packed(2)); // numExports
        data.push(1); // isFieldExported
        data.extend(encode_int_packed(1)); // handle
        data.extend(encode_u32(0xAABBCCDD));
        data.extend(encode_fname("ExistingField"));

        // Second: same path re-exported with larger capacity + field at handle 3
        data.extend(encode_int_packed(11));
        data.extend(encode_int_packed(1)); // isExported again
        data.extend(encode_fstring("/Game/Test.Test_C"));
        data.extend(encode_int_packed(4)); // numExports (larger)
        data.push(1); // isFieldExported
        data.extend(encode_int_packed(3)); // handle
        data.extend(encode_u32(0xAABBCCDD));
        data.extend(encode_fname("ExpandedField"));

        let mut reader = BitReader::new(&data);
        let mut cache = NetGuidCache::new();
        read_net_field_exports(&mut reader, &mut cache).unwrap();

        let group = cache.get_group_by_index(11).unwrap();
        assert_eq!(group.len(), 4);
        assert_eq!(group.get_field(1).unwrap().name, "ExistingField");
        assert_eq!(group.get_field(3).unwrap().name, "ExpandedField");
    }

    #[test]
    fn unknown_path_index_returns_error() {
        // Reference a path_name_index (42) that was never registered.
        let mut data = Vec::new();
        data.extend(encode_int_packed(1)); // 1 export
        data.extend(encode_int_packed(42)); // pathNameIndex
        data.extend(encode_int_packed(0)); // isExported = false (reference)
        // Still need the field-exported flag for the iteration to be well-formed,
        // but the error should fire before reading it. However looking at the C#
        // code, it throws immediately. Let's just provide the minimal bytes.
        // Actually the error is thrown before reading isFieldExported.

        let mut reader = BitReader::new(&data);
        let mut cache = NetGuidCache::new();
        let err = read_net_field_exports(&mut reader, &mut cache).unwrap_err();
        assert!(matches!(err, SchemaError::UnknownPathIndex { index: 42 }));
    }

    #[test]
    fn invalid_handle_is_silently_dropped() {
        // Group has capacity 1, field handle is 2 (out of range).
        let mut data = Vec::new();
        data.extend(encode_int_packed(1));
        data.extend(build_new_group(
            11,
            "/Game/Test.Test_C",
            1,
            Some((2, "OutOfRange")),
        ));

        let mut reader = BitReader::new(&data);
        let mut cache = NetGuidCache::new();
        // Should not error.
        read_net_field_exports(&mut reader, &mut cache).unwrap();
        let group = cache.get_group_by_index(11).unwrap();
        assert!(group.get_field(0).is_none());
    }

    /// An FName is a string AND a number, and the number used to be read and
    /// thrown away. Two handles whose base string is the same then arrived with
    /// the same name, so the manifest's handle-to-name mapping could not tell
    /// them apart -- and nothing said so.
    ///
    /// Unreal stores the number as the displayed suffix plus one, so 0 renders
    /// bare and 1 renders `_0`.
    #[test]
    fn fname_numbers_disambiguate_two_fields_with_one_base_name() {
        let mut data = Vec::new();
        data.extend(encode_int_packed(3)); // 3 exports

        // Group with three slots; handle 0 has number 0, handle 1 number 1,
        // handle 2 number 2. All three are the base string "Value".
        data.extend(encode_int_packed(11));
        data.extend(encode_int_packed(1)); // isExported
        data.extend(encode_fstring("/Game/Test.Test_C"));
        data.extend(encode_int_packed(3)); // numExports
        data.push(1); // isFieldExported
        data.extend(encode_int_packed(0));
        data.extend(encode_u32(0));
        data.extend(encode_fname_numbered("Value", 0));

        for (handle, number) in [(1u32, 1i32), (2, 2)] {
            data.extend(encode_int_packed(11));
            data.extend(encode_int_packed(0)); // reference the existing group
            data.push(1); // isFieldExported
            data.extend(encode_int_packed(handle));
            data.extend(encode_u32(0));
            data.extend(encode_fname_numbered("Value", number));
        }

        let mut reader = BitReader::new(&data);
        let mut cache = NetGuidCache::new();
        read_net_field_exports(&mut reader, &mut cache).unwrap();

        let group = cache.get_group_by_index(11).unwrap();
        assert_eq!(group.get_field(0).unwrap().name, "Value");
        assert_eq!(group.get_field(1).unwrap().name, "Value_0");
        assert_eq!(group.get_field(2).unwrap().name, "Value_1");
    }

    // -- ReadExportGuids tests (ported from ExportDataReaderTests.cs) ---------

    #[test]
    fn export_guid_registers_path() {
        let payload = build_guid_payload(17, "/Game/Test.Test_C");
        let mut data = Vec::new();
        data.extend(encode_int_packed(1)); // 1 guid
        data.extend(encode_i32(payload.len() as i32));
        data.extend(&payload);

        let mut reader = BitReader::new(&data);
        let mut cache = NetGuidCache::new();
        read_export_guids(&mut reader, &mut cache).unwrap();

        assert_eq!(cache.get_path_by_guid(17).unwrap(), "/Game/Test.Test_C");
    }

    #[test]
    fn export_guid_negative_size_returns_error() {
        let mut data = Vec::new();
        data.extend(encode_int_packed(1));
        data.extend(encode_i32(-1));

        let mut reader = BitReader::new(&data);
        let mut cache = NetGuidCache::new();
        let err = read_export_guids(&mut reader, &mut cache).unwrap_err();
        assert!(matches!(err, SchemaError::NegativePayloadSize { size: -1 }));
    }

    #[test]
    fn export_guid_trailing_data_returns_error() {
        let mut payload = build_guid_payload(17, "/Game/Test.Test_C");
        payload.push(0xFF); // trailing byte
        let mut data = Vec::new();
        data.extend(encode_int_packed(1));
        data.extend(encode_i32(payload.len() as i32));
        data.extend(&payload);

        let mut reader = BitReader::new(&data);
        let mut cache = NetGuidCache::new();
        let err = read_export_guids(&mut reader, &mut cache).unwrap_err();
        assert!(matches!(err, SchemaError::TrailingPayloadData { .. }));
    }
    // -- Truncated input rejection --------------------------------------------

    #[test]
    fn truncated_net_field_exports_returns_bitio_error() {
        // Just the count, then nothing.
        let data = encode_int_packed(5); // says 5 exports, but no data follows.
        let mut reader = BitReader::new(&data);
        let mut cache = NetGuidCache::new();
        let err = read_net_field_exports(&mut reader, &mut cache).unwrap_err();
        assert!(matches!(err, SchemaError::Bitio(_)));
    }

    #[test]
    fn truncated_export_guids_returns_bitio_error() {
        // Count says 1 but no payload size follows.
        let data = encode_int_packed(1);
        let mut reader = BitReader::new(&data);
        let mut cache = NetGuidCache::new();
        let err = read_export_guids(&mut reader, &mut cache).unwrap_err();
        assert!(matches!(err, SchemaError::Bitio(_)));
    }

    #[test]
    fn zero_exports_is_valid() {
        let data = encode_int_packed(0);
        let mut reader = BitReader::new(&data);
        let mut cache = NetGuidCache::new();
        let n = read_net_field_exports(&mut reader, &mut cache).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn zero_guids_is_valid() {
        let data = encode_int_packed(0);
        let mut reader = BitReader::new(&data);
        let mut cache = NetGuidCache::new();
        let n = read_export_guids(&mut reader, &mut cache).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn roundtrip_multiple_groups_multiple_fields() {
        // Register 2 groups each with 2 fields, then verify all survive.
        let mut data = Vec::new();
        data.extend(encode_int_packed(4)); // 4 export commands total

        // Group A: path_name_index=1, capacity=3, field at handle 0
        data.extend(build_new_group(1, "/Game/A.A_C", 3, Some((0, "Alpha"))));
        // Group A: add field at handle 2
        data.extend(build_existing_group_field(1, 2, "Gamma"));
        // Group B: path_name_index=2, capacity=2, field at handle 1
        data.extend(build_new_group(2, "/Game/B.B_C", 2, Some((1, "Beta"))));
        // Group B: add field at handle 0
        data.extend(build_existing_group_field(2, 0, "Delta"));

        let mut reader = BitReader::new(&data);
        let mut cache = NetGuidCache::new();
        let n = read_net_field_exports(&mut reader, &mut cache).unwrap();
        assert_eq!(n, 4);

        let a = cache.get_group_by_index(1).unwrap();
        assert_eq!(a.get_field(0).unwrap().name, "Alpha");
        assert_eq!(a.get_field(2).unwrap().name, "Gamma");
        assert!(a.get_field(1).is_none());

        let b = cache.get_group_by_index(2).unwrap();
        assert_eq!(b.get_field(1).unwrap().name, "Beta");
        assert_eq!(b.get_field(0).unwrap().name, "Delta");
    }
}
