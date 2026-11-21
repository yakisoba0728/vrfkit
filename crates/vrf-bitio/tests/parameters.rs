#![forbid(unsafe_code)]

use vrf_bitio::{BitError, BitReader};

#[test]
fn wide_bit_reads_are_rejected_without_advancing() {
    let data = [0xA5; 32];
    for offset in 0..8 {
        for count in [65, 66, 127, 128, u32::MAX] {
            let mut reader = BitReader::new(&data);
            reader.skip_bits(offset).unwrap();
            assert_eq!(
                reader.read_bits(count),
                Err(BitError::InvalidReadWidth { count })
            );
            assert_eq!(reader.position(), offset);
        }
    }
}

#[test]
fn overflowing_rotator_widths_are_rejected_without_advancing() {
    let data = [0; 32];
    for offset in 0..8 {
        for width in [32, 33, 64, u32::MAX] {
            let mut reader = BitReader::new(&data);
            reader.skip_bits(offset).unwrap();
            assert_eq!(
                reader.read_compressed_rotator(width),
                Err(BitError::InvalidRotatorWidth { width })
            );
            assert_eq!(reader.position(), offset);
        }
    }
}

#[test]
fn zero_quantized_vector_scale_is_rejected_without_advancing() {
    // Header 65: scaled one-bit components; the first component is -1.
    let data = [0xC1, 0];
    let mut reader = BitReader::new(&data);
    assert_eq!(
        reader.read_quantized_vector(0),
        Err(BitError::InvalidVectorScale { scale: 0 })
    );
    assert_eq!(reader.position(), 0);
}

#[test]
fn invalid_parameters_are_reported_before_truncation() {
    let mut reader = BitReader::new(&[]);
    assert_eq!(
        reader.read_bits(u32::MAX),
        Err(BitError::InvalidReadWidth { count: u32::MAX })
    );
    assert_eq!(
        reader.read_compressed_rotator(u32::MAX),
        Err(BitError::InvalidRotatorWidth { width: u32::MAX })
    );
    assert_eq!(
        reader.read_quantized_vector(0),
        Err(BitError::InvalidVectorScale { scale: 0 })
    );
    assert_eq!(reader.position(), 0);
}

#[test]
fn rejected_bit_read_keeps_a_nested_window_usable() {
    let data = [0xFF; 10];
    let mut parent = BitReader::new(&data);
    parent.skip_bits(3).unwrap();
    let mut child = parent.sub_reader(70).unwrap();
    child.skip_bits(5).unwrap();
    let mut nested = child.sub_reader(64).unwrap();
    assert_eq!(
        nested.read_bits(65),
        Err(BitError::InvalidReadWidth { count: 65 })
    );
    assert_eq!(nested.position(), 0);
    assert_eq!(nested.read_bits(64), Ok(u64::MAX));
    assert!(nested.at_end());
    assert_eq!(child.position(), 69);
    assert_eq!(parent.position(), 73);
}

fn append_bits(bits: &mut Vec<bool>, value: u64, width: u32) {
    bits.extend((0..width).map(|shift| value & (1 << shift) != 0));
}

fn pack(bits: &[bool]) -> Vec<u8> {
    let mut bytes = vec![0; bits.len().div_ceil(8)];
    for (index, bit) in bits.iter().enumerate() {
        bytes[index / 8] |= u8::from(*bit) << (index % 8);
    }
    bytes
}

#[test]
fn supported_rotator_widths_keep_their_degree_scaling() {
    for width in [0, 1, 8, 16, 31] {
        let range = 1u64 << width;
        let raw = [0, range / 2, range - 1];
        let mut bits = vec![false; 5];
        for value in raw {
            bits.push(true);
            append_bits(&mut bits, value, width);
        }
        let data = pack(&bits);
        let mut reader = BitReader::with_bit_len(&data, bits.len() as u64).unwrap();
        reader.skip_bits(5).unwrap();
        let expected = raw.map(|value| value as f32 * (360.0 / range as f32));
        assert_eq!(reader.read_compressed_rotator(width), Ok(expected));
        assert!(reader.at_end());
    }

    let mut absent = BitReader::with_bit_len(&[0], 3).unwrap();
    assert_eq!(absent.read_compressed_rotator(16), Ok([0.0; 3]));
    assert!(absent.at_end());
}

#[test]
fn positive_vector_scales_keep_scaled_components_finite() {
    // Header 68: scaled four-bit components encoding [3, -2, 4].
    let mut bits = Vec::new();
    append_bits(&mut bits, 68, 7);
    for value in [3, 14, 4] {
        append_bits(&mut bits, value, 4);
    }
    let data = pack(&bits);
    for scale in [1, 10, 100, u32::MAX] {
        let mut reader = BitReader::with_bit_len(&data, bits.len() as u64).unwrap();
        assert_eq!(
            reader.read_quantized_vector(scale),
            Ok([3.0, -2.0, 4.0].map(|value| value / f64::from(scale)))
        );
        assert!(reader.at_end());
    }
}
