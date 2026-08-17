//! Content-block group-path resolution.
//!
//! Every content block has to be attributed to one of the replay's declared
//! export groups before a single field in it can be named, and the rules for
//! doing that mirror the C# `ContentBlockPathResolver`. There are 608,020
//! blocks in the reference replay and docs/archive/PROJECT_STATUS.md 5-P
//! measured this resolution at 371 ms -- the largest single slice of the
//! export after the Parquet writers were moved off the packet loop.
//!
//! # The memo
//!
//! [`BlockPathMemo`] is what this module adds over a straight port of the C#.
//! Resolution is a pure function of
//!
//! - the block header's `is_actor`, `has_rep_layout`, `class_net_guid` and
//!   `object_net_guid`,
//! - the channel index and actor GUID,
//! - the cache's declared group paths (`schema_generation` tracks these),
//! - the cache's GUID -> path and GUID -> outer maps, and
//! - this crate's channel -> (actor, archetype) map.
//!
//! The first two are the memo key. The third is `NetGuidCache::schema_generation`.
//! The last two are what `ChannelState::resolution_generation` exists for --
//! `schema_generation` explicitly does not cover them. A change in either stamp
//! discards the whole memo, so a hit is indistinguishable from a recomputation.
//!
//! The value is the pair `(group path, function count)` rather than just the
//! path, because `resolve_function_count` can *replace* the resolved path (the
//! bare-instance-name branch) and the two must not be memoised apart.
//!
//! Measured on 02d4d478 with a throwaway instrumented build:
//!
//! ```text
//! probes 608,011   hits 489,996 (80.6%)   misses 118,015
//! generation changes 1,823      entries held at the end 64
//! ```
//!
//! The entry count is the answer to the obvious objection. `actor_net_guid` is
//! part of the key and grows monotonically over a replay, so an unbounded memo
//! was the risk; in practice a resolution input moves every ~330 blocks and the
//! table is discarded long before it can grow. The memo costs kilobytes and
//! removes four fifths of the work.

use std::sync::Arc;

use vrf_net::content::ContentBlockHeader;
use vrf_net::types::NetworkGuid;
use vrf_schema::{FxHashMap, NetFieldExportGroup, find_class_net_cache_key, find_replay_path_key};

use super::{ChannelState, ExportSink};

/// Well-known subobject leaf names that map to a fixed class path, tagged with
/// the block kind they apply to.
///
/// The replay uses short "stably named" identifiers for certain built-in
/// components. When no `class_net_guid` is present we fall back to this table,
/// exactly as the C# reference parser does in `ContentBlockPathResolver`.
///
/// The effect entries mirror the C# reference and apply to ClassNetCache blocks.
/// The last two pairs go beyond the C# reference: VALORANT replicates the
/// inventory and Gameplay Ability System components under their Blueprint class
/// names, but the replay declares their property layouts under the native parent
/// class, so a block whose object path is the bare Blueprint name never matches a
/// declared group and every handle it carries stays unnamed. Remapping the
/// Blueprint leaf to the native RepLayout class lets the handles pick up names
/// and types -- `CurrentEquippable` (the spike carrier) included. They are
/// tagged RepLayout-only on purpose: the AbilitySystem `_ClassNetCache` group is
/// declared with an incomplete function table, so remapping its RPC stream to it
/// would mis-parse it. That stream stays unresolved and is brute-forced (fc=34).
const KNOWN_SUBOBJECT_CLASS_PATHS: &[(&str, &str, GroupKind)] = &[
    (
        "ReplayEffect",
        "/Script/ShooterGame.ReplayEffectComponent",
        GroupKind::ClassNetCache,
    ),
    (
        "EffectManager",
        "/Script/ShooterGame.EffectManagerComponent",
        GroupKind::ClassNetCache,
    ),
    (
        "LocationalEffectManager",
        "/Script/ShooterGame.LocationalEffectManagerComponent",
        GroupKind::ClassNetCache,
    ),
    (
        "DamageHandlerComponent",
        "/Script/ShooterGame.DamageableComponent",
        GroupKind::ClassNetCache,
    ),
    (
        "InventoryComponent",
        "/Script/ShooterGame.AresInventory",
        GroupKind::RepLayout,
    ),
    (
        "AbilitiesAndBuffsComponent",
        "/Script/ShooterGame.AresAbilitySystemComponent",
        GroupKind::RepLayout,
    ),
    // Read out of the shipped game rather than inferred, which is what makes
    // these authoritative where the two pairs above were argued from handle
    // shapes. A cooked Blueprint stores each component as a
    // `<Name>_GEN_VARIABLE` export whose class index is a script import hash,
    // and the IoStore global container maps that hash back to a native path --
    // so every pair below is what the asset itself says. The same pass
    // reproduced `InventoryComponent` and `AbilitiesAndBuffsComponent`
    // independently, which is the check on the method.
    //
    // Not regenerated by anything in `tools/`: it needs the installed game and
    // an Oodle-capable IoStore reader, and vrfkit parses replays rather than
    // game files. docs/DATA.md records the procedure. Re-derive it when a build
    // renames a component; nothing here can detect that on its own.
    //
    // Every target is a group the replay itself declares, so the handles pick up
    // names and types the moment the leaf resolves.
    (
        "ZoomStateMachine",
        "/Script/ShooterGame.EquippableStateMachineComponent",
        GroupKind::RepLayout,
    ),
    (
        "SelectBounceStateMachine",
        "/Script/ShooterGame.EquippableStateMachineComponent",
        GroupKind::RepLayout,
    ),
    (
        "StateMachine_Priming",
        "/Script/ShooterGame.EquippableStateMachineComponent",
        GroupKind::RepLayout,
    ),
    (
        "TargetingToggle_StateMachine",
        "/Script/ShooterGame.EquippableStateMachineComponent",
        GroupKind::RepLayout,
    ),
    (
        "SuppressionRoundsStateMachine",
        "/Script/ShooterGame.EquippableStateMachineComponent",
        GroupKind::RepLayout,
    ),
    (
        "BoostStateMachine",
        "/Script/ShooterGame.EquippableStateMachineComponent",
        GroupKind::RepLayout,
    ),
    (
        "Gun_StateMachine",
        "/Script/ShooterGame.EquippableStateMachineComponent",
        GroupKind::RepLayout,
    ),
    (
        "RewindStateMachine",
        "/Script/ShooterGame.EquippableStateMachineComponent",
        GroupKind::RepLayout,
    ),
    (
        "UseAbilityStateMachine",
        "/Script/ShooterGame.EquippableStateMachineComponent",
        GroupKind::RepLayout,
    ),
    // Both ammo counters are the same native component; the Blueprint just
    // instantiates it twice. That is what supersedes the handle addition this
    // group used to need: `AmmoComponent` declares handle 2 as
    // `AuthResourceAmount`, so naming it by hand as `AmmoCount` was a guess in
    // the right place with the wrong word. Magazine reads 0..100, reserve
    // 0..200.
    (
        "MagazineAmmo",
        "/Script/ShooterGame.AmmoComponent",
        GroupKind::RepLayout,
    ),
    (
        "ReserveAmmo",
        "/Script/ShooterGame.AmmoComponent",
        GroupKind::RepLayout,
    ),
    (
        "CalloutRegionTracker",
        "/Script/ShooterGame.CalloutRegionTrackingComponent",
        GroupKind::RepLayout,
    ),
    (
        "VisionComponent",
        "/Script/ShooterGame.ShooterCharacterVisionComponent",
        GroupKind::RepLayout,
    ),
    (
        "HealthDamageSection",
        "/Script/ShooterGame.ChildDamageSectionComponent",
        GroupKind::RepLayout,
    ),
    (
        "UsableComponent_EquippableGroundPickup",
        "/Script/ShooterGame.UsableComponent_EquippableGroundPickups",
        GroupKind::RepLayout,
    ),
    (
        "PMAimToolingPointsTarget",
        "/Script/InputTooling.AimToolingPointsTargetComponent",
        GroupKind::RepLayout,
    ),
    // GAS creates its attribute sets as runtime subobjects rather than as
    // Blueprint components, so this pair does not come from a cooked asset like
    // the ones above -- it is read off the wire. The name is the giveaway and
    // the handles confirm it: all 116 handles the bare group uses are a subset
    // of the 122 the native group declares, every one 32 bits wide, same as the
    // named instance. It is the same attribute set replicated a second time,
    // for actors that are not player characters.
    (
        "AresAttributeSet_2",
        "/Script/ShooterGame.AresAttributeSet",
        GroupKind::RepLayout,
    ),
];

