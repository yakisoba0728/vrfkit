//! `.vrf` container parser: replay info, header, chunk stream, and Oodle decompression.
//!
//! A `.vrf` file is Unreal Engine's local-file replay format: a fixed replay
//! info section (file magic `0x43F4EFDD`, legacy file version 7, custom
//! versions, summary), then chunks of `u32` type, `i32` size and payload. The
//! Header chunk comes first; ReplayData, Checkpoint and Event chunks follow,
//! their payloads possibly Oodle-compressed.
//!
//! [`parse_preamble`] reads the info and the Header chunk and returns the offset
//! where the chunk stream resumes; [`ChunkIterator`] walks it lazily from there.
//! [`decompress_replay_data_with_trailing`] inflates a ReplayData chunk, and
//! `parse_event_chunk` and `parse_checkpoint_chunk` read the other two kinds.
//!
//! # Cargo features
//!
//! All are on by default; each turns off a section of the format a given
//! consumer may never touch.
//!
//! | Feature | Turns off |
//! |---------|-----------|
//! | `oodle` | The `oozextract` dependency. Plaintext chunks still parse; a compressed archive reports [`ContainerError::OodleUnsupported`] |
//! | `event` | `parse_event_chunk` and `EventChunk` |
//! | `checkpoint` | `parse_checkpoint_chunk`, `decompress_checkpoint`, `decompress_checkpoint_with_trailing` and `CheckpointChunk` |
//!
//! Info, header and chunk iteration are never gated: nothing can be read without them.

#![forbid(unsafe_code)]

mod chunk;
mod error;
mod header;
mod info;
mod io;
mod limits;
mod oodle;
mod preamble;

#[cfg(feature = "checkpoint")]
mod checkpoint;
#[cfg(feature = "event")]
mod event;

pub use chunk::{ChunkIterator, ChunkType, RawChunk};
pub use error::ContainerError;
pub use header::{ReplayHeader, ReplayVersion};
pub use info::ReplayInfo;
pub use oodle::{
    ReplayDataMeta, decompress_replay_data, decompress_replay_data_with_trailing,
    parse_replay_data_meta,
};
pub use preamble::{Preamble, PreambleReadError, parse_preamble, read_preamble};

#[cfg(feature = "checkpoint")]
pub use checkpoint::{
    CheckpointChunk, decompress_checkpoint, decompress_checkpoint_with_trailing,
    parse_checkpoint_chunk,
};
#[cfg(feature = "event")]
pub use event::{
    EVENT_PAYLOAD_TIME_TOLERANCE_MS, EventChunk, EventPayload, KNOWN_EVENT_GROUPS, KnownEventGroup,
    event_payload_seconds_matches_time, known_event_payload_name, known_event_payload_tag,
    known_event_word_count, parse_event_chunk, parse_event_payload, parse_known_event_payload,
};

#[cfg(test)]
mod tests;
