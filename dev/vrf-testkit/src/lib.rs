//! Test-only encoders for the formats vrfkit's crates read: an LSB-first bit
//! writer, little-endian byte appenders and a minimal uncompressed replay.
//! Kept out of `vrf-bitio` so a bug mirrored in writer and reader cannot
//! cancel out; the tests below pin the writer to the bytes `vrf-bitio`'s
//! reader tests are built from.

#![forbid(unsafe_code)]

/// A bit string under construction, least significant bit first: the order
/// `BitReader` reads. The encoders are [`BitWrite`] methods.
pub type BitWriter = Vec<bool>;

/// Chainable encoders on a [`BitWriter`].
pub trait BitWrite {
    /// Appends one bit.
    fn bit(&mut self, bit: bool) -> &mut Self;

    /// The bits written so far.
    fn as_bits(&self) -> &[bool];

    /// The low `width` (at most 64) bits of `value`.
    fn bits(&mut self, value: u64, width: u32) -> &mut Self {
        for i in 0..width {
            self.bit((value >> i) & 1 != 0);
        }
        self
    }

    /// `count` copies of `bit`.
    fn repeat(&mut self, bit: bool, count: usize) -> &mut Self {
        for _ in 0..count {
            self.bit(bit);
        }
        self
    }

    /// Another bit string, after these.
    fn extend_bits(&mut self, other: &[bool]) -> &mut Self {
        for &bit in other {
            self.bit(bit);
        }
        self
    }

    fn u8(&mut self, value: u8) -> &mut Self {
        self.bits(u64::from(value), 8)
    }

    fn u16(&mut self, value: u16) -> &mut Self {
        self.bits(u64::from(value), 16)
    }

    fn u32(&mut self, value: u32) -> &mut Self {
        self.bits(u64::from(value), 32)
    }

    fn i32(&mut self, value: i32) -> &mut Self {
        self.u32(value as u32)
    }

    fn f32(&mut self, value: f32) -> &mut Self {
        self.u32(value.to_bits())
    }

    /// Whole bytes, each least significant bit first.
    fn bytes(&mut self, raw: &[u8]) -> &mut Self {
        for &byte in raw {
            self.u8(byte);
        }
        self
    }

    /// A UTF-8 FString, laid out as [`add_fstring`] does.
    fn fstring(&mut self, s: &str) -> &mut Self {
        let mut raw = Vec::new();
        add_fstring(&mut raw, s);
        self.bytes(&raw)
    }

    /// `SerializeIntPacked`: seven value bits per byte, above a continuation
    /// bit that is set while more bytes follow.
    fn int_packed(&mut self, mut value: u32) -> &mut Self {
        loop {
            let more = value > 0x7f;
            self.u8((((value & 0x7f) << 1) | u32::from(more)) as u8);
            value >>= 7;
            if !more {
                return self;
            }
        }
    }

    /// `SerializeInt(value, max)`: only the bits that could still raise the
    /// value without reaching `max`, 32 at most.
    fn serialized_int(&mut self, value: u32, max: u32) -> &mut Self {
        let (mut written, mut mask) = (0u32, 1u32);
        while mask != 0 && written.saturating_add(mask) < max {
            let bit = value & mask != 0;
            self.bit(bit);
            if bit {
                written |= mask;
            }
            mask <<= 1;
        }
        self
    }

    fn bit_len(&self) -> u32 {
        self.as_bits().len() as u32
    }

    /// The bytes, zero-padded to a whole byte, and the exact bit count.
    fn finish(&self) -> (Vec<u8>, u32) {
        (pack(self.as_bits()), self.bit_len())
    }
}

impl BitWrite for BitWriter {
    fn bit(&mut self, bit: bool) -> &mut Self {
        self.push(bit);
        self
    }

    fn as_bits(&self) -> &[bool] {
        self
    }
}

/// Bits to bytes, least significant first; unused high bits of the last byte
/// are zero.
pub fn pack(bits: &[bool]) -> Vec<u8> {
    let mut bytes = vec![0u8; bits.len().div_ceil(8)];
    for (i, _) in bits.iter().enumerate().filter(|(_, bit)| **bit) {
        bytes[i / 8] |= 1 << (i % 8);
    }
    bytes
}

