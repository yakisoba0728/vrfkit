//! Memo of the per-row name lookups: which declared group an interned group
//! path names, and the pooled `Arc<str>` of each handle's name. Without it
//! every field row hashed its group path into `NetGuidCache::by_path` and its
//! name into the [`NameInterner`], and every RPC parameter row also formatted
//! and hashed `"Function.Param"`.
//!
//! Neither half can serve a stale answer. The group position is stamped with
//! `NetGuidCache::schema_generation`, which every path-to-position change
//! bumps; the field itself is read live from the cache, so a field set after
//! the memo was filled is seen. A remembered name is returned only after its
//! text is compared with the name the cache holds now, so a renamed field
//! re-interns instead of shipping the old spelling. What the memo saves is the
//! hashing, not the lookup of the current value.
//!
//! The overlay's resolution is remembered the same way: it is a pure function
//! of the group path (the entry), the handle (the slot), the field name and the
//! checksum (both compared before a remembered resolution is used), against
//! the static table.

use std::sync::Arc;

use vrf_decode::{GroupHashState, OverlayResolution, resolve_overlay};
use vrf_schema::{FxHashMap, NetGuidCache};

use super::TABLE;
use super::intern::{NameInterner, put};

/// Group entries held before the memo starts over. Interned paths number in
/// the hundreds (475 on the reference replay); past the interner's cap every
/// block's path is a fresh `Arc`, and this bound is what stops the memo
/// growing with them.
const MAX_GROUPS: usize = 4_096;

/// Handles remembered per group; a handle past it is answered without the
/// memo. Declared slot counts are far below it, but the handle is off the wire.
const MAX_HANDLES: usize = 1_024;

#[derive(Debug, Clone, Default)]
pub(super) struct FieldNameMemo {
    schema_generation: u64,
    /// Keyed by the address of the interned path's text. The entry holds the
    /// `Arc` itself, so the address cannot be freed and reused by other text
    /// while its entry exists.
    groups: FxHashMap<usize, GroupNames>,
}

#[derive(Debug, Clone)]
pub(super) struct GroupNames {
    _path: Arc<str>,
    /// Position of the group `path` names in `NetGuidCache::groups`.
    group: Option<usize>,
    /// The pooled handle of each field's bare name.
    names: Vec<Option<Arc<str>>>,
    /// The pooled handle of each RPC parameter's `"Function.Param"` name.
    qualified: Vec<Option<Arc<str>>>,
    /// The overlay resolution at each handle, with the inputs it was made for.
    overlay: Vec<Option<RememberedResolution>>,
}

#[derive(Debug, Clone)]
struct RememberedResolution {
    field_name: Option<Arc<str>>,
    checksum: Option<u32>,
    resolution: OverlayResolution,
}

impl FieldNameMemo {
    /// The entry for `path`, created (and the group looked up by text) on first
    /// use and whenever the cache's schema generation has moved.
    pub(super) fn group(&mut self, cache: &NetGuidCache, path: &Arc<str>) -> &mut GroupNames {
        let generation = cache.schema_generation();
        if self.schema_generation != generation || self.groups.len() >= MAX_GROUPS {
            self.groups.clear();
            self.schema_generation = generation;
        }
        let key = Arc::as_ptr(path).cast::<u8>() as usize;
        self.groups.entry(key).or_insert_with(|| GroupNames {
            _path: Arc::clone(path),
            group: cache.group_index_by_path(path),
            names: Vec::new(),
            qualified: Vec::new(),
            overlay: Vec::new(),
        })
    }

    /// `resolve_overlay` against the static table for a field of the group
    /// `path` names; `group_state` must be `path`'s hash state.
    pub(super) fn resolve_overlay(
        &mut self,
        cache: &NetGuidCache,
        path: &Arc<str>,
        group_state: GroupHashState,
        field_name: Option<&str>,
        handle: u32,
        checksum: Option<u32>,
    ) -> OverlayResolution {
        let resolve = || resolve_overlay(&TABLE, path, group_state, field_name, handle, checksum);
        let entry = self.group(cache, path);
        let Some(slot) = slot(&mut entry.overlay, handle) else {
            return resolve();
        };
        if let Some(hit) = slot
            .as_ref()
            .filter(|hit| hit.checksum == checksum && hit.field_name.as_deref() == field_name)
        {
            return hit.resolution;
        }
        let resolution = resolve();
        *slot = Some(RememberedResolution {
            field_name: field_name.map(Arc::from),
            checksum,
            resolution,
        });
        resolution
    }
}

impl GroupNames {
    /// Position of this path's group in `NetGuidCache::groups`, as
    /// `NetGuidCache::group_index_by_path` returned it under this generation.
    pub(super) fn group_index(&self) -> Option<usize> {
        self.group
    }

    /// `name` pooled, for the field at `handle`.
    pub(super) fn name(&mut self, names: &mut NameInterner, handle: u32, name: &str) -> Arc<str> {
        let Some(slot) = slot(&mut self.names, handle) else {
            return names.intern(name);
        };
        if let Some(hit) = slot.as_ref().filter(|hit| ***hit == *name) {
            return Arc::clone(hit);
        }
        let pooled = names.intern(name);
        *slot = Some(Arc::clone(&pooled));
        pooled
    }

