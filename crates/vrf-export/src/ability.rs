//! Reconstruct per-round ability casts from the `fields` stream.
//!
//! `AbilityCastsThisRound` is a replicated array on `PlayerState` that is
//! **reused every round**: a consumer reading its final value sees the last
//! round alone. The rows that carry it arrive in stream order, one leaf row per
//! `AbilityCastsThisRound[<idx>].<Prop>_<checksum>` path, so walking them and
//! emitting on change recovers every round.
//!
//! Measured on build `release-13.10` (22 rounds): final-value reads yield 45
//! located casts, this delta walk yields 2,137. Sorting by `time_ms` loses
//! ~18%, because that column is not monotonic (`docs/USAGE.md`); the walk
//! therefore consumes rows in the order they were written and never sorts.

use std::collections::{HashMap, HashSet};

use crate::record::{AbilityCastRecord, FieldRecord};

const PREFIX: &str = "AbilityCastsThisRound[";

#[derive(Default, Clone)]
struct Pending {
    round: Option<i64>,
    slot: Option<i64>,
    cast_time: Option<f64>,
    loc: Option<(f64, f64, f64)>,
}

/// Streaming extractor. Feed it every `FieldRecord` in write order and collect
/// the rows it emits; it keeps one small state entry per `(actor, index)`.
#[derive(Default)]
pub struct AbilityCastExtractor {
    pending: HashMap<(u32, u32), Pending>,
    /// Seen `(actor, round, slot, cast_time, x, y, z)`. The replicated array
    /// shifts and re-sends whole elements, so an index is not an identity and
    /// the tuple is: the same cast reappears under a different index.
    emitted: HashSet<(u32, i64, i64, u64, u64, u64, u64)>,
    /// Rows whose path descends into a nested array (`Effects[...]`), which are
    /// not casts; kept as a counter so a caller can see the walk skipped them.
    pub nested_rows_skipped: u64,
}

impl AbilityCastExtractor {
    /// Parse `AbilityCastsThisRound[<idx>].<Prop>_<..>` into its parts.
    fn parse(name: &str) -> Option<(u32, &str)> {
        let rest = name.strip_prefix(PREFIX)?;
        let (idx, tail) = rest.split_once("].")?;
        let idx: u32 = idx.parse().ok()?;
        let prop = tail.split('_').next()?;
        if prop.contains('[') {
            return None;
        }
        Some((idx, prop))
    }

    /// `"(-3018.0,-611.1,500.2)"` -> `(-3018.0, -611.1, 500.2)`.
    fn parse_loc(s: &str) -> Option<(f64, f64, f64)> {
        let t = s.trim().trim_start_matches('(').trim_end_matches(')');
        let mut it = t.split(',');
        let x: f64 = it.next()?.trim().parse().ok()?;
        let y: f64 = it.next()?.trim().parse().ok()?;
        let z: f64 = it.next()?.trim().parse().ok()?;
        Some((x, y, z))
    }