/// Bytes to bits, least significant first.
pub fn unpack(bytes: &[u8]) -> BitWriter {
    let mut bits = BitWriter::new();
    bits.bytes(bytes);
    bits
}

macro_rules! add_le {
    ($($name:ident: $ty:ty),*) => {$(
        #[doc = concat!("Appends a little-endian `", stringify!($ty), "`.")]
        pub fn $name(buf: &mut Vec<u8>, value: $ty) {
            buf.extend_from_slice(&value.to_le_bytes());
        }
    )*};
}

add_le!(add_u16: u16, add_u32: u32, add_i32: i32, add_u64: u64, add_i64: i64, add_f32: f32);

/// A UTF-8 FString: i32 length counting the null, the bytes, the null.
pub fn add_fstring(buf: &mut Vec<u8>, s: &str) {
    add_i32(buf, (s.len() + 1) as i32);
    buf.extend_from_slice(s.as_bytes());
    buf.push(0);
}

/// A UTF-16 FString: negative length counting code units (null included),
/// then the units little-endian.
pub fn add_fstring_utf16(buf: &mut Vec<u8>, s: &str) {
    let units: Vec<u16> = s.encode_utf16().chain([0]).collect();
    add_i32(buf, -(units.len() as i32));
    for unit in units {
        add_u16(buf, unit);
    }
}

pub fn add_guid(buf: &mut Vec<u8>, guid: [u32; 4]) {
    for word in guid {
        add_u32(buf, word);
    }
}

/// [`BitWrite::int_packed`], which always fills whole bytes.
pub fn add_int_packed(buf: &mut Vec<u8>, value: u32) {
    buf.extend(pack(BitWriter::new().int_packed(value)));
}

/// The `LocalFileReplay` custom-version GUID, the one the info parser pins.
pub const LOCAL_REPLAY: [u32; 4] = [0x95A4_F03E, 0x7E0B_49E4, 0xBA43_D356, 0x94FF_87D9];

/// The replay info fields a test varies. [`replay_info`] fixes the rest: file
/// version 7, 60000 ms, network version 19, changelist 1234, not live, no
/// encryption key.
pub struct Info {
    pub magic: u32,
    pub custom_versions: Vec<([u32; 4], i32)>,
    pub friendly_name: &'static str,
    pub timestamp: i64,
    pub compressed: bool,
    pub encrypted: bool,
}

/// A valid, uncompressed replay info: the file magic and `LocalFileReplay` 7.
impl Default for Info {
    fn default() -> Self {
        Info {
            magic: 0x43F4_EFDD,
            custom_versions: vec![(LOCAL_REPLAY, 7)],
            friendly_name: "Match",
            timestamp: 42,
            compressed: false,
            encrypted: false,
        }
    }
}

/// The replay info section, in the order `vrf-container`'s `info.rs` reads it.
pub fn replay_info(info: &Info) -> Vec<u8> {
    let mut buf = Vec::new();
    add_u32(&mut buf, info.magic);
    add_u32(&mut buf, 7); // file version
    add_i32(&mut buf, info.custom_versions.len() as i32);
    for &(guid, version) in &info.custom_versions {
        add_guid(&mut buf, guid);
        add_i32(&mut buf, version);
    }
    add_i32(&mut buf, 60_000); // length in ms
    add_u32(&mut buf, 19); // network version
    add_u32(&mut buf, 1234); // changelist
    add_fstring(&mut buf, info.friendly_name);
    add_u32(&mut buf, 0); // is live
    add_i64(&mut buf, info.timestamp);
    add_u32(&mut buf, u32::from(info.compressed));
    add_u32(&mut buf, u32::from(info.encrypted));
    add_i32(&mut buf, 0); // encryption key length
    buf
}

/// One chunk: type, i32 payload size, payload.
pub fn chunk(chunk_type: u32, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    add_u32(&mut buf, chunk_type);
    add_i32(&mut buf, payload.len() as i32);
    buf.extend_from_slice(payload);
    buf
}

