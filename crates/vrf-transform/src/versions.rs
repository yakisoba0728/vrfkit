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

use crate::helpers::{
    initial_prng_a, reverse_bits_u8, reverse_bits_u32, reverse_bits64_without_final_16bit_swap,
    substitute_bytes_u32, substitute_bytes_u64, swap_adjacent_bits_u8, swap_adjacent_bits_u32,
    swap_adjacent_bits_u64,
};
use crate::sbox::{SBOX_8, SBOX_32, SBOX_64};

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

/// `++Ares-Core+release-12.10`
pub struct V12_10;

impl SeededTransform for V12_10 {
    const BRANCH: &'static str = "++Ares-Core+release-12.10";
    const SEED_ADDEND: u32 = 0x12fd_0ee5;
    const INIT_A_OFFSET: u32 = 0x1b;
    const TAIL_XOR: u8 = 0xe5;

    fn word64(mut v: u64, state: u32) -> u64 {
        let ror4 = state.rotate_right(4);
        let ror5 = state.rotate_right(5);
        let ror6 = state.rotate_right(6);
        let ror8 = state.rotate_right(8);
        v = v.rotate_right((ror8 % 63) + 1);
        v = swap_adjacent_bits_u64(v);
        v = v.wrapping_sub(u64::from(ror6));
        v = v.rotate_right((ror5 % 63) + 1);
        // `!(ror4 as u64)` -- the reference NOTs the *zero-extended* value, so
        // the upper 32 bits become ones. Negating before widening would differ.
        swap_adjacent_bits_u64(v ^ !u64::from(ror4))
    }

    fn word32(mut v: u32, state: u32) -> u32 {
        let rot4 = state.rotate_left(4);
        let rot5 = state.rotate_left(5);
        let rot6 = state.rotate_left(6);
        let rot8 = state.rotate_left(8);
        v = v.rotate_right((rot8 % 31) + 1);
        v = swap_adjacent_bits_u32(v);
        v = v.wrapping_sub(rot6);
        v = v.rotate_right((rot5 % 31) + 1);
        // No complement in the 32-bit lane, unlike word64.
        swap_adjacent_bits_u32(v ^ rot4)
    }

    fn byte(mut v: u8, state: u32) -> u8 {
        let addend1 = state.wrapping_mul(0x31) as u8;
        let addend2 = state.wrapping_mul(0x29) as u8;
        v = v.rotate_right((state.wrapping_mul(0x0cc6_db61) % 7) + 1);
        v = swap_adjacent_bits_u8(v);
        v = v.wrapping_sub(addend2);
        v = v.rotate_right((state.wrapping_mul(0x0002_751b) % 7) + 1);
        swap_adjacent_bits_u8(v ^ addend1)
    }
}

/// `++Ares-Core+release-12.11`
pub struct V12_11;

impl SeededTransform for V12_11 {
    const BRANCH: &'static str = "++Ares-Core+release-12.11";
    const SEED_ADDEND: u32 = 0x409d_36a3;
    const INIT_A_OFFSET: u32 = 0x23;
    /// The only known build that adds instead of subtracting.
    const ADD_OFFSET: bool = true;
    const TAIL_XOR: u8 = 0xa3;

    fn word64(mut v: u64, state: u32) -> u64 {
        let ror2 = state.rotate_right(2);
        let ror3 = state.rotate_right(3);
        let ror4 = state.rotate_right(4);
        let ror6 = state.rotate_right(6);
        let ror8 = state.rotate_right(8);
        v = v.rotate_right((ror8 % 63) + 1);
        v = swap_adjacent_bits_u64(v);
        v = v.wrapping_add(u64::from(ror6));
        v = reverse_bits64_without_final_16bit_swap(v);
        v = v.wrapping_sub(u64::from(ror4));
        v = v.wrapping_sub(u64::from(ror3));
        v = v.wrapping_sub(u64::from(ror2));
        swap_adjacent_bits_u64(v)
    }

    fn word32(mut v: u32, state: u32) -> u32 {
        let rol2 = state.rotate_left(2);
        let rol3 = state.rotate_left(3);
        let rol4 = state.rotate_left(4);
        let rol6 = state.rotate_left(6);
        let rol8 = state.rotate_left(8);
        v = v.rotate_right((rol8 % 31) + 1);
        v = swap_adjacent_bits_u32(v);
        v = v.wrapping_add(rol6);
        v = reverse_bits_u32(v);
        v = v.wrapping_sub(rol4);
        v = v.wrapping_sub(rol3);
        v = v.wrapping_sub(rol2);
        swap_adjacent_bits_u32(v)
    }

    fn byte(mut v: u8, state: u32) -> u8 {
        let state_byte = state as u8;
        let rotate_input = state.wrapping_mul(0x0cc6_db61);
        v = v.rotate_right((rotate_input % 7) + 1);
        v = swap_adjacent_bits_u8(v);
        v = v.wrapping_add(state_byte.wrapping_mul(0x29));
        v = reverse_bits_u8(v);
        v = v.wrapping_add(state_byte.wrapping_mul(0x23));
        swap_adjacent_bits_u8(v)
    }
}

/// `++Ares-Core+release-13.00`
pub struct V13_00;

