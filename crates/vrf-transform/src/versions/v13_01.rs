//! `++Ares-Core+release-13.01`
//!
//! The build every replay in the 215-file corpus was recorded on, so this is
//! the transform the golden vectors exercise most heavily.

use super::SeededTransform;
use crate::helpers::{swap_adjacent_bits_u8, swap_adjacent_bits_u32, swap_adjacent_bits_u64};

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
