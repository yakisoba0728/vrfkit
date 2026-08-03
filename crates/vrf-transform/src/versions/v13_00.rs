//! `++Ares-Core+release-13.00`

use super::SeededTransform;
use crate::helpers::{
    reverse_bits_u8, reverse_bits_u32, reverse_bits64_without_final_16bit_swap,
    substitute_bytes_u32, substitute_bytes_u64,
};
use crate::sbox::{SBOX_8, SBOX_32, SBOX_64};

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