/// Everything a block's resolution depends on that is not cache state.
///
/// `object_net_guid` is unused by the actor branch and `channel_index` /
/// `actor_net_guid` by the subobject branch, but all six are kept in the key
/// rather than normalised per branch: a normalisation that drops a field the
/// resolution actually reads is silent byte movement, and the cost of an
/// over-precise key is only extra entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct BlockKey {
    channel_index: u32,
    actor_net_guid: u32,
    class_net_guid: u32,
    object_net_guid: u32,
    is_actor: bool,
    has_rep_layout: bool,
}

/// Memo for [`ExportSink::resolve_block`]. See the module docs for why it is
/// exactly equivalent to recomputing.
#[derive(Debug, Clone, Default)]
pub(super) struct BlockPathMemo {
    /// `NetGuidCache::schema_generation` when `entries` was last valid.
    schema_generation: u64,
    /// `ChannelState::resolution_generation` when `entries` was last valid.
    resolution_generation: u64,
    entries: FxHashMap<BlockKey, (Arc<str>, u32)>,
}

impl BlockPathMemo {
    /// Drop everything if either stamp has moved, then report the current
    /// entry for `key`.
    fn get(&mut self, key: &BlockKey, schema: u64, resolution: u64) -> Option<(Arc<str>, u32)> {
        if self.schema_generation != schema || self.resolution_generation != resolution {
            self.entries.clear();
            self.schema_generation = schema;
            self.resolution_generation = resolution;
            return None;
        }
        self.entries
            .get(key)
            .map(|(path, count)| (Arc::clone(path), *count))
    }

    fn insert(&mut self, key: BlockKey, path: Arc<str>, count: u32) {
        self.entries.insert(key, (path, count));
    }
}

