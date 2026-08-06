//! Scalar primitive decoders, ported from the C# reference's
//! `PrimitiveDecodersScalarTests.cs`.

use crate::decode::{DecodeError, DecodedValue, FieldType, decode_field};

#[test]
fn double_reads_eight_byte_float() {
    let data = 123.5_f64.to_bits().to_le_bytes();
    let result = decode_field(FieldType::Double, &data, 64).unwrap();
    assert_eq!(result, DecodedValue::F64(123.5));
}

#[test]
fn fstring_reads_unreal_string() {
    let mut data = Vec::new();
    // Length = 6 (5 chars + null)
    data.extend_from_slice(&6i32.to_le_bytes());
    data.extend_from_slice(b"Spike\0");
    let bit_count = (data.len() * 8) as u32;
    let result = decode_field(FieldType::FString, &data, bit_count).unwrap();
    assert_eq!(result, DecodedValue::Str("Spike".into()));
}

#[test]
fn fname_hardcoded_reads_a_packed_index() {
    // FArchive.ReadFNameCore: when the leading bit is set the name is a
    // hardcoded table index sent as IntPacked, and the reference renders
    // it as the decimal index -- there is no FString to read.
    //
    // Ignoring that branch is why MulticastNotifyDamage_Point.DamagedBone
    // had to be forced to Raw: 177 of its 581 payloads on 02d4d478 are
    // 9 bits (1 flag + one IntPacked byte), far too short for the
    // FString path, which read past the end and produced mojibake.
    //
    // 9-bit payload: bit0 = 1 (hardcoded), then IntPacked 0 = byte 0x00.
    let data = [0x01u8, 0x00];
    let result = decode_field(FieldType::FName, &data, 9).unwrap();
    assert_eq!(result, DecodedValue::Str("0".into()));
}

#[test]
fn fname_reads_inline_name() {
    let mut bits = Vec::new();
    // bit 0 = false (not hardcoded)
    // Then FString "Bomb" (len=5 including null) + i32 suffix = 0
    let mut payload = Vec::new();
    payload.extend_from_slice(&5i32.to_le_bytes()); // length
    payload.extend_from_slice(b"Bomb\0");
    payload.extend_from_slice(&0i32.to_le_bytes()); // suffix

    // Pack: first bit = 0 (not hardcoded), then the payload bytes
    // We need to shift all payload bits by 1
    let total_bits = 1 + payload.len() * 8;
    let total_bytes = total_bits.div_ceil(8);
    bits.resize(total_bytes, 0);
    // bit 0 = 0 (already zero)
    // Copy payload starting at bit 1
    for (i, &b) in payload.iter().enumerate() {
        for bit_idx in 0..8 {
            let src_bit = (b >> bit_idx) & 1;
            let dst_bit_pos = 1 + i * 8 + bit_idx;
            bits[dst_bit_pos / 8] |= src_bit << (dst_bit_pos % 8);
        }
    }
    let result = decode_field(FieldType::FName, &bits, total_bits as u32).unwrap();
    assert_eq!(result, DecodedValue::Str("Bomb".into()));
}

/// Build the inline (`isHardcoded = 0`) FName shape: a leading zero bit, then
/// an FString, then the i32 instance number.
fn inline_fname_bits(name: &str, number: i32) -> (Vec<u8>, u32) {
    let mut payload = Vec::new();
    let len = i32::try_from(name.len() + 1).unwrap();
    payload.extend_from_slice(&len.to_le_bytes());
    payload.extend_from_slice(name.as_bytes());
    payload.push(0);
    payload.extend_from_slice(&number.to_le_bytes());

    let total_bits = 1 + payload.len() * 8;
    let mut bits = vec![0u8; total_bits.div_ceil(8)];
    for (i, &b) in payload.iter().enumerate() {
        for bit_idx in 0..8 {
            let src_bit = (b >> bit_idx) & 1;
            let dst = 1 + i * 8 + bit_idx;
            bits[dst / 8] |= src_bit << (dst % 8);
        }
    }
    (bits, total_bits as u32)
}

/// The FName instance number is part of the name's identity, so two fields that
/// differ only in it must not decode to the same string.
///
/// Unreal stores the number as `displayed suffix + 1`: 0 means the bare name,
/// and N != 0 displays as `Name_{N-1}`. Discarding it made `Source_1` and
/// `Source_2` both read as `Source`.
#[test]
fn fname_inline_number_renders_the_displayed_suffix() {
    let (bits, total_bits) = inline_fname_bits("Source", 2);
    let result = decode_field(FieldType::FName, &bits, total_bits).unwrap();
    assert_eq!(result, DecodedValue::Str("Source_1".into()));
}

