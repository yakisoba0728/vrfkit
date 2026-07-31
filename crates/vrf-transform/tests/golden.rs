//! Bit-exactness check for all five per-build payload transforms.
//!
//! The vectors are lifted mechanically from the reference implementation's test
//! fixture (see `tools/extract_golden.py`), so a passing run means our port
//! agrees with it byte for byte on every staging boundary of the algorithm.
//!
//! This is the only external check on the transform layer. Everything downstream
//! -- field framing, schema binding, metrics -- is built on the assumption that
//! these bytes are right, so a failure here invalidates all of it.

include!("data/golden_vectors.rs");

use vrf_bitio::BitReader;
use vrf_transform::{TransformVersion, seed_for};

fn from_hex(hex: &str) -> Vec<u8> {
    assert!(
        hex.len() % 2 == 0,
        "hex string must have even length: {hex}"
    );
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex"))
        .collect()
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

#[test]
fn transforms_match_reference_vectors() {
    let payload = from_hex(PAYLOAD_HEX);
    let mut failures = Vec::new();

    for (branch, bit_count, expected_hex) in VECTORS {
        let version = TransformVersion::require(branch).expect("branch is registered");
        let mut out = vec![0u8; TransformVersion::output_byte_count(bit_count)];
        let mut reader = BitReader::new(&payload);
        version
            .decode_from(
                &mut reader,
                bit_count,
                seed_for(bit_count, ACTOR_NET_GUID),
                &mut out,
            )
            .expect("payload is long enough for the vector");

        let actual = to_hex(&out);
        if actual != expected_hex {
            failures.push(format!(
                "{branch} @ {bit_count} bits\n     expected {expected_hex}\n     actual   {actual}"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} vectors mismatched:\n  {}",
        failures.len(),
        VECTORS.len(),
        failures.join("\n  ")
    );
}

#[test]
fn every_registered_build_is_covered() {
    // A new transform must arrive with vectors; otherwise it is unverified code
    // that will happily emit garbage.
    for version in vrf_transform::ALL_VERSIONS {
        let count = VECTORS
            .iter()
            .filter(|(b, _, _)| *b == version.branch())
            .count();
        assert!(count > 0, "{} has no golden vectors", version.branch());
    }
}

#[test]
fn vectors_cover_the_staging_boundaries() {
    // The transform stages 64 -> 32 -> 8 -> tail. If a build's vectors skipped a
    // boundary, an error in one stage could pass unnoticed.
    for version in vrf_transform::ALL_VERSIONS {
        let bits: Vec<usize> = VECTORS
            .iter()
            .filter(|(b, _, _)| *b == version.branch())
            .map(|(_, bits, _)| *bits)
            .collect();
        for required in [0usize, 1, 7, 8, 31, 32, 63, 64, 65] {
            assert!(
                bits.contains(&required),
                "{} lacks a vector at {required} bits (have {bits:?})",
                version.branch()
            );
        }
    }
}

#[test]
fn transform_is_deterministic() {
    let payload = from_hex(PAYLOAD_HEX);
    for (branch, bit_count, _) in VECTORS {
        let version = TransformVersion::require(branch).unwrap();
        let run = || {
            let mut out = vec![0u8; TransformVersion::output_byte_count(bit_count)];
            let mut reader = BitReader::new(&payload);
            version
                .decode_from(
                    &mut reader,
                    bit_count,
                    seed_for(bit_count, ACTOR_NET_GUID),
                    &mut out,
                )
                .unwrap();
            out
        };
        assert_eq!(run(), run(), "{branch} @ {bit_count} is not deterministic");
    }
}

#[test]
fn tail_padding_stays_zero() {
    // The transform must not write into the padding above `bit_count`; if it did,
    // re-encoding a decoded payload could not reproduce the original bytes.
    let payload = from_hex(PAYLOAD_HEX);
    for (branch, bit_count, _) in VECTORS {
        if bit_count == 0 || bit_count % 8 == 0 {
            continue;
        }
        let version = TransformVersion::require(branch).unwrap();
        let mut out = vec![0u8; TransformVersion::output_byte_count(bit_count)];
        let mut reader = BitReader::new(&payload);
        version
            .decode_from(
                &mut reader,
                bit_count,
                seed_for(bit_count, ACTOR_NET_GUID),
                &mut out,
            )
            .unwrap();
        let used = bit_count % 8;
        let padding = out[out.len() - 1] >> used;
        assert_eq!(
            padding, 0,
            "{branch} @ {bit_count} bits left non-zero padding"
        );
    }
}
