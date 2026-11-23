//! One module per export table, each behind its own feature, so a consumer can
//! take `fields` without linking the movement writer. The record structs are
//! not gated: the validation oracle builds records for tables it does not
//! write.

#[cfg(feature = "ability-casts")]
pub mod ability_casts;
#[cfg(feature = "actors")]
pub mod actors;
#[cfg(feature = "checkpoint-context")]
pub mod checkpoints;
#[cfg(feature = "events")]
pub mod events;
#[cfg(feature = "fields")]
pub mod fields;
#[cfg(feature = "movement")]
pub mod movement;
#[cfg(feature = "net-guids")]
pub mod net_guids;
#[cfg(feature = "partials")]
pub mod partials;

/// `RecordBatch::try_new`, with its error as this crate's.
#[cfg(any(
    feature = "ability-casts",
    feature = "actors",
    feature = "checkpoint-context",
    feature = "events",
    feature = "fields",
    feature = "movement",
    feature = "net-guids",
    feature = "partials"
))]
fn batch(
    schema: std::sync::Arc<arrow_schema::Schema>,
    columns: Vec<arrow_array::ArrayRef>,
) -> Result<arrow_array::RecordBatch, crate::ExportError> {
    arrow_array::RecordBatch::try_new(schema, columns)
        .map_err(|e| crate::ExportError::Parquet(e.into()))
}
