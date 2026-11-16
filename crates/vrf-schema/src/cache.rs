//! The replay-wide schema accumulator: export groups by path, index and leaf,
//! and the NetGUID -> path and -> outer maps. State accumulates for the whole
//! replay. The bare-name resolvers over the leaf index live in `resolve`; this
//! module is the storage and the direct lookups.

use crate::error::{Result, SchemaError};
use crate::export::{NetFieldExport, NetFieldExportGroup};
use crate::guid::{NetGuidEntry, NetworkGuid};
use crate::hash::FxHashMap;
use crate::path::for_each_replay_path_key;
use crate::resolve::register_leaf;

/// The path used for the gameplay-tag name table group.
const GAMEPLAY_TAG_GROUP_PATH: &str = "NetworkGameplayTagNodeIndex";

/// Resource policy for one replay-wide schema cache, not wire-format limits.
///
/// The defaults bound amplification by empty, sparsely declared groups. The
/// slot limit covers declared slots, including holes; it is not a cap on total
/// process memory, strings, map storage or spare vector capacity. See
/// docs/SCHEMA_LIMITS.md for the measured scope and configuration example.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaLimits {
    /// Maximum simultaneously registered export groups.
    pub max_groups: usize,
    /// Maximum declared field slots across all registered groups.
    pub max_field_slots: usize,
}

impl Default for SchemaLimits {
    fn default() -> Self {
        Self {
            max_groups: 4_096,
            max_field_slots: 1_048_576,
        }
    }
}

struct GroupAdmission {
    existing_by_path: Option<usize>,
    existing_by_index: Option<usize>,
    field_slots: usize,
}

/// Replay-wide schema accumulator: hashed lookups, and fields indexed by
/// handle within a group ([`NetFieldExportGroup::get_field`]).
#[derive(Default)]
pub struct NetGuidCache {
    /// path (String, ordinal) -> group index into `groups`.
    by_path: FxHashMap<String, usize>,
    /// path_name_index (u32) -> group index into `groups`.
    by_index: FxHashMap<u32, usize>,
    /// Leaf (the text after a path's last `.`) -> group index, or
    /// [`Self::AMBIGUOUS_LEAF`] when two or more groups claim it.
    by_leaf: FxHashMap<String, usize>,
    /// Central storage for all groups.
    groups: Vec<NetFieldExportGroup>,
    limits: SchemaLimits,
    /// Sum of declared slots, including holes, unless public mutable access
    /// invalidated it. The wire readers never resize through that accessor.
    field_slots: usize,
    field_slots_dirty: bool,
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
    /// See [`Self::dropped_field_exports`].
    dropped_field_exports: u64,
    /// See [`Self::replaced_export_groups`].
    replaced_export_groups: u64,
    /// See [`Self::renamed_field_exports`].
    renamed_field_exports: u64,
}

impl NetGuidCache {
    /// `by_leaf` sentinel: two or more groups share the leaf.
    pub(crate) const AMBIGUOUS_LEAF: usize = usize::MAX;

    /// Create an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a cache with explicit resource limits. Raising these limits for
    /// trusted inputs also raises the maximum allocation a declaration allows.
    ///
    /// ```
    /// use vrf_schema::{NetGuidCache, SchemaLimits};
    /// let mut cache = NetGuidCache::with_limits(SchemaLimits {
    ///     max_groups: 8,
    ///     max_field_slots: 128,
    /// });
    /// cache.declare_export_group("Example".into(), 1, 16)?;
    /// assert_eq!(cache.field_slot_count(), 16);
    /// # Ok::<(), vrf_schema::SchemaError>(())
    /// ```
    #[must_use]
    pub fn with_limits(limits: SchemaLimits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }

    /// Resource limits used for subsequent group declarations and additions.
    #[must_use]
    pub fn limits(&self) -> SchemaLimits {
        self.limits
    }

    /// Declared slots across all registered groups, including unpopulated ones.
    #[must_use]
    pub fn field_slot_count(&self) -> usize {
        if self.field_slots_dirty {
            self.groups
                .iter()
                .fold(0usize, |sum, group| sum.saturating_add(group.fields.len()))
        } else {
            self.field_slots
        }
    }

    /// The leaf index, for the resolvers in [`crate::resolve`].
    pub(crate) fn leaf_index(&self) -> &FxHashMap<String, usize> {
        &self.by_leaf
    }

    /// Changes whenever the set of group paths may have: on every successful
    /// [`Self::add_export_group`], [`Self::declare_export_group`] and
    /// [`Self::clear`]. It never tracks field
    /// mutations, so a memo of field contents must not key on it.
    #[must_use]
    pub fn schema_generation(&self) -> u64 {
        self.schema_generation
    }

