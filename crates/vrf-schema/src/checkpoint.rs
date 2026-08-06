//! The two schema tables a Checkpoint chunk carries ahead of its DemoFrame.
//!
//! A checkpoint archive is self-contained: it restates the server's NetGUID
//! cache and its whole net-field export map, then a single DemoFrame. Nothing
//! in it references the ReplayData stream, so it is read into its own
//! [`NetGuidCache`] rather than merged into the live one -- the frame that
//! follows re-opens every actor alive at that instant, and replaying those
//! channel opens through the live reader would corrupt the running channel
//! table.
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
//! All reads are byte-aligned `FBinaryArchive` semantics, as in the DemoFrame
//! grammar itself.
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
//! **The polarity is the opposite of an FName's.** In [`read_fname`] a leading
//! `1` means "hardcoded index"; here a leading `1` means "a string follows".
//! They are different fields and the FName reader must not be pointed at this
//! one.
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
//! The two counts in this section use different encodings, and that is the
//! detail that defeats a first implementation: reading `NumNetFieldExports` as
//! a `u32` yields exactly twice the true value for small counts, because
//! `IntPacked` shifts left by one. The cursor then overruns into the next
//! record and produces plausible garbage rather than an error, which is why
//! [`read_checkpoint_tables`] ends by asserting the prologue's frame offset.

use vrf_bitio::BitReader;

use crate::cache::NetGuidCache;
use crate::error::{Result, SchemaError};
use crate::export::{NetFieldExport, NetFieldExportGroup, render_fname};
use crate::guid::NetworkGuid;

/// Maximum string size for a checkpoint path or name.
const MAX_FSTRING_BYTES: i64 = 1024 * 1024;

/// Sanity bound on the guid-cache entry count. The largest corpus checkpoint
/// carries roughly 12,000; a million would mean a mis-read length.
const MAX_GUID_ENTRIES: u32 = 1_000_000;

/// Sanity bound on the export-group count. The largest corpus checkpoint
/// declares 543.
const MAX_GROUPS: u32 = 100_000;

/// Sanity bound on a single group's declared field-slot count.
const MAX_FIELDS_PER_GROUP: u32 = 65_536;

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
    /// Entries whose path arrived as a hardcoded name-table index rather than
    /// a string. 24.3% of the corpus table; see [`read_checkpoint_tables`].
    pub hardcoded_paths: u32,
    /// Export groups that collided with one already declared in this same
    /// checkpoint -- same `path_name_index`, or a path (or path alias) already
    /// registered.
    ///
    /// A checkpoint restates the entire export map once into a fresh cache, so
    /// this is not the incremental re-export that
    /// [`NetGuidCache::add_export_group`] merges: it is two declarations
    /// claiming one slot. The merge happens anyway -- it is the same cache the
    /// ReplayData stream needs it for -- which means the second group's fields
    /// land in the first group's vector, the second path resolves to the first
    /// group, and `/A`'s populated slots can be overwritten by `/B`'s. All of
    /// that used to happen behind `Ok`, `group_count = 2` and a frame offset
    /// that checks out, because the offset check only proves the cursor stayed
    /// aligned, not that the map means anything.
    ///
    /// Expected to be zero. Non-zero means the group map cannot be trusted for
    /// that checkpoint, and the counter is the only thing that will say so.
    pub group_collisions: u32,
}

