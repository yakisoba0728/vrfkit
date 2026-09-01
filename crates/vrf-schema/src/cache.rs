//! The replay-wide accumulator for all dynamically-received schema state.
//!
//! [`NetGuidCache`] is the central authority for:
//!
//! - **path -> group**: find an export group by its full path string.
//! - **path_name_index -> group**: find an export group by the numeric index the
//!   engine assigns once and reuses for the rest of the replay.
//! - **NetGUID -> object path**: map the 32-bit runtime object ID to its
//!   human-readable path (populated from export-GUID bunches).
//! - **NetGUID -> outer NetGUID**: track the containment hierarchy so callers can
//!   walk from a component to its owning actor.
//! - **GameplayTag index -> name**: the `NetworkGameplayTagNodeIndex` group's
//!   fields double as a tag name table.
//!
//! This state accumulates over the entire replay and is never reset.
//!
//! The bare-name resolvers that sit on top of the leaf index
//! ([`NetGuidCache::unique_leaf_match`] and
//! [`NetGuidCache::resolve_cnc_for_instance_name`]) live in
//! [`crate::resolve`]; this module is the storage and the direct lookups.
//!
//! # Hashing
//!
//! Every map here uses [`FxHashMap`] rather than the standard hasher. See
//! [`crate::hash`] for the measurement and the security trade that motivates it.

use crate::error::{Result, SchemaError};
use crate::export::{NetFieldExport, NetFieldExportGroup};
use crate::guid::{NetGuidEntry, NetworkGuid};
use crate::hash::FxHashMap;
use crate::path::for_each_replay_path_key;
use crate::resolve::register_leaf;

/// The path used for the gameplay-tag name table group.
const GAMEPLAY_TAG_GROUP_PATH: &str = "NetworkGameplayTagNodeIndex";

/// Replay-wide schema accumulator.
///
/// All lookups are O(1) via `HashMap`. Field access within a group is O(1) via
/// direct `Vec` indexing (see [`NetFieldExportGroup::get_field`]).
pub struct NetGuidCache {
    /// path (String, ordinal) -> group index into `groups`.
    by_path: FxHashMap<String, usize>,
    /// path_name_index (u32) -> group index into `groups`.
    by_index: FxHashMap<u32, usize>,
    /// leaf name -> group index. Used by
    /// [`unique_leaf_match`](NetGuidCache::unique_leaf_match) to resolve bare
    /// class names (e.g. `AresAttributeSet`) to their full export group path
    /// (e.g. `/Script/ShooterGame.AresAttributeSet`).
    ///
    /// Mirrors the C# `ContentBlockPathResolver.UniqueLeafMatch` logic: a bare
    /// name is only resolved if exactly ONE group has a path ending with
    /// `.{name}`. Ambiguous names (multiple groups sharing the same leaf) are
    /// stored as `usize::MAX` to signal rejection.
    by_leaf: FxHashMap<String, usize>,
    /// Central storage for all groups.
    groups: Vec<NetFieldExportGroup>,
    /// NetGUID value -> object path string.
    guid_to_path: FxHashMap<u32, String>,
    /// NetGUID value -> outer NetGUID value (containment hierarchy).
    guid_to_outer: FxHashMap<u32, NetworkGuid>,
    /// Bumped whenever the set of group paths changes. See
    /// [`Self::schema_generation`].
    schema_generation: u64,
    /// Bumped whenever `guid_to_path` or `guid_to_outer` changes. See
    /// [`Self::guid_generation`].
    guid_generation: u64,
    /// Net-field exports [`Self::set_field_on_group`] could not place --
    /// either the group was not found or, the only way that can happen from
    /// [`crate::reader::read_net_field_exports`] (which validates the group
    /// exists before this is called), the handle exceeded the group's
    /// declared length. The C# reference logs a warning and moves on; this
    /// crate had no equivalent, so a field dropped this way vanished with no
    /// counter. See [`Self::dropped_field_exports`].
    ///
    /// Scoped to whichever `NetGuidCache` this is: the checkpoint pass builds
    /// a fresh one per chunk (`driver::checkpoints::process_chunk`) and drops
    /// it after, so a drop there never reaches the ReplayData-pass cache this
    /// counter is read from in the manifest.
    dropped_field_exports: u64,
}

impl NetGuidCache {
    /// Sentinel value in `by_leaf` indicating an ambiguous leaf (multiple
    /// groups share the same trailing class name).
    pub(crate) const AMBIGUOUS_LEAF: usize = usize::MAX;