    /// Register a new export group or merge it into the one its path or index
    /// already names, returning that group's index. A path and index naming
    /// two different groups fail without mutating the cache. Resource limits
    /// are checked before mutation, but `group` is already allocated: wire
    /// readers should use [`Self::declare_export_group`] instead.
    pub fn add_export_group(&mut self, group: NetFieldExportGroup) -> Result<usize> {
        let admission = self.admit_group(&group.path, group.path_name_index, group.fields.len())?;
        Ok(self.register_group(group, admission))
    }

    /// Declare a group after checking its identity and aggregate resource use,
    /// before allocating any field slots. Re-declarations merge the same way
    /// as [`Self::add_export_group`]. A refusal leaves the cache unchanged.
    pub fn declare_export_group(
        &mut self,
        path: String,
        path_name_index: u32,
        declared_slots: u32,
    ) -> Result<usize> {
        let admission = self.admit_group(&path, path_name_index, declared_slots as usize)?;
        let group = NetFieldExportGroup::new(path, path_name_index, declared_slots);
        Ok(self.register_group(group, admission))
    }

    fn admit_group(
        &mut self,
        path: &str,
        path_name_index: u32,
        slots: usize,
    ) -> Result<GroupAdmission> {
        let existing_by_path = self.by_path.get(path).copied();
        let existing_by_index = self.by_index.get(&path_name_index).copied();

        if let (Some(path_idx), Some(index_idx)) = (existing_by_path, existing_by_index) {
            if path_idx != index_idx {
                return Err(SchemaError::CrossedExportGroupIdentity {
                    path: path.to_owned(),
                    path_name_index,
                    path_group: self.groups[path_idx].path.clone(),
                    index_group: self.groups[index_idx].path.clone(),
                });
            }
        }

        let existing = existing_by_path.or(existing_by_index);
        let group_count = self
            .groups
            .len()
            .saturating_add(usize::from(existing.is_none()));
        if group_count > self.limits.max_groups {
            return Err(SchemaError::ResourceLimit {
                resource: "export groups",
                requested: group_count,
                limit: self.limits.max_groups,
            });
        }

        if self.field_slots_dirty {
            self.field_slots = self.field_slot_count();
            self.field_slots_dirty = false;
        }
        let old_slots = existing.map_or(0, |idx| self.groups[idx].fields.len());
        let kept_slots = if existing_by_path.is_some() {
            old_slots.max(slots)
        } else {
            slots
        };
        let field_slots = self
            .field_slots
            .saturating_sub(old_slots)
            .checked_add(kept_slots);
        let overflow = || SchemaError::ResourceLimit {
            resource: "field slots",
            requested: field_slots.unwrap_or(usize::MAX),
            limit: self.limits.max_field_slots,
        };
        let field_slots = field_slots.ok_or_else(overflow)?;
        if field_slots > self.limits.max_field_slots {
            return Err(overflow());
        }
        Ok(GroupAdmission {
            existing_by_path,
            existing_by_index,
            field_slots,
        })
    }

    fn register_group(&mut self, group: NetFieldExportGroup, admission: GroupAdmission) -> usize {
        let GroupAdmission {
            existing_by_path,
            existing_by_index,
            field_slots,
        } = admission;
        let idx = if let Some(idx) = existing_by_path {
            // The same class re-declaring or extending its group: its handle
            // slots stay valid and are kept.
            self.groups[idx].merge_from(&group);
            self.groups[idx].path = group.path;
            self.groups[idx].path_name_index = group.path_name_index;
            // The path can have arrived as another spelling and the index can
            // be new; either retires keys, so every lookup is rebuilt.
            self.rebuild_group_indexes();
            idx
        } else if let Some(idx) = existing_by_index {
            // The index was reused for a path never seen: replace the group,
            // because merging would let the old class's slots name handles the
            // new class never declared.
            self.groups[idx] = group;
            self.replaced_export_groups += 1;
            // The old class's spellings and leaf claim go with it.
            self.rebuild_group_indexes();
            idx
        } else {
            // A new group retires no key, so it registers alone rather than by
            // a rebuild; `index_group` says why both leave the same lookups.
            let idx = self.groups.len();
            self.groups.push(group);
            self.index_group(idx);
            idx
        };

        self.field_slots = field_slots;
        self.schema_generation = self.schema_generation.wrapping_add(1);
        idx
    }