impl ExportSink<'_> {
    /// Resolve one content block: set `current_group_path` and return the
    /// function-table capacity a ClassNetCache block's RPC handles are read
    /// against (0 for a RepLayout block, and 0 when the group is unresolved).
    ///
    /// This is the memoised entry point; everything below it is the
    /// computation the memo stands in for.
    pub(super) fn resolve_block(
        &mut self,
        channel_index: u32,
        actor_net_guid: NetworkGuid,
        header: &ContentBlockHeader,
    ) -> u32 {
        let key = BlockKey {
            channel_index,
            actor_net_guid: actor_net_guid.0,
            class_net_guid: header.class_net_guid.0,
            object_net_guid: header.object_net_guid.0,
            is_actor: header.is_actor,
            has_rep_layout: header.has_rep_layout,
        };
        let schema = self.cache.schema_generation();
        let resolution = self.channel_state.resolution_generation;
        if let Some((path, count)) = self.channel_state.block_paths.get(&key, schema, resolution) {
            self.set_current_group_path(path);
            return count;
        }

        let path = self.resolve_group_path(channel_index, actor_net_guid.0, header);
        let interned = self.channel_state.names.intern(&path);
        self.set_current_group_path(interned);
        // A RepLayout block reads its handles against the group directly and
        // needs no function table, so the capacity question does not arise.
        let count = if header.has_rep_layout {
            0
        } else {
            // May replace `current_group_path`, which is why the memo stores
            // the pair and this line comes before the insert.
            self.resolve_function_count(header, channel_index, actor_net_guid.0)
        };
        let resolved = Arc::clone(&self.current_group_path);
        self.channel_state.block_paths.insert(key, resolved, count);
        count
    }

    /// Resolve a content block to the appropriate export group path.
    ///
    /// Follows the same logic as the C# `ContentBlockPathResolver`:
    /// - For **actor** blocks: derive the class path from the channel's archetype
    ///   GUID (outer path + class-name leaf extracted from archetype path).
    /// - For **subobject** blocks: use `class_net_guid` directly.
    ///
    /// For RepLayout blocks the result must match a RepLayout group; for
    /// ClassNetCache blocks the result must match a `*_ClassNetCache` group.
    fn resolve_group_path(
        &self,
        channel_index: u32,
        guid: u32,
        header: &ContentBlockHeader,
    ) -> String {
        if header.is_actor {
            self.resolve_actor_group_path(channel_index, guid, header)
        } else {
            self.resolve_subobject_group_path(header)
        }
    }

    /// Actor path resolution -- mirrors `ResolveCachedActorExportGroupPath` /
    /// `ResolveCachedActorClassPath` from the C# reference.
    ///
    /// Priority order (matching C# `ResolveActorPackageOrClassPath`):
    /// 1. Archetype outer path (package path for the class)
    /// 2. Archetype path itself (if not a CDO path)
    /// 3. Actor GUID path
    ///
    /// For ClassNetCache blocks we then combine the package path with the class
    /// name extracted from the archetype's leaf (stripping `Default__`).
    fn resolve_actor_group_path(
        &self,
        channel_index: u32,
        actor_guid: u32,
        header: &ContentBlockHeader,
    ) -> String {
        // Step 1: Determine the base "package or class" path.
        let (package_path, archetype_path) =
            self.resolve_actor_package_and_archetype(channel_index, actor_guid);

        // Step 2: Combine package path with class name from archetype.
        let combined =
            self.create_combined_candidate(package_path.as_deref(), archetype_path.as_deref());

        // Step 3: Find matching export group using the combined or package path.
        //
        // For ClassNetCache blocks, only accept groups whose canonical path
        // ends with `_ClassNetCache`. Without this check, a lookup key like
        // `AggroBot_PC.AggroBot_PC_C` would match the RepLayout group (14
        // fields) instead of the ClassNetCache group (4 fields), causing
        // ReadSerializedInt to consume the wrong number of bits.
        let want = GroupKind::for_block(header);

        // Try combined path first (most specific), then the package path, then
        // the archetype path when it is not a CDO.
        if let Some(hit) = self.match_group(combined.as_deref(), want) {
            return hit;
        }
        if let Some(hit) = self.match_group(package_path.as_deref(), want) {
            return hit;
        }
        if let Some(arch) = archetype_path.as_deref() {
            if !is_class_default_object_path(arch) {
                if let Some(hit) = self.match_group(Some(arch), want) {
                    return hit;
                }
            }
        }

        // Fallback: try actor GUID path directly.
        if let Some(actor_path) = self.cache.get_path_by_guid(actor_guid) {
            if let Some(hit) = self.match_group(Some(actor_path), want) {
                return hit;
            }
            // UniqueLeafMatch, exactly as the class path and the subobject path
            // already apply it below. A static actor arrives as a bare instance
            // name (`AresWorldSettings`) and the lookup keys above can only
            // match a group whose declared path is that same bare string; the
            // declared group is `/Script/ShooterGame.AresWorldSettings`, so the
            // block fell through to the raw name and every field it carried
            // stayed unnamed. Ambiguous leaves still bind to nothing -- see
            // NetGuidCache::unique_leaf_match -- so this determines the group
            // rather than guessing one.
            //
            // The `_ClassNetCache` guard is load-bearing on this path, not
            // decoration. Below this point `resolve_function_count` runs its own
            // instance-name resolver, and it only runs while current_group_path
            // is still a bare name; a RepLayout group returned here would
            // silence it and hand ReadSerializedInt the wrong capacity. The
            // guard cannot be satisfied by a leaf match unless the actor's own
            // path ends with `_ClassNetCache`, because by_leaf keys are the text
            // after the last `.` and the two suffix arms append `Component` and
            // `_C`, so on a ClassNetCache block this call is inert.
            if let Some(g) = self.cache.unique_leaf_match(actor_path) {
                if want.accepts(g) {
                    return g.path.clone();
                }
            }
            // Blueprint component name -> native parent class (see
            // KNOWN_SUBOBJECT_CLASS_PATHS). A bare name like "InventoryComponent"
            // never leaf-matches its native group "AresInventory", so this remap
            // is the only way those property handles get names.
            if let Some(known) = resolve_known_subobject_class_path(actor_path, want) {
                if let Some(hit) = self.match_group(Some(known), want) {
                    return hit;
                }
            }
            return actor_path.to_owned();
        }

        // Return the best candidate even if it doesn't match a group -- the
        // export format requires a path, and downstream still gets the raw bits.
        combined
            .or(package_path)
            .unwrap_or_else(|| format!("<unknown:{actor_guid}>"))
    }

    /// Subobject path resolution -- mirrors `ResolveSubobjectExportGroupPath` /
    /// `ResolveSubobjectClassPath` from C#.
    fn resolve_subobject_group_path(&self, header: &ContentBlockHeader) -> String {
        let want = GroupKind::for_block(header);

        // Primary: use class_net_guid path.
        if header.class_net_guid.0 != 0 {
            if let Some(class_path) = self.cache.get_path_by_guid(header.class_net_guid.0) {
                if let Some(hit) = self.match_group(Some(class_path), want) {
                    return hit;
                }
                // UniqueLeafMatch: if class_path is a bare name (no separators),
                // try to find a group whose path ends with ".{class_path}".
                // Mirrors C# ContentBlockPathResolver.UniqueLeafMatch.
                if let Some(g) = self.cache.unique_leaf_match(class_path) {
                    if want.accepts(g) {
                        return g.path.clone();
                    }
                }
                if let Some(known) = resolve_known_subobject_class_path(class_path, want) {
                    if let Some(hit) = self.match_group(Some(known), want) {
                        return hit;
                    }
                }
                return class_path.to_owned();
            }
        }

        // Secondary: use object_net_guid for path lookup.
        if header.object_net_guid.0 != 0 {
            if let Some(obj_path) = self.cache.get_path_by_guid(header.object_net_guid.0) {
                // Try outer path (component -> owning class).
                let outer = self.cache.get_outer_path(header.object_net_guid.0);
                if let Some(hit) = self.match_group(outer, want) {
                    return hit;
                }
                if let Some(hit) = self.match_group(Some(obj_path), want) {
                    return hit;
                }
                // UniqueLeafMatch for object path.
                if let Some(g) = self.cache.unique_leaf_match(obj_path) {
                    if want.accepts(g) {
                        return g.path.clone();
                    }
                }
                // Fallback: known subobject class path table. Blueprint component
                // leaf -> native parent class; applies to RepLayout blocks too,
                // not only ClassNetCache, so e.g. InventoryComponent property
                // blocks resolve to AresInventory.
                if let Some(known) = resolve_known_subobject_class_path(obj_path, want) {
                    if let Some(hit) = self.match_group(Some(known), want) {
                        return hit;
                    }
                }
                return obj_path.to_owned();
            }
        }

        let fallback_guid = if header.class_net_guid.0 != 0 {
            header.class_net_guid.0
        } else {
            header.object_net_guid.0
        };
        format!("<unknown:{fallback_guid}>")
    }

    /// Try every lookup key `candidate` generates and return the canonical path
    /// of the first declared group of the wanted kind.
    ///
    /// One function rather than the seven copies of this loop the C# port grew:
    /// each copy had to remember both which key generator to use and to re-apply
    /// the `_ClassNetCache` guard, and the guard is not decoration -- see
    /// `resolve_actor_group_path`.
    fn match_group(&self, candidate: Option<&str>, want: GroupKind) -> Option<String> {
        let candidate = candidate?;
        want.find(candidate, |key| {
            let group = self.cache.get_group_by_path(key)?;
            want.accepts(group).then(|| group.path.clone())
        })
    }

    /// Determine the package path and archetype path for an actor channel.
    ///
    /// Returns `(package_or_class_path, archetype_path)`. Either may be `None`
    /// if the GUID cache doesn't have the mapping yet.
    ///
    /// The two `to_owned()` calls look removable and are not worth removing:
    /// 5-P replaced them with borrows and measured no change (median 1.580 s vs
    /// 1.590 s over interleaved runs), then reverted. What did move the number
    /// is not calling this at all, which is what the memo does.
    pub(super) fn resolve_actor_package_and_archetype(
        &self,
        channel_index: u32,
        actor_guid: u32,
    ) -> (Option<String>, Option<String>) {
        let archetype_guid =
            match channel_archetype(self.channel_state, channel_index, NetworkGuid(actor_guid)) {
                Some(g) if g.is_valid() => g,
                _ => return (None, None),
            };

        let archetype_path = self
            .cache
            .get_path_by_guid(archetype_guid.0)
            .map(|s| s.to_owned());

        // C# priority: outer path of archetype first (gives the package path).
        let package_path = self
            .cache
            .get_outer_path(archetype_guid.0)
            .map(|s| s.to_owned());

        (package_path, archetype_path)
    }

    /// Combine the package path with the class name from the archetype path.
    ///
    /// Mirrors `TryCreateCombinedCandidate` in C#:
    /// - Extract the leaf of `archetype_path` (after last `/`, `.`, or `:`).
    /// - Strip `Default__` prefix if present to get the class name.
    /// - If `package_path` already ends with `.{class_name}`, return as-is.
    /// - Otherwise append `.{class_name}` to `package_path`.
    pub(super) fn create_combined_candidate(
        &self,
        package_path: Option<&str>,
        archetype_path: Option<&str>,
    ) -> Option<String> {
        let pkg = package_path?;
        let class_name = extract_class_name_from_archetype(archetype_path?)?;

        // Check if package path already ends with the class name.
        if ends_with_class_name(pkg, class_name) {
            return Some(pkg.to_owned());
        }

        let mut combined = String::with_capacity(pkg.len() + 1 + class_name.len());
        combined.push_str(pkg);
        combined.push('.');
        combined.push_str(class_name);
        Some(combined)
    }

    /// Determine function_count for a ClassNetCache block.
    ///
    /// The function count equals `NetFieldExportGroup.len()` for the matching
    /// ClassNetCache group. The C# parser uses
    /// `ReadSerializedInt(FunctionsByHandle.Length)` where `FunctionsByHandle`
    /// is sized to `replayGroup.NetFieldExportsLength`, i.e. the number of
    /// declared export slots in the ClassNetCache group.
    ///
    /// If the group cannot be resolved we return 0, which causes the RPC parser
    /// to skip the bits but NOT silently drop them. The caller still records the
    /// raw payload and the oracle counts this as a stream failure.
    ///
    /// May replace `current_group_path`; see the bare-instance-name branch and
    /// the module docs on why the memo stores the pair.
    fn resolve_function_count(
        &mut self,
        header: &ContentBlockHeader,
        channel_index: u32,
        actor_guid: u32,
    ) -> u32 {
        // Fast path: current_group_path was already resolved to a CNC group.
        if let Some(group) = self.cache.get_group_by_path(&self.current_group_path) {
            if is_class_net_cache(group) {
                return group.len();
            }
        }

        // For subobjects: try class_net_guid with ClassNetCache suffix toggle.
        if header.class_net_guid.0 != 0 {
            if let Some(class_path) = self.cache.get_path_by_guid(header.class_net_guid.0) {
                if let Some(len) = self.class_net_cache_len(class_path) {
                    return len;
                }
            }
        }

        // For actors: try deriving from archetype.
        if header.is_actor {
            let (package_path, archetype_path) =
                self.resolve_actor_package_and_archetype(channel_index, actor_guid);
            if let Some(combined) =
                self.create_combined_candidate(package_path.as_deref(), archetype_path.as_deref())
            {
                if let Some(len) = self.class_net_cache_len(&combined) {
                    return len;
                }
            }
        }

        // Schema-driven fallback: when the resolved path is a bare instance
        // name (no path separators), search the replay's own ClassNetCache
        // groups for one whose leaf matches the instance name after applying
        // Unreal naming conventions.
        //
        // This recovers blocks for static actors (BombDestination_A,
        // WindowShieldA1) and stably-named subobjects (ForceModuleManager,
        // AudDeadeyeVOComponent) that have no archetype GUID or class net GUID
        // on the wire. The capacity comes from the matched group's declared
        // NetFieldExportsLength -- never guessed.
        //
        // The C# reference parser also fails on these same blocks. This lookup
        // goes beyond C# by leveraging the replay's own declared schema.
        if is_bare_instance_name(&self.current_group_path) {
            if let Some(group) = self
                .cache
                .resolve_cnc_for_instance_name(&self.current_group_path)
            {
                let len = group.len();
                // Update current_group_path so downstream field/RPC handle
                // lookups use the correct group. Without this, resolved RPCs
                // would emit handle-indexed names instead of proper field names.
                let resolved = self.channel_state.names.intern(&group.path);
                self.set_current_group_path(resolved);
                return len;
            }
        }

        0
    }

    /// Declared length of the `_ClassNetCache` group `candidate` resolves to.
    fn class_net_cache_len(&self, candidate: &str) -> Option<u32> {
        find_class_net_cache_key(candidate, |key| {
            let group = self.cache.get_group_by_path(key)?;
            is_class_net_cache(group).then(|| group.len())
        })
    }
}