/// Read a checkpoint archive's guid cache and export-group map into `cache`,
/// and report where its DemoFrame begins.
///
/// `data` is the decompressed archive from
/// `vrf_container::decompress_checkpoint`. `cache` should be **fresh**: a
/// checkpoint restates the whole schema, and merging it into the live
/// ReplayData cache mixes two independent `path_name_index` numberings.
///
/// # Hardcoded paths
///
/// A quarter of guid entries carry a name-table index instead of a path
/// string. The table is not in the replay, so the text is unrecoverable from
/// the file alone. The index is registered as its decimal rendering, which is
/// exactly what [`read_fname`](crate::read_net_field_exports) already does for
/// hardcoded field names -- consistency matters more than prettiness, and the
/// alternative, dropping the entry, would lose the outer-GUID chain for a
/// quarter of the table.
///
/// # Errors
///
/// Beyond truncation: a path discriminator outside `{0, 1}`, a slot whose
/// declared handle is not its own index, a non-zero reserved prologue word,
/// and a table parse that does not finish exactly where the prologue says the
/// frame begins. Each is a check that the cursor is still aligned; without
/// them a mis-read count yields well-formed nonsense.
pub fn read_checkpoint_tables(data: &[u8], cache: &mut NetGuidCache) -> Result<CheckpointTables> {
    let mut reader = BitReader::new(data);

    // -- Prologue ----------------------------------------------------------
    let frame_offset_word = reader.read_u32()?;
    for offset in [4usize, 8, 12] {
        let value = reader.read_u32()?;
        if value != 0 {
            return Err(SchemaError::CheckpointReservedWordSet { offset, value });
        }
    }
    let guid_count = reader.read_u32()?;
    if guid_count > MAX_GUID_ENTRIES {
        return Err(SchemaError::CheckpointCountOverflow {
            field: "guid entries",
            count: guid_count,
            max: MAX_GUID_ENTRIES,
        });
    }

    // -- GUID cache --------------------------------------------------------
    let mut hardcoded_paths = 0u32;
    for entry in 0..guid_count {
        let net_guid = reader.read_int_packed()?;
        let outer_guid = reader.read_int_packed()?;
        let path_is_string = reader.read_u8()?;
        let path = match path_is_string {
            1 => reader.read_fstring(MAX_FSTRING_BYTES)?,
            0 => {
                hardcoded_paths += 1;
                reader.read_int_packed()?.to_string()
            }
            byte => return Err(SchemaError::CheckpointBadPathKind { entry, byte }),
        };
        // Flags: inferred to be UE's bNoLoad | bIgnoreWhenMissing. Only the
        // two-value distribution is measured, so it is consumed, not judged.
        let _flags = reader.read_u8()?;

        cache.set_net_guid_path(net_guid, path, Some(NetworkGuid(outer_guid)));
    }

    // -- Export group map --------------------------------------------------
    let group_count = reader.read_u32()?;
    if group_count > MAX_GROUPS {
        return Err(SchemaError::CheckpointCountOverflow {
            field: "export groups",
            count: group_count,
            max: MAX_GROUPS,
        });
    }

    let mut exported_fields = 0u32;
    let mut group_collisions = 0u32;
    for _ in 0..group_count {
        let path = reader.read_fstring(MAX_FSTRING_BYTES)?;
        let path_name_index = reader.read_int_packed()?;
        // IntPacked, not u32. See the module docs.
        let declared = reader.read_int_packed()?;
        if declared > MAX_FIELDS_PER_GROUP {
            return Err(SchemaError::CheckpointCountOverflow {
                field: "fields in a group",
                count: declared,
                max: MAX_FIELDS_PER_GROUP,
            });
        }

        // Tested before the add, and with exactly the two lookups
        // `add_export_group` merges on: `by_path` (which includes the path
        // aliases it registers) and `by_index`. The cache is fresh per
        // checkpoint, so a hit here means this checkpoint declared the slot
        // twice, not that a previous frame did. See
        // [`CheckpointTables::group_collisions`] for what the merge then does.
        if cache.get_group_by_index(path_name_index).is_some()
            || cache.get_group_by_path(&path).is_some()
        {
            group_collisions += 1;
        }

        cache.add_export_group(NetFieldExportGroup::new(
            path.clone(),
            path_name_index,
            declared,
        ));

        for slot in 0..declared {
            if reader.read_u8()? == 0 {
                continue;
            }
            let handle = reader.read_int_packed()?;
            if handle != slot {
                return Err(SchemaError::CheckpointHandleNotSlot {
                    group: path,
                    slot,
                    handle,
                });
            }
            let compatible_checksum = reader.read_u32()?;
            let name = read_fname(&mut reader)?;
            cache.set_field_on_group(
                path_name_index,
                NetFieldExport {
                    handle,
                    compatible_checksum,
                    name,
                },
            );
            exported_fields += 1;
        }
    }

    // -- The one end-to-end check -----------------------------------------
    let map_end = (reader.position() / 8) as usize;
    let expected = frame_offset_word as usize + 8;
    if map_end != expected {
        return Err(SchemaError::CheckpointFrameOffsetMismatch { map_end, expected });
    }

    Ok(CheckpointTables {
        guid_count,
        group_count,
        exported_fields,
        frame_offset: map_end,
        hardcoded_paths,
        group_collisions,
    })
}