/// Two FNames differing only in the instance number decode to different
/// strings. This is the property the discard destroyed.
#[test]
fn fname_inline_numbers_do_not_collide() {
    let (a_bits, a_len) = inline_fname_bits("Source", 1);
    let (b_bits, b_len) = inline_fname_bits("Source", 2);
    let a = decode_field(FieldType::FName, &a_bits, a_len).unwrap();
    let b = decode_field(FieldType::FName, &b_bits, b_len).unwrap();
    assert_ne!(a, b, "Source_0 and Source_1 must not decode alike");
    assert_eq!(a, DecodedValue::Str("Source_0".into()));
    assert_eq!(b, DecodedValue::Str("Source_1".into()));
}

#[test]
fn byte_array_reads_packed_count_and_bytes() {
    // IntPacked 3 = byte (3 << 1) = 0x06
    let data = vec![0x06u8, 0x10, 0x20, 0x30];
    let bit_count = (data.len() * 8) as u32;
    let result = decode_field(FieldType::ByteArray { max_bytes: 8 }, &data, bit_count).unwrap();
    assert_eq!(result, DecodedValue::Str("102030".into()));
}

#[test]
fn guid_reads_four_le_words() {
    let mut data = Vec::new();
    data.extend_from_slice(&0x00112233u32.to_le_bytes());
    data.extend_from_slice(&0x44556677u32.to_le_bytes());
    data.extend_from_slice(&0x8899AABBu32.to_le_bytes());
    data.extend_from_slice(&0xCCDDEEFFu32.to_le_bytes());
    let result = decode_field(FieldType::Guid, &data, 128).unwrap();
    assert_eq!(
        result,
        DecodedValue::Str("00112233-4455-6677-8899-aabbccddeeff".into())
    );
}

#[test]
fn serialized_int_reads_value_using_known_maximum() {
    // max=16 -> value_bits = 4. value 5 = 0b0101 in 4 bits.
    let data = [0x05u8];
    let result = decode_field(FieldType::SerializedInt { max: 16 }, &data, 4).unwrap();
    assert_eq!(result, DecodedValue::I64(5));
}

#[test]
fn uint64_reads_eight_byte_unsigned() {
    let data = 0x0102030405060708u64.to_le_bytes();
    let result = decode_field(FieldType::UInt64, &data, 64).unwrap();
    assert_eq!(result, DecodedValue::I64(0x0102030405060708i64));
}

/// A `UInt64` with its sign bit set cannot fit in the `i64` the overlay stores
/// without a silent wrap to a negative number. It must be rejected loudly
/// instead. Values at or below `i64::MAX` decode exactly as before.
#[test]
fn uint64_above_i64_max_is_rejected_not_wrapped() {
    // High bit set: i64::MAX + 1 = 0x8000_0000_0000_0000.
    let over = i64::MAX as u64 + 1;
    let data = over.to_le_bytes();
    let result = decode_field(FieldType::UInt64, &data, 64);
    assert!(matches!(
        result,
        Err(DecodeError::UnsignedOverflow { value }) if value == over
    ));
    // Boundary: i64::MAX itself still decodes to the positive I64.
    let data = (i64::MAX as u64).to_le_bytes();
    let result = decode_field(FieldType::UInt64, &data, 64).unwrap();
    assert_eq!(result, DecodedValue::I64(i64::MAX));
}

#[test]
fn enum_remaining_bits_reads_all_remaining() {
    // 3 bits = value 3 (0b111 but only 0b011 = 3)
    let data = [0b00000011u8]; // low 3 bits = 011
    let result = decode_field(FieldType::EnumRemainingBits, &data, 3).unwrap();
    assert_eq!(result, DecodedValue::I64(3));
}

/// A payload too wide for the type must not come back as its low 32 bits.
///
/// `decode_enum_remaining_bits` read `min(bits_left, 32)` and returned, and
/// `decode_field` exempted this one type from the not-fully-consumed check --
/// so the bits above 32 were dropped without reaching any counter, any error,
/// or the `skipped_bits` tally. The C# reference throws here. This follows
/// `UnsignedOverflow`'s rule instead: a value that cannot be represented is an
/// error, not a plausible wrong number.
///
/// Latent on this corpus -- handles 215/216 reach 47 at most across 71
/// replays, so nothing triggers it today. That is exactly why it needs a test.
#[test]
fn enum_remaining_bits_wider_than_32_errors_instead_of_truncating() {
    let data = [0xFFu8; 5];
    let err = decode_field(FieldType::EnumRemainingBits, &data, 40).unwrap_err();
    assert!(
        matches!(err, DecodeError::NotFullyConsumed { remaining: 8 }),
        "expected the leftover to be reported, got {err:?}"
    );
}

