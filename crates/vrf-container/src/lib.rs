//! `.vrf` container parser: replay info, header, chunk stream, and Oodle decompression.
//!
//! # Container layout
//!
//! A VALORANT replay file (`.vrf`) uses Unreal Engine's local-file replay format.
//! It starts with a fixed **replay info** section, followed by a stream of typed
//! **chunks**. The first chunk must always be a **Header** chunk.
//!
//! ```text
//! +-------------------------------------------+
//! | ReplayInfo                                |
//! |  +- FileMagic (0x43F4EFDD)                |
//! |  +- LegacyFileVersion (7)                 |
//! |  +- CustomVersionContainer                |
//! |  +- Summary (LengthInMs ... EncryptionKey)|
//! +-------------------------------------------+
//! | Chunk 0 (Header)                          |
//! |  +- ChunkType (u32) + SizeInBytes (i32)   |
//! |  +- Header payload                        |
//! +-------------------------------------------+
//! | Chunk 1..N (ReplayData/Checkpoint/Event)  |
//! |  +- ChunkType (u32) + SizeInBytes (i32)   |
//! |  +- Payload (possibly Oodle-compressed)   |
//! +-------------------------------------------+
//! ```
//!
//! # Module map
//!
//! | Module | Responsibility |
//! |--------|----------------|
//! | `limits` | Every magic number and bound, each traced to its C# source |
//! | `chunk` | [`ChunkType`], [`RawChunk`], [`ChunkIterator`] -- the framing layer |
//! | `info` | The leading replay-info section |
//! | `header` | The Header chunk's payload |
//! | `preamble` | Info + Header together, the usual entry point |
//! | `oodle` | ReplayData framing and archive decompression |
//! | `event` | Event chunks: the server's own labelled timeline |
//! | `checkpoint` | Checkpoint chunks: full-state snapshots |
//!
//! # Chunk iteration
//!
//! Use [`ChunkIterator`] to walk chunks lazily without buffering the whole file.
//! For the common case (parse info + header, then iterate data chunks), see
//! [`parse_preamble`] which returns both the parsed preamble and the offset for
//! the remaining chunks.
//!
//! # Oodle decompression
//!
//! ReplayData chunk payloads are compressed with Oodle (Kraken/Mermaid/Selkie).
//! Use [`decompress_replay_data`] to decode a raw chunk payload into plaintext
//! bytes suitable for packet framing.
//!
//! # Event chunks
//!
//! Event chunk payloads are uncompressed and carry the server's own labelled
//! game timeline. Use [`parse_event_chunk`] to read one into an [`EventChunk`];
//! its inner payload is handed back raw, for the reason documented there.
//!
//! # Cargo features
//!
//! All are on by default; each turns off a section of the format a given
//! consumer may never touch.
//!
//! | Feature | Turns off |
//! |---------|-----------|
//! | `oodle` | The `oozextract` dependency. Plaintext chunks still parse; a compressed archive reports [`ContainerError::OodleUnsupported`] |
//! | `event` | [`parse_event_chunk`] and [`EventChunk`] |
//! | `checkpoint` | [`parse_checkpoint_chunk`], [`decompress_checkpoint`] and [`CheckpointChunk`] |
//!
//! The info, header and chunk-iteration layers are **not** gated: every reader
//! of the format needs them to find anything at all, so a flag over them would
//! only offer a build that cannot open a file.

#![forbid(unsafe_code)]

mod chunk;
mod error;
mod header;
mod info;
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
pub use preamble::{Preamble, parse_preamble};

#[cfg(feature = "checkpoint")]
pub use checkpoint::{CheckpointChunk, decompress_checkpoint, parse_checkpoint_chunk};
#[cfg(feature = "event")]
pub use event::{EventChunk, parse_event_chunk};

#[cfg(test)]
mod tests;
