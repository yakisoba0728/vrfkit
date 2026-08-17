//! Replay path normalisation and alias generation.
//!
//! The engine uses several equivalent representations for the same logical path:
//! with/without a `Default__` prefix on the leaf, with/without `/_Core/` in a
//! `/Game/Characters/` path, and with/without the `_ClassNetCache` suffix. This
//! module enumerates all forms so that consumers can register callbacks against
//! any of them.
//!
//! The rules are **ordinal exact-match** (case-sensitive, no regex, no fuzzy
//! matching), faithfully replicating `Replay.Unreal/Parsing/ReplayPath.cs`.

/// The suffix that marks an export group as an RPC (ClassNetCache) group.
///
/// Public because it is a wire-format discriminator, not an implementation
/// detail: every consumer that has to tell an RPC group from a replicated
/// property group tests for exactly this suffix, and the Python adapter under
/// `tools/` carries its own copy. `crates/vrfkit/tests/adapter_contract.rs`
/// pins the two together, so renaming this constant's VALUE fails the Rust
/// suite instead of silently reclassifying every RPC downstream as a
/// replicated property.
pub const CLASS_NET_CACHE_SUFFIX: &str = "_ClassNetCache";
const CORE_SEGMENT: &str = "/_Core/";
const CHARACTERS_ROOT: &str = "/Game/Characters/";
const DEFAULT_OBJECT_PREFIX: &str = "Default__";

/// Visit every lookup key for an export-group path, in order.
///
/// The first key is always `path` itself, then any valid aliases (the
/// `Default__` prefix toggle, the `/_Core/` substitution). Order matches the
/// C# `ReplayPath.LookupKeys` enumeration.
///
/// # Why a visitor and not a `Vec<String>`
///
/// This used to return one. The sink calls it once per content block -- 608,020
/// times per replay -- and the vector plus its first element were two heap
/// allocations on every call, whether or not an alias existed. Most paths have
/// no alias at all: `default_object_alias` declines anything qualified without
/// the prefix, and `core_alias` declines anything outside `/Game/Characters/`.
/// So the common case allocated twice to hand back a copy of a string the
/// caller already had.
///
/// Handing out `&str` makes that case allocation-free. An alias still costs one
/// `String`, because it is genuinely new text.
pub fn for_each_replay_path_key(path: &str, mut visit: impl FnMut(&str)) {
    visit(path);
    if let Some(alias) = default_object_alias(path) {
        visit(&alias);
    }
    if let Some(alias) = core_alias(path) {
        visit(&alias);
    }
}

/// Visit lookup keys until `probe` accepts one, and return what it gave back.
///
/// The short-circuiting form of [`for_each_replay_path_key`], which is what
/// every resolution site actually wants: try each spelling, stop at the first
/// that resolves. Aliases past the hit are never built.
pub fn find_replay_path_key<T>(path: &str, mut probe: impl FnMut(&str) -> Option<T>) -> Option<T> {
    if let Some(hit) = probe(path) {
        return Some(hit);
    }
    if let Some(alias) = default_object_alias(path) {
        if let Some(hit) = probe(&alias) {
            return Some(hit);
        }
    }
    if let Some(alias) = core_alias(path) {
        if let Some(hit) = probe(&alias) {
            return Some(hit);
        }
    }
    None
}

/// Visit lookup keys including `_ClassNetCache` suffix variations until `probe`
/// accepts one.
///
/// Mirrors `ReplayPath.ClassNetCacheLookupKeys`: for each base key, the key
/// itself, then the key with the suffix removed (if present) or appended (if
/// absent). One scratch buffer serves every toggled spelling instead of a
/// `String` per key.
pub fn find_class_net_cache_key<T>(
    path: &str,
    mut probe: impl FnMut(&str) -> Option<T>,
) -> Option<T> {
    let mut toggled = String::new();
    find_replay_path_key(path, |key| {
        if let Some(hit) = probe(key) {
            return Some(hit);
        }
        toggled.clear();
        match key.strip_suffix(CLASS_NET_CACHE_SUFFIX) {
            // A path that is nothing but the suffix strips to the empty string,
            // which is not a key. The old vector form skipped it too.
            Some("") => return None,
            Some(stripped) => toggled.push_str(stripped),
            None => {
                toggled.push_str(key);
                toggled.push_str(CLASS_NET_CACHE_SUFFIX);
            }
        }
        probe(&toggled)
    })
}

