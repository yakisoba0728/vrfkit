//! A string pool for the two name columns of `fields.parquet`.
//!
//! # Why
//!
//! The reference replay emits 1,246,812 field rows and the Parquet writer used
//! to buffer 131,072 of them before flushing a row group. With `String` columns
//! that was up to three heap allocations per row and ~393,000 live allocations
//! at the flush peak -- while the whole replay only ever names **475** distinct
//! group paths and 4,557 distinct field names between them. Interning replaces
//! the allocation with a refcount increment and makes the buffered rows share
//! one copy of each name.
//!
//! The pool is not a cache in front of a slow computation: `intern` still hashes
//! the string it is given. What it buys is the allocation, the memcpy, and the
//! retained bytes -- not a lookup.
//!
//! Arrow never sees the `Arc`. The dictionary builders are fed `&str` exactly as
//! before, so the value sequence per row group, the dictionary and the encoded
//! bytes are unchanged.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::Arc;

/// Ceiling on pooled names.
///
/// Not a memory tuning knob -- a bound on wire-driven input. An RPC parameter
/// whose handle the group does not name is emitted as `"{function}._h{handle}"`
/// and the handle is `IntPacked - 1` straight off the wire, so a corrupt or
/// unknown-build payload can mint unbounded distinct names. Past the cap
/// `intern` still returns a correct `Arc<str>`, just an unshared one, so the
/// only thing lost is sharing.
///
/// Measured on 02d4d478 with a throwaway instrumented build: 1,449,542 intern
/// calls over the export, pooling 4,557 distinct names. The cap is an order of
/// magnitude above that -- headroom for a longer match or a different game
/// build, and a ceiling for a corrupt one.
const MAX_POOLED_NAMES: usize = 65_536;

/// Pool of interned names plus the scratch buffer used to build them.
///
/// The scratch buffer lives here rather than at the call sites so that
/// [`Self::intern_fmt`] can build a name and pool it without the caller
/// allocating a `String` that is thrown away one line later.
#[derive(Debug, Clone, Default)]
pub struct NameInterner {
    pool: HashSet<Arc<str>>,
    scratch: String,
}

impl NameInterner {
    /// Pool `s` and return the shared handle.
    pub fn intern(&mut self, s: &str) -> Arc<str> {
        if let Some(existing) = self.pool.get(s) {
            return Arc::clone(existing);
        }
        let interned: Arc<str> = Arc::from(s);
        if self.pool.len() < MAX_POOLED_NAMES {
            self.pool.insert(Arc::clone(&interned));
        }
        interned
    }

    /// Build a name with `f` into the internal scratch buffer, then pool it.
    ///
    /// This is the allocation-free path for the composed names -- RPC
    /// parameters (`"Function.Param"`), array leaves (`"Rounds[0].Damage"`) and
    /// struct-blob members -- which together are the majority of rows. Building
    /// them with `format!` would allocate once per row and then throw the
    /// allocation away on a pool hit.
    pub fn intern_fmt(&mut self, f: impl FnOnce(&mut String)) -> Arc<str> {
        // Split the borrow: `f` writes into `scratch` while `pool` is untouched.
        let Self { pool, scratch } = self;
        scratch.clear();
        f(scratch);
        if let Some(existing) = pool.get(scratch.as_str()) {
            return Arc::clone(existing);
        }
        let interned: Arc<str> = Arc::from(scratch.as_str());
        if pool.len() < MAX_POOLED_NAMES {
            pool.insert(Arc::clone(&interned));
        }
        interned
    }

    /// `intern_fmt` for the common `"{a}{sep}{b}"` shape.
    ///
    /// Spelled out rather than left to `format_args!` at each call site so the
    /// three-argument join reads the same everywhere it appears.
    pub fn intern_join(&mut self, a: &str, sep: char, b: &str) -> Arc<str> {
        self.intern_fmt(|out| {
            out.push_str(a);
            out.push(sep);
            out.push_str(b);
        })
    }

    /// Number of distinct names currently pooled. Diagnostic only.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.pool.len()
    }
}

/// Write `args` into `out`. `write!` into a `String` cannot fail, and the
/// alternative -- `let _ = write!(...)` at a dozen call sites -- reads as if a
/// failure were being ignored.
pub fn put(out: &mut String, args: std::fmt::Arguments<'_>) {
    out.write_fmt(args)
        .expect("formatting into a String cannot fail");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two interns of equal text must share one allocation -- that sharing is
    /// the entire point, and `Arc::ptr_eq` is the only way to observe it.
    #[test]
    fn equal_names_share_one_allocation() {
        let mut interner = NameInterner::default();
        let a = interner.intern("/Script/ShooterGame.ShooterCharacter");
        let b = interner.intern("/Script/ShooterGame.ShooterCharacter");
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(interner.len(), 1);
    }

    /// The built-in-place path must agree with the plain one, including sharing.
    #[test]
    fn a_formatted_name_pools_with_its_plain_twin() {
        let mut interner = NameInterner::default();
        let plain = interner.intern("Fire.Damage");
        let built = interner.intern_join("Fire", '.', "Damage");
        assert_eq!(&*built, "Fire.Damage");
        assert!(Arc::ptr_eq(&plain, &built));
        assert_eq!(interner.len(), 1);
    }

    /// Past the cap the pool stops growing but `intern` keeps returning the
    /// right text. A wire-driven name storm must cost sharing, never
    /// correctness.
    #[test]
    fn the_pool_stops_growing_but_never_stops_being_correct() {
        let mut interner = NameInterner::default();
        for i in 0..(MAX_POOLED_NAMES + 64) {
            let name = interner.intern_fmt(|out| put(out, format_args!("Fn._h{i}")));
            assert_eq!(&*name, format!("Fn._h{i}"));
        }
        assert_eq!(interner.len(), MAX_POOLED_NAMES);
        // A name minted after the cap is still correct, just unshared.
        let over = interner.intern("a name the pool never saw");
        assert_eq!(&*over, "a name the pool never saw");
    }
}
