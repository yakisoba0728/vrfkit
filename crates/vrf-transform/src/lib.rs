//! Payload transforms for VALORANT replay content blocks.
//!
//! # What is transformed, and when
//!
//! The obfuscation is applied per **content block payload**, not to the file, a
//! chunk, or a packet. Content block *headers* and their declared bit lengths are
//! plaintext; only the field data that follows is transformed. That split has a
//! useful consequence: a replay can be framed into blocks without decoding any
//! of them, so framing stays sequential while the expensive per-block decode can
//! be spread across threads.
//!
//! # Seed derivation
//!
//! ```text
//! seed = (bit_count as u32) ^ actor_net_guid
//! ```
//!
//! Both inputs come from the surrounding stream, so nothing is stored in the file
//! that identifies the key. See [`seed_for`].
//!
//! # Per-build variation
//!
//! The algorithm skeleton has been stable from release-12.10 to release-13.02.
//! What changes per build is two constants and the order of a handful of bit
//! primitives; see [`versions`]. Adding a build means writing one `impl` with
//! two constants and three word functions.
//!
//! # Example
//!
//! ```
//! use vrf_bitio::BitReader;
//! use vrf_transform::{TransformVersion, seed_for};
//!
//! let payload = [0xBFu8, 0xDF, 0x6F];
//! let bit_count = 20;
//! let version = TransformVersion::from_branch("++Ares-Core+release-13.01").unwrap();
//!
//! let mut reader = BitReader::new(&payload);
//! let mut out = vec![0u8; TransformVersion::output_byte_count(bit_count)];
//! version.decode_from(&mut reader, bit_count, seed_for(bit_count, 2), &mut out).unwrap();
//! ```
//!
//! # Module map
//!
//! | Module | Responsibility |
//! |--------|----------------|
//! | `lib` | [`seed_for`], [`TransformVersion`] dispatch, the staging driver |
//! | [`helpers`] | PRNG and bit primitives shared by every build |
//! | [`versions`] | One file per build, plus the [`versions::SeededTransform`] trait |
//! | [`sbox`] | Generated substitution tables (`tools/extract_sboxes.py`) |
//!
//! # Cargo features
//!
//! **None, deliberately.** Gating individual builds is the obvious candidate --
//! a consumer that only ever sees release-13.01 replays links four transforms
//! it cannot use -- but it cannot be done without changing this crate's public
//! API, which is frozen:
//!
//! - [`ALL_VERSIONS`] is `[TransformVersion; 5]`. Its *type* encodes the build
//!   count, so dropping one build changes the type.
//! - [`TransformVersion`]'s variants are public. Gating one removes a name that
//!   callers match on.
//!
//! Making this work needs `ALL_VERSIONS` to become `&'static [TransformVersion]`
//! and callers to stop matching variants exhaustively. That is a reasonable
//! change and it is recommended, but it is an API change rather than a feature
//! flag, so it is not made here. The cost of not gating is small in any case:
//! the five `impl`s are branch-free arithmetic, and the only sizeable data is
//! the three S-box tables (used by release-13.00 and release-13.02 alone).

#![forbid(unsafe_code)]

pub mod helpers;
pub mod sbox;
pub mod versions;

use versions::{SeededTransform, V12_10, V12_11, V13_00, V13_01, V13_02};
use vrf_bitio::{BitError, BitReader, Result as BitResult};

/// Derive the transform seed for a content block.
///
/// `bit_count` is the block's declared payload length and `actor_net_guid` the
/// network GUID of the actor channel carrying it. Both are read from plaintext
/// parts of the stream.
#[must_use]
#[inline]
pub const fn seed_for(bit_count: usize, actor_net_guid: u32) -> u32 {
    (bit_count as u32) ^ actor_net_guid
}

