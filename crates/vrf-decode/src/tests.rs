//! Tests ported from the C# reference:
//! - PrimitiveDecodersScalarTests.cs
//! - PrimitiveDecodersVectorTests.cs
//! - RepLayoutArrayDecodersTests.cs (structural only — DynamicArray is Raw)

#[cfg(test)]
mod scalar {
    use crate::decode::{DecodedValue, FieldType, decode_field};

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

    #[test]
    fn enum_remaining_bits_reads_all_remaining() {
        // 3 bits = value 3 (0b111 but only 0b011 = 3)
        let data = [0b00000011u8]; // low 3 bits = 011
        let result = decode_field(FieldType::EnumRemainingBits, &data, 3).unwrap();
        assert_eq!(result, DecodedValue::I64(3));
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
}

#[cfg(test)]
mod vector {
    use crate::decode::{DecodedValue, FieldType, decode_field};
    use crate::types::RotatorQuantization;

    /// Helper: build a quantized vector bitstream.
    /// Format: SerializedInt(128) header + 3×componentBitCount signed components.
    fn write_quantized_vector(
        x: f64,
        y: f64,
        z: f64,
        scale_factor: u32,
        component_bit_count: u32,
    ) -> (Vec<u8>, u32) {
        let mut bits: Vec<bool> = Vec::new();
        // Header: info = componentBitCount | (1 << 6) — indicates scaled integer
        let info = component_bit_count | (1 << 6);
        write_serialized_int(&mut bits, info, 1 << 7);
        // Components
        let xi = (x * f64::from(scale_factor)).round() as i64;
        let yi = (y * f64::from(scale_factor)).round() as i64;
        let zi = (z * f64::from(scale_factor)).round() as i64;
        write_signed_bits(&mut bits, xi, component_bit_count);
        write_signed_bits(&mut bits, yi, component_bit_count);
        write_signed_bits(&mut bits, zi, component_bit_count);
        bits_to_bytes(&bits)
    }

    fn write_serialized_int(bits: &mut Vec<bool>, value: u32, max: u32) {
        let mut written_value = 0u32;
        let mut mask = 1u32;
        while written_value + mask < max {
            let bit = (value & mask) != 0;
            bits.push(bit);
            if bit {
                written_value |= mask;
            }
            mask <<= 1;
        }
    }

    fn write_signed_bits(bits: &mut Vec<bool>, value: i64, count: u32) {
        let mask = if count == 64 {
            u64::MAX
        } else {
            (1u64 << count) - 1
        };
        let raw = (value as u64) & mask;
        for i in 0..count {
            bits.push((raw >> i) & 1 != 0);
        }
    }

    fn write_compressed_short_rotator_component(bits: &mut Vec<bool>, value: u16) {
        bits.push(value != 0);
        if value != 0 {
            for i in 0..16 {
                bits.push((value >> i) & 1 != 0);
            }
        }
    }

    fn write_compressed_byte_rotator_component(bits: &mut Vec<bool>, value: u8) {
        bits.push(value != 0);
        if value != 0 {
            for i in 0..8 {
                bits.push((value >> i) & 1 != 0);
            }
        }
    }

    fn write_int_packed(bits: &mut Vec<bool>, mut value: u32) {
        loop {
            let mut byte_val = ((value & 0x7F) << 1) as u8;
            value >>= 7;
            if value != 0 {
                byte_val |= 1;
            }
            for i in 0..8 {
                bits.push((byte_val >> i) & 1 != 0);
            }
            if value == 0 {
                break;
            }
        }
    }

    fn bits_to_bytes(bits: &[bool]) -> (Vec<u8>, u32) {
        let byte_count = bits.len().div_ceil(8);
        let mut bytes = vec![0u8; byte_count];
        for (i, &b) in bits.iter().enumerate() {
            if b {
                bytes[i / 8] |= 1 << (i % 8);
            }
        }
        (bytes, bits.len() as u32)
    }

    #[test]
    fn vector_float_reads_three_floats() {
        let mut data = Vec::new();
        data.extend_from_slice(&1.25f32.to_le_bytes());
        data.extend_from_slice(&(-2.5f32).to_le_bytes());
        data.extend_from_slice(&3.75f32.to_le_bytes());
        let result = decode_field(FieldType::VectorFloat, &data, 96).unwrap();
        match result {
            DecodedValue::Str(s) => assert_eq!(s, "(1.25,-2.5,3.75)"),
            _ => panic!("expected Str"),
        }
    }

