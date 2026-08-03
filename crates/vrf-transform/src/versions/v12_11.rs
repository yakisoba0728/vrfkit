//! `++Ares-Core+release-12.11`

use super::SeededTransform;
use crate::helpers::{
    reverse_bits_u8, reverse_bits_u32, reverse_bits64_without_final_16bit_swap,
    swap_adjacent_bits_u8, swap_adjacent_bits_u32, swap_adjacent_bits_u64,
};

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