    /// Create an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            by_path: FxHashMap::default(),
            by_index: FxHashMap::default(),
            by_leaf: FxHashMap::default(),
            groups: Vec::new(),
            guid_to_path: FxHashMap::default(),
            guid_to_outer: FxHashMap::default(),
            schema_generation: 0,
            guid_generation: 0,
            dropped_field_exports: 0,
        }
    }

    /// The leaf index, for the resolvers in [`crate::resolve`].
    pub(crate) fn leaf_index(&self) -> &FxHashMap<String, usize> {
        &self.by_leaf
    }

    /// A counter that changes whenever the set of group paths changes.
    ///
    /// Callers that memoise a *pure function of the group paths* -- "which group
    /// does this path resolve to", "is this function name unique across all
    /// groups" -- can stamp the memo with this value and discard it when the
    /// value moves. It is bumped by every [`Self::add_export_group`] and by
    /// [`Self::clear`], which are the only operations that add a path, add a
    /// path alias, or remove one.
    ///
    /// It deliberately does NOT track field mutations
    /// ([`Self::set_field_on_group`]): a memo of field contents would be unsound
    /// to key on this. Only path-set queries may use it. A successful
    /// `add_export_group` always bumps the generation because a supported merge
    /// can replace canonical path metadata and its aliases.
    #[must_use]
    pub fn schema_generation(&self) -> u64 {
        self.schema_generation
    }

    /// Register a new export group or merge it with an existing one.
    ///
    /// If exactly one coordinate already exists, that canonical group adopts
    /// both incoming coordinates and merges the fields. If the path and index
    /// identify different groups, the update fails without mutating the cache.
    ///
    /// Returns the index of the canonical group.
    pub fn add_export_group(&mut self, group: NetFieldExportGroup) -> Result<usize> {
        let existing_by_path = self.by_path.get(&group.path).copied();
        let existing_by_index = self.by_index.get(&group.path_name_index).copied();

        if let (Some(path_idx), Some(index_idx)) = (existing_by_path, existing_by_index) {
            if path_idx != index_idx {
                return Err(SchemaError::CrossedExportGroupIdentity {
                    path: group.path,
                    path_name_index: group.path_name_index,
                    path_group: self.groups[path_idx].path.clone(),
                    index_group: self.groups[index_idx].path.clone(),
                });
            }
        }

        let idx = if let Some(idx) = existing_by_path {
            // Same path: this is the same class re-declaring (or extending)
            // its own export group, so its previously-set handle slots are
            // still valid and are preserved.
            self.groups[idx].merge_from(&group);
            self.groups[idx].path = group.path;
            self.groups[idx].path_name_index = group.path_name_index;
            idx
        } else if let Some(idx) = existing_by_index {
            // Index matched, path did not: `path_name_index` was reused for a
            // path this cache has never seen, which the module doc says the
            // engine does over a replay's lifetime as old FNames are freed and
            // reassigned. The group at `idx` belongs to whatever class held
            // that index before -- merging would let its handle slots (field
            // names, `compatible_checksum`) survive into the new class, so a
            // content block resolved against the new path would read the OLD
            // class's field name/type at a handle the NEW class never
            // declared. Replace the group outright instead of merging into
            // it.
            self.groups[idx] = group;
            idx
        } else {
            let idx = self.groups.len();
            self.groups.push(group);
            idx
        };

        self.rebuild_group_indexes();
        self.schema_generation = self.schema_generation.wrapping_add(1);
        Ok(idx)
    }

    /// Rebuild every group-derived lookup after a canonical identity changes.
    fn rebuild_group_indexes(&mut self) {
        self.by_path.clear();
        self.by_index.clear();
        self.by_leaf.clear();
        for (idx, group) in self.groups.iter().enumerate() {
            for_each_replay_path_key(&group.path, |alias| {
                self.by_path.insert(alias.to_owned(), idx);
            });
            self.by_index.insert(group.path_name_index, idx);
            register_leaf(&mut self.by_leaf, &group.path, idx);
        }
    }

    /// Look up a group by its `path_name_index`.
    #[must_use]
    pub fn get_group_by_index(&self, path_name_index: u32) -> Option<&NetFieldExportGroup> {
        self.by_index
            .get(&path_name_index)
            .map(|&i| &self.groups[i])
    }

    /// Get a mutable reference to a group by its `path_name_index`.
    #[must_use]
    pub fn get_group_by_index_mut(
        &mut self,
        path_name_index: u32,
    ) -> Option<&mut NetFieldExportGroup> {
        self.by_index
            .get(&path_name_index)
            .copied()
            .map(move |i| &mut self.groups[i])
    }

    /// Look up a group by its full path (ordinal, case-sensitive).
    ///
    /// The hottest lookup in the crate: the sink probes it 2,989,695 times over
    /// the reference replay's export, once per candidate key per content block.
    #[must_use]
    pub fn get_group_by_path(&self, path: &str) -> Option<&NetFieldExportGroup> {
        self.by_path.get(path).map(|&i| &self.groups[i])
    }

    /// A counter that changes whenever a NetGUID -> path or NetGUID -> outer
    /// mapping changes.
    ///
    /// `set_net_guid_path` is called both through
    /// [`crate::reader::read_export_guids`] (frame-level ExportData, run once
    /// per frame ahead of that frame's packet loop) and through per-block
    /// export-GUID bunches during packet processing; neither caller routes
    /// through the same object that owns a group-path resolution memo, so a
    /// memo built from those resolutions has no other way to see this map
    /// change. Callers that memoise a function of `guid_to_path` /
    /// `guid_to_outer` must stamp with this and discard on a mismatch, the
    /// same pattern as [`Self::schema_generation`].
    #[must_use]
    pub fn guid_generation(&self) -> u64 {
        self.guid_generation
    }

    /// Register a NetGUID -> path mapping (from export GUID bunches).
    ///
    /// A no-op write -- the same path and outer this GUID already has -- does
    /// not bump [`Self::guid_generation`]. This isn't only about the redundant
    /// hashmap writes: [`crate::checkpoint`] reads a fresh `NetGuidCache` per
    /// checkpoint, but the frame-level ExportData section
    /// ([`crate::reader::read_export_guids`]) calls this once per exported
    /// GUID on *every* frame that re-declares one, with no pre-check of its
    /// own (unlike `vrfkit`'s `register_path`, which skips the call entirely
    /// when nothing changed, for its own reason -- an allocation, not this
    /// one). Without the check here, a replay that keeps re-sending a GUID's
    /// path bumps `guid_generation` every such frame, which -- now that a
    /// group-path resolution memo keys on this generation -- would collapse
    /// the memo's hit rate to near zero on exactly that traffic.
    pub fn set_net_guid_path(&mut self, net_guid: u32, path: String, outer: Option<NetworkGuid>) {
        let outer = outer.filter(|g| g.is_valid());
        if self.guid_to_path.get(&net_guid).map(String::as_str) == Some(path.as_str())
            && self.guid_to_outer.get(&net_guid).copied() == outer
        {
            return;
        }
        self.guid_to_path.insert(net_guid, path);
        match outer {
            Some(g) => {
                self.guid_to_outer.insert(net_guid, g);
            }
            None => {
                self.guid_to_outer.remove(&net_guid);
            }
        }
        self.guid_generation = self.guid_generation.wrapping_add(1);
    }

    /// Resolve a NetGUID to its object path.
    #[must_use]
    pub fn get_path_by_guid(&self, net_guid: u32) -> Option<&str> {
        self.guid_to_path.get(&net_guid).map(String::as_str)
    }

    /// Get the outer (containing) NetGUID for a given NetGUID.
    #[must_use]
    pub fn get_outer_guid(&self, net_guid: u32) -> Option<NetworkGuid> {
        self.guid_to_outer.get(&net_guid).copied()
    }

    /// Every registered NetGUID with its path and outer GUID.
    ///
    /// Order is unspecified (backed by a `HashMap`); sort if determinism
    /// matters.
    #[must_use]
    pub fn net_guid_entries(&self) -> Vec<NetGuidEntry<'_>> {
        self.guid_to_path
            .iter()
            .map(|(&net_guid, path)| NetGuidEntry {
                net_guid,
                path: path.as_str(),
                outer_net_guid: self.guid_to_outer.get(&net_guid).map(|o| o.0),
            })
            .collect()
    }

    /// Walk the outer chain to resolve the outer object's path.
    #[must_use]
    pub fn get_outer_path(&self, net_guid: u32) -> Option<&str> {
        let outer = self.get_outer_guid(net_guid)?;
        self.get_path_by_guid(outer.0)
    }

    /// Look up a gameplay-tag name by its network index.
    ///
    /// Tags are stored in the `NetworkGameplayTagNodeIndex` export group, where
    /// each field's handle is the tag index and its name is the tag string.
    #[must_use]
    pub fn get_gameplay_tag_name(&self, tag_index: u32) -> Option<&str> {
        let group = self.get_group_by_path(GAMEPLAY_TAG_GROUP_PATH)?;
        group.get_field(tag_index).map(|f| f.name.as_str())
    }

    /// Set a field directly on the group identified by `path_name_index`.
    ///
    /// Returns `true` if the group was found and the field handle was in range.
    pub fn set_field_on_group(&mut self, path_name_index: u32, field: NetFieldExport) -> bool {
        let placed = if let Some(group) = self.get_group_by_index_mut(path_name_index) {
            group.set_field(field)
        } else {
            false
        };
        if !placed {
            self.dropped_field_exports += 1;
        }
        placed
    }

    /// Net-field exports dropped by [`Self::set_field_on_group`]. See its doc.
    #[must_use]
    pub fn dropped_field_exports(&self) -> u64 {
        self.dropped_field_exports
    }

    /// Remove all state. Intended for tests or replay-boundary resets.
    pub fn clear(&mut self) {
        self.schema_generation = self.schema_generation.wrapping_add(1);
        self.guid_generation = self.guid_generation.wrapping_add(1);
        self.by_path.clear();
        self.by_index.clear();
        self.by_leaf.clear();
        self.groups.clear();
        self.guid_to_path.clear();
        self.guid_to_outer.clear();
    }

    /// Number of registered groups.
    #[must_use]
    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    /// Read-only access to all registered groups.
    ///
    /// Insertion-ordered, so it is stable across runs regardless of how the
    /// maps above hash their keys.
    #[must_use]
    pub fn groups(&self) -> &[NetFieldExportGroup] {
        &self.groups
    }
}