/// A 12.10 header, in the order `vrf-container`'s `header.rs` reads it: no
/// custom versions, a 3-byte Valorant skip, flags `HasStreamingFixes |
/// GameSpecificFrameData` (every frame carries that section) and two
/// game-specific data entries.
pub fn header_payload() -> Vec<u8> {
    header_payload_for_branch("++Ares-Core+release-12.10", 0, &[3, 0, 0, 0, 49, 56, 0])
}

/// [`header_payload`] with its branch, custom-version count (20 zero bytes
/// each) and Valorant skip (count word, then the bytes) replaced.
pub fn header_payload_for_branch(
    branch: &str,
    custom_version_count: i32,
    valorant_skip: &[u8],
) -> Vec<u8> {
    let mut buf = Vec::new();
    add_u32(&mut buf, 0x2CF5_A13D); // network magic
    add_u32(&mut buf, 19); // network version
    add_i32(&mut buf, custom_version_count);
    for _ in 0..custom_version_count {
        buf.extend_from_slice(&[0u8; 20]);
    }
    add_u32(&mut buf, 0x1122_3344); // network checksum
    add_u32(&mut buf, 32); // engine net proto version
    add_u32(&mut buf, 0x5566_7788); // game net proto version
    add_guid(
        &mut buf,
        [0x0011_2233, 0x4455_6677, 0x8899_AABB, 0xCCDD_EEFF],
    );
    add_u16(&mut buf, 12); // major
    add_u16(&mut buf, 10); // minor
    add_u16(&mut buf, 1); // patch
    add_u32(&mut buf, 123_456); // changelist
    add_fstring(&mut buf, branch);
    buf.extend_from_slice(valorant_skip);
    add_u32(&mut buf, 1001); // UE4 version
    add_u32(&mut buf, 1002); // UE5 version
    add_u32(&mut buf, 1003); // package version license
    add_i32(&mut buf, 1); // one level name
    add_fstring(&mut buf, "Ascent");
    add_u32(&mut buf, 42); // level time
    add_u32(&mut buf, 0b1010); // HasStreamingFixes | GameSpecificFrameData
    add_i32(&mut buf, 2); // game-specific data entries
    add_fstring(&mut buf, "valorant");
    add_fstring(&mut buf, "competitive");
    for rate in [15.0, 30.0, 33.3, 250.0] {
        add_f32(&mut buf, rate);
    }
    add_fstring(&mut buf, "Windows");
    buf.extend_from_slice(&[7, 3]); // build config, build target type
    buf
}

/// An Oodle archive around one uncompressed Kraken block holding `plain`, then
/// `unread` bytes inside the declared compressed size that no block reads.
/// `0x4C 0x06` is block-header nibble `0xC` with the uncompressed bit set, then
/// Kraken without checksums (oozextract 0.5.4's block-header parse).
pub fn archive_with_unread_input(plain: &[u8], unread: usize) -> Vec<u8> {
    let mut archive = Vec::new();
    add_i32(&mut archive, plain.len() as i32); // decompressed size
    add_i32(&mut archive, (2 + plain.len() + unread) as i32); // compressed size
    archive.extend_from_slice(&[0x4C, 0x06]);
    archive.extend_from_slice(plain);
    archive.extend(std::iter::repeat_n(0xAB, unread));
    archive
}