    #[test]
    fn vector_double_reads_three_doubles() {
        let mut data = Vec::new();
        data.extend_from_slice(&1.25f64.to_le_bytes());
        data.extend_from_slice(&(-2.5f64).to_le_bytes());
        data.extend_from_slice(&3.75f64.to_le_bytes());
        let result = decode_field(FieldType::VectorDouble, &data, 192).unwrap();
        match result {
            DecodedValue::Str(s) => assert_eq!(s, "(1.25,-2.5,3.75)"),
            _ => panic!("expected Str"),
        }
    }

    #[test]
    fn quantized_vector_scale1() {
        let (data, bit_count) = write_quantized_vector(10.0, -2.0, 3.0, 1, 6);
        let result =
            decode_field(FieldType::VectorNetQuantize { scale: 1 }, &data, bit_count).unwrap();
        match result {
            DecodedValue::Str(s) => assert_eq!(s, "(10,-2,3)"),
            _ => panic!("expected Str"),
        }
    }

    #[test]
    fn quantized_vector_scale10() {
        let (data, bit_count) = write_quantized_vector(1.2, -3.4, 5.6, 10, 7);
        let result =
            decode_field(FieldType::VectorNetQuantize { scale: 10 }, &data, bit_count).unwrap();
        match result {
            DecodedValue::Str(s) => {
                // Parse back to check within tolerance
                let nums: Vec<f64> = s
                    .trim_matches(|c| c == '(' || c == ')')
                    .split(',')
                    .map(|n| n.parse::<f64>().unwrap())
                    .collect();
                assert!((nums[0] - 1.2).abs() < 1e-9, "x={}", nums[0]);
                assert!((nums[1] - (-3.4)).abs() < 1e-9, "y={}", nums[1]);
                assert!((nums[2] - 5.6).abs() < 1e-9, "z={}", nums[2]);
            }
            _ => panic!("expected Str"),
        }
    }

    #[test]
    fn quantized_vector_scale100() {
        let (data, bit_count) = write_quantized_vector(1.23, -4.56, 7.89, 100, 11);
        let result = decode_field(
            FieldType::VectorNetQuantize { scale: 100 },
            &data,
            bit_count,
        )
        .unwrap();
        match result {
            DecodedValue::Str(s) => {
                let nums: Vec<f64> = s
                    .trim_matches(|c| c == '(' || c == ')')
                    .split(',')
                    .map(|n| n.parse::<f64>().unwrap())
                    .collect();
                assert!((nums[0] - 1.23).abs() < 1e-9);
                assert!((nums[1] - (-4.56)).abs() < 1e-9);
                assert!((nums[2] - 7.89).abs() < 1e-9);
            }
            _ => panic!("expected Str"),
        }
    }

    #[test]
    fn rep_movement_decodes_required_fields() {
        let mut bits: Vec<bool> = Vec::new();
        // 4 flag bits: all false
        bits.extend([false, false, false, false]);
        // Location: VectorNetQuantize100 (scale=100, componentBitCount=11)
        let info = 11u32 | (1 << 6);
        write_serialized_int(&mut bits, info, 1 << 7);
        let xi = (1.23_f64 * 100.0).round() as i64;
        let yi = (-4.56_f64 * 100.0).round() as i64;
        let zi = (7.89_f64 * 100.0).round() as i64;
        write_signed_bits(&mut bits, xi, 11);
        write_signed_bits(&mut bits, yi, 11);
        write_signed_bits(&mut bits, zi, 11);
        // Rotation short: all zero (3 bits, all false = no rotation data)
        write_compressed_short_rotator_component(&mut bits, 0);
        write_compressed_short_rotator_component(&mut bits, 0);
        write_compressed_short_rotator_component(&mut bits, 0);
        // Linear velocity: VectorNetQuantize(1), scale=1, componentBitCount=6
        let linfo = 6u32 | (1 << 6);
        write_serialized_int(&mut bits, linfo, 1 << 7);
        write_signed_bits(&mut bits, 10, 6);
        write_signed_bits(&mut bits, -2, 6);
        write_signed_bits(&mut bits, 3, 6);

        let (data, bit_count) = bits_to_bytes(&bits);
        let result = decode_field(
            FieldType::RepMovement {
                rotation: RotatorQuantization::ShortComponents,
            },
            &data,
            bit_count,
        )
        .unwrap();
        match result {
            DecodedValue::Str(s) => {
                assert!(s.starts_with("mov(loc="), "got: {s}");
                assert!(s.contains("rot(0,0,0)"), "got: {s}");
                assert!(s.contains("vel=(10,-2,3)"), "got: {s}");
            }
            _ => panic!("expected Str"),
        }
    }