impl SeededTransform for V13_00 {
    const BRANCH: &'static str = "++Ares-Core+release-13.00";
    const SEED_ADDEND: u32 = 0x2949_b6ef;
    const INIT_A_OFFSET: u32 = 0x11;
    const TAIL_XOR: u8 = 0xef;

    fn word64(mut v: u64, state: u32) -> u64 {
        let ror1 = state.rotate_right(1);
        let ror3 = state.rotate_right(3);
        let ror6 = state.rotate_right(6);
        let ror8 = state.rotate_right(8);
        v = v.wrapping_add(u64::from(ror8));
        v = reverse_bits64_without_final_16bit_swap(v);
        v = v.wrapping_add(u64::from(ror6)) ^ u64::from(ror3);
        v = substitute_bytes_u64(v, &SBOX_64);
        v.rotate_right((ror1 % 63) + 1)
    }

    fn word32(mut v: u32, state: u32) -> u32 {
        let rol1 = state.rotate_left(1);
        let rol3 = state.rotate_left(3);
        let rol6 = state.rotate_left(6);
        let rol8 = state.rotate_left(8);
        v = v.wrapping_add(rol8);
        v = reverse_bits_u32(v);
        // Complement present here but not in word64; `~` binds tighter than `^`.
        v = !v.wrapping_add(rol6) ^ rol3;
        v = substitute_bytes_u32(v, &SBOX_32);
        v.rotate_right((rol1 % 31) + 1)
    }

    fn byte(mut v: u8, state: u32) -> u8 {
        let mix = state.wrapping_mul(0x533) as u8;
        v = v.wrapping_add(mix.wrapping_mul(0x1b));
        v = reverse_bits_u8(v);
        v = !v.wrapping_add(mix.wrapping_mul(0x33)) ^ mix;
        v = SBOX_8[v as usize];
        v.rotate_right((state.wrapping_mul(0x0b) % 7) + 1)
    }
}

/// `++Ares-Core+release-13.01`
pub struct V13_01;

impl SeededTransform for V13_01 {
    const BRANCH: &'static str = "++Ares-Core+release-13.01";
    const SEED_ADDEND: u32 = 0xe62f_cd5c;
    const INIT_A_OFFSET: u32 = 0x24;
    const TAIL_XOR: u8 = 0x5c;

    fn word64(mut v: u64, state: u32) -> u64 {
        v = swap_adjacent_bits_u64(!v) ^ !u64::from(state.rotate_right(5));
        v = !v.rotate_right((state.rotate_right(4) % 63) + 1);
        v.wrapping_add(u64::from(state.rotate_right(1)))
    }

    fn word32(mut v: u32, state: u32) -> u32 {
        // Note the asymmetry with word64: the rotated state is not complemented
        // here, and the rotations are left rather than right.
        v = swap_adjacent_bits_u32(!v) ^ state.rotate_left(5);
        v = !v.rotate_right((state.rotate_left(4) % 31) + 1);
        v.wrapping_add(state.rotate_left(1))
    }

    fn byte(mut v: u8, state: u32) -> u8 {
        let state11 = state.wrapping_mul(0x0b);
        let mix = state11.wrapping_mul(0x533);
        v = swap_adjacent_bits_u8(!v) ^ mix.wrapping_mul(0x0b) as u8;
        v = !v.rotate_right((mix % 7) + 1);
        v.wrapping_add(state11 as u8)
    }
}

/// `++Ares-Core+release-13.02`
pub struct V13_02;

impl SeededTransform for V13_02 {
    const BRANCH: &'static str = "++Ares-Core+release-13.02";
    const SEED_ADDEND: u32 = 0x9e81_a37c;
    const INIT_A_OFFSET: u32 = 0x04;
    const TAIL_XOR: u8 = 0x7c;

    fn word64(mut v: u64, state: u32) -> u64 {
        let ror2 = state.rotate_right(2);
        let ror3 = state.rotate_right(3);
        let ror6 = state.rotate_right(6);
        v = substitute_bytes_u64(v, &SBOX_64);
        v = reverse_bits64_without_final_16bit_swap(v);
        v = !v.wrapping_sub(u64::from(ror6));
        v = reverse_bits64_without_final_16bit_swap(v);
        v = v.rotate_left((ror3 % 63) + 1);
        v.rotate_right((ror2 % 63) + 1)
    }

    fn word32(mut v: u32, state: u32) -> u32 {
        let rol2 = state.rotate_left(2);
        let rol3 = state.rotate_left(3);
        let rol6 = state.rotate_left(6);
        v = substitute_bytes_u32(v, &SBOX_32);
        v = reverse_bits_u32(v);
        v = !v.wrapping_sub(rol6);
        v = reverse_bits_u32(v);
        v = v.rotate_left((rol3 % 31) + 1);
        v.rotate_right((rol2 % 31) + 1)
    }

    fn byte(mut v: u8, state: u32) -> u8 {
        let mix_a = state.wrapping_mul(0x79);
        let mix_b = mix_a.wrapping_mul(0x0b);
        v = SBOX_8[v as usize];
        v = reverse_bits_u8(v);
        v = !v.wrapping_sub(mix_b.wrapping_mul(0x33) as u8);
        v = reverse_bits_u8(v);
        v = v.rotate_left((mix_b % 7) + 1);
        v.rotate_right((mix_a % 7) + 1)
    }
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
