//! The file front end reuses slice parsing, including failure precedence.

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use vrf_container::{PreambleReadError, parse_preamble, read_preamble};
use vrf_testkit::*;

fn preamble() -> Vec<u8> {
    let mut bytes = replay_info(&Info::default());
    bytes.extend(chunk(0, &header_payload()));
    bytes
}

#[test]
fn every_truncated_preamble_matches_the_slice_error() {
    let bytes = preamble();
    for end in 0..bytes.len() {
        let expected = parse_preamble(&bytes[..end]).unwrap_err().to_string();
        let actual = read_preamble(&mut Cursor::new(&bytes[..end])).unwrap_err();
        assert_eq!(actual.to_string(), expected, "prefix length {end}");
        assert!(matches!(actual, PreambleReadError::Container(_)));
    }
}

#[test]
fn invalid_first_chunks_keep_range_checks_and_error_precedence() {
    for kind in [0, 1, 2, 3, 99] {
        for size in [-1, 0, 1, i32::MAX] {
            let mut bytes = replay_info(&Info::default());
            add_u32(&mut bytes, kind);
            add_i32(&mut bytes, size);
            let expected = parse_preamble(&bytes).unwrap_err().to_string();
            let actual = read_preamble(&mut Cursor::new(&bytes)).unwrap_err();
            assert_eq!(actual.to_string(), expected, "kind={kind}, size={size}");
        }
    }
}

struct Counted {
    cursor: Cursor<Vec<u8>>,
    read_bytes: usize,
}

impl Read for Counted {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.cursor.read(buf)?;
        self.read_bytes += n;
        Ok(n)
    }
}

impl Seek for Counted {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.cursor.seek(pos)
    }
}

#[test]
fn maximum_info_and_large_header_are_bounded_and_match_slice_parsing() {
    let mut versions = vec![(LOCAL_REPLAY, 7)];
    versions.extend((1..1024).map(|i| ([i, 0, 0, 0], 0)));
    let mut bytes = replay_info(&Info {
        custom_versions: versions,
        friendly_name: "",
        ..Default::default()
    });
    // Largest admitted friendly name including its NUL terminator.
    let friendly_at = 12 + 1024 * 20 + 12;
    let mut friendly = (65536_i32).to_le_bytes().to_vec();
    friendly.extend(std::iter::repeat_n(b'x', 65535));
    friendly.push(0);
    bytes.splice(friendly_at..friendly_at + 5, friendly);
    bytes.truncate(bytes.len() - 4);
    add_i32(&mut bytes, 4096);
    bytes.extend([1; 4096]);
    let mut header = header_payload();
    header.extend(vec![0xab; 256 * 1024]);
    bytes.extend(chunk(0, &header));
    let expected = format!("{:?}", parse_preamble(&bytes).unwrap());
    let preamble_len = bytes.len();
    bytes.extend(vec![0xcd; 2 * 1024 * 1024]);
    let mut input = Counted {
        cursor: Cursor::new(bytes),
        read_bytes: 0,
    };
    input.cursor.set_position(17); // the function always starts at byte zero
    let actual = read_preamble(&mut input).unwrap();
    assert_eq!(format!("{actual:?}"), expected);
    assert_eq!(actual.remaining_offset, preamble_len);
    assert_eq!(input.cursor.position(), preamble_len as u64);
    assert!(input.read_bytes <= preamble_len + 8);
}

#[test]
fn small_metadata_does_not_read_a_large_following_replay() {
    let mut bytes = preamble();
    let end = bytes.len();
    bytes.extend(vec![0; 2 * 1024 * 1024]);
    let mut input = Counted {
        cursor: Cursor::new(bytes),
        read_bytes: 0,
    };
    assert_eq!(read_preamble(&mut input).unwrap().remaining_offset, end);
    assert!(input.read_bytes < 96 * 1024);
    assert_eq!(input.cursor.position(), end as u64);
}

#[test]
fn io_failures_remain_typed_and_distinct_from_container_errors() {
    struct Failed;
    impl Read for Failed {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "blocked"))
        }
    }
    impl Seek for Failed {
        fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
            Ok(100)
        }
    }
    match read_preamble(&mut Failed).unwrap_err() {
        PreambleReadError::Io(error) => assert_eq!(error.kind(), io::ErrorKind::PermissionDenied),
        other => panic!("lost typed I/O error: {other:?}"),
    }
}
