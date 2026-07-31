//! Primitives shared by every build's payload transform.
//!
//! The interesting observation about VALORANT's payload obfuscation is how
//! little of it changes between game builds. The PRNG, its multiplier, the seed
//! mixing skeleton, the 64/32/8-bit staging and the tail handling have been
//! byte-identical across release-12.10 through release-13.02. What a new build
//! rotates is two constants and the *order* in which these primitives are
//! applied inside each word loop.
//!
//! Keeping the vocabulary here, and letting each version supply only its three
//! word functions, is what makes supporting a new build a small, reviewable
//! change instead of a fresh reverse-engineering project.

/// Multiplier used by both PRNG seeds. Unchanged across all known builds.
pub const MULTIPLIER: u64 = 0x2545_f491_4f6c_dd1d;

/// Seed the second PRNG lane.
///
/// Identical in every known build, which is why it lives here rather than on the
/// per-version trait.
#[inline]
#[must_use]
pub const fn initial_prng_b(seed: u32) -> u64 {
    let mixed = (((seed >> 15) ^ seed) >> 12) ^ (seed << 25) ^ seed;
    (mixed as u64).wrapping_mul(MULTIPLIER)
}

/// Seed the first PRNG lane.
///
/// Every known build uses this exact expression and varies only `seed_addend`,
/// `init_a_offset`, and whether the offset is subtracted or added. release-12.11
/// is the sole build that adds; the rest subtract.
#[inline]
#[must_use]
pub const fn initial_prng_a(
    seed: u32,
    seed_addend: u32,
    init_a_offset: u32,
    add_offset: bool,
) -> u64 {
    let seed_plus = seed.wrapping_add(seed_addend);
    let offset_term = if add_offset {
        seed.wrapping_add(init_a_offset)
    } else {
        seed.wrapping_sub(init_a_offset)
    }
    .wrapping_mul(0x0200_0000);
    let mixed = (((seed_plus >> 15) ^ seed_plus) >> 12) ^ offset_term ^ seed_plus;
    (mixed as u64).wrapping_mul(MULTIPLIER)
}

/// Advance the PRNG one step and return the keystream byte for this word.
///
/// A xoshiro-style lane mix: sum the lanes for the output, then rotate and XOR
/// them forward. The high 32 bits of the sum become the next `state`, which is
/// what the word functions rotate to derive their per-word constants.
#[inline]
pub fn advance_state(state: &mut u32, prng_a: &mut u64, prng_b: &mut u64) -> u8 {
    let sum = prng_b.wrapping_add(*prng_a);
    *prng_b ^= *prng_a;
    *prng_a = prng_a.rotate_right(9) ^ (*prng_b << 14) ^ *prng_b;
    *prng_b = prng_b.rotate_left(36);
    *state = (sum >> 32) as u32;
    *state as u8
}

/// Swap each even/odd bit pair.
#[inline]
#[must_use]
pub const fn swap_adjacent_bits_u64(v: u64) -> u64 {
    ((v & 0x5555_5555_5555_5555) << 1) | ((v >> 1) & 0x5555_5555_5555_5555)
}

/// Swap each even/odd bit pair.
#[inline]
#[must_use]
pub const fn swap_adjacent_bits_u32(v: u32) -> u32 {
    ((v & 0x5555_5555) << 1) | ((v >> 1) & 0x5555_5555)
}

/// Swap each even/odd bit pair.
#[inline]
#[must_use]
pub const fn swap_adjacent_bits_u8(v: u8) -> u8 {
    ((v & 0x55) << 1) | ((v >> 1) & 0x55)
}

/// A 64-bit reversal that omits the 16-bit swap stage.
///
/// Deliberately *not* [`u64::reverse_bits`]: the 1/2/4/8-bit swaps run, then the
/// halves are exchanged, but the 16-bit stage is skipped. The result is a
/// permutation of the bits that is not a plain reversal, so substituting
/// `reverse_bits` here silently produces wrong plaintext.
#[inline]
#[must_use]
pub const fn reverse_bits64_without_final_16bit_swap(mut v: u64) -> u64 {
    v = ((v & 0x5555_5555_5555_5555) << 1) | ((v >> 1) & 0x5555_5555_5555_5555);
    v = ((v & 0x3333_3333_3333_3333) << 2) | ((v >> 2) & 0x3333_3333_3333_3333);
    v = ((v & 0x0F0F_0F0F_0F0F_0F0F) << 4) | ((v >> 4) & 0x0F0F_0F0F_0F0F_0F0F);
    v = ((v & 0x00FF_00FF_00FF_00FF) << 8) | ((v >> 8) & 0x00FF_00FF_00FF_00FF);
    // Reference writes `(v << 32) | (v >> 32)`, which for a 64-bit word is
    // exactly a half-width rotation.
    v.rotate_left(32)
}

/// Full 32-bit reversal, written out to mirror the reference implementation.
#[inline]
#[must_use]
pub const fn reverse_bits_u32(mut v: u32) -> u32 {
    v = ((v & 0x5555_5555) << 1) | ((v >> 1) & 0x5555_5555);
    v = ((v & 0x3333_3333) << 2) | ((v >> 2) & 0x3333_3333);
    v = ((v & 0x0F0F_0F0F) << 4) | ((v >> 4) & 0x0F0F_0F0F);
    v = ((v & 0x00FF_00FF) << 8) | ((v >> 8) & 0x00FF_00FF);
    // Reference writes `(v << 16) | (v >> 16)`, a half-width rotation.
    v.rotate_left(16)
}