/// Read an FName from a byte-aligned archive.
///
/// Duplicated from `reader.rs` rather than shared because the two callers read
/// different fields that merely look alike: this one's leading byte is
/// `bHardcoded`, while a guid entry's is `PathIsString` with the opposite
/// meaning. Keeping them apart is what stops the wrong one being reused.
///
/// The *rendering* is shared, though -- see [`render_fname`]. The two readers
/// may not be merged, but a name must mean the same thing whichever one
/// produced it, and this one used to drop the instance number just as the other
/// did.
fn read_fname(reader: &mut BitReader<'_>) -> Result<String> {
    if reader.read_u8()? != 0 {
        Ok(reader.read_int_packed()?.to_string())
    } else {
        let name = reader.read_fstring(MAX_FSTRING_BYTES)?;
        let number = reader.read_i32()?;
        Ok(render_fname(name, number))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(net_guid, outer_guid, path or None for a hardcoded name, name index)`.
    type GuidSpec<'a> = (u32, u32, Option<&'a str>, u32);
    /// `(group path, declared slot count, exported (handle, name) pairs)`.
    type GroupSpec<'a> = (&'a str, u32, &'a [(u32, &'a str)]);

    /// Build an archive: prologue, guid entries, group map, then `frame`.
    fn build(guids: &[GuidSpec<'_>], groups: &[GroupSpec<'_>], frame: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        for (guid, outer, path, name_index) in guids {
            push_packed(&mut body, *guid);
            push_packed(&mut body, *outer);
            match path {
                Some(p) => {
                    body.push(1);
                    push_fstring(&mut body, p);
                }
                None => {
                    body.push(0);
                    push_packed(&mut body, *name_index);
                }
            }
            body.push(0x03);
        }
        body.extend_from_slice(&(groups.len() as u32).to_le_bytes());
        for (path, declared, fields) in groups {
            push_fstring(&mut body, path);
            push_packed(&mut body, 7);
            push_packed(&mut body, *declared);
            for slot in 0..*declared {
                match fields.iter().find(|(h, _)| *h == slot) {
                    Some((h, name)) => {
                        body.push(1);
                        push_packed(&mut body, *h);
                        body.extend_from_slice(&0xdead_beefu32.to_le_bytes());
                        body.push(0); // FName: not hardcoded
                        push_fstring(&mut body, name);
                        body.extend_from_slice(&0i32.to_le_bytes());
                    }
                    None => body.push(0),
                }
            }
        }
        let mut out = Vec::new();
        // The frame offset is measured from byte 8, so it is (20 + body) - 8.
        out.extend_from_slice(&((20 + body.len() - 8) as u32).to_le_bytes());
        out.extend_from_slice(&[0u8; 12]);
        out.extend_from_slice(&(guids.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(frame);
        out
    }

    fn push_packed(out: &mut Vec<u8>, mut value: u32) {
        loop {
            let mut b = ((value & 0x7F) << 1) as u8;
            value >>= 7;
            if value != 0 {
                b |= 1;
            }
            out.push(b);
            if value == 0 {
                return;
            }
        }
    }

    /// UTF-16LE with a negative length, which is how every corpus string
    /// arrives.
    fn push_fstring(out: &mut Vec<u8>, s: &str) {
        let units: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
        out.extend_from_slice(&(-(units.len() as i32)).to_le_bytes());
        for u in units {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }

    #[test]
    fn reads_both_tables_and_lands_on_the_frame() {
        let archive = build(
            &[
                (7, 0, Some("/Game/Maps/Ascent/Ascent"), 0),
                (5, 7, None, 216),
            ],
            &[(
                "/Script/ShooterGame.Thing",
                4,
                &[(1, "Health"), (3, "Armor")],
            )],
            &[0xAB; 32],
        );
        let mut cache = NetGuidCache::new();
        let t = read_checkpoint_tables(&archive, &mut cache).unwrap();

        assert_eq!(t.guid_count, 2);
        assert_eq!(t.group_count, 1);
        assert_eq!(t.exported_fields, 2);
        assert_eq!(t.hardcoded_paths, 1);
        assert_eq!(t.frame_offset, archive.len() - 32);
        assert_eq!(cache.get_path_by_guid(7), Some("/Game/Maps/Ascent/Ascent"));
        // A hardcoded path is registered as its decimal index, not dropped.
        assert_eq!(cache.get_path_by_guid(5), Some("216"));
        assert_eq!(cache.get_outer_guid(5), Some(NetworkGuid(7)));
        let g = cache
            .get_group_by_path("/Script/ShooterGame.Thing")
            .unwrap();
        assert_eq!(g.get_field(1).map(|f| f.name.as_str()), Some("Health"));
        assert_eq!(g.get_field(3).map(|f| f.name.as_str()), Some("Armor"));
        assert!(g.get_field(0).is_none(), "unexported slot must stay empty");
    }

    /// The trap that defeats a first implementation: reading the per-group
    /// count as a u32 doubles it and overruns. The frame-offset check is the
    /// only thing standing between that and well-formed nonsense, so it has to
    /// be seen failing.
    #[test]
    fn a_desynced_table_is_rejected_not_silently_accepted() {
        let mut archive = build(
            &[(7, 0, Some("/Game/X"), 0)],
            &[("/Script/G.Thing", 2, &[(0, "A")])],
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

    /// The checkpoint reader discarded the FName number exactly as the
    /// ReplayData reader did, so two slots whose base string matches came out
    /// with one name. Both sites now render the same way: number 0 is the bare
    /// name, and any other number is the base plus `_{number - 1}`, which is
    /// how Unreal displays it.
    #[test]
    fn fname_numbers_survive_into_the_checkpoint_field_names() {
        // Hand-built: the shared `build` helper always writes number 0.
        let mut body = Vec::new();
        body.extend_from_slice(&1u32.to_le_bytes()); // one group
        push_fstring(&mut body, "/Script/G.Thing");
        push_packed(&mut body, 7);
        push_packed(&mut body, 2); // two slots
        for (slot, number) in [(0u32, 0i32), (1, 1)] {
            body.push(1); // exported
            push_packed(&mut body, slot);
            body.extend_from_slice(&0u32.to_le_bytes()); // checksum
            body.push(0); // FName: not hardcoded
            push_fstring(&mut body, "Value");
            body.extend_from_slice(&number.to_le_bytes());
        }

        let mut archive = Vec::new();
        archive.extend_from_slice(&((20 + body.len() - 8) as u32).to_le_bytes());
        archive.extend_from_slice(&[0u8; 12]);
        archive.extend_from_slice(&0u32.to_le_bytes()); // no guid entries
        archive.extend_from_slice(&body);

        let mut cache = NetGuidCache::new();
        read_checkpoint_tables(&archive, &mut cache).unwrap();

        let g = cache.get_group_by_path("/Script/G.Thing").unwrap();
        assert_eq!(g.get_field(0).map(|f| f.name.as_str()), Some("Value"));
        assert_eq!(g.get_field(1).map(|f| f.name.as_str()), Some("Value_0"));
    }

    /// A checkpoint restates the whole export map into a fresh cache, so two
    /// groups sharing a `path_name_index` inside ONE checkpoint is not the
    /// incremental re-export that [`NetGuidCache::add_export_group`] merges --
    /// it is two different paths claiming one slot.
    ///
    /// The merge is left in place: it is correct for the ReplayData stream that
    /// shares the cache, and changing it here would change what every corpus
    /// checkpoint resolves to on evidence this crate cannot see. What the
    /// collision does is pinned below, and counted, so a run says so instead of
    /// returning `Ok` with `group_count = 2` and a frame offset that checks out.
    #[test]
    fn two_groups_at_one_index_are_counted_as_a_collision() {
        // `build` writes path_name_index 7 for every group, so two groups is
        // exactly the collision.
        let archive = build(
            &[],
            &[("/Script/G.A", 0, &[]), ("/Script/G.B", 0, &[])],
            &[0u8; 8],
        );
        let mut cache = NetGuidCache::new();
        let t = read_checkpoint_tables(&archive, &mut cache).unwrap();

        assert_eq!(t.group_count, 2, "the wire declared two groups");
        assert_eq!(t.group_collisions, 1);
        // The consequence the counter names: /B resolves to the group /A made.
        assert_eq!(
            cache
                .get_group_by_path("/Script/G.B")
                .map(|g| g.path.as_str()),
            Some("/Script/G.A"),
            "the second group was merged into the first, so its path is a lie"
        );
    }

    /// Two groups at different indices are the ordinary case and must not be
    /// counted.
    #[test]
    fn two_groups_at_different_indices_are_not_a_collision() {
        let mut body = Vec::new();
        body.extend_from_slice(&2u32.to_le_bytes()); // two groups
        for (path, index) in [("/Script/G.A", 7u32), ("/Script/G.B", 8)] {
            push_fstring(&mut body, path);
            push_packed(&mut body, index);
            push_packed(&mut body, 0); // no field slots
        }

        let mut archive = Vec::new();
        archive.extend_from_slice(&((20 + body.len() - 8) as u32).to_le_bytes());
        archive.extend_from_slice(&[0u8; 12]);
        archive.extend_from_slice(&0u32.to_le_bytes()); // no guid entries
        archive.extend_from_slice(&body);

        let mut cache = NetGuidCache::new();
        let t = read_checkpoint_tables(&archive, &mut cache).unwrap();
        assert_eq!(t.group_count, 2);
        assert_eq!(t.group_collisions, 0);
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
        // Hand-build one group whose single exported slot lies about its
        // handle. Attaching a real name to the wrong handle is the failure
        // mode this check exists for, and it reads as valid data.
        let mut body = Vec::new();
        body.extend_from_slice(&1u32.to_le_bytes()); // one group
        push_fstring(&mut body, "/Script/G.Thing");
        push_packed(&mut body, 7);
        push_packed(&mut body, 2); // two slots
        body.push(0); // slot 0 not exported
        body.push(1); // slot 1 exported
        push_packed(&mut body, 9); // ... but claims handle 9
        body.extend_from_slice(&0u32.to_le_bytes());
        body.push(1);
        push_packed(&mut body, 216);

        let mut archive = Vec::new();
        archive.extend_from_slice(&((20 + body.len() - 8) as u32).to_le_bytes());
        archive.extend_from_slice(&[0u8; 12]);
        archive.extend_from_slice(&0u32.to_le_bytes()); // no guid entries
        archive.extend_from_slice(&body);

        let mut cache = NetGuidCache::new();
        let err = read_checkpoint_tables(&archive, &mut cache).unwrap_err();
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