/// Which family of export group a block may bind to.
///
/// The distinction selects both the lookup-key generator and the acceptance
/// test, and the two must move together: a ClassNetCache block that binds to a
/// RepLayout group gets the wrong handle width, which is a decode failure, not
/// a naming one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupKind {
    RepLayout,
    ClassNetCache,
}

impl GroupKind {
    fn for_block(header: &ContentBlockHeader) -> Self {
        if header.has_rep_layout {
            Self::RepLayout
        } else {
            Self::ClassNetCache
        }
    }

    /// Probe each of this kind's lookup keys, in order, until one is accepted.
    ///
    /// The key generators are visitors rather than `Vec<String>` builders, so a
    /// path with no alias -- the common case -- costs no allocation at all.
    fn find<T>(self, path: &str, probe: impl FnMut(&str) -> Option<T>) -> Option<T> {
        match self {
            Self::RepLayout => find_replay_path_key(path, probe),
            Self::ClassNetCache => find_class_net_cache_key(path, probe),
        }
    }

    fn accepts(self, group: &NetFieldExportGroup) -> bool {
        match self {
            Self::RepLayout => true,
            Self::ClassNetCache => is_class_net_cache(group),
        }
    }
}

fn is_class_net_cache(group: &NetFieldExportGroup) -> bool {
    group.path.ends_with(vrf_schema::CLASS_NET_CACHE_SUFFIX)
}