    /// `"{function}.{param}"` pooled, for the parameter at `handle`.
    pub(super) fn qualified(
        &mut self,
        names: &mut NameInterner,
        handle: u32,
        function: &str,
        param: &str,
    ) -> Arc<str> {
        let build = |names: &mut NameInterner| {
            names.intern_fmt(|out| put(out, format_args!("{function}.{param}")))
        };
        let Some(slot) = slot(&mut self.qualified, handle) else {
            return build(names);
        };
        if let Some(hit) = slot
            .as_ref()
            .filter(|hit| is_qualified(hit, function, param))
        {
            return Arc::clone(hit);
        }
        let pooled = build(names);
        *slot = Some(Arc::clone(&pooled));
        pooled
    }
}

/// `handle`'s slot in `slots`, grown on demand; `None` past [`MAX_HANDLES`].
fn slot<T: Clone>(slots: &mut Vec<Option<T>>, handle: u32) -> Option<&mut Option<T>> {
    let index = usize::try_from(handle).ok().filter(|&i| i < MAX_HANDLES)?;
    if slots.len() <= index {
        slots.resize(index + 1, None);
    }
    slots.get_mut(index)
}

/// Whether `text` is exactly `"{function}.{param}"`.
fn is_qualified(text: &str, function: &str, param: &str) -> bool {
    text.len() == function.len() + 1 + param.len()
        && text.starts_with(function)
        && text[function.len()..].starts_with('.')
        && text.ends_with(param)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vrf_schema::{NetFieldExport, NetFieldExportGroup};

    fn cache_with(path: &str, index: u32, fields: &[(u32, &str)]) -> NetGuidCache {
        let mut cache = NetGuidCache::new();
        let mut group = NetFieldExportGroup::new(path.to_owned(), index, 8);
        for &(handle, name) in fields {
            assert!(group.set_field(NetFieldExport {
                handle,
                compatible_checksum: 0,
                name: name.to_owned(),
            }));
        }
        cache.add_export_group(group).unwrap();
        cache
    }

    /// A remembered name is compared with the cache's current text: a rename
    /// re-interns rather than shipping the old spelling.
    #[test]
    fn a_renamed_field_is_not_served_its_old_name() {
        let path: Arc<str> = Arc::from("/Script/Game.Thing");
        let mut interner = NameInterner::default();
        let mut memo = FieldNameMemo::default();
        let cache = cache_with(&path, 1, &[(2, "Health")]);
        let entry = memo.group(&cache, &path);
        assert_eq!(entry.group_index(), Some(0));
        assert_eq!(&*entry.name(&mut interner, 2, "Health"), "Health");
        assert_eq!(&*entry.name(&mut interner, 2, "Shield"), "Shield");
        assert_eq!(
            &*entry.qualified(&mut interner, 2, "Fire", "Damage"),
            "Fire.Damage"
        );
        assert_eq!(
            &*entry.qualified(&mut interner, 2, "Fire", "Amount"),
            "Fire.Amount"
        );
        assert_eq!(
            &*entry.qualified(&mut interner, 2, "Fir", "e.Amount"),
            "Fir.e.Amount"
        );
        // Past the remembered handles the answer is still right.
        assert_eq!(&*entry.name(&mut interner, u32::MAX, "Far"), "Far");
    }

    /// A remembered resolution is served only for the name and checksum it was
    /// made for: `Owner` resolves by name (an engine object reference), an
    /// unknown name does not, and the same handle must not carry one to the
    /// other.
    #[test]
    fn a_remembered_resolution_is_only_reused_for_its_own_inputs() {
        let path: Arc<str> = Arc::from("/Script/Game.Thing");
        let state = vrf_decode::group_hash_state(&path);
        let cache = NetGuidCache::new();
        let mut memo = FieldNameMemo::default();
        let inputs = [
            (Some("Owner"), None),
            (Some("NotAFieldAnywhere"), None),
            (Some("Owner"), None),
            (None, None),
            (Some("NotAFieldAnywhere"), Some(1_435_614_478)),
            (Some("NotAFieldAnywhere"), None),
        ];
        let direct: Vec<_> = (inputs.iter())
            .map(|&(name, checksum)| resolve_overlay(&TABLE, &path, state, name, 3, checksum))
            .collect();
        assert_ne!(direct[0], direct[1], "the inputs must resolve differently");
        assert_ne!(
            direct[4], direct[5],
            "the checksum must change the resolution"
        );
        for (&(name, checksum), want) in inputs.iter().zip(&direct) {
            let got = memo.resolve_overlay(&cache, &path, state, name, 3, checksum);
            assert_eq!(got, *want, "{name:?} / {checksum:?}");
        }
    }

    /// A schema change discards the remembered position, so a path the cache
    /// moved is looked up again.
    #[test]
    fn a_schema_change_drops_the_remembered_group() {
        let path: Arc<str> = Arc::from("/Script/Game.Thing");
        let mut memo = FieldNameMemo::default();
        let mut cache = NetGuidCache::new();
        assert_eq!(memo.group(&cache, &path).group_index(), None);
        let group = NetFieldExportGroup::new(path.to_string(), 1, 4);
        cache.add_export_group(group).unwrap();
        assert_eq!(memo.group(&cache, &path).group_index(), Some(0));
    }
}
