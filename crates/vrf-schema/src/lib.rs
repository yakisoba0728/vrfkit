//! Dynamic schema received inline from the replay stream.
//!
//! # Why this exists
//!
//! Unreal Engine replays do **not** ship a fixed, pre-declared schema. Instead,
//! the server transmits *net field export groups* -- the mapping from numeric
//! field handles to human-readable names -- **inside the replay stream itself at
//! runtime**. A parser cannot hard-code these mappings; it must receive and
//! accumulate them exactly as the engine sends them.
//!
//! This crate captures that mechanism:
//!
//! * [`NetFieldExport`] -- a single field descriptor (handle + name + checksum).
//! * [`NetFieldExportGroup`] -- one named group holding many field descriptors.
//! * [`NetGuidCache`] -- the replay-wide accumulator that indexes groups by path
//!   and by numeric `path_name_index`, plus the NetGUID->object-path mapping.
//! * [`read_net_field_exports`] / [`read_export_guids`] -- functions that consume
//!   the wire format from a [`vrf_bitio::BitReader`] and populate the cache.
//!
//! # Performance contract
//!
//! Field-handle lookups happen millions of times per replay (once per replicated
//! property in every content block). Groups are looked up by `path_name_index`
//! via `HashMap` (O(1)), and fields within a group are accessed by direct `Vec`
//! indexing (O(1), no hashing overhead).
//!
//! # The handle->name mapping is replay-supplied, not hard-coded
//!
//! This is crucial: the numeric handle `3` might mean `"Location"` in one group
//! and `"Rotation"` in another, and the assignment can differ between game
//! builds. The parser must never assume a fixed handle->name mapping; it always
//! reads the schema the replay provides.
//!
//! # Module map
//!
//! | Module | Responsibility |
//! |--------|----------------|
//! | `guid` | [`NetworkGuid`], [`ExportFlags`], [`NetGuidEntry`] -- wire vocabulary |
//! | `export` | [`NetFieldExport`], [`NetFieldExportGroup`] -- one group's fields |
//! | `cache` | [`NetGuidCache`] storage and the direct lookups |
//! | `resolve` | Bare-name resolution over the leaf index |
//! | `path` | Alias generation (`Default__`, `/_Core/`, `_ClassNetCache`) |
//! | `hash` | The hasher the cache's maps use, and why it is not the default |
//! | `reader` | The ReplayData wire format for exports and export GUIDs |
//! | `checkpoint` | The two tables a Checkpoint archive carries |
//!
//! # Cargo features
//!
//! | Feature | Default | Effect |
//! |---------|---------|--------|
//! | `checkpoint` | on | [`read_checkpoint_tables`] and [`CheckpointTables`]. A consumer reading only the ReplayData stream never calls them |
//!
//! The cache, the export types and the ReplayData readers are not gated: they
//! are the crate's reason for existing, and every consumer needs all three.

#![forbid(unsafe_code)]

mod cache;
mod error;
mod export;
mod guid;
mod hash;
mod path;
mod reader;
mod resolve;

#[cfg(feature = "checkpoint")]
mod checkpoint;

pub use cache::NetGuidCache;
pub use error::SchemaError;
pub use export::{NetFieldExport, NetFieldExportGroup};
pub use guid::{ExportFlags, NetGuidEntry, NetworkGuid};
pub use path::{class_net_cache_lookup_keys, replay_path_lookup_keys};
pub use reader::{read_export_guids, read_net_field_exports};

#[cfg(feature = "checkpoint")]
pub use checkpoint::{CheckpointTables, read_checkpoint_tables};