/// A path with no separators and no `<unknown:` marker -- the shape the
/// instance-name resolver is allowed to see.
fn is_bare_instance_name(path: &str) -> bool {
    !path.contains('/') && !path.contains('.') && !path.contains(':') && !path.starts_with('<')
}

/// Extract the class name from an archetype path by taking the leaf and
/// stripping a `Default__` prefix if present.
///
/// Example: `/Game/Characters/AggroBot/AggroBot_PC.Default__AggroBot_PC_C`
/// -> `AggroBot_PC_C`.
fn extract_class_name_from_archetype(archetype_path: &str) -> Option<&str> {
    if archetype_path.is_empty() {
        return None;
    }
    let leaf_start = archetype_path.rfind(['/', '.', ':']).map_or(0, |i| i + 1);
    let leaf = &archetype_path[leaf_start..];
    if leaf.is_empty() {
        return None;
    }
    Some(leaf.strip_prefix("Default__").unwrap_or(leaf))
}

/// Check whether `path` already ends with `.{class_name}` or `:{class_name}`.
fn ends_with_class_name(path: &str, class_name: &str) -> bool {
    let sep_index = path.len().wrapping_sub(class_name.len() + 1);
    if sep_index >= path.len() {
        return false;
    }
    let sep = path.as_bytes()[sep_index];
    (sep == b'.' || sep == b':') && path[sep_index + 1..] == *class_name
}