    #[test]
    fn rep_movement_decodes_optional_fields() {
        let mut bits: Vec<bool> = Vec::new();
        // 4 flag bits: all true
        bits.extend([true, true, true, true]);
        // Location
        let info = 11u32 | (1 << 6);
        write_serialized_int(&mut bits, info, 1 << 7);
        let xi = (1.23_f64 * 100.0).round() as i64;
        let yi = (-4.56_f64 * 100.0).round() as i64;
        let zi = (7.89_f64 * 100.0).round() as i64;
        write_signed_bits(&mut bits, xi, 11);
        write_signed_bits(&mut bits, yi, 11);
        write_signed_bits(&mut bits, zi, 11);
        // Rotation short: pitch=90°(16384), yaw=180°(32768), roll=270°(49152)
        write_compressed_short_rotator_component(&mut bits, 16384);
        write_compressed_short_rotator_component(&mut bits, 32768);
        write_compressed_short_rotator_component(&mut bits, 49152);
        // Linear velocity
        let linfo = 6u32 | (1 << 6);
        write_serialized_int(&mut bits, linfo, 1 << 7);
        write_signed_bits(&mut bits, 10, 6);
        write_signed_bits(&mut bits, -2, 6);
        write_signed_bits(&mut bits, 3, 6);
        // Angular velocity (bRepPhysics=true)
        let ainfo = 5u32 | (1 << 6);
        write_serialized_int(&mut bits, ainfo, 1 << 7);
        write_signed_bits(&mut bits, -4, 5);
        write_signed_bits(&mut bits, 5, 5);
        write_signed_bits(&mut bits, -6, 5);
        // Server frame (bRepServerFrame=true)
        write_int_packed(&mut bits, 123);
        // Server physics handle (bRepServerHandle=true)
        write_int_packed(&mut bits, 456);

        let (data, bit_count) = bits_to_bytes(&bits);
        let result = decode_field(
            FieldType::RepMovement {
                rotation: RotatorQuantization::ShortComponents,
            },
            &data,
            bit_count,
        )
        .unwrap();
        match result {
            DecodedValue::Str(s) => {
                assert!(s.contains("angvel="), "got: {s}");
                assert!(s.contains("sf=123"), "got: {s}");
            }
            _ => panic!("expected Str"),
        }
    }

    #[test]
    fn rep_movement_byte_quantized_rotation() {
        let mut bits: Vec<bool> = Vec::new();
        // 4 flag bits: all false
        bits.extend([false, false, false, false]);
        // Location
        let info = 11u32 | (1 << 6);
        write_serialized_int(&mut bits, info, 1 << 7);
        let xi = (1.23_f64 * 100.0).round() as i64;
        let yi = (-4.56_f64 * 100.0).round() as i64;
        let zi = (7.89_f64 * 100.0).round() as i64;
        write_signed_bits(&mut bits, xi, 11);
        write_signed_bits(&mut bits, yi, 11);
        write_signed_bits(&mut bits, zi, 11);
        // Rotation byte: 64→90°, 128→180°, 192→270°
        write_compressed_byte_rotator_component(&mut bits, 64);
        write_compressed_byte_rotator_component(&mut bits, 128);
        write_compressed_byte_rotator_component(&mut bits, 192);
        // Linear velocity
        let linfo = 6u32 | (1 << 6);
        write_serialized_int(&mut bits, linfo, 1 << 7);
        write_signed_bits(&mut bits, 10, 6);
        write_signed_bits(&mut bits, -2, 6);
        write_signed_bits(&mut bits, 3, 6);

        let (data, bit_count) = bits_to_bytes(&bits);
        let result = decode_field(
            FieldType::RepMovement {
                rotation: RotatorQuantization::ByteComponents,
            },
            &data,
            bit_count,
        )
        .unwrap();
        match result {
            DecodedValue::Str(s) => {
                // Should contain rotation ~90,180,270
                assert!(s.contains("rot(90,180,270)"), "got: {s}");
            }
            _ => panic!("expected Str"),
        }
    }
}

#[cfg(test)]
mod overlay_tests {
    use crate::OVERLAY_TABLE;
    use crate::decode::FieldType;
    use crate::overlay::{OverlayEntry, OverlayStats, OverlayTable, apply_overlay};

    #[test]
    fn table_is_sorted() {
        let table = &OVERLAY_TABLE;
        for window in table.windows(2) {
            let cmp = window[0]
                .group_path
                .cmp(window[1].group_path)
                .then_with(|| window[0].field_name.cmp(window[1].field_name));
            assert!(
                cmp.is_lt() || cmp.is_eq(),
                "table not sorted at {:?} vs {:?}",
                (window[0].group_path, window[0].field_name),
                (window[1].group_path, window[1].field_name)
            );
        }
    }

    #[test]
    fn lookup_finds_known_field() {
        let table = OverlayTable::new(&OVERLAY_TABLE);
        let ft = table.lookup(
            "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C",
            "CompetitiveTier",
        );
        assert_eq!(ft, Some(FieldType::Int32));
    }

