//! Counters and the per-field breakdown an overlay pass accumulates: the export
//! summary's only view of the overlay, with the report saying *which* field
//! failed. The reference replay records zero decode errors, so all but the
//! plain counters is a cold path, written for clarity.

use std::collections::HashMap;

use vrf_bitio::BitError;

use crate::decode::{DecodeError, FieldType};
use crate::ftext::FTextTreeError;

/// Why a decode failed, so the operator knows whether to fix the overlay type,
/// the bit-count expectation or something structural. It is the report's only
/// per-cause column (`field_name` says which field), so every failure maps to
/// its own cause through exhaustive matches with no wildcard: a new error
/// variant has to be classified before it compiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum DecodeErrorKind {
    /// BitReader reached EOF before the decoder finished consuming: the type
    /// needs more bits than the field carries.
    Eof,
    /// Decoder finished but bits remained unconsumed: the type is narrower
    /// than the field, or not its layout.
    Residual,
    /// Zero-bit payload with a non-zero-expecting type.
    ZeroBits,
    /// The bits are not a valid encoding of the type: an IntPacked that never
    /// terminates or overflows, a length prefix longer than the payload or its
    /// cap, a string that is not UTF-8 or UTF-16, a framing rule the payload
    /// breaks. `Eof` and `Residual` say the type and the field disagree about
    /// length; this says the content itself is not the type.
    Malformed,
    /// The bits decoded, but to something the decoder refuses rather than
    /// render wrong: an unsigned value past `i64::MAX`, a non-finite
    /// component, an FText history or FName number with no display form, a
    /// length over the table's configured cap -- or a table parameter (a zero
    /// quantization scale or `SerializedInt` maximum) no value can be read
    /// against. Usually a table entry to revisit rather than a wire problem.
    Rejected,
}

/// `pad`, not `write_str`: the report prints this in a padded column, and
/// `write_str` ignores the width it is formatted with.
impl std::fmt::Display for DecodeErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(match self {
            Self::Eof => "EOF",
            Self::Residual => "Residual",
            Self::ZeroBits => "ZeroBits",
            Self::Malformed => "Malformed",
            Self::Rejected => "Rejected",
        })
    }
}

impl DecodeErrorKind {
    /// The kind of a bit-level read failure, shared by every decoder that
    /// reports into [`OverlayErrorReport`]. Only [`BitError::Eof`] is an EOF.
    /// Invalid caller parameters are `Rejected`, not `Malformed`: no payload
    /// bit is at fault when a width, scale or maximum is invalid.
    pub fn from_bit_error(err: &BitError) -> Self {
        match err {
            BitError::Eof { .. } => Self::Eof,
            BitError::MalformedIntPacked { .. }
            | BitError::IntPackedOverflow { .. }
            | BitError::InvalidLength { .. }
            | BitError::InvalidBitLength { .. }
            | BitError::InvalidString { .. } => Self::Malformed,
            BitError::InvalidSerializedIntMax { .. }
            | BitError::InvalidReadWidth { .. }
            | BitError::InvalidRotatorWidth { .. }
            | BitError::InvalidVectorScale { .. } => Self::Rejected,
        }
    }

