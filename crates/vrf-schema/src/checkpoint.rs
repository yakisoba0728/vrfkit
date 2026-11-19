//! The two schema tables a Checkpoint chunk carries ahead of its DemoFrame.
//!
//! A checkpoint archive is self-contained (the server's NetGUID cache and
//! whole export map, then one DemoFrame), so it is read into its own
//! [`NetGuidCache`]: its frame re-opens every live actor, and replaying those
//! opens through the live reader would corrupt the running channel table.
//!
//! # Archive layout
//!
//! ```text
//! +0   u32  frame offset      -- the DemoFrame begins at this + 8
//! +4   u32  0                 -- reserved, zero in every corpus checkpoint
//! +8   u32  0
//! +12  u32  0
//! +16  u32  guid entry count
//! +20  GuidCacheEntry x count
//!      u32  export group count
//!      NetFieldExportGroup x count
//!      <- the DemoFrame starts here, and this offset must equal (+0) + 8
//! ```
//!
//! All reads are byte-aligned `FBinaryArchive`, as in the DemoFrame grammar.
//!
//! # GuidCacheEntry
//!
//! ```text
//! NetGUID      : IntPacked
//! OuterGUID    : IntPacked
//! PathIsString : u8            -- 0 or 1 only
//!   if 1: PathName  : FString  -- no trailing i32, unlike an FName
//!   if 0: NameIndex : IntPacked
//! Flags        : u8
//! ```
//!
//! `PathIsString`'s polarity is the opposite of an FName's leading byte, where
//! nonzero means "hardcoded index"; the FName reader must not be pointed at it.
//!
//! # NetFieldExportGroup
//!
//! ```text
//! PathName           : FString
//! PathNameIndex      : IntPacked
//! NumNetFieldExports : IntPacked        <-- IntPacked, and the count at the
//!                                           head of the section is a u32
//! repeat, slot index i = 0..N:
//!     bExported : u8
//!     if bExported:
//!         Handle             : IntPacked   -- always == i
//!         CompatibleChecksum : u32
//!         ExportName         : FName
//! ```
//!
//! Read as a u32, `NumNetFieldExports` doubles small counts and overruns into
//! plausible garbage, which is why [`read_checkpoint_tables`] ends by asserting
//! the prologue's frame offset.

use std::borrow::Cow;

use vrf_bitio::{BitError, BitReader};

use crate::cache::NetGuidCache;
use crate::error::{Result, SchemaError};
use crate::export::NetFieldExport;
use crate::guid::NetworkGuid;
use crate::reader::{MAX_FIELDS_PER_GROUP, MAX_FSTRING_BYTES, read_fname};

/// Streaming observer for checkpoint schema records. Borrowed strings are valid
/// only for the callback; the cache receives its own owned copy afterwards.
/// Every method defaults to a no-op.
pub trait CheckpointTableSink {
    type Error;

    #[allow(clippy::too_many_arguments)]
    fn on_guid_entry(
        &mut self,
        _ordinal: u32,
        _guid: u32,
        _outer: u32,
        _path_is_string: bool,
        _literal_path: Option<&str>,
        _name_index: Option<u32>,
        _flags: u8,
    ) -> core::result::Result<(), Self::Error> {
        Ok(())
    }