    #[test]
    fn equippable_used_is_an_object_net_guid() {
        // The C# descriptor attaches a custom decoder
        // (DamageParameters.cs:51 -> ValorantPayloadDecoders.Equippable), which
        // extract_descriptors.py cannot see through, so it lands in table.rs as
        // Raw. That decoder is exactly archive.ReadIntPacked(), i.e. our
        // ObjectNetGuid. Leaving it Raw forces consumers to guess the encoding;
        // the adapter guessed a fixed 16-bit LE integer and produced values that
        // were never valid NetGUIDs. tools/apply_type_corrections.py restores
        // the real type.
        let table = OverlayTable::new(&OVERLAY_TABLE);
        for group in [
            "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Base",
            "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Point",
        ] {
            assert_eq!(
                table.lookup(group, "EquippableUsed"),
                Some(FieldType::ObjectNetGuid),
                "EquippableUsed must decode as a net GUID in {group}",
            );
        }
    }

    #[test]
    fn lookup_returns_none_for_unknown() {
        let table = OverlayTable::new(&OVERLAY_TABLE);
        let ft = table.lookup("nonexistent", "field");
        assert_eq!(ft, None);
    }

    #[test]
    fn apply_overlay_decodes_int32() {
        let entries: &[OverlayEntry] = &[OverlayEntry {
            group_path: "/test",
            field_name: "Health",
            field_type: FieldType::Int32,
        }];
        let table = OverlayTable::new(entries);
        let mut stats = OverlayStats::default();
        let data = 100i32.to_le_bytes();
        let result = apply_overlay(&table, "/test", Some("Health"), Some(&data), 32, &mut stats);
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.value_i64, Some(100));
        assert_eq!(stats.decoded_ok, 1);
    }

    #[test]
    fn apply_overlay_returns_none_for_no_field_name() {
        let entries: &[OverlayEntry] = &[OverlayEntry {
            group_path: "/test",
            field_name: "Health",
            field_type: FieldType::Int32,
        }];
        let table = OverlayTable::new(entries);
        let mut stats = OverlayStats::default();
        let result = apply_overlay(&table, "/test", None, Some(&[0; 4]), 32, &mut stats);
        assert!(result.is_none());
        assert_eq!(stats.no_field_name, 1);
    }

    #[test]
    fn apply_overlay_graceful_on_decode_failure() {
        let entries: &[OverlayEntry] = &[OverlayEntry {
            group_path: "/test",
            field_name: "Broken",
            field_type: FieldType::FString, // needs more than 1 bit
        }];
        let table = OverlayTable::new(entries);
        let mut stats = OverlayStats::default();
        let data = [0x01u8]; // only 1 bit — FString needs at least 32 bits for length
        let result = apply_overlay(&table, "/test", Some("Broken"), Some(&data), 1, &mut stats);
        // Should return Some but with all values None (decode failure)
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.value_i64, None);
        assert_eq!(r.value_str, None);
        assert_eq!(stats.decoded_err, 1);
    }

    /// Byte-sized properties nested inside replicated arrays are written with
    /// only their significant bits, so the decoder must take its width from the
    /// payload rather than assuming 8.
    ///
    /// This is not hypothetical: `CombatReport` `AssistType` arrives as a 5-bit
    /// payload, and a fixed 8-bit read left all 364 of its rows in a real replay
    /// untyped while every neighbouring field decoded fine.
    #[test]
    fn byte_takes_its_width_from_the_payload() {
        use crate::decode::{DecodedValue, FieldType, decode_field};

        // 5 significant bits holding 9 (0b01001), padded to one byte.
        let data = [0b0000_1001u8];
        for width in [1u32, 3, 5, 8] {
            let v = decode_field(FieldType::EnumByte, &data, width)
                .unwrap_or_else(|e| panic!("width {width} should decode: {e:?}"));
            let mask = ((1u16 << width) - 1) as u8;
            let expected = i64::from(0b0000_1001u8 & mask);
            assert_eq!(v, DecodedValue::I64(expected), "width {width}");
        }
    }

    /// A payload wider than a byte is not a byte field. Truncating to the low 8
    /// bits would emit a plausible wrong number, so it is reported instead.
    #[test]
    fn byte_rejects_payloads_wider_than_eight_bits() {
        use crate::decode::{FieldType, decode_field};

        let data = [0xFFu8, 0xFF];
        // 12 bits declared: the nominal 8-bit read leaves 4 unconsumed, which
        // decode_field turns into an error rather than a truncated value.
        assert!(decode_field(FieldType::Byte, &data, 12).is_err());
    }
}