/// Full 8-bit reversal, written out to mirror the reference implementation.
#[inline]
#[must_use]
pub const fn reverse_bits_u8(mut v: u8) -> u8 {
    v = ((v & 0x55) << 1) | ((v >> 1) & 0x55);
    v = ((v & 0x33) << 2) | ((v >> 2) & 0x33);
    ((v & 0x0F) << 4) | ((v >> 4) & 0x0F)
}

/// Apply a byte substitution table to each byte of a 64-bit word.
#[inline]
#[must_use]
pub fn substitute_bytes_u64(v: u64, table: &[u8; 256]) -> u64 {
    let mut out = 0u64;
    let mut i = 0;
    while i < 8 {
        let b = ((v >> (i * 8)) & 0xFF) as u8;
        out |= u64::from(table[b as usize]) << (i * 8);
        i += 1;
    }
    out
}

/// Apply a byte substitution table to each byte of a 32-bit word.
#[inline]
#[must_use]
pub fn substitute_bytes_u32(v: u32, table: &[u8; 256]) -> u32 {
    let mut out = 0u32;
    let mut i = 0;
    while i < 4 {
        let b = ((v >> (i * 8)) & 0xFF) as u8;
        out |= u32::from(table[b as usize]) << (i * 8);
        i += 1;
    }
    out
}

/// Read a little-endian `u64` at `offset`.
///
/// The slice bounds are guaranteed by the caller's loop condition (the loop only
/// runs while at least 64 bits remain), so this cannot fail in practice; the
/// `expect` documents that invariant rather than guarding untrusted input.
#[inline]
#[must_use]
pub fn read_u64(buf: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        buf[offset..offset + 8]
            .try_into()
            .expect("8 bytes in range"),
    )
}

/// Read a little-endian `u32` at `offset`.
#[inline]
#[must_use]
pub fn read_u32(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        buf[offset..offset + 4]
            .try_into()
            .expect("4 bytes in range"),
    )
}

/// Write a little-endian `u64` at `offset`.
#[inline]
pub fn write_u64(buf: &mut [u8], offset: usize, value: u64) {
    buf[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

/// Write a little-endian `u32` at `offset`.
#[inline]
pub fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse32_matches_intrinsic() {
        for v in [0u32, 1, 0x8000_0000, 0xDEAD_BEEF, u32::MAX, 0x0F0F_0F0F] {
            assert_eq!(reverse_bits_u32(v), v.reverse_bits(), "value {v:#x}");
        }
    }

    #[test]
    fn reverse8_matches_intrinsic() {
        for v in 0u8..=255 {
            assert_eq!(reverse_bits_u8(v), v.reverse_bits(), "value {v:#x}");
        }
    }

    #[test]
    fn reverse64_variant_is_not_a_plain_reversal() {
        // The skipped 16-bit stage is the whole point; if this ever starts
        // matching `reverse_bits` the port has drifted.
        let v = 0x0123_4567_89AB_CDEFu64;
        assert_ne!(reverse_bits64_without_final_16bit_swap(v), v.reverse_bits());
    }

    #[test]
    fn reverse64_variant_is_an_involution_free_permutation() {
        // Applying it twice must return the original: each stage is a pairwise
        // swap, and the final half-exchange is its own inverse.
        for v in [
            0u64,
            1,
            u64::MAX,
            0x0123_4567_89AB_CDEF,
            0xFEDC_BA98_7654_3210,
        ] {
            let once = reverse_bits64_without_final_16bit_swap(v);
            assert_eq!(
                reverse_bits64_without_final_16bit_swap(once),
                v,
                "value {v:#x}"
            );
        }
    }

    #[test]
    fn swap_adjacent_is_an_involution() {
        for v in [
            0u64,
            1,
            0xAAAA_AAAA_AAAA_AAAA,
            0x5555_5555_5555_5555,
            u64::MAX,
        ] {
            assert_eq!(swap_adjacent_bits_u64(swap_adjacent_bits_u64(v)), v);
        }
        for v in 0u8..=255 {
            assert_eq!(swap_adjacent_bits_u8(swap_adjacent_bits_u8(v)), v);
        }
    }

    #[test]
    fn substitute_bytes_applies_table_per_lane() {
        let mut table = [0u8; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            *slot = (255 - i) as u8;
        }
        assert_eq!(substitute_bytes_u32(0x0001_0203, &table), 0xFFFE_FDFC);
        assert_eq!(
            substitute_bytes_u64(0x0000_0000_0000_00FF, &table),
            0xFFFF_FFFF_FFFF_FF00
        );
    }

    #[test]
    fn word_io_round_trips() {
        let mut buf = [0u8; 16];
        write_u64(&mut buf, 0, 0x0123_4567_89AB_CDEF);
        write_u32(&mut buf, 8, 0xDEAD_BEEF);
        assert_eq!(read_u64(&buf, 0), 0x0123_4567_89AB_CDEF);
        assert_eq!(read_u32(&buf, 8), 0xDEAD_BEEF);
        // Little-endian byte order is part of the wire format.
        assert_eq!(buf[0], 0xEF);
    }
}