    fn on_export_group(
        &mut self,
        _ordinal: u32,
        _path_name_index: u32,
        _group_path: &str,
        _declared_slots: u32,
    ) -> core::result::Result<(), Self::Error> {
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn on_export_field(
        &mut self,
        _group_ordinal: u32,
        _path_name_index: u32,
        _slot: u32,
        _handle: u32,
        _checksum: u32,
        _rendered_name: &str,
        _exported_flag: u8,
        _fname_kind: u8,
        _base: Option<&str>,
        _index: Option<u32>,
        _number: Option<i32>,
    ) -> core::result::Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CheckpointReadError<E> {
    #[error(transparent)]
    Schema(#[from] SchemaError),
    #[error("checkpoint table observer failed")]
    Sink(E),
}

impl<E> From<BitError> for CheckpointReadError<E> {
    fn from(error: BitError) -> Self {
        Self::Schema(error.into())
    }
}

/// The largest corpus checkpoint carries about 12,000 guid entries.
const MAX_GUID_ENTRIES: u32 = 1_000_000;

/// The largest corpus checkpoint declares 543 export groups.
const MAX_GROUPS: u32 = 100_000;

/// Bound indexed-path expansion before cache storage and detached output copies.
/// This counts UTF-8 bytes across declarations, not total process memory.
const MAX_RESOLVED_GUID_PATH_BYTES: usize = 256 * 1024 * 1024;

/// What [`read_checkpoint_tables`] consumed, for the caller to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckpointTables {
    /// GUID cache entries read.
    pub guid_count: u32,
    /// Export groups read.
    pub group_count: u32,
    /// Field slots that were actually exported (the rest are holes).
    pub exported_fields: u32,
    /// Byte offset where the DemoFrame begins.
    pub frame_offset: usize,
    /// Entries whose path arrived as an index into the literal paths (not an EName).
    pub indexed_paths: u32,
    /// Literal GUID-path entries read, in either mode.
    pub literal_paths: u32,
    /// Wire indices resolved through preceding literal paths.
    pub resolved_path_indices: u32,
}

/// Interpretation of checkpoint GUID path indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointPathMode {
    /// The index in decimal, for comparing with legacy output.
    LegacyDecimal,
    /// Resolve an index into this checkpoint's preceding literal paths (the default).
    LiteralPathTable,
}

/// Read a checkpoint archive's guid cache and export-group map into `cache`,
/// and report where its DemoFrame begins.
///
/// `data` is the decompressed archive (`vrf_container::decompress_checkpoint`).
/// `cache` must be fresh: a checkpoint restates the whole schema, and merging
/// it into the live cache would combine independent snapshots.
///
/// # GUID path indices
///
/// An indexed path is a zero-based position among the literal paths before it
/// in this checkpoint; references are not appended, and the table resets per
/// call. It matches the main stream's own paths on 1,018 replays of 24 builds
/// (docs/CHECKPOINT_PATH_RESOLUTION.md; the export guards repeat the check):
/// agreement between two readers, not an engine specification. An index past
/// the preceding literals is rejected; raw indices stay available to
/// [`CheckpointTableSink`], and [`CheckpointPathMode::LegacyDecimal`] (via
/// [`read_checkpoint_tables_with_sink_mode`]) reproduces legacy paths.
///
/// # Errors
///
/// Beyond truncation, each `Checkpoint*` [`SchemaError`] is a check that the
/// cursor is still aligned. `PathIsString` accepts only 0 and 1; the FName
/// kind and exported-slot flag accept any nonzero byte.
/// GUID paths also have a cumulative 256 MiB resolved UTF-8 byte budget per
/// checkpoint, including repeated GUIDs and literal/legacy paths. Exceeding it
/// returns [`SchemaError::ResourceLimit`] before cloning an indexed path,
/// invoking that entry's observer or storing it. Prior callbacks are provisional.
pub fn read_checkpoint_tables(data: &[u8], cache: &mut NetGuidCache) -> Result<CheckpointTables> {
    let mut sink = NoopCheckpointTableSink;
    match read_checkpoint_tables_with_sink(data, cache, &mut sink) {
        Ok(tables) => Ok(tables),
        Err(CheckpointReadError::Schema(error)) => Err(error),
        Err(CheckpointReadError::Sink(never)) => match never {},
    }
}

struct NoopCheckpointTableSink;

impl CheckpointTableSink for NoopCheckpointTableSink {
    type Error = core::convert::Infallible;
}

/// [`read_checkpoint_tables`], delivering each record to `sink` after its wire
/// checks and before it is stored in `cache`. A sink error stops the read
/// before any later record or DemoFrame byte is consumed.
pub fn read_checkpoint_tables_with_sink<S: CheckpointTableSink>(
    data: &[u8],
    cache: &mut NetGuidCache,
    sink: &mut S,
) -> core::result::Result<CheckpointTables, CheckpointReadError<S::Error>> {
    read_checkpoint_tables_with_sink_mode(data, cache, sink, CheckpointPathMode::LiteralPathTable)
}

/// Read checkpoint tables with an explicit path interpretation.
pub fn read_checkpoint_tables_with_sink_mode<S: CheckpointTableSink>(
    data: &[u8],
    cache: &mut NetGuidCache,
    sink: &mut S,
    mode: CheckpointPathMode,
) -> core::result::Result<CheckpointTables, CheckpointReadError<S::Error>> {
    read_checkpoint_tables_with_path_limit(data, cache, sink, mode, MAX_RESOLVED_GUID_PATH_BYTES)
}

fn read_checkpoint_tables_with_path_limit<S: CheckpointTableSink>(
    data: &[u8],
    cache: &mut NetGuidCache,
    sink: &mut S,
    mode: CheckpointPathMode,
    max_path_bytes: usize,
) -> core::result::Result<CheckpointTables, CheckpointReadError<S::Error>> {
    let mut reader = BitReader::new(data);

    let frame_offset_word = reader.read_u32()?;
    for offset in [4usize, 8, 12] {
        let value = reader.read_u32()?;
        if value != 0 {
            return Err(SchemaError::CheckpointReservedWordSet { offset, value }.into());
        }
    }
    let guid_count = reader.read_u32()?;
    if guid_count > MAX_GUID_ENTRIES {
        return Err(SchemaError::CheckpointCountOverflow {
            field: "guid entries",
            count: guid_count,
            max: MAX_GUID_ENTRIES,
        }
        .into());
    }

    let mut indexed_paths = 0u32;
    let mut literal_paths: Vec<String> = Vec::new();
    let mut literal_count = 0u32;
    let mut resolved_path_indices = 0u32;
    let mut resolved_path_bytes = 0usize;
    for entry in 0..guid_count {
        let net_guid = reader.read_int_packed()?;
        let outer_guid = reader.read_int_packed()?;
        let path_kind = reader.read_u8()?;
        let (path_is_string, path, name_index) = match path_kind {
            1 => {
                let path = reader.read_fstring(MAX_FSTRING_BYTES)?;
                (true, Cow::Owned(path), None)
            }
            0 => {
                indexed_paths += 1;
                let index = reader.read_int_packed()?;
                let path = match mode {
                    CheckpointPathMode::LegacyDecimal => Cow::Owned(index.to_string()),
                    CheckpointPathMode::LiteralPathTable => {
                        let Some(path) = literal_paths.get(index as usize) else {
                            return Err(SchemaError::CheckpointPathIndexOutOfBounds {
                                entry,
                                index,
                                literals: literal_paths.len() as u32,
                            }
                            .into());
                        };
                        resolved_path_indices += 1;
                        Cow::Borrowed(path.as_str())
                    }
                };
                (false, path, Some(index))
            }
            byte => return Err(SchemaError::CheckpointBadPathKind { entry, byte }.into()),
        };
        // Preserve the flags byte without assigning unverified bit meanings.
        let flags = reader.read_u8()?;

        // A short index can expand to a long literal. Check before cloning;
        // sharing only the cache would still amplify detached summary rows.
        let requested = resolved_path_bytes.saturating_add(path.len());
        if requested > max_path_bytes {
            return Err(SchemaError::ResourceLimit {
                resource: "checkpoint resolved GUID path bytes",
                requested,
                limit: max_path_bytes,
            }
            .into());
        }
        resolved_path_bytes = requested;
        let path = path.into_owned();
        if path_is_string {
            literal_count += 1;
            if mode == CheckpointPathMode::LiteralPathTable {
                literal_paths.push(path.clone());
            }
        }

        sink.on_guid_entry(
            entry,
            net_guid,
            outer_guid,
            path_is_string,
            path_is_string.then_some(path.as_str()),
            name_index,
            flags,
        )
        .map_err(CheckpointReadError::Sink)?;

        cache.set_net_guid_path(net_guid, path, Some(NetworkGuid(outer_guid)));
    }

    let group_count = reader.read_u32()?;
    if group_count > MAX_GROUPS {
        return Err(SchemaError::CheckpointCountOverflow {
            field: "export groups",
            count: group_count,
            max: MAX_GROUPS,
        }
        .into());
    }

    let mut exported_fields = 0u32;
    for group_ordinal in 0..group_count {
        let path = reader.read_fstring(MAX_FSTRING_BYTES)?;
        let path_name_index = reader.read_int_packed()?;
        // IntPacked, not u32. See the module docs.
        let declared = reader.read_int_packed()?;
        if declared > MAX_FIELDS_PER_GROUP {
            return Err(SchemaError::CheckpointCountOverflow {
                field: "fields in a group",
                count: declared,
                max: MAX_FIELDS_PER_GROUP,
            }
            .into());
        }

        // The two lookups `add_export_group` merges on (`by_path` with aliases,
        // `by_index`): in a fresh cache, a hit means a group declared twice.
        if cache.get_group_by_index(path_name_index).is_some()
            || cache.get_group_by_path(&path).is_some()
        {
            return Err(SchemaError::CheckpointGroupCollision {
                path,
                path_name_index,
            }
            .into());
        }

        sink.on_export_group(group_ordinal, path_name_index, &path, declared)
            .map_err(CheckpointReadError::Sink)?;

        cache.declare_export_group(path.clone(), path_name_index, declared)?;

        for slot in 0..declared {
            let exported_flag = reader.read_u8()?;
            if exported_flag == 0 {
                continue;
            }
            let handle = reader.read_int_packed()?;
            if handle != slot {
                return Err(SchemaError::CheckpointHandleNotSlot {
                    group: path,
                    slot,
                    handle,
                }
                .into());
            }
            let compatible_checksum = reader.read_u32()?;
            let name = read_fname(&mut reader)?;
            sink.on_export_field(
                group_ordinal,
                path_name_index,
                slot,
                handle,
                compatible_checksum,
                &name.rendered,
                exported_flag,
                name.kind,
                name.base.as_deref(),
                name.index,
                name.number,
            )
            .map_err(CheckpointReadError::Sink)?;
            cache.set_field_on_group(
                path_name_index,
                NetFieldExport {
                    handle,
                    compatible_checksum,
                    name: name.rendered,
                },
            );
            exported_fields += 1;
        }
    }

    // The one end-to-end check.
    let map_end = (reader.position() / 8) as usize;
    let expected = frame_offset_word as usize + 8;
    if map_end != expected {
        return Err(SchemaError::CheckpointFrameOffsetMismatch { map_end, expected }.into());
    }

    Ok(CheckpointTables {
        guid_count,
        group_count,
        exported_fields,
        frame_offset: map_end,
        indexed_paths,
        literal_paths: literal_count,
        resolved_path_indices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vrf_testkit::{add_fstring_utf16, add_i32, add_int_packed, add_u32};

    /// `(net_guid, outer_guid, path or None for a hardcoded name, name index)`.
    type GuidSpec<'a> = (u32, u32, Option<&'a str>, u32);
    /// `(path, path name index, declared slots, exported (handle, name, FName number))`.
    type GroupSpec<'a> = (&'a str, u32, u32, &'a [(u32, &'a str, i32)]);
    type GuidEvent = (u32, bool, Option<String>, Option<u32>, u8);
    type FieldEvent = (
        u32,
        u32,
        u32,
        u32,
        String,
        u8,
        u8,
        Option<String>,
        Option<u32>,
        Option<i32>,
    );

    /// The prologue -- frame offset, three reserved zero words, the GUID entry
    /// count -- then `body` (the GUID entries and the group map) and `frame`.
    fn archive(guid_count: u32, body: &[u8], frame: &[u8]) -> Vec<u8> {
        // The frame offset is measured from byte 8, so it is (20 + body) - 8.
        let mut out = ((20 + body.len() - 8) as u32).to_le_bytes().to_vec();
        out.extend_from_slice(&[0u8; 12]);
        out.extend_from_slice(&guid_count.to_le_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(frame);
        out
    }

    /// Build an archive: prologue, guid entries, group map, then `frame`.
    fn build(guids: &[GuidSpec<'_>], groups: &[GroupSpec<'_>], frame: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        for &(guid, outer, path, name_index) in guids {
            add_int_packed(&mut body, guid);
            add_int_packed(&mut body, outer);
            match path {
                Some(p) => {
                    body.push(1);
                    add_fstring_utf16(&mut body, p);
                }
                None => {
                    body.push(0);
                    add_int_packed(&mut body, name_index);
                }
            }
            body.push(0x03);
        }
        add_u32(&mut body, groups.len() as u32);
        for &(path, index, declared, fields) in groups {
            add_fstring_utf16(&mut body, path);
            add_int_packed(&mut body, index);
            add_int_packed(&mut body, declared);
            for slot in 0..declared {
                match fields.iter().find(|field| field.0 == slot) {
                    Some(&(handle, name, number)) => {
                        body.push(1);
                        add_int_packed(&mut body, handle);
                        add_u32(&mut body, 0xdead_beef);
                        body.push(0); // FName: not hardcoded
                        add_fstring_utf16(&mut body, name);
                        add_i32(&mut body, number);
                    }
                    None => body.push(0),
                }
            }
        }
        archive(guids.len() as u32, &body, frame)
    }

    #[derive(Default)]
    struct RecordingSink {
        guids: Vec<GuidEvent>,
        groups: Vec<(u32, u32, String, u32)>,
        fields: Vec<FieldEvent>,
    }

    impl CheckpointTableSink for RecordingSink {
        type Error = ();

        fn on_guid_entry(
            &mut self,
            ordinal: u32,
            _: u32,
            _: u32,
            path_is_string: bool,
            literal_path: Option<&str>,
            name_index: Option<u32>,
            flags: u8,
        ) -> core::result::Result<(), Self::Error> {
            self.guids.push((
                ordinal,
                path_is_string,
                literal_path.map(str::to_owned),
                name_index,
                flags,
            ));
            Ok(())
        }

        fn on_export_group(
            &mut self,
            ordinal: u32,
            path_name_index: u32,
            group_path: &str,
            declared_slots: u32,
        ) -> core::result::Result<(), Self::Error> {
            self.groups.push((
                ordinal,
                path_name_index,
                group_path.to_owned(),
                declared_slots,
            ));
            Ok(())
        }

        fn on_export_field(
            &mut self,
            group_ordinal: u32,
            path_name_index: u32,
            slot: u32,
            handle: u32,
            checksum: u32,
            rendered_name: &str,
            exported_flag: u8,
            fname_kind: u8,
            base: Option<&str>,
            index: Option<u32>,
            number: Option<i32>,
        ) -> core::result::Result<(), Self::Error> {
            self.fields.push((
                group_ordinal,
                path_name_index,
                slot,
                handle,
                rendered_name.to_owned(),
                exported_flag,
                fname_kind,
                base.map(str::to_owned),
                index,
                number,
            ));
            assert_eq!(checksum, 0xdead_beef);
            Ok(())
        }
    }

    #[test]
    fn observer_reports_raw_variants_before_cache_storage() {
        let archive = build(
            &[(7, 0, Some("/Game/X"), 0), (8, 7, None, 0)],
            &[("/Script/G.Thing", 7, 2, &[(1, "Value", 1)])],
            &[],
        );
        let mut cache = NetGuidCache::new();
        let mut sink = RecordingSink::default();
        let tables = read_checkpoint_tables_with_sink(&archive, &mut cache, &mut sink).unwrap();

        assert_eq!(tables.exported_fields, 1);
        assert_eq!(sink.guids[0], (0, true, Some("/Game/X".into()), None, 3));
        assert_eq!(sink.guids[1], (1, false, None, Some(0), 3));
        assert_eq!(sink.groups, vec![(0, 7, "/Script/G.Thing".into(), 2)]);
        assert_eq!(
            sink.fields,
            vec![(
                0,
                7,
                1,
                1,
                "Value_0".into(),
                1,
                0,
                Some("Value".into()),
                None,
                Some(1),
            )]
        );
        assert_eq!(cache.get_path_by_guid(7), Some("/Game/X"));
    }

    struct StopAfterFirstGuid(u32);

    impl CheckpointTableSink for StopAfterFirstGuid {
        type Error = &'static str;

        fn on_guid_entry(
            &mut self,
            _: u32,
            _: u32,
            _: u32,
            _: bool,
            _: Option<&str>,
            _: Option<u32>,
            _: u8,
        ) -> core::result::Result<(), Self::Error> {
            self.0 += 1;
            Err("stop")
        }

        fn on_export_group(
            &mut self,
            _: u32,
            _: u32,
            _: &str,
            _: u32,
        ) -> core::result::Result<(), Self::Error> {
            panic!("reader consumed a later record after sink failure")
        }

        fn on_export_field(
            &mut self,
            _: u32,
            _: u32,
            _: u32,
            _: u32,
            _: u32,
            _: &str,
            _: u8,
            _: u8,
            _: Option<&str>,
            _: Option<u32>,
            _: Option<i32>,
        ) -> core::result::Result<(), Self::Error> {
            panic!("reader consumed a later record after sink failure")
        }
    }

    #[test]
    fn observer_failure_stops_before_cache_or_later_records() {
        let archive = build(
            &[
                (7, 0, Some("/Game/First"), 0),
                (8, 0, Some("/Game/Second"), 0),
            ],
            &[("/Script/G.Thing", 7, 0, &[])],
            &[0xA5],
        );
        let mut cache = NetGuidCache::new();
        let mut sink = StopAfterFirstGuid(0);
        let error = read_checkpoint_tables_with_sink(&archive, &mut cache, &mut sink).unwrap_err();

        assert!(matches!(error, CheckpointReadError::Sink("stop")));
        assert_eq!(sink.0, 1);
        assert!(cache.get_path_by_guid(7).is_none());
        assert!(cache.get_path_by_guid(8).is_none());
        assert_eq!(cache.group_count(), 0);
    }

    #[test]
    fn observer_sees_zero_slot_group() {
        let archive = build(&[], &[("/Script/G.Empty", 7, 0, &[])], &[0xA5]);
        let mut cache = NetGuidCache::new();
        let mut sink = RecordingSink::default();
        read_checkpoint_tables_with_sink(&archive, &mut cache, &mut sink).unwrap();

        assert_eq!(sink.groups, vec![(0, 7, "/Script/G.Empty".into(), 0)]);
        assert!(sink.fields.is_empty());
    }

    #[test]
    fn observer_preserves_nonzero_wire_flags_and_hardcoded_fname_index() {
        let mut body = 1u32.to_le_bytes().to_vec(); // one group
        add_fstring_utf16(&mut body, "/Script/G.Raw");
        add_int_packed(&mut body, 42);
        add_int_packed(&mut body, 1);
        body.push(9); // nonzero bExported is accepted verbatim
        add_int_packed(&mut body, 0);
        add_u32(&mut body, 0xdead_beef);
        body.push(2); // nonzero FName kind is a hardcoded index, verbatim
        add_int_packed(&mut body, 216);

        let mut cache = NetGuidCache::new();
        let mut sink = RecordingSink::default();
        read_checkpoint_tables_with_sink(&archive(0, &body, &[]), &mut cache, &mut sink).unwrap();

        assert_eq!(
            sink.fields,
            vec![(0, 42, 0, 0, "216".into(), 9, 2, None, Some(216), None)]
        );
        assert_eq!(
            cache
                .get_group_by_index(42)
                .and_then(|group| group.get_field(0))
                .map(|field| field.name.as_str()),
            Some("216")
        );
    }

    const RESOLVED: CheckpointPathMode = CheckpointPathMode::LiteralPathTable;

    #[test]
    fn path_budget_rejects_before_observer_and_cache_mutation() {
        for last in [(9, 0, None, 0), (7, 0, None, 0), (9, 0, Some("CD"), 0)] {
            let archive = build(&[(7, 0, Some("AB"), 0), (8, 7, None, 0), last], &[], &[]);
            let mut cache = NetGuidCache::new();
            let mut sink = RecordingSink::default();
            let error = read_checkpoint_tables_with_path_limit(
                &archive, &mut cache, &mut sink, RESOLVED, 4,
            )
            .unwrap_err();
            assert!(matches!(
                error,
                CheckpointReadError::Schema(SchemaError::ResourceLimit {
                    resource: "checkpoint resolved GUID path bytes",
                    requested: 6,
                    limit: 4
                })
            ));
            assert_eq!(
                sink.guids,
                vec![
                    (0, true, Some("AB".into()), None, 3),
                    (1, false, None, Some(0), 3)
                ]
            );
            assert_eq!(cache.net_guid_entries().len(), 2);
            assert_eq!(cache.get_path_by_guid(7), Some("AB"));
            assert_eq!(cache.get_path_by_guid(8), Some("AB"));
            assert_eq!(cache.get_outer_guid(8), Some(NetworkGuid(7)));
            assert!(cache.get_path_by_guid(9).is_none());
        }
    }

    #[test]
    fn path_budget_accepts_exact_boundary_and_resets_for_each_read() {
        let archive = build(&[(7, 0, Some("ABC"), 0), (8, 0, None, 0)], &[], &[0xa5]);
        for _ in 0..2 {
            let mut cache = NetGuidCache::new();
            let tables = read_checkpoint_tables_with_path_limit(
                &archive,
                &mut cache,
                &mut NoopCheckpointTableSink,
                RESOLVED,
                6,
            )
            .unwrap();
            assert_eq!(tables.frame_offset, archive.len() - 1);
            assert_eq!(tables.guid_count, 2);
            assert_eq!(tables.literal_paths, 1);
            assert_eq!(tables.resolved_path_indices, 1);
            assert_eq!(cache.get_path_by_guid(8), Some("ABC"));
        }
    }

    #[test]
    fn path_budget_counts_decoded_utf8_bytes() {
        let archive = build(&[(7, 0, Some("\u{754c}"), 0)], &[], &[]);
        let mut cache = NetGuidCache::new();
        let mut sink = RecordingSink::default();
        let error =
            read_checkpoint_tables_with_path_limit(&archive, &mut cache, &mut sink, RESOLVED, 2)
                .unwrap_err();
        assert!(matches!(
            error,
            CheckpointReadError::Schema(SchemaError::ResourceLimit {
                requested: 3,
                limit: 2,
                ..
            })
        ));
        assert!(sink.guids.is_empty());
        assert!(cache.net_guid_entries().is_empty());
    }

    #[test]
    fn path_budget_includes_legacy_decimal_paths() {
        let archive = build(&[(7, 0, Some("AB"), 0), (8, 0, None, 12345)], &[], &[]);
        for limit in [6, 7] {
            let mut cache = NetGuidCache::new();
            let mut sink = RecordingSink::default();
            let result = read_checkpoint_tables_with_path_limit(
                &archive,
                &mut cache,
                &mut sink,
                CheckpointPathMode::LegacyDecimal,
                limit,
            );
            if limit == 7 {
                assert!(result.is_ok());
                assert_eq!(cache.get_path_by_guid(8), Some("12345"));
                assert_eq!(sink.guids[1], (1, false, None, Some(12345), 3));
            } else {
                assert!(matches!(
                    result,
                    Err(CheckpointReadError::Schema(SchemaError::ResourceLimit {
                        requested: 7,
                        limit: 6,
                        ..
                    }))
                ));
                assert_eq!(sink.guids.len(), 1);
                assert!(cache.get_path_by_guid(8).is_none());
            }
        }
    }

    #[test]
    fn path_budget_preserves_truncation_and_index_error_precedence() {
        let mut truncated = build(&[(7, 0, Some("ABC"), 0)], &[], &[]);
        truncated.truncate(truncated.len() - 5); // remove the flags and group count
        let mut cache = NetGuidCache::new();
        let error = read_checkpoint_tables_with_path_limit(
            &truncated,
            &mut cache,
            &mut NoopCheckpointTableSink,
            RESOLVED,
            0,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            CheckpointReadError::Schema(SchemaError::Bitio(_))
        ));
        assert!(cache.net_guid_entries().is_empty());

        let invalid_index = build(&[(7, 0, None, 0)], &[], &[]);
        let error = read_checkpoint_tables_with_path_limit(
            &invalid_index,
            &mut cache,
            &mut NoopCheckpointTableSink,
            RESOLVED,
            0,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            CheckpointReadError::Schema(SchemaError::CheckpointPathIndexOutOfBounds {
                entry: 0,
                index: 0,
                literals: 0
            })
        ));
    }

    #[test]
    fn path_budget_preserves_earlier_sink_error() {
        let archive = build(&[(7, 0, Some("AB"), 0), (8, 0, None, 0)], &[], &[]);
        let mut cache = NetGuidCache::new();
        let mut sink = StopAfterFirstGuid(0);
        let error =
            read_checkpoint_tables_with_path_limit(&archive, &mut cache, &mut sink, RESOLVED, 2)
                .unwrap_err();
        assert!(matches!(error, CheckpointReadError::Sink("stop")));
        assert_eq!(sink.0, 1);
        assert!(cache.net_guid_entries().is_empty());
    }

    #[test]
    fn zero_path_budget_permits_empty_paths_but_not_a_byte() {
        for path in ["", "A"] {
            let archive = build(&[(7, 0, Some(path), 0)], &[], &[]);
            let mut cache = NetGuidCache::new();
            let result = read_checkpoint_tables_with_path_limit(
                &archive,
                &mut cache,
                &mut NoopCheckpointTableSink,
                RESOLVED,
                0,
            );
            if path.is_empty() {
                assert!(result.is_ok());
                assert_eq!(cache.get_path_by_guid(7), Some(""));
            } else {
                assert!(matches!(
                    result,
                    Err(CheckpointReadError::Schema(SchemaError::ResourceLimit {
                        requested: 1,
                        limit: 0,
                        ..
                    }))
                ));
                assert!(cache.net_guid_entries().is_empty());
            }
        }
    }

    #[test]
    fn resolved_default_preserves_raw_observers_and_explicit_legacy_mode() {
        let archive = build(
            &[
                (7, 0, Some("/First"), 0),
                (8, 7, Some("123"), 0),
                (9, 7, None, 0),
                (10, 8, None, 1),
            ],
            &[("/Script/G.Thing", 7, 2, &[(1, "Value", 0)])],
            &[0xa5],
        );
        let mut legacy_cache = NetGuidCache::new();
        let mut legacy_sink = RecordingSink::default();
        let legacy = read_checkpoint_tables_with_sink_mode(
            &archive,
            &mut legacy_cache,
            &mut legacy_sink,
            CheckpointPathMode::LegacyDecimal,
        )
        .unwrap();
        let mut cache = NetGuidCache::new();
        let mut sink = RecordingSink::default();
        let measured = read_checkpoint_tables_with_sink(&archive, &mut cache, &mut sink).unwrap();
        assert_eq!(sink.guids, legacy_sink.guids);
        assert_eq!(sink.groups, legacy_sink.groups);
        assert_eq!(sink.fields, legacy_sink.fields);
        assert_eq!(measured.frame_offset, archive.len() - 1);
        assert_eq!(measured.frame_offset, legacy.frame_offset);
        assert_eq!(measured.guid_count, legacy.guid_count);
        assert_eq!(measured.exported_fields, legacy.exported_fields);
        assert_eq!(measured.indexed_paths, 2);
        assert_eq!(measured.literal_paths, 2);
        assert_eq!(legacy.literal_paths, 2);
        assert_eq!(measured.resolved_path_indices, 2);
        assert_eq!(legacy.resolved_path_indices, 0);
        assert_eq!(legacy_cache.get_path_by_guid(9), Some("0"));
        assert_eq!(legacy_cache.get_path_by_guid(10), Some("1"));
        assert_eq!(cache.get_path_by_guid(9), Some("/First"));
        assert_eq!(cache.get_path_by_guid(10), Some("123"));
        assert_eq!(cache.get_outer_guid(10), Some(NetworkGuid(8)));
    }

    #[test]
    fn resolved_references_do_not_append_to_the_literal_table() {
        let archive = build(
            &[
                (7, 0, Some("A"), 0),
                (8, 0, None, 0),
                (9, 0, Some("B"), 0),
                (10, 0, None, 1),
            ],
            &[],
            &[],
        );
        let mut cache = NetGuidCache::new();
        let measured = read_checkpoint_tables_with_sink_mode(
            &archive,
            &mut cache,
            &mut NoopCheckpointTableSink,
            RESOLVED,
        )
        .unwrap();
        assert_eq!(cache.get_path_by_guid(10), Some("B"));
        assert_eq!(
            (measured.literal_paths, measured.resolved_path_indices),
            (2, 2)
        );
    }

    #[test]
    fn resolved_duplicate_literal_occurrences_each_append() {
        // No corpus duplicate was observed; this defines only the hypothesis.
        let archive = build(
            &[
                (7, 0, Some("A"), 0),
                (8, 0, Some("A"), 0),
                (9, 0, Some("B"), 0),
                (10, 0, None, 2),
            ],
            &[],
            &[],
        );
        let mut cache = NetGuidCache::new();
        let measured = read_checkpoint_tables_with_sink_mode(
            &archive,
            &mut cache,
            &mut NoopCheckpointTableSink,
            RESOLVED,
        )
        .unwrap();
        assert_eq!(cache.get_path_by_guid(10), Some("B"));
        assert_eq!(measured.literal_paths, 3);
    }

    #[test]
    fn resolved_table_resets_between_checkpoint_reads() {
        let first = build(&[(7, 0, Some("A"), 0)], &[], &[]);
        let second = build(&[(8, 0, None, 0)], &[], &[]);
        let mut cache = NetGuidCache::new();
        read_checkpoint_tables_with_sink_mode(
            &first,
            &mut cache,
            &mut NoopCheckpointTableSink,
            RESOLVED,
        )
        .unwrap();
        let error = read_checkpoint_tables_with_sink_mode(
            &second,
            &mut cache,
            &mut NoopCheckpointTableSink,
            RESOLVED,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            CheckpointReadError::Schema(SchemaError::CheckpointPathIndexOutOfBounds {
                entry: 0,
                index: 0,
                literals: 0
            })
        ));
        assert!(cache.get_path_by_guid(8).is_none());
    }

    #[test]
    fn resolved_forward_and_upper_bound_indices_are_rejected() {
        for (guids, entry, index, literals) in [
            (vec![(7, 0, None, 0), (8, 0, Some("Later"), 0)], 0, 0, 0),
            (vec![(7, 0, Some("A"), 0), (8, 0, None, 1)], 1, 1, 1),
            (
                vec![(7, 0, Some("A"), 0), (8, 0, None, u32::MAX)],
                1,
                u32::MAX,
                1,
            ),
        ] {
            let archive = build(&guids, &[], &[]);
            let error = read_checkpoint_tables_with_sink_mode(
                &archive,
                &mut NetGuidCache::new(),
                &mut NoopCheckpointTableSink,
                RESOLVED,
            )
            .unwrap_err();
            assert!(matches!(error, CheckpointReadError::Schema(
                SchemaError::CheckpointPathIndexOutOfBounds {
                    entry: e, index: i, literals: l
                }) if (e, i, l) == (entry, index, literals)));
        }
    }

    #[test]
    fn reads_both_tables_and_lands_on_the_frame() {
        let archive = build(
            &[(7, 0, Some("/Game/Maps/Ascent/Ascent"), 0), (5, 7, None, 0)],
            &[(
                "/Script/ShooterGame.Thing",
                7,
                4,
                &[(1, "Health", 0), (3, "Armor", 2)],
            )],
            &[0xAB; 32],
        );
        let mut cache = NetGuidCache::new();
        let t = read_checkpoint_tables(&archive, &mut cache).unwrap();

        assert_eq!(t.guid_count, 2);
        assert_eq!(t.group_count, 1);
        assert_eq!(t.exported_fields, 2);
        assert_eq!(t.indexed_paths, 1);
        assert_eq!(t.frame_offset, archive.len() - 32);
        assert_eq!(cache.get_path_by_guid(7), Some("/Game/Maps/Ascent/Ascent"));
        // The non-observer wrapper also uses the checkpoint-local path table.
        assert_eq!(cache.get_path_by_guid(5), Some("/Game/Maps/Ascent/Ascent"));
        assert_eq!(cache.get_outer_guid(5), Some(NetworkGuid(7)));
        let g = cache
            .get_group_by_path("/Script/ShooterGame.Thing")
            .unwrap();
        assert_eq!(g.get_field(1).map(|f| f.name.as_str()), Some("Health"));
        assert_eq!(g.get_field(3).map(|f| f.name.as_str()), Some("Armor_1"));
        assert!(g.get_field(0).is_none(), "unexported slot must stay empty");
    }

    /// The frame-offset check is the only guard against a misread count, so
    /// it has to be seen failing.
    #[test]
    fn a_desynced_table_is_rejected_not_silently_accepted() {
        let mut archive = build(
            &[(7, 0, Some("/Game/X"), 0)],
            &[("/Script/G.Thing", 7, 2, &[(0, "A", 0)])],
            &[0u8; 16],
        );
        // Move the declared frame offset one byte on: the tables still parse,
        // and only the end-to-end check can tell.
        let w0 = u32::from_le_bytes(archive[0..4].try_into().unwrap());
        archive[0..4].copy_from_slice(&(w0 + 1).to_le_bytes());
        let mut cache = NetGuidCache::new();
        let err = read_checkpoint_tables(&archive, &mut cache).unwrap_err();
        assert!(
            matches!(err, SchemaError::CheckpointFrameOffsetMismatch { .. }),
            "expected a frame-offset mismatch, got {err}"
        );
    }

    /// Within one checkpoint (a fresh cache), two paths at one index are a
    /// collision, not the re-export `add_export_group` merges; so is one path,
    /// or an alias spelling of it, declared again at another index. The
    /// rejected group never reaches the sink.
    #[test]
    fn two_groups_at_one_index_fail_before_returning_an_untrusted_cache() {
        let archive = build(
            &[],
            &[("/Script/G.A", 7, 0, &[]), ("/Script/G.B", 7, 0, &[])],
            &[0u8; 8],
        );
        let mut sink = RecordingSink::default();
        let err = read_checkpoint_tables_with_sink(&archive, &mut NetGuidCache::new(), &mut sink);

        assert!(matches!(
            err,
            Err(CheckpointReadError::Schema(
                SchemaError::CheckpointGroupCollision {
                    path_name_index: 7,
                    ..
                }
            ))
        ));
        assert_eq!(sink.groups.len(), 1, "the rejected group reached the sink");

        for (first, second) in [
            ("/Script/G.A", "/Script/G.A"),
            (
                "/Game/Characters/Jett/Jett_C",
                "/Game/Characters/_Core/Jett/Jett_C",
            ),
        ] {
            let archive = build(&[], &[(first, 7, 0, &[]), (second, 8, 0, &[])], &[]);
            match read_checkpoint_tables(&archive, &mut NetGuidCache::new()) {
                Err(SchemaError::CheckpointGroupCollision {
                    path,
                    path_name_index,
                }) => assert_eq!(
                    (path.as_str(), path_name_index),
                    (second, 8),
                    "{first} then {second}"
                ),
                other => panic!("{first} then {second}: expected a collision, got {other:?}"),
            }
        }
    }

    /// The control for the collision test: two groups at different indices.
    #[test]
    fn two_groups_at_different_indices_are_not_a_collision() {
        let archive = build(
            &[],
            &[("/Script/G.A", 7, 0, &[]), ("/Script/G.B", 8, 0, &[])],
            &[],
        );
        let mut cache = NetGuidCache::new();
        let t = read_checkpoint_tables(&archive, &mut cache).unwrap();
        assert_eq!(t.group_count, 2);
        assert_eq!(cache.group_count(), 2);
    }

    #[test]
    fn a_third_path_discriminator_is_an_error() {
        let mut archive = build(&[(7, 0, Some("/Game/X"), 0)], &[], &[]);
        archive[22] = 2; // the PathIsString byte of entry 0
        let mut cache = NetGuidCache::new();
        let err = read_checkpoint_tables(&archive, &mut cache).unwrap_err();
        assert!(
            matches!(
                err,
                SchemaError::CheckpointBadPathKind { entry: 0, byte: 2 }
            ),
            "got {err}"
        );
    }

    #[test]
    fn a_nonzero_reserved_word_is_an_error() {
        let mut archive = build(&[], &[], &[]);
        archive[8..12].copy_from_slice(&7u32.to_le_bytes());
        let mut cache = NetGuidCache::new();
        let err = read_checkpoint_tables(&archive, &mut cache).unwrap_err();
        assert!(
            matches!(
                err,
                SchemaError::CheckpointReservedWordSet {
                    offset: 8,
                    value: 7
                }
            ),
            "got {err}"
        );
    }

    #[test]
    fn a_handle_that_is_not_its_slot_is_an_error() {
        // One exported slot that lies about its handle: a real name on the
        // wrong handle would read as valid data.
        let mut body = 1u32.to_le_bytes().to_vec(); // one group
        add_fstring_utf16(&mut body, "/Script/G.Thing");
        add_int_packed(&mut body, 7);
        add_int_packed(&mut body, 2); // two slots
        body.push(0); // slot 0 not exported
        body.push(1); // slot 1 exported
        add_int_packed(&mut body, 9); // ... but claims handle 9
        add_u32(&mut body, 0);
        body.push(1);
        add_int_packed(&mut body, 216);

        let mut cache = NetGuidCache::new();
        let err = read_checkpoint_tables(&archive(0, &body, &[]), &mut cache).unwrap_err();
        assert!(
            matches!(
                err,
                SchemaError::CheckpointHandleNotSlot {
                    slot: 1,
                    handle: 9,
                    ..
                }
            ),
            "got {err}"
        );
    }
}