/// Run a build's transform over `buf` in place.
///
/// `buf` must already hold the payload's bits, LSB-first, with the final byte's
/// padding zeroed -- [`BitReader::copy_bits_to`] guarantees both. Stale padding
/// would be folded into the tail byte and corrupt it.
///
/// Processing is staged 64 bits at a time, then 32, then 8, then the remaining
/// 1..7 bits, advancing the PRNG once per stage iteration. The staging order is
/// part of the format: the keystream position depends on how many words of each
/// width came before.
pub fn transform_in_place<T: SeededTransform>(
    buf: &mut [u8],
    bit_count: usize,
    seed: u32,
) -> BitResult<()> {
    if bit_count == 0 {
        return Ok(());
    }
    let available = (buf.len() as u64).saturating_mul(8);
    if bit_count as u64 > available {
        return Err(BitError::InvalidBitLength {
            requested: bit_count as u64,
            available,
        });
    }

    let mut state = seed;
    // Before the first PRNG advance the keystream byte is just the low seed byte.
    // A payload shorter than 8 bits never advances, so the tail XOR below relies
    // on this initial value.
    let mut stream_byte = seed as u8;
    let mut prng_a = T::initial_prng_a(seed);
    let mut prng_b = helpers::initial_prng_b(seed);
    let mut offset = 0usize;
    let mut left = bit_count;

    while left > 63 {
        let value = T::word64(helpers::read_u64(buf, offset), state);
        helpers::write_u64(buf, offset, value);
        stream_byte = helpers::advance_state(&mut state, &mut prng_a, &mut prng_b);
        offset += 8;
        left -= 64;
    }
    while left > 31 {
        let value = T::word32(helpers::read_u32(buf, offset), state);
        helpers::write_u32(buf, offset, value);
        stream_byte = helpers::advance_state(&mut state, &mut prng_a, &mut prng_b);
        offset += 4;
        left -= 32;
    }
    while left > 7 {
        buf[offset] = T::byte(buf[offset], state);
        stream_byte = helpers::advance_state(&mut state, &mut prng_a, &mut prng_b);
        offset += 1;
        left -= 8;
    }
    if left != 0 {
        // Only the bits that are actually part of the payload are touched; the
        // mask keeps the padding at zero so a re-encode stays byte-identical.
        let mask = 0xffu8 >> (7 - ((bit_count - 1) & 7));
        buf[offset] ^= mask & (stream_byte ^ T::TAIL_XOR);
    }
    Ok(())
}

/// A game build's payload transform, selected by replay branch string.
///
/// Dispatch happens once per content block, and each arm calls a monomorphised
/// [`transform_in_place`], so the inner word loops carry no indirection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransformVersion {
    /// `++Ares-Core+release-12.10`
    V1210,
    /// `++Ares-Core+release-12.11`
    V1211,
    /// `++Ares-Core+release-13.00`
    V1300,
    /// `++Ares-Core+release-13.01`
    V1301,
    /// `++Ares-Core+release-13.02`
    V1302,
}

/// Every transform this build of the crate knows about.
pub const ALL_VERSIONS: [TransformVersion; 5] = [
    TransformVersion::V1210,
    TransformVersion::V1211,
    TransformVersion::V1300,
    TransformVersion::V1301,
    TransformVersion::V1302,
];

/// A replay whose branch has no registered transform.
///
/// Reported rather than worked around: guessing a transform yields plausible-
/// looking garbage instead of an error, and downstream metrics cannot tell the
/// difference. The branch string is carried so callers can name it in a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedBranch {
    /// The replay branch that could not be matched.
    pub branch: String,
}

impl core::fmt::Display for UnsupportedBranch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "no payload transform is registered for replay branch '{}'; known branches: {}",
            self.branch,
            ALL_VERSIONS.map(TransformVersion::branch).join(", ")
        )
    }
}

impl core::error::Error for UnsupportedBranch {}

impl TransformVersion {
    /// Look up a transform by exact replay branch string.
    #[must_use]
    pub fn from_branch(branch: &str) -> Option<Self> {
        ALL_VERSIONS.into_iter().find(|v| v.branch() == branch)
    }

    /// Look up a transform, returning a descriptive error when unknown.
    pub fn require(branch: &str) -> core::result::Result<Self, UnsupportedBranch> {
        Self::from_branch(branch).ok_or_else(|| UnsupportedBranch {
            branch: branch.to_owned(),
        })
    }

