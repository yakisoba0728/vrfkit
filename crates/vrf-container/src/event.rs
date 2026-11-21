//! Event chunk parser. An Event chunk is one entry of the server's own labelled
//! timeline (a round start, a death, a plant); the payload's shape is on
//! [`EventChunk`].
//!
//! # Wire layout
//!
//! | Offset | Type | Field |
//! |--------|------|-------|
//! | 0 | FString | Id |
//! | ... | FString | Group |
//! | ... | FString | Metadata |
//! | ... | u32 | Time1 |
//! | ... | u32 | Time2 |
//! | ... | i32 | SizeInBytes |
//! | ... | [u8; SizeInBytes] | payload |

use vrf_bitio::BitReader;

use crate::error::ContainerError;
use crate::io::{declared_body, read_fstring, read_i32, read_u32};
use crate::limits::MAX_FSTRING_BYTES;

/// A parsed Event chunk: the six header fields plus its raw payload, borrowed
/// from the chunk bytes. The payload is
/// `[u32 group tag][N x u32 words][FString "EReplayEventGroup::<Name>"][f32 seconds]`
/// with no word count: `N` per group ([`KNOWN_EVENT_GROUPS`]) is measured, not a
/// format guarantee. So [`parse_event_payload`] takes `N` from the caller and
/// requires exact consumption, and [`parse_known_event_payload`] also requires
/// the measured tag and name.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EventChunk<'a> {
    /// Server-assigned event id, `<replay-guid>_<32 hex digits>` in every
    /// corpus file. Emitted as the wire gives it; no structure is assumed.
    pub id: String,
    /// Event group, e.g. `characterDeath`, `roundStarted`, `spikePlanted`.
    pub group: String,
    /// Free-form metadata; empty is what the wire says, not a missing value.
    pub metadata: String,
    /// First timestamp in milliseconds.
    pub time1: u32,
    /// Second timestamp in milliseconds; equal to `time1` in every corpus file.
    pub time2: u32,
    /// Declared payload size. Validated non-negative and within the chunk.
    pub size_in_bytes: i32,
    /// The payload bytes, exactly `size_in_bytes` of them.
    pub payload: &'a [u8],
    /// Bytes after the payload this layout does not account for; 0 in all
    /// 208,242 Event chunks of 1,014 replays, 11.06-13.06.
    pub trailing_bytes: usize,
}

/// An Event payload's values (shape on [`EventChunk`]), named by wire type: only
/// some groups have evidence for what each word means.
#[derive(Debug, Clone, PartialEq)]
pub struct EventPayload {
    pub tag: u32,
    /// The caller-established number of group-dependent words.
    pub words: Vec<u32>,
    pub name: String,
    pub seconds: f32,
}

/// One Event group whose payload layout the corpus established. The only list
/// is [`KNOWN_EVENT_GROUPS`], which `crates/vrfkit/tests/adapter_contract.rs`
/// compares with `tools/to_valplay_bundle.py`'s allowlists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownEventGroup {
    /// The Event chunk's `group` string, exactly as the wire spells it.
    pub group: &'static str,
    /// Group-dependent `u32` words between the tag and the FString.
    pub word_count: usize,
    /// The public enum-name FString the payload must carry.
    pub payload_name: &'static str,
    /// The leading group tag.
    pub payload_tag: u32,
}

/// Every Event group with a corpus-established payload layout.
pub const KNOWN_EVENT_GROUPS: [KnownEventGroup; 7] = [
    KnownEventGroup {
        group: "characterDeath",
        word_count: 2,
        payload_name: "EReplayEventGroup::CharacterDeath",
        payload_tag: 8,
    },
    KnownEventGroup {
        group: "characterUltimateUsed",
        word_count: 1,
        payload_name: "EReplayEventGroup::CharacterUltimateUsed",
        payload_tag: 11,
    },
    KnownEventGroup {
        group: "roundStarted",
        word_count: 1,
        payload_name: "EReplayEventGroup::RoundStart",
        payload_tag: 2,
    },
    KnownEventGroup {
        group: "switchTeams",
        word_count: 1,
        payload_name: "EReplayEventGroup::SwitchTeams",
        payload_tag: 3,
    },
    KnownEventGroup {
        group: "spikePlanted",
        word_count: 0,
        payload_name: "EReplayEventGroup::SpikePlanted",
        payload_tag: 4,
    },
    KnownEventGroup {
        group: "spikeDefused",
        word_count: 0,
        payload_name: "EReplayEventGroup::SpikeDefused",
        payload_tag: 5,
    },
    KnownEventGroup {
        group: "spikeExploded",
        word_count: 0,
        payload_name: "EReplayEventGroup::SpikeExploded",
        payload_tag: 6,
    },
];

/// A `while` loop over bytes because the accessors are `const fn`: on MSRV 1.86
/// neither iterators nor `==` on `&str`/`&[u8]` are const.
const fn known_event_group(group: &str) -> Option<KnownEventGroup> {
    let wanted = group.as_bytes();
    let mut index = 0;
    while index < KNOWN_EVENT_GROUPS.len() {
        let entry = KNOWN_EVENT_GROUPS[index];
        if bytes_equal(entry.group.as_bytes(), wanted) {
            return Some(entry);
        }
        index += 1;
    }
    None
}

