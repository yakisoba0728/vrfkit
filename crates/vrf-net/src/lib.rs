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
//!
//! # Module layout
//!
//! One module per layer above, plus the pieces they share:
//!
//! | module | layer |
//! |---|---|
//! | [`packet`] | sentinel sizing, bunch headers, partial-sequence tracking |
//! | [`bunch`] | bunch header struct, partial reassembly |
//! | [`pipeline`] | the reader that drives all of it, and the sink trait |
//! | [`content`] | content block headers |
//! | [`field`] | field and RPC streams |
//! | [`net_guid`] | `InternalLoadObject` |
//! | [`stats`] | counters and the diagnostic event log |
//! | [`types`], [`error`] | shared wire types and the error enum |
//!
//! [`pipeline`] is itself split -- channel lifecycle, spawn data and the
//! per-block framing loop are separate private submodules -- because those
//! three run at rates three orders of magnitude apart and are read against
//! different parts of the wire format. Its public surface is unchanged by that
//! split.
//!
//! # Features
//!
//! - `diagnostics` (default): the per-failure event log in [`stats`]. Turning
//!   it off removes [`stats::DiagnosticEvent`] and the machinery that builds
//!   one; the counters that say *how much* was discarded stay in every build,
//!   because losing them would mean losing data silently. Nothing else in this
//!   crate is optional: packets, bunches, content blocks and fields are one
//!   state machine and cannot be taken apart.

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