    /// The replay branch string this transform decodes.
    #[must_use]
    pub const fn branch(self) -> &'static str {
        match self {
            Self::V1210 => V12_10::BRANCH,
            Self::V1211 => V12_11::BRANCH,
            Self::V1300 => V13_00::BRANCH,
            Self::V1301 => V13_01::BRANCH,
            Self::V1302 => V13_02::BRANCH,
        }
    }

    /// Bytes needed to hold `bit_count` bits.
    #[must_use]
    pub const fn output_byte_count(bit_count: usize) -> usize {
        bit_count.div_ceil(8)
    }

    /// Transform `buf` in place.
    pub fn apply(self, buf: &mut [u8], bit_count: usize, seed: u32) -> BitResult<()> {
        match self {
            Self::V1210 => transform_in_place::<V12_10>(buf, bit_count, seed),
            Self::V1211 => transform_in_place::<V12_11>(buf, bit_count, seed),
            Self::V1300 => transform_in_place::<V13_00>(buf, bit_count, seed),
            Self::V1301 => transform_in_place::<V13_01>(buf, bit_count, seed),
            Self::V1302 => transform_in_place::<V13_02>(buf, bit_count, seed),
        }
    }

    /// Copy `bit_count` bits out of `reader` into `out`, then transform them.
    ///
    /// This is the shape the parser uses: the payload is never materialised
    /// twice, and `out` is expected to be a reused scratch buffer.
    pub fn decode_from(
        self,
        reader: &mut BitReader<'_>,
        bit_count: usize,
        seed: u32,
        out: &mut [u8],
    ) -> BitResult<()> {
        reader.copy_bits_to(out, bit_count as u64)?;
        self.apply(
            &mut out[..Self::output_byte_count(bit_count)],
            bit_count,
            seed,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_bit_count_xor_actor_guid() {
        assert_eq!(seed_for(287, 2), 287 ^ 2);
        assert_eq!(seed_for(0, 0), 0);
    }

    #[test]
    fn branch_lookup_round_trips() {
        for v in ALL_VERSIONS {
            assert_eq!(TransformVersion::from_branch(v.branch()), Some(v));
        }
    }

    #[test]
    fn unknown_branch_is_an_error_naming_the_branch() {
        let err = TransformVersion::require("++Ares-Core+release-99.99").unwrap_err();
        assert_eq!(err.branch, "++Ares-Core+release-99.99");
        let text = err.to_string();
        assert!(text.contains("release-99.99"), "{text}");
        assert!(text.contains("release-13.02"), "{text}");
    }

    #[test]
    fn zero_bits_is_a_noop() {
        let mut buf = [0xAAu8; 4];
        for v in ALL_VERSIONS {
            v.apply(&mut buf, 0, 1234).unwrap();
            assert_eq!(buf, [0xAAu8; 4]);
        }
    }

    #[test]
    fn output_byte_count_rounds_up() {
        assert_eq!(TransformVersion::output_byte_count(0), 0);
        assert_eq!(TransformVersion::output_byte_count(1), 1);
        assert_eq!(TransformVersion::output_byte_count(8), 1);
        assert_eq!(TransformVersion::output_byte_count(9), 2);
        assert_eq!(TransformVersion::output_byte_count(287), 36);
    }

    #[test]
    fn apply_does_not_panic_on_an_undersized_buffer() {
        let mut buf = [0u8; 1];
        assert_eq!(
            TransformVersion::V1301.apply(&mut buf, 65, 0).unwrap_err(),
            BitError::InvalidBitLength {
                requested: 65,
                available: 8,
            }
        );
    }

    #[test]
    fn distinct_builds_produce_distinct_output() {
        // A regression guard against copy-paste errors between version impls:
        // no two builds may agree on a non-trivial payload.
        let payload = [0xBFu8, 0xDF, 0x6F, 0x9E, 0xA1, 0xF2, 0x7B, 0xA0, 0x11];
        let bit_count = 65;
        let mut outputs = Vec::new();
        for v in ALL_VERSIONS {
            let mut buf = vec![0u8; TransformVersion::output_byte_count(bit_count)];
            let mut r = BitReader::new(&payload);
            v.decode_from(&mut r, bit_count, seed_for(bit_count, 2), &mut buf)
                .unwrap();
            assert!(
                !outputs.contains(&buf),
                "{} duplicates another build",
                v.branch()
            );
            outputs.push(buf);
        }
    }
}