/// Toggle the `Default__` prefix on the leaf of a path.
///
/// - If the path starts with `Default__` (indicating it *is* a bare leaf with
///   the prefix), strip it.
/// - If the path is a bare leaf (no path separators), prepend `Default__`.
/// - Otherwise (has separators but no `Default__` prefix on a sub-path leaf),
///   returns `None`.
fn default_object_alias(path: &str) -> Option<String> {
    if let Some(rest) = path.strip_prefix(DEFAULT_OBJECT_PREFIX) {
        return Some(rest.to_owned());
    }
    // Only apply reverse (add prefix) if the path is a bare leaf (no separators).
    if !path.contains('/') && !path.contains('.') && !path.contains(':') {
        let mut prefixed = String::with_capacity(DEFAULT_OBJECT_PREFIX.len() + path.len());
        prefixed.push_str(DEFAULT_OBJECT_PREFIX);
        prefixed.push_str(path);
        return Some(prefixed);
    }
    None
}

/// Swap `/_Core/` segment with `/` or insert `_Core/` after `/Game/Characters/`.
///
/// Mirrors `ReplayPath.TryGetAlias`.
fn core_alias(path: &str) -> Option<String> {
    // If path contains `/_Core/`, replace first occurrence with `/`.
    if let Some(idx) = path.find(CORE_SEGMENT) {
        let mut alias = String::with_capacity(path.len());
        alias.push_str(&path[..idx]);
        alias.push('/');
        alias.push_str(&path[idx + CORE_SEGMENT.len()..]);
        return Some(alias);
    }
    // If path starts with /Game/Characters/, insert _Core/ after it.
    if let Some(rest) = path.strip_prefix(CHARACTERS_ROOT) {
        let mut alias = String::with_capacity(CHARACTERS_ROOT.len() + 6 + rest.len());
        alias.push_str(CHARACTERS_ROOT);
        alias.push_str("_Core/");
        alias.push_str(rest);
        return Some(alias);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collect what a visitor emits. The assertions below are on the exact
    /// sequence, not on membership: the ORDER is the contract -- it decides
    /// which spelling of an ambiguous path wins a lookup -- and the old
    /// `contains` assertions could not see a reordering at all.
    fn replay_keys(path: &str) -> Vec<String> {
        let mut out = Vec::new();
        for_each_replay_path_key(path, |k| out.push(k.to_owned()));
        out
    }

    /// The same for the ClassNetCache generator, which has no visit-all form
    /// in the public API because nothing needs one -- a probe that never
    /// accepts walks every key.
    fn cnc_keys(path: &str) -> Vec<String> {
        let mut out = Vec::new();
        let hit: Option<()> = find_class_net_cache_key(path, |k| {
            out.push(k.to_owned());
            None
        });
        assert!(hit.is_none());
        out
    }

    #[test]
    fn the_original_path_is_always_the_first_key() {
        assert_eq!(replay_keys("/Game/Test.Test_C"), ["/Game/Test.Test_C"]);
    }

    #[test]
    fn default_prefix_stripped() {
        assert_eq!(
            replay_keys("Default__Test_C"),
            ["Default__Test_C", "Test_C"]
        );
    }

    #[test]
    fn bare_leaf_gains_default_prefix() {
        assert_eq!(replay_keys("Test_C"), ["Test_C", "Default__Test_C"]);
    }

    #[test]
    fn core_segment_removed() {
        assert_eq!(
            replay_keys("/Game/Characters/_Core/Jett/Jett_C"),
            [
                "/Game/Characters/_Core/Jett/Jett_C",
                "/Game/Characters/Jett/Jett_C"
            ]
        );
    }

    #[test]
    fn characters_root_gains_core() {
        assert_eq!(
            replay_keys("/Game/Characters/Jett/Jett_C"),
            [
                "/Game/Characters/Jett/Jett_C",
                "/Game/Characters/_Core/Jett/Jett_C"
            ]
        );
    }

    #[test]
    fn class_net_cache_suffix_toggled_after_each_base_key() {
        // A bare leaf also produces the Default__ alias, and each base key is
        // immediately followed by its toggled spelling.
        assert_eq!(
            cnc_keys("Test_ClassNetCache"),
            [
                "Test_ClassNetCache",
                "Test",
                "Default__Test_ClassNetCache",
                "Default__Test"
            ]
        );
        assert_eq!(
            cnc_keys("Test"),
            [
                "Test",
                "Test_ClassNetCache",
                "Default__Test",
                "Default__Test_ClassNetCache"
            ]
        );
    }

    /// Stripping the suffix off a path that is nothing else leaves the empty
    /// string, which is not a key -- so `_ClassNetCache` contributes only
    /// itself, while its `Default__` alias still strips to something real.
    #[test]
    fn a_path_that_is_only_the_suffix_yields_no_stripped_key() {
        assert_eq!(
            cnc_keys("_ClassNetCache"),
            ["_ClassNetCache", "Default___ClassNetCache", "Default__"]
        );
    }

    /// The visitors replaced two `Vec<String>` builders. Nothing else pins that
    /// they still enumerate the same keys in the same order, and the order is
    /// what decides which spelling wins a lookup -- so the old implementations
    /// live on here as the reference, and every case above plus the awkward
    /// ones are checked against them.
    #[test]
    fn the_visitors_agree_with_the_vector_forms_they_replaced() {
        fn old_replay(path: &str) -> Vec<String> {
            let mut keys = Vec::with_capacity(4);
            keys.push(path.to_owned());
            if let Some(alias) = default_object_alias(path) {
                keys.push(alias);
            }
            if let Some(alias) = core_alias(path) {
                keys.push(alias);
            }
            keys
        }
        fn old_cnc(path: &str) -> Vec<String> {
            let mut keys = Vec::new();
            for key in &old_replay(path) {
                keys.push(key.clone());
                if let Some(stripped) = key.strip_suffix(CLASS_NET_CACHE_SUFFIX) {
                    if !stripped.is_empty() {
                        keys.push(stripped.to_owned());
                    }
                } else {
                    keys.push(format!("{key}{CLASS_NET_CACHE_SUFFIX}"));
                }
            }
            keys
        }

        for path in [
            "/Game/Test.Test_C",
            "Default__Test_C",
            "Test_C",
            "/Game/Characters/_Core/Jett/Jett_C",
            "/Game/Characters/Jett/Jett_C",
            "/Game/Abilities/Grenade.Grenade_C",
            "Test_ClassNetCache",
            "_ClassNetCache",
            "Default___ClassNetCache",
            "/Game/Characters/_Core/Jett/Jett_C_ClassNetCache",
            "Default__",
            "",
            "/",
            "a",
        ] {
            assert_eq!(replay_keys(path), old_replay(path), "replay keys: {path:?}");
            assert_eq!(cnc_keys(path), old_cnc(path), "cnc keys: {path:?}");
        }
    }

    #[test]
    fn no_alias_for_qualified_path_without_core() {
        assert_eq!(
            replay_keys("/Game/Abilities/Grenade.Grenade_C"),
            ["/Game/Abilities/Grenade.Grenade_C"]
        );
    }

    /// The point of the visitor form: a probe that accepts early must stop the
    /// walk, so aliases past the hit are never built.
    #[test]
    fn find_short_circuits_on_the_first_acceptance() {
        let mut seen = Vec::new();
        let hit = find_replay_path_key("Test_C", |k| {
            seen.push(k.to_owned());
            Some(k.len())
        });
        assert_eq!(hit, Some("Test_C".len()));
        assert_eq!(seen, ["Test_C"], "the Default__ alias must not be built");
    }
}
