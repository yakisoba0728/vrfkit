//! Bare-name resolution: turning the short names the wire carries into the
//! fully-qualified export groups the replay declared.
//!
//! # Why this exists at all
//!
//! A content block frequently identifies its actor or subobject by a bare
//! instance name -- `AresWorldSettings`, `BombDestination_A`,
//! `EquippableStateMachine` -- while the export group that describes its fields
//! is declared under a qualified path such as
//! `/Script/ShooterGame.AresWorldSettings`. Nothing on the wire connects the
//! two. These resolvers close that gap using only the replay's own declared
//! schema, never a hardcoded table of actor or map names.
//!
//! # The ambiguity rule is the whole contract
//!
//! Both resolvers bind a name only when exactly **one** group can claim it. A
//! name claimed by two groups resolves to nothing, deliberately: guessing which
//! one is meant hands `ReadSerializedInt` the wrong field capacity, and it then
//! consumes the wrong number of bits and produces plausible garbage rather than
//! an error. The [`AMBIGUOUS_LEAF`](NetGuidCache::AMBIGUOUS_LEAF) sentinel in
//! `by_leaf` is how that is recorded at registration time.

use crate::cache::NetGuidCache;
use crate::export::NetFieldExportGroup;
use crate::hash::FxHashMap;

/// Whether a name is a qualified path rather than a bare leaf.
///
/// Both resolvers reject qualified paths up front. Written as one pass over the
/// bytes because it runs on the export's hot path -- `unique_leaf_match` alone
/// is entered 174,485 times for the reference replay -- and the three separate
/// `contains` calls it replaces walked the string three times.
///
/// Byte-wise rather than char-wise is safe here: `/`, `.` and `:` are ASCII, and
/// UTF-8 never encodes an ASCII byte inside a multi-byte sequence.
#[inline]
pub(crate) fn has_path_separator(name: &str) -> bool {
    name.bytes().any(|b| matches!(b, b'/' | b'.' | b':'))
}

/// Longest `stem + suffix` candidate built without touching the heap.
///
/// Every observed leaf is far shorter: the longest in the reference replay's
/// schema is 61 bytes, and the longest suffix appended below is 23.
const JOIN_STACK_CAP: usize = 128;

/// Call `f` with `a` and `b` concatenated, without allocating when the result
/// fits in [`JOIN_STACK_CAP`] bytes.
///
/// The resolvers probe `by_leaf` with a name plus one of a few fixed suffixes.
/// Building that with a `String` cost one malloc per probe, which the reference
/// replay pays 149,035 times in `unique_leaf_match` (the count of calls that
/// miss the exact leaf and go on to the suffixed forms) plus 17,318 times
/// across `resolve_cnc_for_instance_name`'s stem attempts. The key is only
/// needed for the duration of a `HashMap::get`, so it never has to own storage
/// that outlives the call.
#[inline]
fn with_joined<R>(a: &str, b: &str, f: impl FnOnce(&str) -> R) -> R {
    let total = a.len() + b.len();
    if total <= JOIN_STACK_CAP {
        let mut buf = [0u8; JOIN_STACK_CAP];
        buf[..a.len()].copy_from_slice(a.as_bytes());
        buf[a.len()..total].copy_from_slice(b.as_bytes());
        // Concatenating two `&str` always yields valid UTF-8, so this branch is
        // the one that runs. Falling through to the owned path on `Err` rather
        // than unwrapping keeps the function total -- there is no input that can
        // make it panic.
        if let Ok(joined) = core::str::from_utf8(&buf[..total]) {
            return f(joined);
        }
    }
    let mut owned = String::with_capacity(total);
    owned.push_str(a);
    owned.push_str(b);
    f(&owned)
}

/// Register the leaf component of a group path in the `by_leaf` index.
///
/// Extracts the trailing class name after the last `.` in the path. If another
/// group already claimed this leaf, marks it as ambiguous.
pub(crate) fn register_leaf(by_leaf: &mut FxHashMap<String, usize>, path: &str, idx: usize) {
    // Extract the leaf: the part after the last '.' in the path.
    let leaf = match path.rfind('.') {
        Some(dot_pos) => &path[dot_pos + 1..],
        None => return, // No dot separator -> not a qualified path, skip.
    };
    if leaf.is_empty() {
        return;
    }
    by_leaf
        .entry(leaf.to_owned())
        .and_modify(|existing| {
            if *existing != idx {
                *existing = NetGuidCache::AMBIGUOUS_LEAF;
            }
        })
        .or_insert(idx);
}