const fn bytes_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

/// The corpus-established word count for an Event group. `None` (an unknown
/// group) means no claim; `Some(0)` is the measured layout of the spike groups.
#[must_use]
pub const fn known_event_word_count(group: &str) -> Option<usize> {
    match known_event_group(group) {
        Some(entry) => Some(entry.word_count),
        None => None,
    }
}

/// The public enum-name FString measured for a known Event group. Requiring it
/// keeps an arbitrary payload string out of a searchable column.
#[must_use]
pub const fn known_event_payload_name(group: &str) -> Option<&'static str> {
    match known_event_group(group) {
        Some(entry) => Some(entry.payload_name),
        None => None,
    }
}

/// The group tag measured for a known Event group, so an enum reorder fails
/// closed instead of publishing a stale tag.
#[must_use]
pub const fn known_event_payload_tag(group: &str) -> Option<u32> {
    match known_event_group(group) {
        Some(entry) => Some(entry.payload_tag),
        None => None,
    }
}

/// Bound for [`event_payload_seconds_matches_time`]: the largest
/// `|seconds * 1000 - Time1|` over 208,242 Event chunks (1,014 replays,
/// 11.06-13.06) is 0.999878 ms, the integer-ms quantisation; 1.001 adds float noise.
pub const EVENT_PAYLOAD_TIME_TOLERANCE_MS: f64 = 1.001;

/// Whether the inner payload's seconds value agrees with the Event chunk's
/// integer millisecond time; a stale offset or a non-finite value does not.
#[must_use]
pub fn event_payload_seconds_matches_time(time_ms: u32, seconds: f32) -> bool {
    seconds.is_finite()
        && (f64::from(seconds) * 1000.0 - f64::from(time_ms)).abs()
            <= EVENT_PAYLOAD_TIME_TOLERANCE_MS
}

/// Parse an Event chunk's inner payload with an established word count. `None`
/// if the count runs past the payload, a primitive is malformed, or bytes are
/// left over, so a changed arity cannot yield values read out of the FString.
#[must_use]
pub fn parse_event_payload(payload: &[u8], word_count: usize) -> Option<EventPayload> {
    // tag + words + FString length + f32, checked before allocating `words`.
    let fixed_bytes = 12usize.checked_add(word_count.checked_mul(4)?)?;
    if fixed_bytes > payload.len() {
        return None;
    }

    let mut reader = BitReader::new(payload);
    let tag = reader.read_u32().ok()?;
    let mut words = Vec::with_capacity(word_count);
    for _ in 0..word_count {
        words.push(reader.read_u32().ok()?);
    }
    let name = reader.read_fstring(MAX_FSTRING_BYTES).ok()?;
    let seconds = reader.read_f32().ok()?;
    if !reader.at_end() {
        return None;
    }

    Some(EventPayload {
        tag,
        words,
        name,
        seconds,
    })
}

/// Parse the payload of a measured Event group: arity, tag and enum-name
/// FString must all match, else `None`.
#[must_use]
pub fn parse_known_event_payload(group: &str, payload: &[u8]) -> Option<EventPayload> {
    let known = known_event_group(group)?;
    let parsed = parse_event_payload(payload, known.word_count)?;
    (parsed.name == known.payload_name && parsed.tag == known.payload_tag).then_some(parsed)
}

/// Parse an Event chunk: the region `data[chunk.data_offset .. + chunk.size_in_bytes]`
/// of a [`RawChunk`](crate::RawChunk) of type [`ChunkType::Event`](crate::ChunkType::Event).
///
/// # Errors
///
/// [`ContainerError::FString`] if a string runs past the chunk,
/// [`ContainerError::Truncated`] if another field or the payload does, and
/// [`ContainerError::InvalidEventPayloadSize`] if `SizeInBytes` is negative.
pub fn parse_event_chunk(payload: &[u8]) -> Result<EventChunk<'_>, ContainerError> {
    let mut reader = BitReader::new(payload);

    let id = read_fstring(&mut reader, "event id", MAX_FSTRING_BYTES)?;
    let group = read_fstring(&mut reader, "event group", MAX_FSTRING_BYTES)?;
    let metadata = read_fstring(&mut reader, "event metadata", MAX_FSTRING_BYTES)?;
    let time1 = read_u32(&mut reader, "event time1")?;
    let time2 = read_u32(&mut reader, "event time2")?;
    let size_in_bytes = read_i32(&mut reader, "event payload size")?;

    let (body, trailing_bytes) = declared_body(
        payload,
        &reader,
        size_in_bytes,
        |size| ContainerError::InvalidEventPayloadSize { size },
        "event payload",
    )?;

    Ok(EventChunk {
        id,
        group,
        metadata,
        time1,
        time2,
        size_in_bytes,
        payload: body,
        trailing_bytes,
    })
}
