//! The hasher the cache's internal maps use.
//!
//! # Why not the standard hasher
//!
//! Every map in [`NetGuidCache`](crate::NetGuidCache) is keyed by data the
//! replay supplies, and they are probed on the export's hottest path: the
//! sink resolves a group for each of ~780k content blocks, and each resolution
//! costs several `get_group_by_path` probes plus a `get_path_by_guid` per
//! actor, class and subobject GUID it touches.
//!
//! `std`'s default is SipHash-1-3, which is chosen for HashDoS resistance on
//! attacker-controlled keys in a network service. Two of these maps
//! (`guid_to_path`, `guid_to_outer`, `by_index`) are keyed by a bare `u32`,
//! where SipHash's keying, block setup and finalisation cost far more than the
//! probe they protect. This hasher reduces a `u32` key to one rotate, one XOR
//! and one multiply.
//!
//! # What is given up, and why that is acceptable here
//!
//! This is **not** a HashDoS-resistant hash. A crafted replay could in
//! principle pick GUIDs or paths that collide and drive a map probe quadratic.
//! That is a real and deliberate trade, made because:
//!
//! - the map sizes are bounded by the same replay's own declared counts, which
//!   are already range-checked (`MAX_GUID_ENTRIES`, `MAX_GROUPS`), so the worst
//!   case is bounded work on one local file rather than an unbounded stall in a
//!   shared service; and
//! - this crate parses local files the operator chose to open, not requests
//!   from an untrusted peer.
//!
//! If this ever moves behind a network boundary, revert these maps to
//! `std::collections::HashMap`'s default hasher. The map types inside `cache.rs`
//! are private, so that change stays confined there; the hasher itself is
//! published (`pub mod hash`) because `vrfkit`'s sink makes the same trade for
//! its own locally-sourced, bounded-key maps.
//!
//! # Provenance
//!
//! The mix is rustc's own `FxHasher` (`rustc_hash`), which rustc uses for the
//! same reason: bounded, locally-sourced keys where the cryptographic strength
//! is not buying anything. It is reproduced here rather than taken as a
//! dependency to keep the crate's dependency set at `vrf-bitio` + `thiserror`.

use std::hash::{BuildHasherDefault, Hasher};

/// A `HashMap` using [`FxHasher`].
pub type FxHashMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FxHasher>>;

/// A `HashSet` using [`FxHasher`].
pub type FxHashSet<T> = std::collections::HashSet<T, BuildHasherDefault<FxHasher>>;

/// The multiplier: the fractional bits of the golden ratio scaled to 64 bits.
/// Taken verbatim from `rustc_hash` so the mixing quality is the one that has
/// been exercised by rustc rather than something invented here.
const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

/// A fast, non-cryptographic hasher for locally-sourced keys.
///
/// Deliberately minimal: `write` folds the input 8 bytes at a time and the
/// integer paths bypass the byte loop entirely, which is what makes the `u32`
/// keyed maps cheap.
#[derive(Default, Clone)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        // Rotating before the XOR is what stops the high bits of successive
        // words from cancelling; the multiply then diffuses low bits upward.
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
        while let Some((chunk, tail)) = rest.split_first_chunk::<8>() {
            self.add(u64::from_le_bytes(*chunk));
            rest = tail;
        }
        if let Some((chunk, tail)) = rest.split_first_chunk::<4>() {
            self.add(u64::from(u32::from_le_bytes(*chunk)));
            rest = tail;
        }
        if let Some((chunk, tail)) = rest.split_first_chunk::<2>() {
            self.add(u64::from(u16::from_le_bytes(*chunk)));
            rest = tail;
        }
        if let Some(&byte) = rest.first() {
            self.add(u64::from(byte));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        // Swapping the halves puts the well-mixed high bits where a `HashMap`
        // reads its bucket index from.
        self.hash.rotate_left(20)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::Hash;

    fn hash_of<T: Hash>(value: &T) -> u64 {
        let mut h = FxHasher::default();
        value.hash(&mut h);
        h.finish()
    }

    #[test]
    fn equal_keys_hash_equal() {
        assert_eq!(hash_of(&17u32), hash_of(&17u32));
        assert_eq!(
            hash_of(&"/Script/ShooterGame.AresAttributeSet"),
            hash_of(&"/Script/ShooterGame.AresAttributeSet")
        );
    }

    #[test]
    fn distinct_small_integers_do_not_collide() {
        // The u32 maps are keyed by NetGUIDs, which are small and dense. A mix
        // that collapsed them would turn every probe into a bucket walk, so
        // this is the property that actually matters for this crate.
        let mut seen = std::collections::HashSet::new();
        for guid in 0u32..10_000 {
            assert!(seen.insert(hash_of(&guid)), "collision at {guid}");
        }
    }

    #[test]
    fn distinct_paths_do_not_collide_across_the_reference_shapes() {
        // Real group paths share long prefixes; a hash that only looked at the
        // first word would collide en masse on these.
        let paths = [
            "/Script/ShooterGame.AresAttributeSet",
            "/Script/ShooterGame.AresAbilitySystemComponent",
            "/Script/ShooterGame.AresAbilitySystemComponent_ClassNetCache",
            "/Game/Characters/_Core/Jett/Jett_C",
            "/Game/Characters/_Core/Jett/Jett_C_ClassNetCache",
            "/Game/Maps/Ascent/Ascent",
        ];
        let mut seen = std::collections::HashSet::new();
        for p in paths {
            assert!(seen.insert(hash_of(&p)), "collision on {p}");
        }
    }

    #[test]
    fn byte_slice_length_changes_the_hash() {
        // A trailing-chunk bug that ignored the tail would make these equal.
        assert_ne!(hash_of(&"Ares"), hash_of(&"Ares "));
        assert_ne!(hash_of(&"AresAttribute"), hash_of(&"AresAttributeS"));
    }

    #[test]
    fn works_as_a_hashmap_hasher() {
        let mut map: FxHashMap<u32, &str> = FxHashMap::default();
        for i in 0..1000u32 {
            map.insert(i, "x");
        }
        assert_eq!(map.len(), 1000);
        for i in 0..1000u32 {
            assert_eq!(map.get(&i), Some(&"x"));
        }
        assert_eq!(map.get(&1000), None);
    }
}