/// The boundary still decodes: 32 bits is representable, 33 is not.
#[test]
fn enum_remaining_bits_reads_a_full_32() {
    let data = [0xFFu8, 0xFF, 0xFF, 0xFF];
    let result = decode_field(FieldType::EnumRemainingBits, &data, 32).unwrap();
    assert_eq!(result, DecodedValue::I64(u32::MAX as i64));
}

#[test]
fn gameplay_tag_reads_packed_index() {
    // IntPacked 252: 252 = 0b11111100, split: chunk0=252&0x7F=124, chunk1=252>>7=1
    // byte0 = (124 << 1) | 1 = 249, byte1 = (1 << 1) | 0 = 2
    let data = [249u8, 2u8];
    let result = decode_field(FieldType::GameplayTag, &data, 16).unwrap();
    assert_eq!(result, DecodedValue::I64(252));
}

#[test]
fn bool_reads_single_bit() {
    let data = [0x01u8];
    let result = decode_field(FieldType::Bool, &data, 1).unwrap();
    assert_eq!(result, DecodedValue::Bool(true));

    let data = [0x00u8];
    let result = decode_field(FieldType::Bool, &data, 1).unwrap();
    assert_eq!(result, DecodedValue::Bool(false));
}

#[test]
fn int32_reads_signed() {
    let data = (-42i32).to_le_bytes();
    let result = decode_field(FieldType::Int32, &data, 32).unwrap();
    assert_eq!(result, DecodedValue::I64(-42));
}

#[test]
fn float_reads_ieee754_single() {
    let data = 1.25f32.to_bits().to_le_bytes();
    let result = decode_field(FieldType::Float, &data, 32).unwrap();
    assert_eq!(result, DecodedValue::F64(1.25));
}

#[test]
fn object_net_guid_reads_int_packed() {
    // IntPacked value 0x3F: byte = (0x3F << 1) | 0 = 0x7E
    let data = [0x7Eu8];
    let result = decode_field(FieldType::ObjectNetGuid, &data, 8).unwrap();
    assert_eq!(result, DecodedValue::I64(0x3F));
}

