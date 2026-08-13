//! Counters and the per-field breakdown an overlay pass accumulates.
//!
//! These are the export summary's only view of what the overlay did, and the
//! error report is what tells an operator *which* field to look at rather than
//! just how many failed. The reference replay currently records zero decode
//! errors, so everything here except the plain counters is a cold path; it is
//! written for clarity, not for speed.

use std::collections::HashMap;

use crate::decode::FieldType;

/// Categorisation of a decode failure -- distinguishes root cause so the
/// operator knows whether to fix the overlay type, the bit-count expectation,
/// or something structural.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecodeErrorKind {
    /// BitReader reached EOF before the decoder finished consuming.
    Eof,
    /// Decoder finished but bits remained unconsumed.
    Residual,
    /// Zero-bit payload with a non-zero-expecting type.
    ZeroBits,
}

impl std::fmt::Display for DecodeErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Eof => f.write_str("EOF"),
            Self::Residual => f.write_str("Residual"),
            Self::ZeroBits => f.write_str("ZeroBits"),
        }
    }
}

impl DecodeErrorKind {
    const fn sort_key(self) -> u8 {
        match self {
            Self::Eof => 0,
            Self::Residual => 1,
            Self::ZeroBits => 2,
        }
    }
}

/// Accumulates per-(group, field, type, bit_count, error_kind) counts so the
/// operator can identify the dominant decode-error sources after an export run.
///
/// Designed for long-lived use: call [`Self::record`] on every failure, then
/// [`Self::top_n`] to get the sorted report.
#[derive(Debug, Clone, Default)]
pub struct OverlayErrorReport {
    /// Key: (group_path, field_name, field_type_tag, bit_count, error_kind).
    /// Value: occurrence count.
    counts: HashMap<(String, String, String, u32, DecodeErrorKind), u64>,
}

/// One row from the error report, sorted by descending count.
#[derive(Debug, Clone)]
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
        // Deterministic ordering: descending count, then by the row's identity
        // fields so equal-count buckets have a stable order run-to-run. A plain
        // `Reverse(count)` left ties in HashMap iteration order, which varied
        // per run and made the printed error report non-reproducible.
        rows.sort_by(|a, b| {
            b.count.cmp(&a.count).then_with(|| {
                a.group_path
                    .cmp(&b.group_path)
                    .then_with(|| a.field_name.cmp(&b.field_name))
                    .then_with(|| a.declared_type.cmp(&b.declared_type))
                    .then_with(|| a.bit_count.cmp(&b.bit_count))
                    .then_with(|| a.error_kind.sort_key().cmp(&b.error_kind.sort_key()))
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
    /// unresolved field name at that handle.
    ///
    /// The fallback exists for handles the wire does not name, and for handles
    /// it names only as a bare decimal FName index (`"248"`), which says
    /// nothing about the property. It used to fire for a real conflicting name
    /// too: with the descriptor mapping handle 7 to `OldField: Int32` and the
    /// replay declaring `NewField` there carrying a `Float`, both name probes
    /// missed, the stale handle mapping was reused, and `1.0f32` was reported
    /// as `value_i64 = 1065353216` with `decoded_ok` incremented and
    /// `Decode errors` still zero. That is the exact shape of a game patch
    /// moving a property, and resolution was documented as fail-closed.
    ///
    /// Such a field is now left untyped and counted here rather than typed
    /// wrongly. Untyped is a state this export already models honestly
    /// (`raw_bits` is always present); a confident wrong number is not.
    ///
    /// It is deliberately NOT routed through `decoded_err`: nothing failed to
    /// decode, the overlay declined to claim a type. Counting it as a decode
    /// error would move the corpus off `Decode errors: 0` for a field that was
    /// never decoded at all.
    pub handle_conflicts_refused: u64,
    /// Detailed per-field error breakdown (populated only when reporting is on).
    pub error_report: OverlayErrorReport,
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
    fn top_n_is_identical_across_independently_built_reports() {
        // Each report owns a HashMap with its own RandomState, so tied buckets
        // iterate in a different order per instance. Sorting on count alone let
        // that leak into the printed report; two runs disagreed.
        let first = tied_report().top_n(TIED.len());
        let second = tied_report().top_n(TIED.len());
        let key = |rows: &[OverlayErrorRow]| -> Vec<(String, String)> {
            rows.iter()
                .map(|r| (r.group_path.clone(), r.field_name.clone()))
                .collect()
        };
        assert_eq!(key(&first), key(&second));
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
        for kind in [
            DecodeErrorKind::ZeroBits,
            DecodeErrorKind::Residual,
            DecodeErrorKind::Eof,
        ] {
            report.record("Group", "Field", FieldType::Int32, 32, kind);
        }

        let kinds: Vec<DecodeErrorKind> = report
            .top_n(3)
            .into_iter()
            .map(|row| row.error_kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                DecodeErrorKind::Eof,
                DecodeErrorKind::Residual,
                DecodeErrorKind::ZeroBits,
            ]
        );
    }
}
