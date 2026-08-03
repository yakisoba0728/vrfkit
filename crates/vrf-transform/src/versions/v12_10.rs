//! `++Ares-Core+release-12.10`

use super::SeededTransform;
use crate::helpers::{swap_adjacent_bits_u8, swap_adjacent_bits_u32, swap_adjacent_bits_u64};

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
