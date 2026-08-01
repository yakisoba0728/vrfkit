//! The replay-wide accumulator for all dynamically-received schema state.
//!
//! [`NetGuidCache`] is the central authority for:
//!
//! - **path → group**: find an export group by its full path string.
//! - **path_name_index → group**: find an export group by the numeric index the
//!   engine assigns once and reuses for the rest of the replay.
//! - **NetGUID → object path**: map the 32-bit runtime object ID to its
//!   human-readable path (populated from export-GUID bunches).
//! - **NetGUID → outer NetGUID**: track the containment hierarchy so callers can
//!   walk from a component to its owning actor.
//! - **GameplayTag index → name**: the `NetworkGameplayTagNodeIndex` group's
//!   fields double as a tag name table.
//!
//! This state accumulates over the entire replay and is never reset.

use std::collections::HashMap;

use crate::export::{NetFieldExport, NetFieldExportGroup};
use crate::path::replay_path_lookup_keys;

/// A 32-bit network GUID referencing a replicated object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetworkGuid(pub u32);

impl NetworkGuid {
    /// The zero GUID is invalid (never assigned by the engine).
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }

    /// GUID 1 is the "default" object.
    #[must_use]
    pub const fn is_default(self) -> bool {
        self.0 == 1
    }

    /// Dynamic objects have an even GUID (bit 0 clear).
    #[must_use]
    pub const fn is_dynamic(self) -> bool {
        self.is_valid() && (self.0 & 1) == 0
    }
}

/// Flags on an exported NetGUID payload, controlling which optional fields
/// follow the GUID value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportFlags(pub u8);

impl ExportFlags {
    pub const NONE: Self = Self(0);
    pub const HAS_PATH: Self = Self(1 << 0);
    #[allow(dead_code)]
    pub const NO_LOAD: Self = Self(1 << 1);
    pub const HAS_NETWORK_CHECKSUM: Self = Self(1 << 2);

    #[must_use]
    pub const fn contains(self, flag: Self) -> bool {
        (self.0 & flag.0) == flag.0
    }
}

/// The path used for the gameplay-tag name table group.
const GAMEPLAY_TAG_GROUP_PATH: &str = "NetworkGameplayTagNodeIndex";

/// One registered NetGUID and what the replay said about it.
///
/// Produced by [`NetGuidCache::net_guid_entries`] so exporters can persist the
/// containment hierarchy. Downstream consumers need it to walk from a
/// subobject (e.g. a weapon's `FiringState`) to the actor that owns it; that
/// chain is the only route from a shot event to the equippable that fired it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetGuidEntry<'a> {
    /// The GUID itself.
    pub net_guid: u32,
    /// Object path as the replay declared it.
    pub path: &'a str,
    /// Containing object's GUID, when the replay declared one.
    pub outer_net_guid: Option<u32>,
}

/// Replay-wide schema accumulator.
///
/// All lookups are O(1) via `HashMap`. Field access within a group is O(1) via
/// direct `Vec` indexing (see [`NetFieldExportGroup::get_field`]).
pub struct NetGuidCache {
    /// path (String, ordinal) → group index into `groups`.
    by_path: HashMap<String, usize>,
    /// path_name_index (u32) → group index into `groups`.
    by_index: HashMap<u32, usize>,
    /// leaf name → group index. Used by [`Self::unique_leaf_match`] to resolve
    /// bare class names (e.g. `AresAttributeSet`) to their full export group
    /// path (e.g. `/Script/ShooterGame.AresAttributeSet`).
    ///
    /// Mirrors the C# `ContentBlockPathResolver.UniqueLeafMatch` logic: a bare
    /// name is only resolved if exactly ONE group has a path ending with
    /// `.{name}`. Ambiguous names (multiple groups sharing the same leaf) are
    /// stored as `usize::MAX` to signal rejection.
    by_leaf: HashMap<String, usize>,
    /// Central storage for all groups.
    groups: Vec<NetFieldExportGroup>,
    /// NetGUID value → object path string.
    guid_to_path: HashMap<u32, String>,
    /// NetGUID value → outer NetGUID value (containment hierarchy).
    guid_to_outer: HashMap<u32, NetworkGuid>,
}