/// Check whether a path is a "Class Default Object" path (leaf starts with
/// `Default__`). Mirrors `ReplayPath.IsClassDefaultObjectPath`.
fn is_class_default_object_path(path: &str) -> bool {
    let leaf_start = path.rfind(['/', '.', ':']).map_or(0, |i| i + 1);
    path[leaf_start..].starts_with("Default__")
}

/// Look up a known subobject class path by the leaf name of the object path.
///
/// Some subobjects use "stably named" identifiers (no class_net_guid). The
/// replay only tells us the object name; this table provides the class path.
fn resolve_known_subobject_class_path(object_path: &str, want: GroupKind) -> Option<&'static str> {
    let leaf_start = object_path.rfind(['/', '.', ':']).map_or(0, |i| i + 1);
    let leaf = &object_path[leaf_start..];
    KNOWN_SUBOBJECT_CLASS_PATHS
        .iter()
        .find(|(name, _, kind)| *name == leaf && *kind == want)
        .map(|(_, class_path, _)| *class_path)
}

/// An archetype GUID together with the actor it was read for.
///
/// The actor half is what stops a recycled channel number from decoding a new
/// actor under the previous one's schema. Channel indices are reused, the
/// archetype was recorded per channel and never removed, and a *static* actor
/// carries no archetype of its own to displace the stale one -- so resolution
/// read the old GUID first and bound the block to the old class. Nothing fails
/// in that state: the fields get names, the values get types, and the rows are
/// exported under a class the actor never had.
///
/// Destroyed channels are removed; dormancy closes retain the entry because a
/// wake-up for the same actor need not repeat its archetype. The actor stamp is
/// still load-bearing while a dormant entry survives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ChannelArchetype {
    actor: NetworkGuid,
    archetype: NetworkGuid,
}

/// Register an archetype for a channel and, if that changed anything, tell the
/// memo. Called from `on_actor_open`.
pub(super) fn set_channel_archetype(
    state: &mut ChannelState,
    channel_index: u32,
    actor: NetworkGuid,
    archetype: NetworkGuid,
) {
    let entry = ChannelArchetype { actor, archetype };
    if state.archetypes.get(&channel_index) == Some(&entry) {
        return;
    }
    state.archetypes.insert(channel_index, entry);
    state.note_resolution_input_changed();
}

/// The archetype GUID recorded for `channel_index`, but only if it was read for
/// `actor`. See [`ChannelArchetype`].
pub(super) fn channel_archetype(
    state: &ChannelState,
    channel_index: u32,
    actor: NetworkGuid,
) -> Option<NetworkGuid> {
    state
        .archetypes
        .get(&channel_index)
        .filter(|entry| entry.actor == actor)
        .map(|entry| entry.archetype)
}

/// Retire the archetype state owned by a destroyed channel index.
pub(super) fn retire_channel_archetype(state: &mut ChannelState, channel_index: u32) {
    if state.archetypes.remove(&channel_index).is_some() {
        state.note_resolution_input_changed();
    }
}

#[cfg(test)]
mod tests {
    #![allow(unused_must_use)]

    use super::*;
    use crate::sink::RecordBuffers;
    use vrf_net::pipeline::{ActorChannelState, ReplicationSink};
    use vrf_schema::NetGuidCache;

    /// Build a cache holding `groups` as declared export groups and mapping
    /// `guid` to `guid_path`, then run one actor content block through a sink
    /// and report the group path it resolved to.
    fn actor_group_path_for(
        groups: &[&str],
        guid: u32,
        guid_path: &str,
        has_rep_layout: bool,
    ) -> String {
        let mut cache = NetGuidCache::new();
        for (i, path) in groups.iter().enumerate() {
            cache.add_export_group(vrf_schema::NetFieldExportGroup::new(
                (*path).to_owned(),
                i as u32 + 1,
                4,
            ));
        }
        cache.set_net_guid_path(guid, guid_path.to_owned(), None);
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);

