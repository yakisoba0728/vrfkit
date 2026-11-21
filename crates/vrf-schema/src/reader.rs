//! Byte-aligned readers for a DemoFrame's ExportData section.
//!
//! ## [`read_net_field_exports`]
//!
//! ```text
//! numLayoutCmdExports: IntPacked
//! for each export:
//!   pathNameIndex:  IntPacked
//!   isExported:    IntPacked (1 = new group, 0 = reference existing, else an error)
//!   if isExported:
//!     pathName:    FString (i32 length + UTF-8/UTF-16 bytes)
//!     numExports:  IntPacked (declared field-slot count)
//!   isFieldExported: u8 (a byte, not a bit)
//!   if isFieldExported:
//!     handle:             IntPacked
//!     compatibleChecksum: u32
//!     name:               FName (u8 kind, then IntPacked index or FString + i32 number)
//! ```
//!
//! ## [`read_export_guids`]
//!
//! ```text
//! numGuids: IntPacked
//! for each guid:
//!   payloadSize: i32
//!   payload[payloadSize]:
//!     netGuid:     IntPacked (0 ends the object)
//!     exportFlags: u8
//!     if HasPath:
//!       outerGuid:        (this object layout, recursively)
//!       pathName:         FString
//!       if HasNetworkChecksum: u32
//! ```

use vrf_bitio::BitReader;

use crate::cache::NetGuidCache;
use crate::error::{Result, SchemaError};
use crate::export::{NetFieldExport, render_fname};
use crate::guid::{ExportFlags, NetworkGuid};

/// Cap on a path-name FString, so a corrupt prefix cannot size an allocation.
pub(crate) const MAX_FSTRING_BYTES: i64 = 1024 * 1024;

const MAX_NET_GUID_RECURSION: u32 = 16;

/// Slot cap shared with the checkpoint form: no corpus group reaches it, and it
/// keeps a corrupt count from sizing an allocation.
pub(crate) const MAX_FIELDS_PER_GROUP: u32 = 65_536;

/// A byte-aligned FName as sent, with the name the schema stores.
#[cfg_attr(not(feature = "checkpoint"), allow(dead_code))]
pub(crate) struct FName {
    /// The index in decimal, or [`render_fname`] of `base` and `number`.
    pub(crate) rendered: String,
    pub(crate) kind: u8,
    pub(crate) base: Option<String>,
    pub(crate) index: Option<u32>,
    pub(crate) number: Option<i32>,
}

/// A u8 kind, then an IntPacked index (nonzero kind) or an FString and i32
/// number (kind 0).
pub(crate) fn read_fname(reader: &mut BitReader<'_>) -> Result<FName> {
    let kind = reader.read_u8()?;
    if kind != 0 {
        let index = reader.read_int_packed()?;
        return Ok(FName {
            rendered: index.to_string(),
            kind,
            base: None,
            index: Some(index),
            number: None,
        });
    }
    let base = reader.read_fstring(MAX_FSTRING_BYTES)?;
    let number = reader.read_i32()?;
    Ok(FName {
        rendered: render_fname(base.clone(), number),
        kind,
        base: Some(base),
        index: None,
        number: Some(number),
    })
}

/// Read one frame's net-field export commands (new groups, and fields added to
/// known ones) into `cache`. Returns the number of commands read.
#[must_use = "the export count is a tally; bind it or discard it explicitly"]
pub fn read_net_field_exports(reader: &mut BitReader<'_>, cache: &mut NetGuidCache) -> Result<u32> {
    let num_exports = reader.read_int_packed()?;

    for _ in 0..num_exports {
        let path_name_index = reader.read_int_packed()?;
        let is_exported = match reader.read_int_packed()? {
            0 => false,
            1 => true,
            value => {
                return Err(SchemaError::BadExportedFlag {
                    path_name_index,
                    value,
                });
            }
        };

        if is_exported {
            let path_name = reader.read_fstring(MAX_FSTRING_BYTES)?;
            let num_fields = reader.read_int_packed()?;

            if num_fields > MAX_FIELDS_PER_GROUP {
                return Err(SchemaError::FieldCountOverflow {
                    count: num_fields,
                    max: MAX_FIELDS_PER_GROUP,
                });
            }

            cache.declare_export_group(path_name, path_name_index, num_fields)?;
        } else if cache.get_group_by_index(path_name_index).is_none() {
            return Err(SchemaError::UnknownPathIndex {
                index: path_name_index,
            });
        }

        if reader.read_u8()? == 0 {
            continue; // isFieldExported
        }
        // Struct fields evaluate in the order written, which is the wire order.
        let field = NetFieldExport {
            handle: reader.read_int_packed()?,
            compatible_checksum: reader.read_u32()?,
            name: read_fname(reader)?.rendered,
        };
        // An out-of-range handle is dropped and counted (`dropped_field_exports`).
        cache.set_field_on_group(path_name_index, field);
    }

    Ok(num_exports)
}

