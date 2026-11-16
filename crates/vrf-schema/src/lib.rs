//! Dynamic schema received inline from the replay stream.
//!
//! Unreal replays ship no fixed schema: *net field export groups*, the mapping
//! from numeric field handles to names, arrive inside the stream and are
//! accumulated exactly as sent. A handle means different fields in different
//! groups and builds, so none is ever hard-coded.
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
//! | `reader` | The ReplayData wire format for exports and export GUIDs, and [`load_object`] (vrf-net's too) |
//! | `checkpoint` | The two tables a Checkpoint archive carries |
//!
//! # Cargo features
//!
//! | Feature | Default | Effect |
//! |---------|---------|--------|
//! | `checkpoint` | on | [`read_checkpoint_tables`] and [`CheckpointTables`]. A consumer reading only the ReplayData stream never calls them |
//!
//! The cache, the export types and the ReplayData readers are ungated.

#![forbid(unsafe_code)]

mod cache;
mod error;
mod export;
mod guid;
pub mod hash;
mod path;
mod reader;
mod resolve;

#[cfg(feature = "checkpoint")]
mod checkpoint;

pub use cache::{NetGuidCache, SchemaLimits};
pub use error::SchemaError;
pub use export::{NetFieldExport, NetFieldExportGroup};
pub use guid::{ExportFlags, NetGuidEntry, NetworkGuid};
pub use hash::{FxHashMap, FxHashSet};
pub use path::{
    CLASS_NET_CACHE_SUFFIX, find_class_net_cache_key, find_replay_path_key,
    for_each_replay_path_key, has_path_separator,
};
pub use reader::{load_object, read_export_guids, read_net_field_exports};

#[cfg(feature = "checkpoint")]
pub use checkpoint::{
    CheckpointPathMode, CheckpointReadError, CheckpointTableSink, CheckpointTables,
    read_checkpoint_tables, read_checkpoint_tables_with_sink,
    read_checkpoint_tables_with_sink_mode,
};