    /// The kind of an overlay decode failure. A refusal (the bits read fine but
    /// say something the decoder will not return) is `Rejected`: a mistyped
    /// FText, this repo's costliest bug shape, must not read as leftover bits.
    pub(crate) fn from_decode_error(err: &DecodeError) -> Self {
        match err {
            DecodeError::BitIo(bit) => Self::from_bit_error(bit),
            DecodeError::NotFullyConsumed { .. } => Self::Residual,
            // Unreachable: the overlay returns before decoding Raw/Skip.
            DecodeError::RawOrSkip => Self::ZeroBits,
            DecodeError::UnsignedOverflow { .. }
            | DecodeError::UnsupportedTextHistory { .. }
            | DecodeError::NonFiniteComponent { .. }
            | DecodeError::InvalidQuantizationScale { .. }
            | DecodeError::InvalidFNameNumber { .. }
            | DecodeError::ByteArrayLengthCapExceeded { .. } => Self::Rejected,
            DecodeError::FTextTree(tree) => match tree {
                FTextTreeError::BitIo(bit) => Self::from_bit_error(bit),
                FTextTreeError::TrailingBits { .. } => Self::Residual,
                FTextTreeError::UnsupportedHistory { .. }
                | FTextTreeError::UnsupportedNameForm
                | FTextTreeError::UnsupportedArgumentTag { .. }
                | FTextTreeError::NegativeNameSuffix { .. }
                | FTextTreeError::NonFiniteNumber => Self::Rejected,
                FTextTreeError::InvalidArgumentCount { .. }
                | FTextTreeError::InvalidEmptyForm
                | FTextTreeError::InvalidBool { .. }
                | FTextTreeError::StringTooLong { .. }
                | FTextTreeError::MissingStringTerminator
                | FTextTreeError::DepthLimit { .. }
                | FTextTreeError::NodeLimit { .. } => Self::Malformed,
            },
        }
    }
}

/// Per-(group, field, type, bit_count, error_kind) failure counts: call
/// [`Self::record`] on every failure, then [`Self::top_n`] for the dominant
/// sources.
#[derive(Debug, Clone, Default)]
pub struct OverlayErrorReport {
    /// Key: (group_path, field_name, field_type_tag, bit_count, error_kind).
    /// Value: occurrence count.
    counts: HashMap<(String, String, String, u32, DecodeErrorKind), u64>,
}

/// One row from the error report, sorted by descending count.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct OverlayErrorRow {
    pub count: u64,
    pub group_path: String,
    pub field_name: String,
    pub declared_type: String,
    pub bit_count: u32,
    pub error_kind: DecodeErrorKind,
}

impl OverlayErrorReport {
    /// Record a decode failure.
    pub fn record(
        &mut self,
        group_path: &str,
        field_name: &str,
        field_type: FieldType,
        bit_count: u32,
        kind: DecodeErrorKind,
    ) {
        let key = (
            group_path.to_owned(),
            field_name.to_owned(),
            format!("{field_type:?}"),
            bit_count,
            kind,
        );
        *self.counts.entry(key).or_insert(0) += 1;
    }

    /// Merge another report into this one (additive counts).
    pub fn merge_from(&mut self, other: &Self) {
        for (key, &count) in &other.counts {
            *self.counts.entry(key.clone()).or_insert(0) += count;
        }
    }

    /// Return the top `n` error sources sorted by descending count.
    pub fn top_n(&self, n: usize) -> Vec<OverlayErrorRow> {
        let mut rows: Vec<OverlayErrorRow> = self
            .counts
            .iter()
            .map(|((gp, fn_, dt, bc, ek), &cnt)| OverlayErrorRow {
                count: cnt,
                group_path: gp.clone(),
                field_name: fn_.clone(),
                declared_type: dt.clone(),
                bit_count: *bc,
                error_kind: *ek,
            })
            .collect();
        // Descending count, then the row's identity: ties left in HashMap
        // order varied run to run and made the report non-reproducible.
        rows.sort_by(|a, b| {
            b.count.cmp(&a.count).then_with(|| {
                a.group_path
                    .cmp(&b.group_path)
                    .then_with(|| a.field_name.cmp(&b.field_name))
                    .then_with(|| a.declared_type.cmp(&b.declared_type))
                    .then_with(|| a.bit_count.cmp(&b.bit_count))
                    .then_with(|| a.error_kind.cmp(&b.error_kind))
            })
        });
        rows.truncate(n);
        rows
    }

    /// Total number of distinct error buckets.
    pub fn bucket_count(&self) -> usize {
        self.counts.len()
    }

    /// Total errors across all buckets.
    pub fn total_errors(&self) -> u64 {
        self.counts.values().sum()
    }
}

