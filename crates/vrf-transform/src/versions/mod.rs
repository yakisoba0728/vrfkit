//! Per-build transform definitions.
//!
//! Each build supplies exactly two constants and three word functions. Every
//! other moving part -- PRNG, staging, tail XOR -- is shared, so a diff between
//! two builds shows precisely what Riot rotated.
//!
//! ## Observed shape of a build change
//!
//! | build | seed addend | offset | offset sign | S-box |
//! |---|---|---|---|---|
//! | release-12.10 | `0x12fd0ee5` | `0x1b` | subtract | no |
//! | release-12.11 | `0x409d36a3` | `0x23` | **add** | no |
//! | release-13.00 | `0x2949b6ef` | `0x11` | subtract | yes |
//! | release-13.01 | `0xe62fcd5c` | `0x24` | subtract | no |
//! | release-13.02 | `0x9e81a37c` | `0x04` | subtract | yes |
//!
//! In all five, `TAIL_XOR == SEED_ADDEND & 0xff`. That is asserted per version
//! rather than assumed, so a future build that breaks the pattern fails a test
//! instead of silently corrupting the final partial byte of every payload.
//!
//! ## One file per build
//!
//! The five `impl`s are deliberately kept in separate files. They are near-
//! identical in shape and differ only in the order of a handful of bit
//! primitives, which is exactly the situation where a copy-paste error is
//! invisible in review; a per-build file makes `git log` on one build show only
//! that build's history. The `distinct_builds_produce_distinct_output` test in
//! the crate root is the backstop that catches such an error anyway.

use crate::helpers::initial_prng_a;

mod v12_10;
mod v12_11;
mod v13_00;
mod v13_01;
mod v13_02;

pub use v12_10::V12_10;
pub use v12_11::V12_11;
pub use v13_00::V13_00;
pub use v13_01::V13_01;
pub use v13_02::V13_02;

/// One build's payload transform.
///
/// Implemented as a trait with associated constants so the driver monomorphises:
/// there is no virtual dispatch inside the per-word loops, which run once per
/// 8 bytes of every content block (~780k blocks per replay).
pub trait SeededTransform {
    /// Replay branch string this transform decodes, e.g. `++Ares-Core+release-13.01`.
    const BRANCH: &'static str;
    /// Added to the seed when deriving the first PRNG lane.
    const SEED_ADDEND: u32;
    /// Offset applied to the raw seed when deriving the first PRNG lane.
    const INIT_A_OFFSET: u32;
    /// Whether the offset is added (`true`) or subtracted (`false`).
    const ADD_OFFSET: bool = false;
    /// XORed into the final partial byte alongside the keystream byte.
    const TAIL_XOR: u8;

    /// Seed the first PRNG lane.
    #[must_use]
    fn initial_prng_a(seed: u32) -> u64 {
        initial_prng_a(
            seed,
            Self::SEED_ADDEND,
            Self::INIT_A_OFFSET,
            Self::ADD_OFFSET,
        )
    }

    /// Transform one aligned 64-bit word.
    #[must_use]
    fn word64(value: u64, state: u32) -> u64;
    /// Transform one aligned 32-bit word.
    #[must_use]
    fn word32(value: u32, state: u32) -> u32;
    /// Transform one byte.
    #[must_use]
    fn byte(value: u8, state: u32) -> u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(branch, seed addend, init-a offset, adds offset, tail xor)` for every
    /// registered build.
    ///
    /// Collected at runtime rather than compared as associated constants: a
    /// `assert_eq!` between two consts folds to a tautology that the compiler (and
    /// clippy) can see through, which defeats the point of checking it.
    fn build_table() -> Vec<(&'static str, u32, u32, bool, u8)> {
        fn row<T: SeededTransform>() -> (&'static str, u32, u32, bool, u8) {
            (
                T::BRANCH,
                T::SEED_ADDEND,
                T::INIT_A_OFFSET,
                T::ADD_OFFSET,
                T::TAIL_XOR,
            )
        }
        vec![
            row::<V12_10>(),
            row::<V12_11>(),
            row::<V13_00>(),
            row::<V13_01>(),
            row::<V13_02>(),
        ]
    }

    /// Across every known build the tail XOR byte is the low byte of the seed
    /// addend. Encoding that as a test (not as a derivation) means a future build
    /// that breaks the pattern is caught here rather than corrupting the last
    /// partial byte of every payload it decodes.
    #[test]
    fn tail_xor_is_low_byte_of_seed_addend() {
        for (branch, seed_addend, _, _, tail_xor) in build_table() {
            assert_eq!(
                tail_xor,
                (seed_addend & 0xff) as u8,
                "{branch}: TAIL_XOR should be the low byte of SEED_ADDEND {seed_addend:#010x}"
            );
        }
    }

    #[test]
    fn only_12_11_adds_the_offset() {
        let adding: Vec<&str> = build_table()
            .into_iter()
            .filter(|row| row.3)
            .map(|row| row.0)
            .collect();
        assert_eq!(adding, vec![V12_11::BRANCH]);
    }

    #[test]
    fn build_constants_are_all_distinct() {
        // Two builds sharing a seed addend would almost certainly mean a
        // copy-paste error in a newly added transform.
        let table = build_table();
        for (i, a) in table.iter().enumerate() {
            for b in &table[i + 1..] {
                assert_ne!(a.1, b.1, "{} and {} share a seed addend", a.0, b.0);
            }
        }
    }
}
