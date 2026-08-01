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
//!   isFieldExported: u8 boolean (1 byte, not 1 bit — this is FBinaryArchive)
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

use crate::cache::{ExportFlags, NetGuidCache, NetworkGuid};
use crate::error::{Result, SchemaError};
use crate::export::{NetFieldExport, NetFieldExportGroup};

/// Maximum string size allowed when reading path names (guard against corrupt
/// length prefixes allocating unbounded memory).
const MAX_FSTRING_BYTES: i64 = 1024 * 1024; // 1 MiB

/// Maximum recursion depth for nested NetGUID objects.
const MAX_NET_GUID_RECURSION: u32 = 16;

/// Read an FName from a **byte-aligned** archive.
///
/// FName on the wire (FBinaryArchive variant):
///   isHardcoded: u8 (1 byte boolean)
///   if hardcoded: nameIndex = IntPacked → returned as decimal string
///   else: name = FString, number = i32 (discarded)
fn read_fname(reader: &mut BitReader<'_>) -> Result<String> {
    let is_hardcoded = reader.read_u8()? != 0;
    if is_hardcoded {
        let name_index = reader.read_int_packed()?;
        Ok(name_index.to_string())
    } else {
        let name = reader.read_fstring(MAX_FSTRING_BYTES)?;
        let _number = reader.read_i32()?; // discarded (instance number)
        Ok(name)
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

            let group = NetFieldExportGroup::new(path_name, path_name_index, num_fields);
            cache.add_export_group(group);
        } else {
            // Reference to an existing group by index — it must already be known.
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

    // ── Test helpers mirroring the C# test's byte-building functions ──────────

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
        let mut bytes = Vec::new();
        bytes.push(0); // isHardcoded = false
        bytes.extend(encode_fstring(s));
        bytes.extend_from_slice(&0i32.to_le_bytes()); // instance number
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

    // ── ReadNetFieldExports tests (ported from ExportDataReaderTests.cs) ─────

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

    // ── ReadExportGuids tests (ported from ExportDataReaderTests.cs) ─────────

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

    // ── NetGuidCache unit tests (ported from NetGuidCacheTests.cs) ───────────

    #[test]
    fn cache_stores_group_by_path_and_index() {
        let mut cache = NetGuidCache::new();
        let group = NetFieldExportGroup::new("/Game/Test.Test_C".into(), 7, 2);
        cache.add_export_group(group);

        assert!(cache.get_group_by_path("/Game/Test.Test_C").is_some());
        assert!(cache.get_group_by_index(7).is_some());
    }

    #[test]
    fn cache_merge_expands_and_preserves() {
        let mut cache = NetGuidCache::new();
        let mut group = NetFieldExportGroup::new("/Game/Test.Test_C".into(), 7, 2);
        group.set_field(NetFieldExport {
            handle: 1,
            compatible_checksum: 17,
            name: "ExistingField".into(),
        });
        cache.add_export_group(group);

        // Re-add with larger capacity.
        let expanded = NetFieldExportGroup::new("/Game/Test.Test_C".into(), 7, 4);
        cache.add_export_group(expanded);

        let result = cache.get_group_by_index(7).unwrap();
        assert_eq!(result.len(), 4);
        assert_eq!(result.get_field(1).unwrap().name, "ExistingField");
    }

    #[test]
    fn cache_set_net_guid_path_stores_and_resolves() {
        let mut cache = NetGuidCache::new();
        cache.set_net_guid_path(17, "/Game/Test.Test_C".into(), None);

        assert_eq!(cache.get_path_by_guid(17).unwrap(), "/Game/Test.Test_C");
    }

    #[test]
    fn cache_outer_guid_chain() {
        let mut cache = NetGuidCache::new();
        let outer = NetworkGuid(11);
        cache.set_net_guid_path(17, "Default__Test_C".into(), Some(outer));
        cache.set_net_guid_path(11, "/Game/Test.Test_C".into(), None);

        assert_eq!(cache.get_outer_guid(17).unwrap(), outer);
        assert_eq!(cache.get_outer_path(17).unwrap(), "/Game/Test.Test_C");
    }

    #[test]
    fn cache_net_guid_entries_yields_guid_path_and_outer() {
        let mut cache = NetGuidCache::new();
        cache.set_net_guid_path(11, "/Game/Test.Test_C".into(), None);
        cache.set_net_guid_path(17, "FiringState".into(), Some(NetworkGuid(11)));

        let mut entries = cache.net_guid_entries();
        entries.sort_by_key(|e| e.net_guid);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].net_guid, 11);
        assert_eq!(entries[0].path, "/Game/Test.Test_C");
        assert_eq!(entries[0].outer_net_guid, None);
        assert_eq!(entries[1].net_guid, 17);
        assert_eq!(entries[1].path, "FiringState");
        assert_eq!(entries[1].outer_net_guid, Some(11));
    }

    #[test]
    fn cache_clear_removes_all() {
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new("/Game/Test.Test_C".into(), 7, 2));
        cache.set_net_guid_path(17, "/Game/Test.Test_C".into(), None);

        cache.clear();

        assert!(cache.get_group_by_path("/Game/Test.Test_C").is_none());
        assert!(cache.get_group_by_index(7).is_none());
        assert!(cache.get_path_by_guid(17).is_none());
        assert_eq!(cache.group_count(), 0);
    }

    #[test]
    fn cache_gameplay_tag_lookup() {
        let mut cache = NetGuidCache::new();
        let mut group = NetFieldExportGroup::new("NetworkGameplayTagNodeIndex".into(), 99, 5);
        group.set_field(NetFieldExport {
            handle: 2,
            compatible_checksum: 0,
            name: "Ability.Active".into(),
        });
        cache.add_export_group(group);

        assert_eq!(cache.get_gameplay_tag_name(2).unwrap(), "Ability.Active");
        assert!(cache.get_gameplay_tag_name(4).is_none()); // unpopulated slot
        assert!(cache.get_gameplay_tag_name(99).is_none()); // out of range
    }

    // ── Path alias lookup tests ──────────────────────────────────────────────

    #[test]
    fn alias_lookup_via_cache() {
        let mut cache = NetGuidCache::new();
        let group = NetFieldExportGroup::new("/Game/Characters/_Core/Jett/Jett_C".into(), 50, 1);
        cache.add_export_group(group);

        // Should be reachable via the core-stripped alias.
        assert!(
            cache
                .get_group_by_path("/Game/Characters/Jett/Jett_C")
                .is_some()
        );
    }

    // ── Truncated input rejection ────────────────────────────────────────────

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

    // ── UniqueLeafMatch suffix extension tests ───────────────────────────────

    #[test]
    fn unique_leaf_match_exact() {
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Script/ShooterGame.AresAttributeSet".into(),
            20,
            3,
        ));
        // Exact leaf match: "AresAttributeSet" → leaf is "AresAttributeSet".
        let g = cache.unique_leaf_match("AresAttributeSet").unwrap();
        assert_eq!(g.path, "/Script/ShooterGame.AresAttributeSet");
    }

    #[test]
    fn unique_leaf_match_component_suffix() {
        // Bare name "EquippableStateMachine" should match a group whose leaf is
        // "EquippableStateMachineComponent" via the +Component suffix fallback.
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Script/ShooterGame.EquippableStateMachineComponent".into(),
            30,
            5,
        ));
        let g = cache.unique_leaf_match("EquippableStateMachine").unwrap();
        assert_eq!(
            g.path,
            "/Script/ShooterGame.EquippableStateMachineComponent"
        );
    }

    #[test]
    fn unique_leaf_match_c_suffix() {
        // Bare name "Comp_Projectile_FloatCurveMovement" should match a group
        // whose leaf is "Comp_Projectile_FloatCurveMovement_C" via +_C suffix.
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Game/Characters/Components/Comp_Projectile_FloatCurveMovement.Comp_Projectile_FloatCurveMovement_C".into(),
            40,
            3,
        ));
        let g = cache
            .unique_leaf_match("Comp_Projectile_FloatCurveMovement")
            .unwrap();
        assert_eq!(
            g.path,
            "/Game/Characters/Components/Comp_Projectile_FloatCurveMovement.Comp_Projectile_FloatCurveMovement_C"
        );
    }

    #[test]
    fn unique_leaf_match_rejects_qualified_paths() {
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Script/ShooterGame.TestComponent".into(),
            50,
            2,
        ));
        // Paths with separators should never trigger leaf matching.
        assert!(cache.unique_leaf_match("/Script/Test").is_none());
        assert!(cache.unique_leaf_match("ShooterGame.Test").is_none());
        assert!(cache.unique_leaf_match("Game:Test").is_none());
    }

    #[test]
    fn unique_leaf_match_ambiguous_returns_none() {
        let mut cache = NetGuidCache::new();
        // Two groups with the same leaf "TestComponent" → ambiguous.
        cache.add_export_group(NetFieldExportGroup::new(
            "/Script/A.TestComponent".into(),
            60,
            1,
        ));
        cache.add_export_group(NetFieldExportGroup::new(
            "/Script/B.TestComponent".into(),
            61,
            1,
        ));
        // Exact leaf is ambiguous, but "Test" + "Component" also hits the same
        // ambiguous entry, so still returns None.
        assert!(cache.unique_leaf_match("TestComponent").is_none());
        assert!(cache.unique_leaf_match("Test").is_none());
    }

    #[test]
    fn unique_leaf_match_no_match_returns_none() {
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Script/ShooterGame.SomethingElse".into(),
            70,
            1,
        ));
        assert!(cache.unique_leaf_match("NonexistentThing").is_none());
    }

    // -- resolve_cnc_for_instance_name tests --

    #[test]
    fn cnc_resolve_exact_class_name() {
        // AresAbilitySystem -> AresAbilitySystemComponent_ClassNetCache (Component suffix)
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Script/ShooterGame.AresAbilitySystemComponent_ClassNetCache".into(),
            80,
            1,
        ));
        let g = cache
            .resolve_cnc_for_instance_name("AresAbilitySystem")
            .unwrap();
        assert_eq!(
            g.path,
            "/Script/ShooterGame.AresAbilitySystemComponent_ClassNetCache"
        );
        assert_eq!(g.len(), 1);
    }

    #[test]
    fn cnc_resolve_component_suffix() {
        // ForceModuleManager -> ForceModuleManagerComponent_ClassNetCache
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Script/ShooterGame.ForceModuleManagerComponent_ClassNetCache".into(),
            81,
            4,
        ));
        let g = cache
            .resolve_cnc_for_instance_name("ForceModuleManager")
            .unwrap();
        assert_eq!(
            g.path,
            "/Script/ShooterGame.ForceModuleManagerComponent_ClassNetCache"
        );
        assert_eq!(g.len(), 4);
    }

    #[test]
    fn cnc_resolve_blueprint_c_suffix() {
        // AudDeadeyeVOComponent -> AudDeadeyeVOComponent_C_ClassNetCache
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Game/Audio/VOComponent/AudDeadeyeVoComponent.AudDeadeyeVOComponent_C_ClassNetCache"
                .into(),
            82,
            3,
        ));
        let g = cache
            .resolve_cnc_for_instance_name("AudDeadeyeVOComponent")
            .unwrap();
        assert!(g.path.ends_with("_ClassNetCache"));
        assert_eq!(g.len(), 3);
    }

    #[test]
    fn cnc_resolve_instance_suffix_stripping() {
        // BombDestination_A -> strip _A -> BombDestination -> _C_ClassNetCache
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Game/GameModes/Bomb/BombDestination.BombDestination_C_ClassNetCache".into(),
            83,
            3,
        ));
        let g = cache
            .resolve_cnc_for_instance_name("BombDestination_A")
            .unwrap();
        assert_eq!(
            g.path,
            "/Game/GameModes/Bomb/BombDestination.BombDestination_C_ClassNetCache"
        );
        // Also works for _B variant:
        let g2 = cache
            .resolve_cnc_for_instance_name("BombDestination_B")
            .unwrap();
        assert_eq!(g.path, g2.path);
    }

    #[test]
    fn cnc_resolve_trailing_digit_strip() {
        // WindowShieldA1 -> strip digits -> WindowShieldA -> strip uppercase ->
        // WindowShield -> _C_ClassNetCache
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Game/Interactable/WindowShield.WindowShield_C_ClassNetCache".into(),
            84,
            5,
        ));
        let g = cache
            .resolve_cnc_for_instance_name("WindowShieldA1")
            .unwrap();
        assert_eq!(
            g.path,
            "/Game/Interactable/WindowShield.WindowShield_C_ClassNetCache"
        );
    }

    #[test]
    fn cnc_resolve_multi_segment_strip() {
        // AmbientAudio_Ascent_Defender_SoundA_003 -> strips segments until
        // AmbientAudio matches via _C_ClassNetCache
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Game/Audio/Core/AmbientAudio.AmbientAudio_C_ClassNetCache".into(),
            85,
            1,
        ));
        let g = cache
            .resolve_cnc_for_instance_name("AmbientAudio_Ascent_Defender_SoundA_003")
            .unwrap();
        assert_eq!(
            g.path,
            "/Game/Audio/Core/AmbientAudio.AmbientAudio_C_ClassNetCache"
        );
    }

    #[test]
    fn cnc_resolve_variant_suffix() {
        // MeleeAttackState_Alt -> strip _Alt -> MeleeAttackState ->
        // Component_ClassNetCache
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Script/ShooterGame.MeleeAttackStateComponent_ClassNetCache".into(),
            86,
            2,
        ));
        let g = cache
            .resolve_cnc_for_instance_name("MeleeAttackState_Alt")
            .unwrap();
        assert_eq!(
            g.path,
            "/Script/ShooterGame.MeleeAttackStateComponent_ClassNetCache"
        );
    }

    #[test]
    fn cnc_resolve_no_match_returns_none() {
        // AbilitiesAndBuffsComponent has no CNC group in schema -- must still
        // fail (return None) so the oracle counts it as function_count=0.
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Script/ShooterGame.SomethingUnrelated_ClassNetCache".into(),
            87,
            5,
        ));
        assert!(
            cache
                .resolve_cnc_for_instance_name("AbilitiesAndBuffsComponent")
                .is_none()
        );
    }

    #[test]
    fn cnc_resolve_rejects_qualified_paths() {
        // Fully-qualified paths must not trigger instance name resolution.
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Script/ShooterGame.TestComponent_ClassNetCache".into(),
            88,
            2,
        ));
        assert!(
            cache
                .resolve_cnc_for_instance_name("/Script/ShooterGame.Test")
                .is_none()
        );
        assert!(
            cache
                .resolve_cnc_for_instance_name("ShooterGame.Test")
                .is_none()
        );
    }

    #[test]
    fn cnc_resolve_ambiguous_returns_none() {
        // If two groups share the same CNC leaf, resolution must return None
        // (ambiguous) rather than guessing.
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Script/A.SharedName_ClassNetCache".into(),
            89,
            3,
        ));
        cache.add_export_group(NetFieldExportGroup::new(
            "/Script/B.SharedName_ClassNetCache".into(),
            90,
            5,
        ));
        assert!(cache.resolve_cnc_for_instance_name("SharedName").is_none());
    }

    #[test]
    fn cnc_resolve_grenade_indicator_bounce() {
        // GrenadeExplodeIndicator_Bounce -> strip _Bounce ->
        // GrenadeExplodeIndicator -> _C_ClassNetCache
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Game/Abilities/GrenadeExplodeIndicator.GrenadeExplodeIndicator_C_ClassNetCache"
                .into(),
            91,
            1,
        ));
        let g = cache
            .resolve_cnc_for_instance_name("GrenadeExplodeIndicator_Bounce")
            .unwrap();
        assert!(g.path.ends_with("_ClassNetCache"));
    }

    #[test]
    fn cnc_resolve_switch_exact_name() {
        // Switch_BlackMarket_2 -> first tries full name with _C_ClassNetCache
        // suffix which matches directly without stripping.
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Game/Maps/Switch_BlackMarket_2.Switch_BlackMarket_2_C_ClassNetCache".into(),
            92,
            4,
        ));
        let g = cache
            .resolve_cnc_for_instance_name("Switch_BlackMarket_2")
            .unwrap();
        assert_eq!(
            g.path,
            "/Game/Maps/Switch_BlackMarket_2.Switch_BlackMarket_2_C_ClassNetCache"
        );
    }
}