    /// Rebuild every lookup from `groups` after a merge or replacement, which
    /// can retire keys that cannot be taken back one at a time:
    /// `AMBIGUOUS_LEAF` does not record its claimants, and a spelling one
    /// group gives up may belong to another that `by_path` no longer names.
    fn rebuild_group_indexes(&mut self) {
        self.by_path.clear();
        self.by_index.clear();
        self.by_leaf.clear();
        for idx in 0..self.groups.len() {
            self.index_group(idx);
        }
    }

    /// Register group `idx` in the lookups: every spelling of its path, its
    /// `path_name_index`, and its leaf. A later registration wins a spelling or
    /// an index; a second claimant turns a leaf ambiguous.
    ///
    /// For a group just pushed this leaves what a full rebuild would, since the
    /// maps already equal a rebuild of the older groups; the cache tests hold
    /// every arm to that (docs/PERFORMANCE_NOTES.md#registering-a-new-export-group).
    fn index_group(&mut self, idx: usize) {
        let group = &self.groups[idx];
        for_each_replay_path_key(&group.path, |key| {
            self.by_path.insert(key.to_owned(), idx);
        });
        self.by_index.insert(group.path_name_index, idx);
        register_leaf(&mut self.by_leaf, &group.path, idx);
    }

    /// Look up a group by its `path_name_index`.
    #[must_use]
    pub fn get_group_by_index(&self, path_name_index: u32) -> Option<&NetFieldExportGroup> {
        self.by_index
            .get(&path_name_index)
            .map(|&i| &self.groups[i])
    }

    /// A group by its `path_name_index`, mutably, for its field slots only:
    /// changing its `path` or `path_name_index` would leave the lookups stale.
    #[must_use]
    pub fn get_group_by_index_mut(
        &mut self,
        path_name_index: u32,
    ) -> Option<&mut NetFieldExportGroup> {
        let index = *self.by_index.get(&path_name_index)?;
        // The caller can resize the public fields vector. Recount before the
        // next admission so that it cannot bypass or stale the slot budget.
        self.field_slots_dirty = true;
        Some(&mut self.groups[index])
    }

    /// Look up a group by its full path (ordinal, case-sensitive): the hottest
    /// lookup, 2,989,695 probes over the reference replay's export.
    #[must_use]
    pub fn get_group_by_path(&self, path: &str) -> Option<&NetFieldExportGroup> {
        self.by_path.get(path).map(|&i| &self.groups[i])
    }

    /// Changes whenever a NetGUID -> path or -> outer mapping does; a memo over
    /// those maps stamps with it, as with [`Self::schema_generation`].
    #[must_use]
    pub fn guid_generation(&self) -> u64 {
        self.guid_generation
    }