/// An Oodle archive declaring 64 bytes of output whose one Mermaid quantum makes
/// oozextract 0.5.4 panic instead of returning an error: block header
/// `0x0C 0x0A` (Mermaid), quantum header `00 00 07` (8 bytes follow), chunk
/// header `88 00 05` (LZ, mode 1, `src_used` 5), five zero bytes. Mermaid's LZ
/// table asserts `src_used >= 10` (`mermaid.rs` `read_lz_table`), and nothing
/// before it checks.
pub fn archive_that_panics_the_codec() -> Vec<u8> {
    let body = [
        0x0C, 0x0A, 0x00, 0x00, 0x07, 0x88, 0x00, 0x05, 0, 0, 0, 0, 0,
    ];
    let mut archive = Vec::new();
    add_i32(&mut archive, 64); // decompressed size
    add_i32(&mut archive, body.len() as i32); // compressed size
    archive.extend_from_slice(&body);
    archive
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: u32 = u32::MAX;

    #[track_caller]
    fn check(written: &mut BitWriter, bytes: &[u8], bit_len: u32) {
        assert_eq!(written.finish(), (bytes.to_vec(), bit_len));
    }

    /// Each expected value is the input of the `vrf-bitio` reader test named
    /// above it, or follows the reader's rule where no reader test pins it.
    #[test]
    fn the_bit_writer_emits_the_bytes_the_reader_tests_read() {
        let w = BitWriter::new;
        // reads_least_significant_bit_first
        let lsb_first = [true, false, true, false, false, true, false, true];
        check(w().extend_bits(&lsb_first), &[0xA5], 8);
        check(w().bits(0xA5, 8), &[0xA5], 8);
        // int_packed_single_byte
        check(w().int_packed(0x3F), &[0x3F << 1], 8);
        // int_packed_multi_byte_is_little_endian_in_chunks: (44 << 1) | 1, 2 << 1
        check(w().int_packed(300), &[0x59, 0x04], 16);
        // int_packed_accepts_the_largest_representable_fifth_chunk: 15 << 1 last
        check(w().int_packed(MAX), &[0xFF, 0xFF, 0xFF, 0xFF, 0x1E], 40);
        // int_packed_is_bit_aligned_not_byte_aligned: (0x3F << 1) << 1
        check(w().bit(false).int_packed(0x3F), &[0xFC, 0x00], 9);
        // serialized_int_spends_log2_bits_then_maybe_one_more
        check(w().serialized_int(3, 4), &[0b11], 2);
        check(w().serialized_int(4, 5), &[0b100], 3);
        // serialized_int_max_one_consumes_nothing
        check(w().serialized_int(0, 1), &[], 0);
        // The top of the range: 31 bits, then a 32nd.
        check(w().serialized_int(0, MAX), &[0; 4], 32);
        check(w().serialized_int(0x8000_0005, MAX), &[5, 0, 0, 0x80], 32);
        // fstring_utf8_strips_null
        check(w().fstring("abc"), &[4, 0, 0, 0, b'a', b'b', b'c', 0], 64);
        check(w().u8(0x12).u16(0x3456), &[0x12, 0x56, 0x34], 24);
        check(w().i32(-2), &[0xFE, 0xFF, 0xFF, 0xFF], 32);
        // 0b111, then 1.5f32 (0x3FC0_0000) three bits up: 0x1_FE00_0007.
        check(w().repeat(true, 3).f32(1.5), &[0x07, 0, 0, 0xFE, 0x01], 35);
    }

    #[test]
    fn the_byte_appenders_emit_the_bytes_the_reader_tests_read() {
        // fstring_utf16_uses_negative_length
        let mut utf16 = Vec::new();
        add_fstring_utf16(&mut utf16, "hi");
        assert_eq!(utf16, [0xFD, 0xFF, 0xFF, 0xFF, b'h', 0, b'i', 0, 0, 0]);

        let mut packed = Vec::new();
        add_int_packed(&mut packed, 300);
        assert_eq!(packed, [0x59, 0x04]);

        let lsb_first = [true, false, true, false, false, true, false, true];
        assert_eq!(unpack(&[0xA5]), lsb_first);
        assert_eq!(pack(&lsb_first[..5]), [0b0_0101]);
    }

    /// `compressed` and `encrypted` are the two words before the key length.
    #[test]
    fn replay_info_flags_land_in_their_words() {
        let compressed = replay_info(&Info {
            compressed: true,
            ..Info::default()
        });
        let encrypted = replay_info(&Info {
            encrypted: true,
            ..Info::default()
        });
        assert_eq!(
            compressed[compressed.len() - 12..][..8],
            [1, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            encrypted[encrypted.len() - 12..][..8],
            [0, 0, 0, 0, 1, 0, 0, 0]
        );
    }
}