        let header = ContentBlockHeader {
            has_rep_layout,
            is_actor: true,
            ..ContentBlockHeader::default()
        };
        // No archetype is registered for this channel, so package/archetype
        // resolution yields nothing and the actor-GUID fallback is the only
        // path left -- which is the case this test is about.
        sink.on_content_block(3, NetworkGuid(guid), &header);
        sink.current_group_path.to_string()
    }

    /// An actor whose own NetGUID path is an exact, unique leaf of a declared
    /// group must reach that group.
    ///
    /// The class path (`resolve_subobject_group_path`, primary) and the
    /// subobject path (same function, secondary) both call `unique_leaf_match`
    /// before falling back to the raw path. The actor-GUID fallback did not, so
    /// `AresWorldSettings` -- an exact unique leaf of
    /// `/Script/ShooterGame.AresWorldSettings` -- shipped as a bare instance
    /// name with every one of its fields unnamed.
    #[test]
    fn an_actor_path_that_is_a_unique_leaf_reaches_its_declared_group() {
        assert_eq!(
            actor_group_path_for(
                &["/Script/ShooterGame.AresWorldSettings"],
                42,
                "AresWorldSettings",
                true,
            ),
            "/Script/ShooterGame.AresWorldSettings",
        );
    }

    /// A Blueprint component whose bare class name differs from its native
    /// replicated group ("InventoryComponent" vs "/Script/ShooterGame.AresInventory")
    /// must still reach the native group via KNOWN_SUBOBJECT_CLASS_PATHS, so its
    /// RepLayout property handles -- `CurrentEquippable` (the spike carrier)
    /// included -- pick up names. Leaf matching cannot do this: the bare
    /// Blueprint leaf is not the native group's leaf.
    #[test]
    fn a_blueprint_component_name_reaches_its_native_parent_group() {
        assert_eq!(
            actor_group_path_for(
                &["/Script/ShooterGame.AresInventory"],
                100,
                "InventoryComponent",
                true,
            ),
            "/Script/ShooterGame.AresInventory",
        );
    }

    /// The pairs read out of the cooked game, spot-checked across the three
    /// shapes they come in: one native class instantiated under several
    /// Blueprint names (`EquippableStateMachineComponent`), two components of
    /// the same class in one Blueprint (`MagazineAmmo` and `ReserveAmmo`, both
    /// `AmmoComponent`), and a class from another module entirely
    /// (`/Script/InputTooling`).
    #[test]
    fn component_names_read_from_the_game_reach_their_native_groups() {
        for (leaf, native) in [
            (
                "ZoomStateMachine",
                "/Script/ShooterGame.EquippableStateMachineComponent",
            ),
            (
                "Gun_StateMachine",
                "/Script/ShooterGame.EquippableStateMachineComponent",
            ),
            ("MagazineAmmo", "/Script/ShooterGame.AmmoComponent"),
            ("ReserveAmmo", "/Script/ShooterGame.AmmoComponent"),
            (
                "CalloutRegionTracker",
                "/Script/ShooterGame.CalloutRegionTrackingComponent",
            ),
            (
                "VisionComponent",
                "/Script/ShooterGame.ShooterCharacterVisionComponent",
            ),
            (
                "PMAimToolingPointsTarget",
                "/Script/InputTooling.AimToolingPointsTargetComponent",
            ),
        ] {
            assert_eq!(
                actor_group_path_for(&[native], 100, leaf, true),
                native,
                "{leaf}",
            );
        }
    }

    /// `AresAttributeSet_2` is the one remap not read from a cooked asset: GAS
    /// builds its attribute sets as runtime subobjects, so no Blueprint lists
    /// them. It rests on the wire instead -- all 116 handles the bare group
    /// uses are a subset of the 122 the native group declares, every one 32
    /// bits wide.
    #[test]
    fn the_second_attribute_set_reaches_the_same_native_group() {
        assert_eq!(
            actor_group_path_for(
                &["/Script/ShooterGame.AresAttributeSet"],
                100,
                "AresAttributeSet_2",
                true,
            ),
            "/Script/ShooterGame.AresAttributeSet",
        );
    }

    /// The remap only fires for names it was given. A Blueprint component this
    /// table says nothing about keeps its bare path rather than being attached
    /// to whichever native group looks close.
    #[test]
    fn an_unlisted_component_name_is_not_remapped() {
        assert_eq!(
            actor_group_path_for(
                &["/Script/ShooterGame.AmmoComponent"],
                100,
                "SomeUnlistedComponent",
                true,
            ),
            "SomeUnlistedComponent",
        );
    }

    /// Ambiguity stays silent. Two declared groups sharing a leaf mark it
    /// `AMBIGUOUS_LEAF`, and the actor keeps its raw path rather than binding
    /// to whichever group happened to be registered first. This is the
    /// property that makes leaf matching a determination and not a guess.
    #[test]
    fn an_ambiguous_actor_leaf_binds_to_nothing() {
        assert_eq!(
            actor_group_path_for(
                &[
                    "/Script/ShooterGame.AresWorldSettings",
                    "/Game/Maps/Ascent.AresWorldSettings",
                ],
                42,
                "AresWorldSettings",
                true,
            ),
            "AresWorldSettings",
        );
    }

    /// A ClassNetCache actor block must NOT be captured by a RepLayout group.
    ///
    /// `resolve_function_count` has its own instance-name resolver
    /// (`resolve_cnc_for_instance_name`) that runs only while
    /// `current_group_path` is still a bare name. Returning a RepLayout group
    /// here would silence that resolver and hand `ReadSerializedInt` the wrong
    /// capacity, so the `_ClassNetCache` guard has to reject the match and
    /// leave the bare name in place.
    #[test]
    fn a_class_net_cache_actor_block_is_not_captured_by_a_rep_layout_leaf() {
        assert_eq!(
            actor_group_path_for(
                &["/Script/ShooterGame.AresWorldSettings"],
                42,
                "AresWorldSettings",
                false,
            ),
            "AresWorldSettings",
        );
    }

    /// Build an `ActorChannelState` for one channel open.
    fn channel_open(channel_index: u32, actor: u32, archetype: u32) -> ActorChannelState {
        ActorChannelState {
            channel_index,
            is_open: true,
            is_dormant: false,
            actor_net_guid: NetworkGuid(actor),
            archetype_net_guid: NetworkGuid(archetype),
            level_guid: NetworkGuid(0),
            spawn_location: None,
            spawn_rotation: None,
            spawn_scale: None,
            spawn_velocity: None,
            open_packet_id: 0,
        }
    }

    /// A reused channel must not decode its new actor under the old one's
    /// schema.
    ///
    /// Channel numbers are recycled. The archetype was recorded per channel and
    /// never removed, so after a dynamic actor closed, the next actor to open on
    /// that channel number inherited its archetype -- and a static actor carries
    /// no archetype of its own to displace it. Resolution then reads the stale
    /// GUID first and binds the block to the previous actor's class, which does
    /// not fail: it names fields, types values and exports them under the wrong
    /// class. That is the wrong-but-plausible output this project ranks below a
    /// crash.
    ///
    /// The archetype is now stamped with the actor it was read for and only
    /// answers for that actor, which is strictly narrower than keying on the
    /// channel alone: the dormancy case, where the *same* actor re-opens without
    /// re-sending its archetype, still resolves exactly as before.
    #[test]
    fn a_reused_channel_does_not_inherit_the_previous_actors_archetype() {
        let mut cache = NetGuidCache::new();
        cache.add_export_group(vrf_schema::NetFieldExportGroup::new(
            "/Game/Effects/Smoke.Smoke_C".to_owned(),
            1,
            4,
        ));
        // GUID 8 is the dynamic actor's archetype; its outer names the class.
        cache.set_net_guid_path(
            8,
            "Default__Smoke_C".to_owned(),
            Some(vrf_schema::NetworkGuid(9)),
        );
        cache.set_net_guid_path(9, "/Game/Effects/Smoke".to_owned(), None);
        // GUID 77 is a static actor that opens later on the same channel and
        // brings no archetype with it.
        cache.set_net_guid_path(77, "SomeStaticProp".to_owned(), None);

        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);

        let header = ContentBlockHeader {
            has_rep_layout: true,
            is_actor: true,
            ..ContentBlockHeader::default()
        };

        // The dynamic actor opens on channel 5 and resolves to its class.
        sink.on_actor_open(&channel_open(5, 42, 8));
        sink.on_content_block(5, NetworkGuid(42), &header);
        assert_eq!(
            &*sink.current_group_path, "/Game/Effects/Smoke.Smoke_C",
            "the dynamic actor must reach its own class",
        );

        // It closes, and the channel number is handed to a static actor.
        sink.on_actor_close(5, NetworkGuid(42), false);
        sink.on_actor_open(&channel_open(5, 77, 0));
        sink.on_content_block(5, NetworkGuid(77), &header);
        assert_eq!(
            &*sink.current_group_path, "SomeStaticProp",
            "the static actor must not be decoded under the previous actor's class",
        );
    }

    /// ...and the dormancy re-open keeps working.
    ///
    /// A channel that closes for dormancy and later re-opens for the *same*
    /// actor may not repeat the archetype. Clearing the archetype on close
    /// would lose that actor's class for the rest of the replay; stamping it
    /// with the actor GUID does not.
    #[test]
    fn the_same_actor_reopening_without_an_archetype_keeps_its_class() {
        let mut cache = NetGuidCache::new();
        cache.add_export_group(vrf_schema::NetFieldExportGroup::new(
            "/Game/Effects/Smoke.Smoke_C".to_owned(),
            1,
            4,
        ));
        cache.set_net_guid_path(
            8,
            "Default__Smoke_C".to_owned(),
            Some(vrf_schema::NetworkGuid(9)),
        );
        cache.set_net_guid_path(9, "/Game/Effects/Smoke".to_owned(), None);

        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);

        let header = ContentBlockHeader {
            has_rep_layout: true,
            is_actor: true,
            ..ContentBlockHeader::default()
        };

        sink.on_actor_open(&channel_open(5, 42, 8));
        sink.on_actor_close(5, NetworkGuid(42), true);
        // Woken: same actor, same channel, no archetype on the wire.
        sink.on_actor_open(&channel_open(5, 42, 0));
        sink.on_content_block(5, NetworkGuid(42), &header);
        assert_eq!(
            &*sink.current_group_path, "/Game/Effects/Smoke.Smoke_C",
            "a dormancy wake must not lose the actor's class",
        );
    }

    /// The memo must not answer for a block whose resolution inputs moved.
    ///
    /// This drives the exact staleness the generation stamps exist to prevent:
    /// the same key resolved twice, with a GUID -> path registration in
    /// between. Without `note_resolution_input_changed` in `register_path` the
    /// second call returns the first call's answer, which is byte movement no
    /// other test in this crate would catch.
    #[test]
    fn a_guid_path_registration_invalidates_the_memo() {
        use vrf_net::net_guid::GuidPathSink;

        let mut cache = NetGuidCache::new();
        cache.add_export_group(vrf_schema::NetFieldExportGroup::new(
            "/Script/ShooterGame.AresWorldSettings".to_owned(),
            1,
            4,
        ));
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();

        let header = ContentBlockHeader {
            has_rep_layout: true,
            is_actor: true,
            ..ContentBlockHeader::default()
        };

        // First pass: GUID 42 has no path at all, so the block falls through to
        // the unknown marker.
        {
            let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
            sink.on_content_block(3, NetworkGuid(42), &header);
            assert_eq!(&*sink.current_group_path, "<unknown:42>");
        }
        // Second pass: the same block, after the wire declared the GUID's path.
        {
            let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);
            sink.register_path(42, "AresWorldSettings", NetworkGuid(0));
            sink.on_content_block(3, NetworkGuid(42), &header);
            assert_eq!(
                &*sink.current_group_path, "/Script/ShooterGame.AresWorldSettings",
                "the memo answered with a resolution its inputs had invalidated"
            );
        }
    }

    /// A repeat registration that changes nothing must not invalidate the memo.
    ///
    /// The whole memo depends on the generation staying still while the replay
    /// re-declares mappings the cache already holds; if every `register_path`
    /// bumped it, the hit rate would collapse to zero and the memo would be
    /// pure overhead.
    #[test]
    fn a_redundant_registration_leaves_the_memo_alone() {
        use vrf_net::net_guid::GuidPathSink;

        let mut cache = NetGuidCache::new();
        let mut channel_state = ChannelState::new();
        let mut records = RecordBuffers::default();
        let mut sink = ExportSink::new(&mut cache, &mut channel_state, &mut records);

        sink.register_path(42, "AresWorldSettings", NetworkGuid(7));
        let after_first = sink.channel_state.resolution_generation;
        sink.register_path(42, "AresWorldSettings", NetworkGuid(7));
        assert_eq!(sink.channel_state.resolution_generation, after_first);

        // A different outer for the same path is a real change: `outer_net_guid`
        // is a column of net_guids.parquet and an input to resolution.
        sink.register_path(42, "AresWorldSettings", NetworkGuid(0));
        assert_ne!(sink.channel_state.resolution_generation, after_first);
    }
}