    /// Register a NetGUID -> path mapping; an invalid outer removes the old
    /// one. A write that changes nothing neither allocates nor bumps
    /// [`Self::guid_generation`]: frame ExportData re-declares GUIDs every
    /// frame, which would collapse a memo's hit rate.
    pub fn set_net_guid_path(
        &mut self,
        net_guid: u32,
        path: impl AsRef<str> + Into<String>,
        outer: Option<NetworkGuid>,
    ) {
        let outer = outer.filter(|g| g.is_valid());
        if self.guid_to_path.get(&net_guid).map(String::as_str) == Some(path.as_ref())
            && self.guid_to_outer.get(&net_guid).copied() == outer
        {
            return;
        }
        self.guid_to_path.insert(net_guid, path.into());
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

    /// Every registered NetGUID with its path and outer GUID, in unspecified
    /// (hash) order; sort if determinism matters.
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

    /// A gameplay-tag name by network index: the `NetworkGameplayTagNodeIndex`
    /// group's field handles are tag indices and its names the tags.
    #[must_use]
    pub fn get_gameplay_tag_name(&self, tag_index: u32) -> Option<&str> {
        let group = self.get_group_by_path(GAMEPLAY_TAG_GROUP_PATH)?;
        group.get_field(tag_index).map(|f| f.name.as_str())
    }

    /// Set a field on the group at `path_name_index`. `false`, counted in
    /// [`Self::dropped_field_exports`], when the group or the handle is absent.
    pub fn set_field_on_group(&mut self, path_name_index: u32, field: NetFieldExport) -> bool {
        let group = (self.by_index.get(&path_name_index)).map(|&i| &mut self.groups[i]);
        let old = group.as_ref().and_then(|g| g.get_field(field.handle));
        self.renamed_field_exports += u64::from(old.is_some_and(|old| old.name != field.name));
        let placed = group.is_some_and(|g| g.set_field(field));
        self.dropped_field_exports += u64::from(!placed);
        placed
    }

    /// Net-field exports [`Self::set_field_on_group`] could not place (an
    /// unknown index, or a handle past the declared slots); the manifest
    /// reports this.
    #[must_use]
    pub fn dropped_field_exports(&self) -> u64 {
        self.dropped_field_exports
    }

    /// Groups whose `path_name_index` a new path took over, replacing them
    /// with no rows moved: later rows carry the new class's names.
    #[must_use]
    pub fn replaced_export_groups(&self) -> u64 {
        self.replaced_export_groups
    }

    /// Field exports that overwrote a populated handle with another name.
    #[must_use]
    pub fn renamed_field_exports(&self) -> u64 {
        self.renamed_field_exports
    }

    /// Remove all groups and GUID mappings (not the three tallies) and bump
    /// both generations. For tests or replay-boundary resets.
    pub fn clear(&mut self) {
        self.schema_generation = self.schema_generation.wrapping_add(1);
        self.guid_generation = self.guid_generation.wrapping_add(1);
        self.by_path.clear();
        self.by_index.clear();
        self.by_leaf.clear();
        self.groups.clear();
        self.field_slots = 0;
        self.field_slots_dirty = false;
        self.guid_to_path.clear();
        self.guid_to_outer.clear();
    }

    /// Number of registered groups.
    #[must_use]
    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    /// All registered groups, in insertion order, so stable across runs
    /// regardless of hashing.
    #[must_use]
    pub fn groups(&self) -> &[NetFieldExportGroup] {
        &self.groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::NetFieldExport;

    #[test]
    fn property_exports_never_shadow_the_class_net_cache_group() {
        const CLASS: &str = "/Script/ShooterGame.DamageableComponent";
        const RPC: &str = "/Script/ShooterGame.DamageableComponent_ClassNetCache";

        for properties_first in [false, true] {
            let mut cache = NetGuidCache::new();
            let mut properties = NetFieldExportGroup::new(CLASS.into(), 1, 4);
            properties.set_field(NetFieldExport {
                handle: 0,
                compatible_checksum: 11,
                name: "Property".into(),
            });
            let mut functions = NetFieldExportGroup::new(RPC.into(), 2, 9);
            functions.set_field(NetFieldExport {
                handle: 0,
                compatible_checksum: 22,
                name: "MulticastNotifyDamage_Base".into(),
            });

            if properties_first {
                cache.add_export_group(properties).unwrap();
                cache.add_export_group(functions).unwrap();
            } else {
                cache.add_export_group(functions).unwrap();
                cache.add_export_group(properties).unwrap();
            }

            // A late property re-declaration must not replace the function
            // handle capacity or field names, regardless of insertion order.
            cache
                .add_export_group(NetFieldExportGroup::new(CLASS.into(), 1, 4))
                .unwrap();
            let property_group = cache.get_group_by_path(CLASS).unwrap();
            let rpc_group = cache.get_group_by_path(RPC).unwrap();
            assert_eq!(property_group.path_name_index, 1);
            assert_eq!(property_group.len(), 4);
            assert_eq!(property_group.get_field(0).unwrap().name, "Property");
            assert_eq!(rpc_group.path_name_index, 2);
            assert_eq!(rpc_group.len(), 9);
            assert_eq!(
                rpc_group.get_field(0).unwrap().name,
                "MulticastNotifyDamage_Base"
            );
            assert_eq!(cache.get_group_by_index(2).unwrap().path, RPC);
            assert_eq!(cache.group_count(), 2);
        }
    }

    /// A `path_name_index` reused for another path must not hand the new class
    /// the old class's handle table.
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

        // A re-declaration of the same path merges: not a replacement.
        cache
            .add_export_group(NetFieldExportGroup::new("/Script/G.Old".into(), 7, 2))
            .unwrap();
        assert_eq!(cache.replaced_export_groups(), 0);
        cache
            .add_export_group(NetFieldExportGroup::new("/Script/G.New".into(), 7, 2))
            .unwrap();
        assert_eq!(cache.replaced_export_groups(), 1);

        let group = cache.get_group_by_index(7).unwrap();
        assert_eq!(group.path, "/Script/G.New");
        assert!(
            group.get_field(0).is_none(),
            "handle 0 must not carry the old class's field after the index was reused: {:?}",
            group.get_field(0)
        );
    }

    /// A handle re-exported under another name is counted; the same name
    /// again, or a first export at a new handle, is not.
    #[test]
    fn a_renamed_field_export_is_counted() {
        let mut cache = NetGuidCache::new();
        cache
            .add_export_group(NetFieldExportGroup::new("/Script/G.A".into(), 7, 2))
            .unwrap();
        for (handle, name, renamed) in [(0, "A", 0), (0, "A", 0), (1, "B", 0), (0, "C", 1)] {
            let field = NetFieldExport {
                handle,
                compatible_checksum: 0,
                name: name.into(),
            };
            assert!(cache.set_field_on_group(7, field));
            assert_eq!(cache.renamed_field_exports(), renamed, "{handle} {name}");
        }
        let group = cache.get_group_by_index(7).unwrap();
        assert_eq!(group.get_field(0).unwrap().name, "C");
    }

    /// An unknown `path_name_index` refuses the field and counts the drop.
    /// `read_net_field_exports` checks the group first, so only a direct call
    /// reaches this branch.
    #[test]
    fn set_field_on_unregistered_group_returns_false_and_counts_the_drop() {
        let mut cache = NetGuidCache::new();
        cache
            .add_export_group(NetFieldExportGroup::new("/Script/G.A".into(), 7, 1))
            .unwrap();
        assert_eq!(cache.dropped_field_exports(), 0);

        let placed = cache.set_field_on_group(
            8,
            NetFieldExport {
                handle: 0,
                compatible_checksum: 0,
                name: "Orphan".into(),
            },
        );

        assert!(!placed);
        assert_eq!(cache.dropped_field_exports(), 1);
        // The group registered at another index must not have received it.
        let group = cache.get_group_by_index(7).unwrap();
        assert_eq!(group.populated_fields().count(), 0);
        assert_eq!(cache.group_count(), 1);
    }

    /// A redundant `set_net_guid_path` (same path, same outer) must not bump
    /// `guid_generation`; see the method.
    #[test]
    fn a_redundant_set_net_guid_path_call_does_not_bump_guid_generation() {
        let mut cache = NetGuidCache::new();
        cache.set_net_guid_path(17, "/Game/Test.Test_C", None);
        let after_first = cache.guid_generation();

        cache.set_net_guid_path(17, "/Game/Test.Test_C", None);
        assert_eq!(cache.guid_generation(), after_first, "no change, no bump");

        cache.set_net_guid_path(17, "/Game/Test.Other_C", None);
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
        cache.set_net_guid_path(17, "Default__Test_C", Some(outer));
        cache.set_net_guid_path(11, "/Game/Test.Test_C", None);

        assert_eq!(cache.get_outer_guid(17).unwrap(), outer);
        assert_eq!(cache.get_outer_path(17).unwrap(), "/Game/Test.Test_C");
    }

    #[test]
    fn cache_net_guid_entries_yields_guid_path_and_outer() {
        let mut cache = NetGuidCache::new();
        cache.set_net_guid_path(11, "/Game/Test.Test_C", None);
        cache.set_net_guid_path(17, "FiringState", Some(NetworkGuid(11)));

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
        cache.set_net_guid_path(17, "/Game/Test.Test_C", None);

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

    use std::collections::BTreeMap;

    /// `by_path`, `by_index` and `by_leaf`, sorted so a mismatch prints as a
    /// readable diff.
    type Lookups = (
        BTreeMap<String, usize>,
        BTreeMap<u32, usize>,
        BTreeMap<String, usize>,
    );

    fn lookups(cache: &NetGuidCache) -> Lookups {
        (
            cache.by_path.iter().map(|(k, &v)| (k.clone(), v)).collect(),
            cache.by_index.iter().map(|(&k, &v)| (k, v)).collect(),
            cache.by_leaf.iter().map(|(k, &v)| (k.clone(), v)).collect(),
        )
    }

    /// `(path, path_name_index, declared slots)` for every group, in storage
    /// order.
    fn identities(cache: &NetGuidCache) -> Vec<(String, u32, usize)> {
        cache
            .groups
            .iter()
            .map(|g| (g.path.clone(), g.path_name_index, g.fields.len()))
            .collect()
    }

    fn spellings(path: &str) -> Vec<String> {
        let mut out = Vec::new();
        for_each_replay_path_key(path, |key| out.push(key.to_owned()));
        out
    }

    /// The leaf `register_leaf` files a path under, restated: the text after
    /// the last `.`, when there is a dot and that text is not empty.
    fn leaf_of(path: &str) -> Option<&str> {
        let dot = path.rfind('.')?;
        Some(&path[dot + 1..]).filter(|leaf| !leaf.is_empty())
    }

    /// `add_export_group`'s contract without an index: each lookup is a scan
    /// that answers from the definition. A spelling or an index belongs to the
    /// last group that has it, a leaf to its group only when exactly one group
    /// ends in it (else `AMBIGUOUS_LEAF`). Nothing depends on the order the
    /// cache wrote its maps, so agreeing means they hold what a rebuild would.
    #[derive(Default)]
    struct Model {
        groups: Vec<(String, u32, usize)>,
    }

    impl Model {
        fn owner_of_spelling(&self, key: &str) -> Option<usize> {
            self.groups
                .iter()
                .rposition(|(path, _, _)| spellings(path).iter().any(|s| s == key))
        }

        fn owner_of_index(&self, index: u32) -> Option<usize> {
            self.groups.iter().rposition(|&(_, i, _)| i == index)
        }

        fn lookups(&self) -> Lookups {
            let mut by_path = BTreeMap::new();
            let mut by_index = BTreeMap::new();
            let mut by_leaf = BTreeMap::new();
            for (path, index, _) in &self.groups {
                for key in spellings(path) {
                    let owner = self.owner_of_spelling(&key);
                    by_path.insert(key, owner.expect("a group has its own spellings"));
                }
                let owner = self.owner_of_index(*index);
                by_index.insert(*index, owner.expect("a group has its own index"));
                if let Some(leaf) = leaf_of(path) {
                    let claimants: Vec<usize> = (0..self.groups.len())
                        .filter(|&i| leaf_of(&self.groups[i].0) == Some(leaf))
                        .collect();
                    let entry = match claimants[..] {
                        [only] => only,
                        _ => NetGuidCache::AMBIGUOUS_LEAF,
                    };
                    by_leaf.insert(leaf.to_owned(), entry);
                }
            }
            (by_path, by_index, by_leaf)
        }
    }

    /// Which arm of `add_export_group` a call takes, decided by the model.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Arm {
        /// Neither the path nor the index is known: a new group.
        Fresh,
        /// The same path at the same index: a pure re-declaration.
        Redeclared,
        /// The same path at a new index.
        NewIndex,
        /// Matched through another spelling: the canonical path changes.
        NewSpelling,
        /// The index is reused for an unknown path: the group is replaced.
        Replaced,
        /// The path and the index name different groups: rejected.
        Crossed,
    }

    /// What one call did, for the coverage tallies.
    struct Step {
        arm: Arm,
        /// A new group that registers an alias spelling as well as its path.
        fresh_alias: bool,
        /// A new group with a spelling an older group already had, so the
        /// later-wins rule decided who keeps it.
        took_older_spelling: bool,
        leaves_turned_ambiguous: usize,
        leaves_disambiguated: usize,
    }

    /// Apply one `add_export_group` to the cache and the model, and hold the
    /// cache to the model on all a caller can observe: the index or exact
    /// error, the groups, one generation bump per success, and lookups equal
    /// to the model's and to a rebuild of the cache's own groups. An error
    /// must leave the groups and lookups untouched.
    fn apply(
        cache: &mut NetGuidCache,
        model: &mut Model,
        path: &str,
        index: u32,
        slots: u32,
        what: &str,
    ) -> Step {
        let by_spelling = model.owner_of_spelling(path);
        let by_index = model.owner_of_index(index);
        let arm = match (by_spelling, by_index) {
            (Some(a), Some(b)) if a != b => Arm::Crossed,
            (Some(a), _) if model.groups[a].0 != path => Arm::NewSpelling,
            (Some(a), _) if model.groups[a].1 != index => Arm::NewIndex,
            (Some(_), _) => Arm::Redeclared,
            (None, Some(_)) => Arm::Replaced,
            (None, None) => Arm::Fresh,
        };
        let before = model.lookups();
        let lookups_before = lookups(cache);
        let groups_before = identities(cache);
        let generation = cache.schema_generation();

        let incoming = (path.to_owned(), index, slots as usize);
        let expected: Result<usize> = match (arm, by_spelling, by_index) {
            (Arm::Crossed, Some(a), Some(b)) => Err(SchemaError::CrossedExportGroupIdentity {
                path: path.to_owned(),
                path_name_index: index,
                path_group: model.groups[a].0.clone(),
                index_group: model.groups[b].0.clone(),
            }),
            (Arm::Fresh, _, _) => {
                model.groups.push(incoming);
                Ok(model.groups.len() - 1)
            }
            (Arm::Replaced, _, Some(idx)) => {
                model.groups[idx] = incoming;
                Ok(idx)
            }
            (_, Some(idx), _) => {
                let kept = model.groups[idx].2.max(incoming.2);
                model.groups[idx] = (incoming.0, index, kept);
                Ok(idx)
            }
            _ => unreachable!("{what}: every arm names the group it acts on"),
        };

        let result = cache.add_export_group(NetFieldExportGroup::new(path.into(), index, slots));
        assert_eq!(result, expected, "{what}: wrong outcome");
        let bumps = u64::from(expected.is_ok());
        assert_eq!(
            cache.schema_generation(),
            generation.wrapping_add(bumps),
            "{what}: the generation must move exactly once per success"
        );
        if expected.is_err() {
            assert_eq!(
                identities(cache),
                groups_before,
                "{what}: an error moved a group"
            );
            assert_eq!(
                lookups(cache),
                lookups_before,
                "{what}: an error moved a lookup"
            );
        }
        assert_eq!(identities(cache), model.groups, "{what}: groups diverged");
        assert_eq!(
            cache.field_slot_count(),
            model.groups.iter().map(|group| group.2).sum::<usize>(),
            "{what}: slot accounting diverged from the model"
        );

        let after = model.lookups();
        let held = lookups(cache);
        assert_eq!(held, after, "{what}: the lookups diverged from the model");
        cache.rebuild_group_indexes();
        assert_eq!(
            held,
            lookups(cache),
            "{what}: the lookups differ from a full rebuild of the same groups"
        );

        let ambiguous = |map: &BTreeMap<String, usize>, leaf: &String| {
            map.get(leaf) == Some(&NetGuidCache::AMBIGUOUS_LEAF)
        };
        Step {
            arm,
            fresh_alias: arm == Arm::Fresh && spellings(path).len() > 1,
            took_older_spelling: arm == Arm::Fresh
                && spellings(path).iter().any(|s| before.0.contains_key(s)),
            leaves_turned_ambiguous: after
                .2
                .keys()
                .filter(|leaf| ambiguous(&after.2, leaf) && !ambiguous(&before.2, leaf))
                .count(),
            leaves_disambiguated: before
                .2
                .keys()
                .filter(|leaf| ambiguous(&before.2, leaf) && !ambiguous(&after.2, leaf))
                .count(),
        }
    }

    /// Every arm of `add_export_group`, each call's intended effect written
    /// above it. The arm and effect columns are checked against the model, so
    /// a step that stops reaching its case fails instead of testing less.
    #[test]
    fn every_registration_arm_leaves_the_lookups_a_full_rebuild_would_build() {
        use Arm::*;
        // One class under three spellings. The first two are each other's
        // `/_Core/` alias; the third's alias is the first.
        const JETT: &str = "/Game/Characters/Jett/Jett.Jett_C";
        const JETT_CORE: &str = "/Game/Characters/_Core/Jett/Jett.Jett_C";
        const JETT_ODD: &str = "/Game/_Core/Characters/Jett/Jett.Jett_C";
        // (path, index, slots, arm, took an older spelling, leaves turned
        // ambiguous, leaves disambiguated)
        let script: &[(&str, u32, u32, Arm, bool, usize, usize)] = &[
            // A new path.
            ("/Script/A.Shared", 1, 1, Fresh, false, 0, 0),
            // A second claimant of the leaf `Shared`.
            ("/Script/B.Shared", 2, 1, Fresh, false, 1, 0),
            // The same path again, with more slots.
            ("/Script/A.Shared", 1, 3, Redeclared, false, 0, 0),
            // The same path at a new index: index 1 must go.
            ("/Script/A.Shared", 3, 3, NewIndex, false, 0, 0),
            // Index 2 reused: `B.Shared` goes, and `Shared` is unique again.
            ("/Script/C.Other", 2, 2, Replaced, false, 0, 1),
            // A bare leaf: its `Default__` spelling is registered too.
            ("Foo", 4, 1, Fresh, false, 0, 0),
            // Reached through that alias: the canonical path changes.
            ("Default__Foo", 4, 1, NewSpelling, false, 0, 0),
            // Its alias is the previous group's own path; the later one wins.
            ("Default__Default__Foo", 5, 1, Fresh, true, 0, 0),
            // Its `/_Core/` spelling is registered too.
            (JETT, 6, 1, Fresh, false, 0, 0),
            // Its alias is the previous group's path, and it shares `Jett_C`.
            (JETT_ODD, 7, 1, Fresh, true, 1, 0),
            // Reached through the `/_Core/` alias, at a new index.
            (JETT_CORE, 8, 2, NewSpelling, false, 0, 0),
            // A third claimant of `Jett_C`.
            ("/Script/B.Jett_C", 9, 1, Fresh, false, 0, 0),
            // One of three `Jett_C` claimants goes: still ambiguous. `Other`
            // gains its second claimant.
            ("/Script/D.Other", 7, 1, Replaced, false, 1, 0),
            // The path and the index name different groups: nothing changes.
            ("/Script/A.Shared", 2, 1, Crossed, false, 0, 0),
            // One of two `Other` claimants goes: unique again.
            ("/Script/E.Third", 7, 1, Replaced, false, 0, 1),
            // An empty leaf is not filed at all.
            ("/Game/Empty.", 10, 1, Fresh, false, 0, 0),
        ];

        let mut cache = NetGuidCache::new();
        let mut model = Model::default();
        for (n, &(path, index, slots, arm, older, turned, cleared)) in script.iter().enumerate() {
            let what = format!("step {n}: {path:?} at index {index}");
            let step = apply(&mut cache, &mut model, path, index, slots, &what);
            let effect = (
                step.arm,
                step.took_older_spelling,
                step.leaves_turned_ambiguous,
                step.leaves_disambiguated,
            );
            assert_eq!(
                effect,
                (arm, older, turned, cleared),
                "{what}: the script no longer reaches its case"
            );
        }

        cache.clear();
        let mut model = Model::default();
        assert_eq!(lookups(&cache), model.lookups(), "clear left a lookup");
        let step = apply(&mut cache, &mut model, "Foo", 1, 1, "after clear");
        assert_eq!(step.arm, Fresh);
    }

    /// SplitMix64. A fixed seed walks the same sequences on every run.
    struct SplitMix64(u64);

    impl SplitMix64 {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^= z >> 31;
            (z % n as u64) as usize
        }
    }