impl Default for NetGuidCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::NetFieldExport;
    // -- NetGuidCache unit tests (ported from NetGuidCacheTests.cs) -----------

    #[test]
    fn cache_stores_group_by_path_and_index() {
        let mut cache = NetGuidCache::new();
        let group = NetFieldExportGroup::new("/Game/Test.Test_C".into(), 7, 2);
        cache.add_export_group(group).unwrap();

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
        cache.add_export_group(group).unwrap();

        // Re-add with larger capacity.
        let expanded = NetFieldExportGroup::new("/Game/Test.Test_C".into(), 7, 4);
        cache.add_export_group(expanded).unwrap();

        let result = cache.get_group_by_index(7).unwrap();
        assert_eq!(result.len(), 4);
        assert_eq!(result.get_field(1).unwrap().name, "ExistingField");
    }

    #[test]
    fn crossed_path_and_index_identities_leave_both_groups_unchanged() {
        let mut cache = NetGuidCache::new();
        cache
            .add_export_group(NetFieldExportGroup::new("/Script/G.A".into(), 7, 1))
            .unwrap();
        cache
            .add_export_group(NetFieldExportGroup::new("/Script/G.B".into(), 8, 1))
            .unwrap();

        let err = cache
            .add_export_group(NetFieldExportGroup::new("/Script/G.A".into(), 8, 2))
            .unwrap_err();
        assert!(matches!(
            err,
            SchemaError::CrossedExportGroupIdentity { .. }
        ));

        assert_eq!(cache.group_count(), 2);
        assert_eq!(cache.get_group_by_index(7).unwrap().path, "/Script/G.A");
        assert_eq!(cache.get_group_by_index(8).unwrap().path, "/Script/G.B");
        assert_eq!(
            cache
                .get_group_by_path("/Script/G.A")
                .unwrap()
                .path_name_index,
            7
        );
        assert_eq!(
            cache
                .get_group_by_path("/Script/G.B")
                .unwrap()
                .path_name_index,
            8
        );
    }

    #[test]
    fn same_path_at_new_index_refreshes_the_canonical_index() {
        let mut cache = NetGuidCache::new();
        cache
            .add_export_group(NetFieldExportGroup::new("/Script/G.A".into(), 7, 1))
            .unwrap();

        cache
            .add_export_group(NetFieldExportGroup::new("/Script/G.A".into(), 9, 2))
            .unwrap();

        let group = cache.get_group_by_index(9).unwrap();
        assert_eq!(group.path, "/Script/G.A");
        assert_eq!(group.path_name_index, 9);
        assert_eq!(group.len(), 2);
        assert!(cache.get_group_by_index(7).is_none());
    }

    #[test]
    fn same_index_with_new_path_refreshes_path_and_leaf_indexes() {
        let mut cache = NetGuidCache::new();
        cache
            .add_export_group(NetFieldExportGroup::new("/Script/G.Old".into(), 7, 1))
            .unwrap();

        cache
            .add_export_group(NetFieldExportGroup::new("/Script/G.New".into(), 7, 2))
            .unwrap();

        let group = cache.get_group_by_index(7).unwrap();
        assert_eq!(group.path, "/Script/G.New");
        assert_eq!(group.path_name_index, 7);
        assert_eq!(group.len(), 2);
        assert!(cache.get_group_by_path("/Script/G.Old").is_none());
        assert!(cache.unique_leaf_match("Old").is_none());
        assert_eq!(
            cache.unique_leaf_match("New").unwrap().path,
            "/Script/G.New"
        );
    }

    /// A `path_name_index` reused for a genuinely different path must not
    /// hand the new class the old class's handle table. Before the fix, the
    /// index-only match merged into the existing group (preserving whatever
    /// slots the old class had set), so a content block resolved against the
    /// new path could read the old class's field name/type at a handle the
    /// new class never declared.
    #[test]
    fn same_index_with_new_path_does_not_inherit_old_fields() {
        let mut cache = NetGuidCache::new();
        cache
            .add_export_group(NetFieldExportGroup::new("/Script/G.Old".into(), 7, 2))
            .unwrap();
        cache.set_field_on_group(
            7,
            NetFieldExport {
                handle: 0,
                compatible_checksum: 111,
                name: "OldFieldZero".into(),
            },
        );

        cache
            .add_export_group(NetFieldExportGroup::new("/Script/G.New".into(), 7, 2))
            .unwrap();

        let group = cache.get_group_by_index(7).unwrap();
        assert_eq!(group.path, "/Script/G.New");
        assert!(
            group.get_field(0).is_none(),
            "handle 0 must not carry the old class's field after the index was reused: {:?}",
            group.get_field(0)
        );
    }

    #[test]
    fn cache_set_net_guid_path_stores_and_resolves() {
        let mut cache = NetGuidCache::new();
        cache.set_net_guid_path(17, "/Game/Test.Test_C".into(), None);

        assert_eq!(cache.get_path_by_guid(17).unwrap(), "/Game/Test.Test_C");
    }

    /// A redundant `set_net_guid_path` call -- same path, same outer -- must
    /// not bump `guid_generation`. Frame-level ExportData re-declares a GUID's
    /// path on every frame that re-exports it, with no pre-check of its own
    /// (unlike `vrfkit::sink::register_path`'s deliberate one); without this,
    /// a memo keyed on `guid_generation` would be invalidated on every such
    /// frame regardless of whether the mapping actually changed.
    #[test]
    fn a_redundant_set_net_guid_path_call_does_not_bump_guid_generation() {
        let mut cache = NetGuidCache::new();
        cache.set_net_guid_path(17, "/Game/Test.Test_C".into(), None);
        let after_first = cache.guid_generation();

        cache.set_net_guid_path(17, "/Game/Test.Test_C".into(), None);
        assert_eq!(cache.guid_generation(), after_first, "no change, no bump");

        cache.set_net_guid_path(17, "/Game/Test.Other_C".into(), None);
        assert_ne!(
            cache.guid_generation(),
            after_first,
            "a real change must still bump"
        );
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
        cache
            .add_export_group(NetFieldExportGroup::new("/Game/Test.Test_C".into(), 7, 2))
            .unwrap();
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
        cache.add_export_group(group).unwrap();

        assert_eq!(cache.get_gameplay_tag_name(2).unwrap(), "Ability.Active");
        assert!(cache.get_gameplay_tag_name(4).is_none()); // unpopulated slot
        assert!(cache.get_gameplay_tag_name(99).is_none()); // out of range
    }

    // -- Path alias lookup tests ----------------------------------------------

    #[test]
    fn alias_lookup_via_cache() {
        let mut cache = NetGuidCache::new();
        let group = NetFieldExportGroup::new("/Game/Characters/_Core/Jett/Jett_C".into(), 50, 1);
        cache.add_export_group(group).unwrap();

        // Should be reachable via the core-stripped alias.
        assert!(
            cache
                .get_group_by_path("/Game/Characters/Jett/Jett_C")
                .is_some()
        );
    }
}
