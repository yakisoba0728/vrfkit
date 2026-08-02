//! Unreal Engine replication layer: packets -> bunches -> content blocks -> fields.
//!
//! # Design intent: exhaustive traversal without descriptors
//!
//! This crate intentionally does **not** skip any field payload. Every property
//! and every RPC is emitted as `(handle, bit_count, raw_bits)` to the caller's
//! sink. This is the reason this project exists as a new implementation rather
//! than wrapping an existing one: a replay contains 780 000+ content blocks and
//! 2 400 000+ movement samples, and the upstream parser's "skip if no
//! descriptor" path means most of that data is silently discarded.
//!
//! Descriptor-free traversal is possible because the field stream is
//! self-describing: each field carries its own handle and bit length, so the
//! reader can always advance to the next field without knowing what type the
//! current one is.
//!
//! # Layers
//!
//! ```text
//! packet  : sentinel-trimmed byte slice -> bit stream
//! bunch   : header + partial reassembly state machine
//! content : framing loop; header + payload per block
//! field   : self-describing handle/size stream
//! ```
//!
//! # What is *not* here (injected by the caller)
//!
//! - Export group path resolution (which names map to which handles)
//! - Typed field decoding (interpreting raw bits as floats, vectors, etc.)
//! - NetGuidCache storage (this crate calls a trait for path registration)
//!
//! # Error policy
//!
//! A malformed bunch is discarded and counted; it does not abort the replay.
//! Silent skipping is forbidden: every discard increments a stat counter.

#![forbid(unsafe_code)]

pub mod bunch;
pub mod content;
pub mod error;
pub mod field;
pub mod net_guid;
pub mod packet;
pub mod pipeline;
pub mod stats;
pub mod types;

pub use error::NetError;
pub use pipeline::{PLAYER_CONTROLLER_LEAF, ReplicationReader, ReplicationSink};
pub use stats::NetStats;