/// Statistics from an overlay pass.
#[derive(Debug, Clone, Default)]
pub struct OverlayStats {
    /// Fields where the type was known and decoding succeeded.
    pub decoded_ok: u64,
    /// Fields where the type was known but decoding failed.
    pub decoded_err: u64,
    /// Fields where the type is Raw/Skip (intentionally not decoded).
    pub raw_or_skip: u64,
    /// Fields where (group_path, field_name) had no entry in the table.
    pub not_in_table: u64,
    /// Fields where field_name was None (unmapped handle).
    pub no_field_name: u64,
    /// Handle fallbacks refused because the replay declared a DIFFERENT,
    /// unresolved field name at that handle (docs/OVERLAY_RESOLUTION.md
    /// "Fail-closed on a handle conflict"). Not a `decoded_err`: nothing failed
    /// to decode, the overlay declined to claim a type.
    pub handle_conflicts_refused: u64,
    /// Per-field breakdown of the failures, recorded on every failure.
    pub error_report: OverlayErrorReport,
}

impl OverlayStats {
    /// Add the six counters of `other` into `self`. `error_report` is not
    /// merged: callers fold it into one report shared by every pass, so a
    /// checkpoint-only failure still reaches the summary. The destructure has
    /// no `..`, so a new counter does not compile until it is summed here,
    /// where both the export and `diag` totals come from.
    pub fn merge_counts_from(&mut self, other: &Self) {
        let Self {
            decoded_ok,
            decoded_err,
            raw_or_skip,
            not_in_table,
            no_field_name,
            handle_conflicts_refused,
            error_report: _,
        } = other;
        self.decoded_ok += decoded_ok;
        self.decoded_err += decoded_err;
        self.raw_or_skip += raw_or_skip;
        self.not_in_table += not_in_table;
        self.no_field_name += no_field_name;
        self.handle_conflicts_refused += handle_conflicts_refused;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eight buckets that all carry the same count, so every pair is a tie and
    /// only the tiebreak decides the order. Named out of alphabetical order so
    /// a report that echoes insertion order cannot pass by accident.
    const TIED: [(&str, &str); 8] = [
        ("GroupD", "delta"),
        ("GroupA", "beta"),
        ("GroupC", "gamma"),
        ("GroupA", "alpha"),
        ("GroupB", "epsilon"),
        ("GroupD", "alpha"),
        ("GroupB", "beta"),
        ("GroupC", "alpha"),
    ];

    fn tied_report() -> OverlayErrorReport {
        let mut report = OverlayErrorReport::default();
        for (group, field) in TIED {
            report.record(group, field, FieldType::Int32, 32, DecodeErrorKind::Eof);
        }
        report
    }

    #[test]
    fn top_n_breaks_count_ties_by_group_then_field() {
        let rows = tied_report().top_n(TIED.len());
        let order: Vec<(&str, &str)> = rows
            .iter()
            .map(|r| (r.group_path.as_str(), r.field_name.as_str()))
            .collect();
        assert_eq!(
            order,
            vec![
                ("GroupA", "alpha"),
                ("GroupA", "beta"),
                ("GroupB", "beta"),
                ("GroupB", "epsilon"),
                ("GroupC", "alpha"),
                ("GroupC", "gamma"),
                ("GroupD", "alpha"),
                ("GroupD", "delta"),
            ]
        );
    }

    #[test]
    fn top_n_still_puts_the_biggest_count_first() {
        let mut report = tied_report();
        report.record(
            "GroupD",
            "delta",
            FieldType::Int32,
            32,
            DecodeErrorKind::Eof,
        );
        let rows = report.top_n(1);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 2);
        assert_eq!(rows[0].group_path, "GroupD");
    }