/// Read one frame's exported NetGUID payloads (GUID -> path and outer) into
/// `cache`, each checked for complete consumption. Returns the number read.
#[must_use = "the GUID payload count is a tally; bind it or discard it explicitly"]
pub fn read_export_guids(reader: &mut BitReader<'_>, cache: &mut NetGuidCache) -> Result<u32> {
    let num_guids = reader.read_int_packed()?;
    let mut register = |guid, path, outer: NetworkGuid| {
        cache.set_net_guid_path(guid, path, outer.is_valid().then_some(outer));
    };

    for _ in 0..num_guids {
        let size = reader.read_i32()?;
        if size < 0 {
            return Err(SchemaError::NegativePayloadSize { size });
        }
        let mut payload = reader.sub_reader(size as u64 * 8)?;
        load_object(&mut payload, true, MAX_FSTRING_BYTES, 0, &mut register)?;

        if payload.bits_remaining() >= 8 {
            return Err(SchemaError::TrailingPayloadData {
                remaining: (payload.bits_remaining() / 8) as usize,
            });
        }
    }

    Ok(num_guids)
}

/// Read one NetGUID object reference (the object layout above), recursing
/// into its outer, and pass each path to `register(guid, path, outer)`,
/// outers first. The flags byte is read when `is_exporting` or for the
/// default object (GUID 1); `max_path_bytes` caps the path FString.
pub fn load_object(
    reader: &mut BitReader<'_>,
    is_exporting: bool,
    max_path_bytes: i64,
    depth: u32,
    register: &mut impl FnMut(u32, String, NetworkGuid),
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

    let flags = if is_exporting || net_guid.is_default() {
        ExportFlags(reader.read_u8()?)
    } else {
        ExportFlags::NONE
    };
    if !flags.contains(ExportFlags::HAS_PATH) {
        return Ok(net_guid);
    }
    let outer_guid = load_object(reader, is_exporting, max_path_bytes, depth + 1, register)?;
    let path_name = reader.read_fstring(max_path_bytes)?;
    if flags.contains(ExportFlags::HAS_NETWORK_CHECKSUM) {
        let _checksum = reader.read_u32()?;
    }

    register(net_guid.0, path_name, outer_guid);
    Ok(net_guid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vrf_testkit::{BitWrite, BitWriter, pack};

    type Read = fn(&mut BitReader<'_>, &mut NetGuidCache) -> Result<u32>;

    /// `read` over `w`'s bytes into a fresh cache.
    fn run(w: &BitWriter, read: Read) -> (Result<u32>, NetGuidCache) {
        let mut cache = NetGuidCache::new();
        let result = read(&mut BitReader::new(&pack(w)), &mut cache);
        (result, cache)
    }

    /// isFieldExported, the handle, a fixed checksum and a string FName.
    fn field(w: &mut BitWriter, handle: u32, name: &str, number: i32) {
        w.u8(1).int_packed(handle).u32(0xAABB_CCDD);
        w.u8(0).fstring(name).i32(number);
    }

    /// A net-field export command declaring a group, with an optional field.
    fn new_group(w: &mut BitWriter, index: u32, path: &str, slots: u32, f: Option<(u32, &str)>) {
        w.int_packed(index)
            .int_packed(1)
            .fstring(path)
            .int_packed(slots);
        match f {
            Some((handle, name)) => field(w, handle, name, 0),
            None => {
                w.u8(0);
            }
        }
    }

    /// A reference to an existing group that adds one field.
    fn existing_field(w: &mut BitWriter, index: u32, handle: u32, name: &str, number: i32) {
        field(w.int_packed(index).int_packed(0), handle, name, number);
    }

    /// One length-prefixed export-GUID payload: `guid` with HasPath, outer 0
    /// (invalid, which ends the recursion), `path`, then `trailing`.
    fn guid_payload(w: &mut BitWriter, guid: u32, path: &str, trailing: &[u8]) {
        let mut payload = BitWriter::new();
        payload
            .int_packed(guid)
            .u8(ExportFlags::HAS_PATH.0)
            .int_packed(0);
        payload.fstring(path).bytes(trailing);
        w.i32(payload.len() as i32 / 8).extend_bits(&payload);
    }

    #[test]
    fn registers_exported_group() {
        let mut w = BitWriter::new();
        new_group(w.int_packed(1), 11, "/Game/Test.Test_C", 3, None);
        let (n, cache) = run(&w, read_net_field_exports);
        assert_eq!(n, Ok(1));

        let group = cache.get_group_by_index(11).unwrap();
        assert_eq!(group.path, "/Game/Test.Test_C");
        assert_eq!(group.path_name_index, 11);
        assert_eq!(group.len(), 3);
        assert!(cache.get_group_by_path("/Game/Test.Test_C").is_some());
    }

    #[test]
    fn live_group_rejects_field_count_above_checkpoint_protocol_limit() {
        let mut w = BitWriter::new();
        new_group(w.int_packed(1), 11, "/Game/Test.Test_C", 65_537, None);
        let (n, cache) = run(&w, read_net_field_exports);
        assert_eq!(
            n,
            Err(SchemaError::FieldCountOverflow {
                count: 65_537,
                max: 65_536
            })
        );
        assert_eq!(cache.group_count(), 0);
    }

    #[test]
    fn stores_export_by_handle() {
        let mut w = BitWriter::new();
        new_group(
            w.int_packed(1),
            11,
            "/Game/T.T_C",
            3,
            Some((2, "FieldName")),
        );
        let (n, cache) = run(&w, read_net_field_exports);
        assert_eq!(n, Ok(1));

        let field = cache.get_group_by_index(11).unwrap().get_field(2).unwrap();
        assert_eq!(field.handle, 2);
        assert_eq!(field.compatible_checksum, 0xAABBCCDD);
        assert_eq!(field.name, "FieldName");
    }

    #[test]
    fn re_exported_group_expands_without_losing_existing_fields() {
        let mut w = BitWriter::new();
        new_group(
            w.int_packed(2),
            11,
            "/Game/T.T_C",
            2,
            Some((1, "ExistingField")),
        );
        new_group(&mut w, 11, "/Game/T.T_C", 4, Some((3, "ExpandedField")));
        let (n, cache) = run(&w, read_net_field_exports);
        assert_eq!(n, Ok(2));

        let group = cache.get_group_by_index(11).unwrap();
        assert_eq!(group.len(), 4);
        assert_eq!(group.get_field(1).unwrap().name, "ExistingField");
        assert_eq!(group.get_field(3).unwrap().name, "ExpandedField");
    }

    /// Read as a reference, isExported 2 would overwrite a real field with
    /// whatever bytes follow, and the read would succeed.
    #[test]
    fn an_is_exported_value_other_than_0_or_1_is_an_error() {
        let mut w = BitWriter::new();
        new_group(w.int_packed(2), 11, "/Game/T.T_C", 1, Some((0, "Real")));
        field(w.int_packed(11).int_packed(2), 0, "Overwritten", 0);
        let (n, cache) = run(&w, read_net_field_exports);
        assert_eq!(
            n,
            Err(SchemaError::BadExportedFlag {
                path_name_index: 11,
                value: 2
            })
        );
        let group = cache.get_group_by_index(11).unwrap();
        assert_eq!(group.get_field(0).unwrap().name, "Real");
    }

    /// The error fires before isFieldExported is read.
    #[test]
    fn unknown_path_index_returns_error() {
        let mut w = BitWriter::new();
        w.int_packed(1).int_packed(42).int_packed(0); // one reference, to index 42
        let (n, _) = run(&w, read_net_field_exports);
        assert_eq!(n, Err(SchemaError::UnknownPathIndex { index: 42 }));
    }

    /// A handle past the declared slots is dropped and counted.
    /// `dropped_field_exports` is 0 in all 1,018 corpus manifests, so only this
    /// test shows it moves.
    #[test]
    fn out_of_range_handle_is_dropped_and_counted() {
        let mut w = BitWriter::new();
        new_group(
            w.int_packed(1),
            11,
            "/Game/T.T_C",
            1,
            Some((2, "OutOfRange")),
        );
        let (n, cache) = run(&w, read_net_field_exports);
        assert_eq!(n, Ok(1), "a drop is counted, not an error");

        assert_eq!(cache.dropped_field_exports(), 1);
        let group = cache.get_group_by_index(11).unwrap();
        assert_eq!(group.len(), 1, "a dropped field must not grow the group");
        assert_eq!(group.populated_fields().count(), 0);
    }

    /// Positive control for the test above: without it, a counter that moved
    /// on every field would pass the drop test.
    #[test]
    fn in_range_handle_is_placed_and_not_counted() {
        let mut w = BitWriter::new();
        new_group(w.int_packed(1), 11, "/Game/T.T_C", 1, Some((0, "InRange")));
        let (n, cache) = run(&w, read_net_field_exports);
        assert_eq!(n, Ok(1));

        assert_eq!(cache.dropped_field_exports(), 0);
        let group = cache.get_group_by_index(11).unwrap();
        assert_eq!(group.len(), 1);
        assert_eq!(group.get_field(0).unwrap().name, "InRange");
        assert_eq!(group.populated_fields().count(), 1);
    }

    /// The number is part of the name: 0 renders bare, 1 renders `_0`.
    #[test]
    fn fname_numbers_disambiguate_two_fields_with_one_base_name() {
        let mut w = BitWriter::new();
        new_group(w.int_packed(3), 11, "/Game/T.T_C", 3, Some((0, "Value")));
        existing_field(&mut w, 11, 1, "Value", 1);
        existing_field(&mut w, 11, 2, "Value", 2);
        let (n, cache) = run(&w, read_net_field_exports);
        assert_eq!(n, Ok(3));

        let group = cache.get_group_by_index(11).unwrap();
        assert_eq!(group.get_field(0).unwrap().name, "Value");
        assert_eq!(group.get_field(1).unwrap().name, "Value_0");
        assert_eq!(group.get_field(2).unwrap().name, "Value_1");
    }

    #[test]
    fn export_guid_registers_path() {
        let mut w = BitWriter::new();
        guid_payload(w.int_packed(1), 17, "/Game/Test.Test_C", &[]);
        let (n, cache) = run(&w, read_export_guids);
        assert_eq!(n, Ok(1));
        assert_eq!(cache.get_path_by_guid(17), Some("/Game/Test.Test_C"));
    }

    #[test]
    fn export_guid_negative_size_returns_error() {
        let mut w = BitWriter::new();
        w.int_packed(1).i32(-1);
        let (n, _) = run(&w, read_export_guids);
        assert_eq!(n, Err(SchemaError::NegativePayloadSize { size: -1 }));
    }

    #[test]
    fn export_guid_trailing_data_returns_error() {
        let mut w = BitWriter::new();
        guid_payload(w.int_packed(1), 17, "/Game/Test.Test_C", &[0xFF]);
        let (n, _) = run(&w, read_export_guids);
        assert_eq!(n, Err(SchemaError::TrailingPayloadData { remaining: 1 }));
    }

    /// A count with nothing after it: truncation surfaces as a bit-reader error.
    #[test]
    fn truncated_input_returns_bitio_error() {
        let mut w = BitWriter::new();
        w.int_packed(5);
        for read in [read_net_field_exports, read_export_guids] {
            assert!(matches!(run(&w, read).0, Err(SchemaError::Bitio(_))));
        }
    }

    #[test]
    fn zero_counts_are_valid() {
        let mut w = BitWriter::new();
        w.int_packed(0);
        for read in [read_net_field_exports, read_export_guids] {
            assert_eq!(run(&w, read).0, Ok(0));
        }
    }

    /// Two groups, each with a declared field and one added by reference.
    #[test]
    fn roundtrip_multiple_groups_multiple_fields() {
        let mut w = BitWriter::new();
        new_group(w.int_packed(4), 1, "/Game/A.A_C", 3, Some((0, "Alpha")));
        existing_field(&mut w, 1, 2, "Gamma", 0);
        new_group(&mut w, 2, "/Game/B.B_C", 2, Some((1, "Beta")));
        existing_field(&mut w, 2, 0, "Delta", 0);
        let (n, cache) = run(&w, read_net_field_exports);
        assert_eq!(n, Ok(4));

        let a = cache.get_group_by_index(1).unwrap();
        assert_eq!(a.get_field(0).unwrap().name, "Alpha");
        assert_eq!(a.get_field(2).unwrap().name, "Gamma");
        assert!(a.get_field(1).is_none());

        let b = cache.get_group_by_index(2).unwrap();
        assert_eq!(b.get_field(1).unwrap().name, "Beta");
        assert_eq!(b.get_field(0).unwrap().name, "Delta");
    }
}