    /// Random registration sequences over a small universe, so paths, indexes
    /// and leaves collide constantly: three claimants of one leaf, both
    /// `Default__` directions, a doubled prefix whose alias is another member's
    /// path, three spellings of one `/_Core/` class, a second class on the same
    /// leaf, an empty leaf and a `:` path. Every arm and effect is tallied and
    /// must be non-zero: `apply`'s checks only count for cases reached.
    #[test]
    fn random_registration_sequences_leave_the_lookups_a_full_rebuild_would_build() {
        const PATHS: [&str; 14] = [
            "/Script/A.Shared",
            "/Script/B.Shared",
            "/Script/C.Shared",
            "/Script/A.Other",
            "/Script/A.Shared_ClassNetCache",
            "Foo",
            "Default__Foo",
            "Default__Default__Foo",
            "/Game/Characters/Jett/Jett.Jett_C",
            "/Game/Characters/_Core/Jett/Jett.Jett_C",
            "/Game/_Core/Characters/Jett/Jett.Jett_C",
            "/Game/Characters/Sage/Sage.Jett_C",
            "/Game/Empty.",
            "/Script/X.Y:Z",
        ];
        const SEQUENCES: usize = 300;
        const CALLS: usize = 40;
        const INDEXES: usize = 8;

        let mut rng = SplitMix64(0x7672_666b_6974_0001);
        let mut arms: BTreeMap<Arm, usize> = BTreeMap::new();
        let (mut fresh_alias, mut took_older, mut turned, mut cleared, mut clears) =
            (0, 0, 0, 0, 0);
        for sequence in 0..SEQUENCES {
            let mut cache = NetGuidCache::new();
            let mut model = Model::default();
            for call in 0..CALLS {
                if rng.below(50) == 0 {
                    cache.clear();
                    model = Model::default();
                    assert_eq!(lookups(&cache), model.lookups(), "clear left a lookup");
                    clears += 1;
                    continue;
                }
                let path = PATHS[rng.below(PATHS.len())];
                let index = rng.below(INDEXES) as u32;
                let slots = rng.below(4) as u32;
                let what = format!("sequence {sequence}, call {call}: {path:?} at {index}");
                let step = apply(&mut cache, &mut model, path, index, slots, &what);
                *arms.entry(step.arm).or_default() += 1;
                fresh_alias += usize::from(step.fresh_alias);
                took_older += usize::from(step.took_older_spelling);
                turned += step.leaves_turned_ambiguous;
                cleared += step.leaves_disambiguated;
            }
        }

        let tallies = format!(
            "arms {arms:?}, fresh with alias {fresh_alias}, took an older spelling \
             {took_older}, leaves turned ambiguous {turned}, disambiguated {cleared}, \
             clears {clears}"
        );
        for arm in [
            Arm::Fresh,
            Arm::Redeclared,
            Arm::NewIndex,
            Arm::NewSpelling,
            Arm::Replaced,
            Arm::Crossed,
        ] {
            assert!(
                arms.get(&arm).is_some_and(|&n| n > 0),
                "{arm:?} never taken: {tallies}"
            );
        }
        assert!(fresh_alias > 0, "no new group had an alias: {tallies}");
        assert!(
            took_older > 0,
            "no new group took an older spelling: {tallies}"
        );
        assert!(turned > 0, "no leaf turned ambiguous: {tallies}");
        assert!(cleared > 0, "no ambiguous leaf became unique: {tallies}");
        assert!(clears > 0, "clear was never exercised: {tallies}");
    }
}