    /// Consume one row. Emits at most one cast into `out`.
    pub fn push(&mut self, row: &FieldRecord, out: &mut Vec<AbilityCastRecord>) {
        let Some(name) = row.field_name.as_deref() else {
            return;
        };
        if !name.starts_with(PREFIX) {
            return;
        }
        if name.contains("Effects[") {
            self.nested_rows_skipped += 1;
            return;
        }
        let Some((idx, prop)) = Self::parse(name) else {
            return;
        };
        let key = (row.actor_net_guid, idx);
        let p = self.pending.entry(key).or_default();
        match prop {
            "Round" => {
                // A new round reuses this index: the other members of the
                // previous cast must not leak into the new one.
                if p.round != row.value_i64 {
                    p.cast_time = None;
                    p.loc = None;
                    p.slot = None;
                }
                p.round = row.value_i64;
            }
            "Slot" => p.slot = row.value_i64,
            "CastTime" => p.cast_time = row.value_f64,
            "CastLocation" => {
                if let Some(s) = row.value_str.as_deref() {
                    p.loc = Self::parse_loc(s);
                }
            }
            // Player / RoundPhase / DestroyedCount and anything else we do not
            // need for the row; they still arrive, they just do not emit.
            _ => {}
        }
        let (Some(round), Some(slot), Some(cast_time), Some(loc)) =
            (p.round, p.slot, p.cast_time, p.loc)
        else {
            return;
        };
        if round < 0 {
            return;
        }
        // The replicated array shifts as casts are added and whole snapshots
        // are re-sent, so an index is not an identity and the tuple is: the
        // same cast reappears under a different index.
        if !self.emitted.insert((
            row.actor_net_guid,
            round,
            slot,
            cast_time.to_bits(),
            loc.0.to_bits(),
            loc.1.to_bits(),
            loc.2.to_bits(),
        )) {
            return;
        }
        out.push(AbilityCastRecord {
            time_ms: row.time_ms,
            packet_id: row.packet_id,
            round: round as u32 + 1,
            actor_net_guid: row.actor_net_guid,
            cast_index: idx,
            slot,
            cast_time,
            x: loc.0,
            y: loc.1,
            z: loc.2,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn row(actor: u32, name: &str, i: Option<i64>, f: Option<f64>, s: Option<&str>) -> FieldRecord {
        FieldRecord {
            time_ms: 1000,
            packet_id: 7,
            channel_index: 0,
            actor_net_guid: actor,
            object_net_guid: None,
            group_path: Arc::from("Comp_AbilityStatisticsReplicator"),
            handle: 0,
            field_name: Some(Arc::from(name)),
            compatible_checksum: None,
            bit_count: 0,
            raw_bits: None,
            value_i64: i,
            value_f64: f,
            value_bool: None,
            value_str: s.map(|v| v.to_string()),
        }
    }

    fn cast(actor: u32, idx: u32, round: i64, slot: i64, t: f64, loc: &str) -> Vec<FieldRecord> {
        let p = format!("AbilityCastsThisRound[{idx}].");
        vec![
            row(actor, &format!("{p}Round_22_AA"), Some(round), None, None),
            row(actor, &format!("{p}Slot_12_BB"), Some(slot), None, None),
            row(actor, &format!("{p}CastTime_4_CC"), None, Some(t), None),
            row(
                actor,
                &format!("{p}CastLocation_21_DD"),
                None,
                None,
                Some(loc),
            ),
        ]
    }

    #[test]
    fn repeated_snapshots_emit_once() {
        let mut ex = AbilityCastExtractor::default();
        let mut out = Vec::new();
        for _ in 0..3 {
            for r in cast(562, 0, 0, 3, 28.4, "(-3018.0,-611.0,500.2)") {
                ex.push(&r, &mut out);
            }
        }
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].round, 1);
        assert_eq!(out[0].slot, 3);
        assert_eq!(out[0].x, -3018.0);
    }

    #[test]
    fn round_change_emits_a_new_cast() {
        let mut ex = AbilityCastExtractor::default();
        let mut out = Vec::new();
        for r in cast(562, 0, 0, 3, 9.0, "(1.0,2.0,3.0)") {
            ex.push(&r, &mut out);
        }
        // Same array index, next round, different cast.
        for r in cast(562, 0, 1, 5, 12.0, "(4.0,5.0,6.0)") {
            ex.push(&r, &mut out);
        }
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].round, 2);
        assert_eq!(out[1].slot, 5);
    }

    #[test]
    fn nested_effect_paths_are_skipped() {
        let mut ex = AbilityCastExtractor::default();
        let mut out = Vec::new();
        let r = row(
            562,
            "AbilityCastsThisRound[0].Effects[0].Value_7_EE",
            Some(1),
            None,
            None,
        );
        ex.push(&r, &mut out);
        assert!(out.is_empty());
        assert_eq!(ex.nested_rows_skipped, 1);
    }

    #[test]
    fn incomplete_cast_does_not_emit() {
        let mut ex = AbilityCastExtractor::default();
        let mut out = Vec::new();
        let r = row(
            562,
            "AbilityCastsThisRound[0].Round_22_AA",
            Some(0),
            None,
            None,
        );
        ex.push(&r, &mut out);
        assert!(out.is_empty());
    }
}
