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

const CLASS_NET_CACHE_SUFFIX: &str = "_ClassNetCache";
const CORE_SEGMENT: &str = "/_Core/";
const CHARACTERS_ROOT: &str = "/Game/Characters/";
const DEFAULT_OBJECT_PREFIX: &str = "Default__";

/// Produce all lookup keys for a given export-group path.
///
/// The returned vector always starts with the original `path` and then includes
/// any valid aliases (Default__ prefix toggle, /_Core/ <-> / substitution).
/// Order matches the C# `ReplayPath.LookupKeys` enumeration.
#[must_use]
pub fn replay_path_lookup_keys(path: &str) -> Vec<String> {
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

/// Produce all lookup keys including `_ClassNetCache` suffix variations.
///
/// Mirrors `ReplayPath.ClassNetCacheLookupKeys`: for each base key, yields
/// itself, then the key with the suffix removed (if present) or appended (if
/// absent).
#[must_use]
pub fn class_net_cache_lookup_keys(path: &str) -> Vec<String> {
    let base_keys = replay_path_lookup_keys(path);
    let mut keys = Vec::with_capacity(base_keys.len() * 2);
    for key in &base_keys {
        keys.push(key.clone());
        if let Some(stripped) = key.strip_suffix(CLASS_NET_CACHE_SUFFIX) {
            if !stripped.is_empty() {
                keys.push(stripped.to_owned());
            }
        } else {
            let mut with_suffix = key.clone();
            with_suffix.push_str(CLASS_NET_CACHE_SUFFIX);
            keys.push(with_suffix);
        }
    }
    keys
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

    #[test]
    fn lookup_keys_includes_original() {
        let keys = replay_path_lookup_keys("/Game/Test.Test_C");
        assert!(keys.contains(&"/Game/Test.Test_C".to_owned()));
    }

    #[test]
    fn default_prefix_stripped() {
        let keys = replay_path_lookup_keys("Default__Test_C");
        assert!(keys.contains(&"Default__Test_C".to_owned()));
        assert!(keys.contains(&"Test_C".to_owned()));
    }

    #[test]
    fn bare_leaf_gains_default_prefix() {
        let keys = replay_path_lookup_keys("Test_C");
        assert!(keys.contains(&"Test_C".to_owned()));
        assert!(keys.contains(&"Default__Test_C".to_owned()));
    }

    #[test]
    fn core_segment_removed() {
        let keys = replay_path_lookup_keys("/Game/Characters/_Core/Jett/Jett_C");
        assert!(keys.contains(&"/Game/Characters/Jett/Jett_C".to_owned()));
    }

    #[test]
    fn characters_root_gains_core() {
        let keys = replay_path_lookup_keys("/Game/Characters/Jett/Jett_C");
        assert!(keys.contains(&"/Game/Characters/_Core/Jett/Jett_C".to_owned()));
    }

    #[test]
    fn class_net_cache_suffix_toggled() {
        let keys = class_net_cache_lookup_keys("Test_ClassNetCache");
        assert!(keys.contains(&"Test_ClassNetCache".to_owned()));
        assert!(keys.contains(&"Test".to_owned()));

        let keys2 = class_net_cache_lookup_keys("Test");
        assert!(keys2.contains(&"Test".to_owned()));
        assert!(keys2.contains(&"Test_ClassNetCache".to_owned()));
    }

    #[test]
    fn no_alias_for_qualified_path_without_core() {
        // A fully-qualified path without /_Core/ or /Game/Characters/ prefix
        // should not generate a core alias.
        let keys = replay_path_lookup_keys("/Game/Abilities/Grenade.Grenade_C");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0], "/Game/Abilities/Grenade.Grenade_C");
    }
}
