//! One module per export table.
//!
//! Each module holds that table's row-group size, its dictionary columns, and
//! the row-slice-to-`RecordBatch` conversion -- the three things
//! [`crate::writer::Table`] abstracts over. The buffer-and-flush machinery
//! itself lives once in [`crate::writer`].
//!
//! Every table is behind its own feature so a consumer can take, say, `fields`
//! without linking the movement writer. The record structs are not gated: they
//! carry no Arrow types and callers producing records for a table they do not
//! write (the validation oracle does exactly that) must still be able to name
//! them.

#[cfg(feature = "actors")]
pub mod actors;
#[cfg(feature = "events")]
pub mod events;
#[cfg(feature = "fields")]
pub mod fields;
#[cfg(feature = "movement")]
pub mod movement;
#[cfg(feature = "net-guids")]
pub mod net_guids;