    #[test]
    fn top_n_breaks_identity_ties_by_error_kind() {
        let mut report = OverlayErrorReport::default();
        // Inserted out of order, so a report echoing insertion order fails.
        for kind in [
            DecodeErrorKind::ZeroBits,
            DecodeErrorKind::Rejected,
            DecodeErrorKind::Residual,
            DecodeErrorKind::Malformed,
            DecodeErrorKind::Eof,
        ] {
            report.record("Group", "Field", FieldType::Int32, 32, kind);
        }

        let kinds: Vec<DecodeErrorKind> = report
            .top_n(5)
            .into_iter()
            .map(|row| row.error_kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                DecodeErrorKind::Eof,
                DecodeErrorKind::Residual,
                DecodeErrorKind::ZeroBits,
                DecodeErrorKind::Malformed,
                DecodeErrorKind::Rejected,
            ]
        );
    }

    /// Every `BitError`, and only `Eof` is an EOF.
    #[test]
    fn only_a_bit_level_eof_is_labelled_eof() {
        let cases = [
            (
                BitError::Eof {
                    position: 0,
                    length: 8,
                    requested: 8,
                },
                DecodeErrorKind::Eof,
            ),
            (
                BitError::MalformedIntPacked { position: 0 },
                DecodeErrorKind::Malformed,
            ),
            (
                BitError::IntPackedOverflow { position: 0 },
                DecodeErrorKind::Malformed,
            ),
            (
                BitError::InvalidLength {
                    position: 0,
                    length: 100,
                },
                DecodeErrorKind::Malformed,
            ),
            (
                BitError::InvalidBitLength {
                    requested: 16,
                    available: 8,
                },
                DecodeErrorKind::Malformed,
            ),
            (
                BitError::InvalidString { position: 0 },
                DecodeErrorKind::Malformed,
            ),
            (
                BitError::InvalidSerializedIntMax { max: 0 },
                DecodeErrorKind::Rejected,
            ),
            (
                BitError::InvalidReadWidth { count: 65 },
                DecodeErrorKind::Rejected,
            ),
            (
                BitError::InvalidRotatorWidth { width: 32 },
                DecodeErrorKind::Rejected,
            ),
            (
                BitError::InvalidVectorScale { scale: 0 },
                DecodeErrorKind::Rejected,
            ),
        ];
        for (err, want) in cases {
            assert_eq!(DecodeErrorKind::from_bit_error(&err), want, "{err:?}");
        }
    }

    /// The report prints the kind in a padded column, which a label written
    /// with `write_str` ignores.
    #[test]
    fn kind_labels_honour_the_report_column_width() {
        for kind in [
            DecodeErrorKind::Eof,
            DecodeErrorKind::Residual,
            DecodeErrorKind::ZeroBits,
            DecodeErrorKind::Malformed,
            DecodeErrorKind::Rejected,
        ] {
            let cell = format!("{kind:<9}|");
            assert_eq!(cell.len(), 10, "{cell:?}");
            assert!(cell.starts_with(&kind.to_string()), "{cell:?}");
        }
        // And unpadded where no width is asked for.
        assert_eq!(DecodeErrorKind::Malformed.to_string(), "Malformed");
    }

    fn distinct_counts(base: u64) -> OverlayStats {
        OverlayStats {
            decoded_ok: base + 1,
            decoded_err: base + 2,
            raw_or_skip: base + 3,
            not_in_table: base + 4,
            no_field_name: base + 5,
            handle_conflicts_refused: base + 6,
            error_report: tied_report(),
        }
    }

    /// Each counter lands in its own total, and the per-field breakdown is
    /// left alone: callers merge it into a report shared across passes.
    #[test]
    fn merge_counts_from_sums_every_counter_and_leaves_the_report() {
        let mut total = OverlayStats::default();
        total.merge_counts_from(&distinct_counts(0));
        total.merge_counts_from(&distinct_counts(100));
        assert_eq!(total.decoded_ok, 1 + 101);
        assert_eq!(total.decoded_err, 2 + 102);
        assert_eq!(total.raw_or_skip, 3 + 103);
        assert_eq!(total.not_in_table, 4 + 104);
        assert_eq!(total.no_field_name, 5 + 105);
        assert_eq!(total.handle_conflicts_refused, 6 + 106);
        assert_eq!(total.error_report.total_errors(), 0);
    }
}
