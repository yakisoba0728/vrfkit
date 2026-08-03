//! `++Ares-Core+release-13.02`

use super::SeededTransform;
use crate::helpers::{
    reverse_bits_u8, reverse_bits_u32, reverse_bits64_without_final_16bit_swap,
    substitute_bytes_u32, substitute_bytes_u64,
};
use crate::sbox::{SBOX_8, SBOX_32, SBOX_64};

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