impl NetGuidCache {
    /// Resolve a bare class name to its export group using leaf-suffix matching.
    ///
    /// Mirrors the C# `ContentBlockPathResolver.UniqueLeafMatch`: when a path
    /// has no separators (is a bare name like `AresAttributeSet`), look for a
    /// registered group whose path ends with `.{name}`. Returns the canonical
    /// group only if exactly one such group exists (ambiguous leaves return None).
    ///
    /// Beyond C#'s exact-match logic, also tries common Unreal suffixes:
    /// - `name + "Component"` -- subobject GUIDs often omit the `Component`
    ///   suffix that their export group path includes (e.g. GUID path
    ///   `EquippableStateMachine` -> group `.EquippableStateMachineComponent`).
    /// - `name + "_C"` -- Blueprint-class GUIDs (especially `Comp_*` prefixed)
    ///   map to groups with a `_C` suffix on the leaf.
    ///
    /// This is the bridge between NetGUID paths (often bare class names) and
    /// fully-qualified export group paths.
    #[must_use]
    pub fn unique_leaf_match(&self, bare_name: &str) -> Option<&NetFieldExportGroup> {
        // Only apply to bare names (no path separators).
        if has_path_separator(bare_name) {
            return None;
        }
        // Try exact leaf first.
        if let Some(group) = self.leaf_group(bare_name) {
            return Some(group);
        }
        // Try "name + Component" suffix -- Unreal's most common subobject naming
        // convention stores GUIDs without the suffix but registers export groups
        // with it.
        if let Some(group) = with_joined(bare_name, "Component", |k| self.leaf_group(k)) {
            return Some(group);
        }
        // Try "name + _C" suffix -- Blueprint class GUIDs (Comp_* etc.) register
        // their export group with a _C suffix on the leaf.
        with_joined(bare_name, "_C", |k| self.leaf_group(k))
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
        if has_path_separator(bare_name) {
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

    /// Resolve one `by_leaf` key, rejecting the ambiguity sentinel.
    ///
    /// `usize::MAX` in `by_leaf` means two or more groups share this leaf; the
    /// C# `UniqueLeafMatch` contract is that such a name binds to nothing rather
    /// than to whichever group happened to register first.
    #[inline]
    fn leaf_group(&self, leaf: &str) -> Option<&NetFieldExportGroup> {
        let idx = *self.leaf_index().get(leaf)?;
        if idx == Self::AMBIGUOUS_LEAF {
            return None;
        }
        self.groups().get(idx)
    }

    /// Try ClassNetCache leaf candidates for a given stem.
    ///
    /// Checks `stem_ClassNetCache`, `stemComponent_ClassNetCache`, and
    /// `stem_C_ClassNetCache` in the leaf index.
    fn try_cnc_leaf_candidates(&self, stem: &str) -> Option<&NetFieldExportGroup> {
        // The three suffixes are tried in this order and the first unambiguous
        // hit wins, so a stem that could match under more than one convention
        // resolves the same way it always has.
        for suffix in [
            "_ClassNetCache",
            "Component_ClassNetCache",
            "_C_ClassNetCache",
        ] {
            if let Some(group) = with_joined(stem, suffix, |k| self.lookup_cnc_leaf(k)) {
                return Some(group);
            }
        }
        None
    }

    /// Look up a CNC leaf in `by_leaf`, accepting only unambiguous matches whose
    /// group path actually ends with `_ClassNetCache`.
    fn lookup_cnc_leaf(&self, leaf: &str) -> Option<&NetFieldExportGroup> {
        let group = self.leaf_group(leaf)?;
        if group.path.ends_with("_ClassNetCache") {
            Some(group)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(unused_must_use)]

    use super::*;
    use crate::export::NetFieldExportGroup; // -- UniqueLeafMatch suffix extension tests -------------------------------

    #[test]
    fn unique_leaf_match_exact() {
        let mut cache = NetGuidCache::new();
        cache.add_export_group(NetFieldExportGroup::new(
            "/Script/ShooterGame.AresAttributeSet".into(),
            20,
            3,
        ));
        // Exact leaf match: "AresAttributeSet" -> leaf is "AresAttributeSet".
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
        // Two groups with the same leaf "TestComponent" -> ambiguous.
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