impl NetGuidCache {
    /// Sentinel value in `by_leaf` indicating an ambiguous leaf (multiple
    /// groups share the same trailing class name).
    const AMBIGUOUS_LEAF: usize = usize::MAX;

    /// Create an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            by_path: HashMap::new(),
            by_index: HashMap::new(),
            by_leaf: HashMap::new(),
            groups: Vec::new(),
            guid_to_path: HashMap::new(),
            guid_to_outer: HashMap::new(),
        }
    }

    /// Register a new export group or merge it with an existing one.
    ///
    /// If a group with the same `path` or `path_name_index` already exists, the
    /// incoming fields are merged into it (growing the field vector if needed).
    /// Returns a shared reference to the canonical group.
    pub fn add_export_group(&mut self, group: NetFieldExportGroup) -> usize {
        let existing_by_path = self.by_path.get(&group.path).copied();
        let existing_by_index = self.by_index.get(&group.path_name_index).copied();

        let existing_idx = existing_by_path.or(existing_by_index);

        if let Some(idx) = existing_idx {
            // Merge: grow capacity and overwrite populated slots.
            self.groups[idx].merge_from(&group);
            // Ensure both maps point to the same group.
            self.by_path.insert(group.path.clone(), idx);
            self.by_index.insert(group.path_name_index, idx);
            // Also register all path aliases.
            for alias in replay_path_lookup_keys(&group.path).into_iter().skip(1) {
                self.by_path.entry(alias).or_insert(idx);
            }
            idx
        } else {
            let idx = self.groups.len();
            // Register aliases before pushing.
            let aliases = replay_path_lookup_keys(&group.path);
            for alias in &aliases {
                self.by_path.insert(alias.clone(), idx);
            }
            self.by_index.insert(group.path_name_index, idx);
            // Register leaf-suffix index for UniqueLeafMatch resolution.
            Self::register_leaf(&mut self.by_leaf, &group.path, idx);
            self.groups.push(group);
            idx
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
    #[must_use]
    pub fn get_group_by_path(&self, path: &str) -> Option<&NetFieldExportGroup> {
        self.by_path.get(path).map(|&i| &self.groups[i])
    }

    /// Register a NetGUID → path mapping (from export GUID bunches).
    pub fn set_net_guid_path(&mut self, net_guid: u32, path: String, outer: Option<NetworkGuid>) {
        self.guid_to_path.insert(net_guid, path);
        match outer {
            Some(g) if g.is_valid() => {
                self.guid_to_outer.insert(net_guid, g);
            }
            _ => {
                self.guid_to_outer.remove(&net_guid);
            }
        }
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

    /// Resolve a bare class name to its export group using leaf-suffix matching.
    ///
    /// Mirrors the C# `ContentBlockPathResolver.UniqueLeafMatch`: when a path
    /// has no separators (is a bare name like `AresAttributeSet`), look for a
    /// registered group whose path ends with `.{name}`. Returns the canonical
    /// group only if exactly one such group exists (ambiguous leaves return None).
    ///
    /// Beyond C#'s exact-match logic, also tries common Unreal suffixes:
    /// - `name + "Component"` — subobject GUIDs often omit the `Component`
    ///   suffix that their export group path includes (e.g. GUID path
    ///   `EquippableStateMachine` → group `.EquippableStateMachineComponent`).
    /// - `name + "_C"` — Blueprint-class GUIDs (especially `Comp_*` prefixed)
    ///   map to groups with a `_C` suffix on the leaf.
    ///
    /// This is the bridge between NetGUID paths (often bare class names) and
    /// fully-qualified export group paths.
    #[must_use]
    pub fn unique_leaf_match(&self, bare_name: &str) -> Option<&NetFieldExportGroup> {
        // Only apply to bare names (no path separators).
        if bare_name.contains('/') || bare_name.contains('.') || bare_name.contains(':') {
            return None;
        }
        // Try exact leaf first.
        if let Some(&idx) = self.by_leaf.get(bare_name) {
            if idx != Self::AMBIGUOUS_LEAF {
                return self.groups.get(idx);
            }
        }
        // Try "name + Component" suffix — Unreal's most common subobject naming
        // convention stores GUIDs without the suffix but registers export groups
        // with it.
        let mut suffixed = String::with_capacity(bare_name.len() + 9);
        suffixed.push_str(bare_name);
        suffixed.push_str("Component");
        if let Some(&idx) = self.by_leaf.get(&suffixed) {
            if idx != Self::AMBIGUOUS_LEAF {
                return self.groups.get(idx);
            }
        }
        // Try "name + _C" suffix — Blueprint class GUIDs (Comp_* etc.) register
        // their export group with a _C suffix on the leaf.
        suffixed.clear();
        suffixed.push_str(bare_name);
        suffixed.push_str("_C");
        if let Some(&idx) = self.by_leaf.get(&suffixed) {
            if idx != Self::AMBIGUOUS_LEAF {
                return self.groups.get(idx);
            }
        }
        None
    }

    /// Resolve a bare instance name to a `_ClassNetCache` export group.
    ///
    /// This bridges the gap between actor/subobject instance names (e.g.
    /// `BombDestination_A`, `ForceModuleManager`, `AudDeadeyeVOComponent`) and
    /// their ClassNetCache groups in the replay schema. The replay declares
    /// groups like `BombDestination_C_ClassNetCache` or
    /// `ForceModuleManagerComponent_ClassNetCache` but the wire only gives us
    /// the bare instance name.
    ///
    /// # Strategy
    ///
    /// For each candidate stem (starting with the full name, then progressively
    /// stripping the last `_SEGMENT`), try these leaf lookups in `by_leaf`:
    ///
    /// 1. `stem_ClassNetCache` (exact class, e.g. `AresAbilitySystem` matches
    ///    `AresAbilitySystemComponent_ClassNetCache` via step 2)
    /// 2. `stemComponent_ClassNetCache` (Unreal components often drop the suffix
    ///    in instance names)
    /// 3. `stem_C_ClassNetCache` (Blueprint classes use `_C` to denote the
    ///    compiled class)
    ///
    /// The capacity comes from the matched group's declared
    /// `NetFieldExportsLength` (never guessed). Only unambiguous matches (one
    /// group per leaf) are accepted.
    ///
    /// # Why instance-suffix stripping is needed
    ///
    /// Unreal appends instance identifiers to actor names: `BombDestination_A`,
    /// `WindowShieldA1`, `RespawningWallPlate_2`, `AmbientAudio_Ascent_*`. The
    /// class name is the stem before the instance suffix. Stripping one
    /// underscore segment at a time from the right correctly recovers the class
    /// for all observed patterns without hardcoding any actor or map name.
    #[must_use]
    pub fn resolve_cnc_for_instance_name(&self, bare_name: &str) -> Option<&NetFieldExportGroup> {
        // Only bare names (no path separators).
        if bare_name.contains('/') || bare_name.contains('.') || bare_name.contains(':') {
            return None;
        }

        // Try with the full name first, then progressively shorter stems.
        let mut stem = bare_name;
        loop {
            if let Some(group) = self.try_cnc_leaf_candidates(stem) {
                return Some(group);
            }

            // Strip the last underscore-delimited segment to get a shorter stem.
            // e.g. "BombDestination_A" -> "BombDestination"
            //      "AmbientAudio_Ascent_Defender_SoundA_003" -> ...
            match stem.rfind('_') {
                Some(pos) if pos > 0 => {
                    stem = &bare_name[..pos];
                }
                _ => break,
            }
        }

        // Final fallback: strip trailing digits (handles WindowShieldA1 ->
        // WindowShield, MeleeAttackState1 -> MeleeAttackState). Only attempt
        // this if the name does NOT end with an underscore-separated segment
        // (those were already tried above).
        let trimmed = bare_name.trim_end_matches(|c: char| c.is_ascii_digit());
        if trimmed.len() < bare_name.len() && !trimmed.is_empty() {
            // Also strip a trailing uppercase letter that acts as a site/variant
            // marker (e.g. WindowShieldA1 -> WindowShieldA -> WindowShield).
            let trimmed2 = trimmed.trim_end_matches(|c: char| c.is_ascii_uppercase());
            if trimmed2.len() < trimmed.len() && !trimmed2.is_empty() {
                if let Some(group) = self.try_cnc_leaf_candidates(trimmed2) {
                    return Some(group);
                }
            }
            if trimmed != bare_name {
                if let Some(group) = self.try_cnc_leaf_candidates(trimmed) {
                    return Some(group);
                }
            }
        }

        None
    }

    /// Try ClassNetCache leaf candidates for a given stem.
    ///
    /// Checks `stem_ClassNetCache`, `stemComponent_ClassNetCache`, and
    /// `stem_C_ClassNetCache` in the leaf index.
    fn try_cnc_leaf_candidates(&self, stem: &str) -> Option<&NetFieldExportGroup> {
        // Reusable buffer for candidate construction.
        let base_cap = stem.len() + "_ClassNetCache".len() + "Component".len();
        let mut candidate = String::with_capacity(base_cap);

        // 1. stem_ClassNetCache
        candidate.push_str(stem);
        candidate.push_str("_ClassNetCache");
        if let Some(group) = self.lookup_cnc_leaf(&candidate) {
            return Some(group);
        }

        // 2. stemComponent_ClassNetCache
        candidate.clear();
        candidate.push_str(stem);
        candidate.push_str("Component_ClassNetCache");
        if let Some(group) = self.lookup_cnc_leaf(&candidate) {
            return Some(group);
        }

        // 3. stem_C_ClassNetCache
        candidate.clear();
        candidate.push_str(stem);
        candidate.push_str("_C_ClassNetCache");
        if let Some(group) = self.lookup_cnc_leaf(&candidate) {
            return Some(group);
        }

        None
    }

    /// Look up a CNC leaf in by_leaf, accepting only unambiguous matches whose
    /// group path actually ends with `_ClassNetCache`.
    fn lookup_cnc_leaf(&self, leaf: &str) -> Option<&NetFieldExportGroup> {
        let &idx = self.by_leaf.get(leaf)?;
        if idx == Self::AMBIGUOUS_LEAF {
            return None;
        }
        let group = self.groups.get(idx)?;
        if group.path.ends_with("_ClassNetCache") {
            Some(group)
        } else {
            None
        }
    }

    /// Register the leaf component of a group path in the `by_leaf` index.
    ///
    /// Extracts the trailing class name after the last `.` in the path. If
    /// another group already claimed this leaf, marks it as ambiguous.
    fn register_leaf(by_leaf: &mut HashMap<String, usize>, path: &str, idx: usize) {
        // Extract the leaf: the part after the last '.' in the path.
        let leaf = match path.rfind('.') {
            Some(dot_pos) => &path[dot_pos + 1..],
            None => return, // No dot separator → not a qualified path, skip.
        };
        if leaf.is_empty() {
            return;
        }
        by_leaf
            .entry(leaf.to_owned())
            .and_modify(|existing| {
                if *existing != idx {
                    *existing = Self::AMBIGUOUS_LEAF;
                }
            })
            .or_insert(idx);
    }

    /// Set a field directly on the group identified by `path_name_index`.
    ///
    /// Returns `true` if the group was found and the field handle was in range.
    pub fn set_field_on_group(&mut self, path_name_index: u32, field: NetFieldExport) -> bool {
        if let Some(group) = self.get_group_by_index_mut(path_name_index) {
            group.set_field(field)
        } else {
            false
        }
    }

    /// Remove all state. Intended for tests or replay-boundary resets.
    pub fn clear(&mut self) {
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