/// `FText` decodes to the statistic's name, which nothing else in the export
/// carries.
///
/// `LocalizedStat` was typed `FString` once and produced null on every row,
/// because the wire is an `FText`. It was left untyped on the reasoning that
/// the sibling `Statistic` enum already said the same thing -- but `Statistic`
/// decodes to a bare integer and this repository ships no table mapping those
/// integers to names. It has them only in a comment. So this is in fact the
/// only machine-readable source of `EnemiesBlinded` and the other 28.
///
/// The layout, confirmed on 4,341 of 4,341 rows with zero residual bits: 41
/// header bits ending in a history-type discriminator of 5, then the string
/// table's asset path as an `FString`, then that `FName`'s numeric suffix,
/// then the key. The key is the statistic name.
#[test]
fn ftext_decodes_a_string_table_entry_to_its_key() {
    let vectors: [(u32, &[u8], &str); 3] = [
        // Kills
        (
            785u32,
            &[
                0x00, 0x00, 0x00, 0x00, 0x0B, 0x96, 0x00, 0x00, 0x00, 0x5E, 0x8E, 0xC2, 0xDA, 0xCA,
                0x5E, 0xAA, 0x92, 0x5E, 0x92, 0xDC, 0x8E, 0xC2, 0xDA, 0xCA, 0x5E, 0x86, 0xDE, 0xDA,
                0xC4, 0xC2, 0xE8, 0xA4, 0xCA, 0xE0, 0xDE, 0xE4, 0xE8, 0x5E, 0x82, 0xC4, 0xD2, 0xD8,
                0xD2, 0xE8, 0xF2, 0x8A, 0xCC, 0xCC, 0xCA, 0xC6, 0xE8, 0xE6, 0xBE, 0xA6, 0xE8, 0xE4,
                0xD2, 0xDC, 0xCE, 0xE6, 0x5C, 0x82, 0xC4, 0xD2, 0xD8, 0xD2, 0xE8, 0xF2, 0x8A, 0xCC,
                0xCC, 0xCA, 0xC6, 0xE8, 0xE6, 0xBE, 0xA6, 0xE8, 0xE4, 0xD2, 0xDC, 0xCE, 0xE6, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x96, 0xD2, 0xD8, 0xD8, 0xE6, 0x00,
                0x00,
            ][..],
            "Kills",
        ),
        // EnemiesBlinded
        (
            969u32,
            &[
                0x00, 0x00, 0x00, 0x00, 0x0B, 0xB2, 0x00, 0x00, 0x00, 0x5E, 0x8E, 0xC2, 0xDA, 0xCA,
                0x5E, 0x86, 0xD0, 0xC2, 0xE4, 0xC2, 0xC6, 0xE8, 0xCA, 0xE4, 0xE6, 0x5E, 0x8E, 0xD8,
                0xDE, 0xC4, 0xC2, 0xD8, 0x5E, 0xA6, 0xE8, 0xE4, 0xD2, 0xDC, 0xCE, 0xA8, 0xC2, 0xC4,
                0xD8, 0xCA, 0xE6, 0x5E, 0x86, 0xD0, 0xC2, 0xE4, 0xC2, 0xC6, 0xE8, 0xCA, 0xE4, 0xE6,
                0xBE, 0x8E, 0xD8, 0xDE, 0xC4, 0xC2, 0xD8, 0xBE, 0xA6, 0xE8, 0xE4, 0xD2, 0xDC, 0xCE,
                0xE6, 0x5C, 0x86, 0xD0, 0xC2, 0xE4, 0xC2, 0xC6, 0xE8, 0xCA, 0xE4, 0xE6, 0xBE, 0x8E,
                0xD8, 0xDE, 0xC4, 0xC2, 0xD8, 0xBE, 0xA6, 0xE8, 0xE4, 0xD2, 0xDC, 0xCE, 0xE6, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x1E, 0x00, 0x00, 0x00, 0x8A, 0xDC, 0xCA, 0xDA, 0xD2, 0xCA,
                0xE6, 0x84, 0xD8, 0xD2, 0xDC, 0xC8, 0xCA, 0xC8, 0x00, 0x00,
            ][..],
            "EnemiesBlinded",
        ),
        // AbilityStat_ShotsFired
        (
            1033u32,
            &[
                0x00, 0x00, 0x00, 0x00, 0x0B, 0xB2, 0x00, 0x00, 0x00, 0x5E, 0x8E, 0xC2, 0xDA, 0xCA,
                0x5E, 0x86, 0xD0, 0xC2, 0xE4, 0xC2, 0xC6, 0xE8, 0xCA, 0xE4, 0xE6, 0x5E, 0x8E, 0xD8,
                0xDE, 0xC4, 0xC2, 0xD8, 0x5E, 0xA6, 0xE8, 0xE4, 0xD2, 0xDC, 0xCE, 0xA8, 0xC2, 0xC4,
                0xD8, 0xCA, 0xE6, 0x5E, 0x86, 0xD0, 0xC2, 0xE4, 0xC2, 0xC6, 0xE8, 0xCA, 0xE4, 0xE6,
                0xBE, 0x8E, 0xD8, 0xDE, 0xC4, 0xC2, 0xD8, 0xBE, 0xA6, 0xE8, 0xE4, 0xD2, 0xDC, 0xCE,
                0xE6, 0x5C, 0x86, 0xD0, 0xC2, 0xE4, 0xC2, 0xC6, 0xE8, 0xCA, 0xE4, 0xE6, 0xBE, 0x8E,
                0xD8, 0xDE, 0xC4, 0xC2, 0xD8, 0xBE, 0xA6, 0xE8, 0xE4, 0xD2, 0xDC, 0xCE, 0xE6, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x2E, 0x00, 0x00, 0x00, 0x82, 0xC4, 0xD2, 0xD8, 0xD2, 0xE8,
                0xF2, 0xA6, 0xE8, 0xC2, 0xE8, 0xBE, 0xA6, 0xD0, 0xDE, 0xE8, 0xE6, 0x8C, 0xD2, 0xE4,
                0xCA, 0xC8, 0x00, 0x00,
            ][..],
            "AbilityStat_ShotsFired",
        ),
    ];
    for (bit_count, raw, expected) in vectors {
        assert_eq!(
            decode_field(FieldType::FText, raw, bit_count).unwrap(),
            DecodedValue::Str(expected.to_owned()),
            "{expected}"
        );
    }
}

/// A history type the layout was never observed under is refused, not guessed.
///
/// Every sample carries 5. Another value means a different `ETextHistory`
/// variant with a different payload after the header, and reading it as this
/// one would produce a plausible wrong string -- the failure this type was
/// removed for in the first place.
#[test]
fn ftext_refuses_an_unobserved_history_type() {
    let mut raw = vec![0u8; 16];
    raw[4] = 0x0B; // shifts a history type of 6 into place, not 5
    raw[5] = 0x02;
    assert!(decode_field(FieldType::FText, &raw, 128).is_err());
}
