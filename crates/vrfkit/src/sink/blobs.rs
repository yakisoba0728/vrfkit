//! Additive decoders for two payload shapes the field stream hands over whole.
//!
//! Both are *additive*: the parent row keeps its `raw_bits` and is emitted
//! either way, and these only add rows. A decoder that failed leaves the export
//! exactly as it would have been without it.
//!
//! - **Flattened arrays.** UE serialises a `TArray` of structs by flattening
//!   each element's members onto consecutive handles of the enclosing group, so
//!   the group's own net field exports name them. `decode_struct_array` walks
//!   that, and this module types and emits the leaves.
//! - **Struct blobs.** `RoundResults`, `TeamEconomy` and `RoundInfos` are
//!   opaque to the overlay table but have dedicated decoders in `vrf-decode`.

use vrf_bitio::BitReader;
use vrf_decode::{COMBAT_ROUNDS_SCHEMA, FieldType};
use vrf_schema::NetGuidCache;

use super::intern::put;
use super::{ExportSink, FieldValues, TABLE};

/// The four typed columns a decoded value lands in. At most one is ever
/// populated; see the crate-level note on why this is four nullable columns
/// rather than a union.
type DecodedColumns = (Option<i64>, Option<f64>, Option<bool>, Option<String>);

/// The struct-blob fields that have a dedicated decoder in `vrf-decode`.
#[derive(Clone, Copy)]
enum StructBlob {
    RoundResults,
    TeamEconomy,
    RoundInfos,
}

impl ExportSink<'_> {
    /// Every name the replay declares for `group_path`, indexed by handle.
    ///
    /// This is field-name resolution for a whole group at once, borrowed rather
    /// than cloned. The array walker needs the declaration for each of an
    /// element's flattened members, and resolving per leaf would re-resolve the
    /// group and allocate for every one.
    ///
    /// Empty when the group is unknown, which is exactly the "no declaration"
    /// case `decode_struct_array` falls back from.
    ///
    /// An associated function over `&NetGuidCache` rather than a `&self` method
    /// on purpose: the result borrows for as long as the walker runs, and a
    /// `&self` method would hold all of `self` and collide with the `&mut
    /// self.stats` the same call site needs.
    fn declared_handle_names<'g>(
        cache: &'g NetGuidCache,
        group_path: &str,
    ) -> Vec<Option<&'g str>> {
        let Some(group) = cache.get_group_by_path(group_path) else {
            return Vec::new();
        };
        group
            .fields
            .iter()
            .map(|slot| slot.as_ref().map(|f| f.name.as_str()))
            .collect()
    }

    /// Check if a field name is a known DynamicArray that should be flattened.
    pub(super) fn is_known_array_field(&self, field_name: Option<&str>) -> bool {
        match field_name {
            Some("Rounds") => self.current_group_path.contains("CombatReportComponent"),
            _ => false,
        }
    }

    /// Get the array schema for a known DynamicArray field.
    fn get_array_schema(
        &self,
        field_name: Option<&str>,
    ) -> Option<&'static vrf_decode::ArrayFieldSchema> {
        match field_name {
            Some("Rounds") if self.current_group_path.contains("CombatReportComponent") => {
                Some(&COMBAT_ROUNDS_SCHEMA)
            }
            _ => None,
        }
    }

    /// Flatten a known DynamicArray field and emit one row per leaf.
    pub(super) fn emit_flattened_array(
        &mut self,
        field_name: Option<&str>,
        raw: &[u8],
        bit_count: u32,
    ) {
        let schema = self.get_array_schema(field_name);
        let declared = Self::declared_handle_names(self.cache, &self.current_group_path);
        let flattened = vrf_decode::decode_struct_array(
            raw,
            bit_count,
            schema,
            &declared,
            &mut self.stats.array,
        );
        let parent_name = field_name.unwrap_or("_array");

        // Resolve every leaf's type before touching `self.records`.
        //
        // The overlay table is asked first, keyed on the name the REPLAY
        // declares for that handle. UE flattens an array's element members into
        // consecutive handles on the enclosing group, so the group's own net
        // field export names them -- and the generated table already carries a
        // type for most of them, straight from the C# descriptor.
        //
        // Before this, the only source was `decode_array_leaf`'s hardcoded
        // handle->type match, which is a second copy of knowledge the table
        // already holds and was missing entries. `DeathLocation` is the
        // demonstrable case: handle 104, declared by the replay, typed
        // `VectorDouble` in the table, arriving 3,492 times on 02d4d478 and
        // emitted as an all-null `_h104` because the match had no arm for it.
        //
        // The hardcoded match stays as the fallback: it covers handles whose
        // declared name has no table entry, and dropping it would trade one gap
        // for another.
        let leaf_types: Vec<Option<FieldType>> = flattened
            .iter()
            .map(|f| {
                // The FULL resolution order, not a bare name lookup. An
                // ordinary field gets three steps -- name, b-prefixed name,
                // then handle -> descriptor name -> type -- and a flattened
                // leaf was getting only the first, so the same property could
                // be typed outside an array and untyped inside one.
                let name = declared.get(f.handle as usize).copied().flatten();
                vrf_decode::resolve_field_type(
                    &TABLE,
                    &self.current_group_path,
                    name,
                    Some(f.handle),
                )
                .filter(|ft| !matches!(ft, FieldType::Raw | FieldType::Skip))
            })
            .collect();

        for (f, declared_type) in flattened.iter().zip(leaf_types) {
            // Build full field name: "Rounds[0].RoundNumber" etc. `f.path`
            // already carries its own leading separator.
            let full_name = self.channel_state.names.intern_fmt(|out| {
                out.push_str(parent_name);
                out.push_str(&f.path);
            });

            let (vi, vf, vb, vs) = match declared_type {
                Some(ft) => decode_leaf_with(ft, &f.raw_bits, f.bit_count),
                None => decode_array_leaf(f.handle, &f.raw_bits, f.bit_count),
            };

            self.push_field(FieldValues {
                handle: f.handle,
                field_name: Some(full_name),
                bit_count: f.bit_count,
                raw_bits: Some(f.raw_bits.clone()),
                value_i64: vi,
                value_f64: vf,
                value_bool: vb,
                value_str: vs,
            });
            self.stats.fields_emitted += 1;
        }
    }

    /// The group this block belongs to, with game-mode sibling classes mapped
    /// to the class everything here is keyed on.
    ///
    /// A Swiftplay replay carries `RoundResults` and `TeamEconomy` on
    /// `Swiftplay_EoRCredits_GameState_C`, so a bare `contains("BombGameState")`
    /// silently skips the struct-blob decoders for it -- the export looked
    /// clean and the match had no score, which is section 26 happening again
    /// one game mode over. `vrf_decode::canonical_group` is the single alias
    /// table the overlay uses, so the two cannot disagree about what a game
    /// state is.
    fn canonical_group(&self) -> &str {
        vrf_decode::canonical_group(&self.current_group_path)
    }

    /// Which dedicated decoder owns this field on this group, if any.
    ///
    /// One classifier rather than a predicate and a dispatcher that each spell
    /// the gate out. The field stream asks the predicate whether to hand the
    /// blob over at all and then asks the dispatcher to decode it, so two
    /// copies that disagreed would take the blob off the ordinary path and
    /// then decline it -- the row would lose its decoded leaves and no counter
    /// would move. Section 33 changed this gate for Swiftplay; it had to be
    /// changed in three places.
    fn struct_blob_kind(&self, field_name: Option<&str>) -> Option<StructBlob> {
        match field_name? {
            "RoundResults" if self.canonical_group().contains("BombGameState") => {
                Some(StructBlob::RoundResults)
            }
            "TeamEconomy" if self.canonical_group().contains("BombGameState") => {
                Some(StructBlob::TeamEconomy)
            }
            "RoundInfos" if self.current_group_path.contains("OwnerExclusivePlayerInfo") => {
                Some(StructBlob::RoundInfos)
            }
            _ => None,
        }
    }

    /// Check if a field is a struct blob that has a dedicated decoder.
    pub(super) fn is_struct_blob_field(&self, field_name: Option<&str>) -> bool {
        self.struct_blob_kind(field_name).is_some()
    }

    /// Is this a `MultiItemSlot.MultiContents` blob the additive decoder should
    /// flatten? The parent row stays `Raw` (the overlay does not type it), and
    /// the items are emitted as extra `MultiContents[i]` rows.
    pub(super) fn is_multi_contents_field(&self, field_name: Option<&str>) -> bool {
        matches!(field_name, Some("MultiContents"))
            && self.current_group_path.contains("MultiItemSlot")
    }

    /// Decode a `MultiContents` blob and emit one row per item NetGUID.
    ///
    /// The blob is a RepLayout dynamic array of object references
    /// (`TArray<AAresItem*>`); [`vrf_decode::decode_object_ref_array`] walks the
    /// framing and returns the item actor NetGUIDs. Each lands as a
    /// `MultiContents[i]` row with the NetGUID in `value_i64`, the same column
    /// a single `ItemSlot.Contents` decode populates.
    pub(super) fn emit_multi_contents(&mut self, raw: &[u8], bit_count: u32) {
        let guids = vrf_decode::decode_object_ref_array(raw, bit_count);
        for (i, guid) in guids.iter().enumerate() {
            self.emit_struct_sub_field(
                |out| put(out, format_args!("MultiContents[{i}]")),
                Some(i64::from(*guid)),
                None,
            );
            self.stats.multi_contents_items_emitted += 1;
        }
    }

    /// Decode a struct blob and emit flattened sub-field rows.
    /// Returns true if decoding succeeded and sub-fields were emitted.
    pub(super) fn decode_struct_blob(
        &mut self,
        field_name: &str,
        raw: &[u8],
        bit_count: u32,
    ) -> bool {
        let emitted = match self.struct_blob_kind(Some(field_name)) {
            Some(StructBlob::RoundResults) => self.decode_round_results_blob(raw, bit_count),
            Some(StructBlob::TeamEconomy) => self.decode_team_economy_blob(raw, bit_count),
            Some(StructBlob::RoundInfos) => self.decode_round_infos_blob(raw, bit_count),
            None => false,
        };
        if emitted {
            self.stats.struct_blobs_decoded += 1;
        }
        emitted
    }

    /// Record a struct-blob decode failure instead of dropping it.
    ///
    /// Returns `false` so a call site can `return self.record_blob_failure(..)`
    /// -- the decoders are additive and a failure emits no rows, which is the
    /// same return value the discarding version produced. What is new is that
    /// the run says so.
    fn record_blob_failure(&mut self, err: &dyn std::fmt::Display) -> bool {
        self.stats.struct_blobs_failed += 1;
        if self.stats.struct_blob_first_error.is_none() {
            self.stats.struct_blob_first_error = Some(err.to_string());
        }
        false
    }

    /// Decode RoundResults blob and emit sub-field rows.
    fn decode_round_results_blob(&mut self, raw: &[u8], bit_count: u32) -> bool {
        use vrf_decode::structs::decode_round_results;

        let mut reader = BitReader::with_bit_len(raw, u64::from(bit_count));
        // Scoped so the borrow of `self.cache` ends before the emit loop needs
        // `&mut self`. The decoded elements own their strings, so nothing
        // outlives the declaration.
        let decoded = {
            let declared = Self::declared_handle_names(self.cache, &self.current_group_path);
            decode_round_results(&mut reader, &declared)
        };
        let results = match decoded {
            Ok(results) => results,
            Err(err) => return self.record_blob_failure(&err),
        };

        for rr in &results {
            let index = rr.round_number;
            self.emit_struct_sub_field(
                |out| put(out, format_args!("RoundResults[{index}].RoundNumber")),
                Some(i64::from(rr.round_number)),
                None,
            );
            if let Some(ref team) = rr.winning_team {
                self.emit_struct_sub_field(
                    |out| put(out, format_args!("RoundResults[{index}].WinningTeam")),
                    None,
                    Some(team.clone()),
                );
            }
            // `as_str` lives on the enums in `vrf-decode`; these used to be two
            // fully-qualified matches here, a second copy of the variant list in
            // a crate that does not own the types.
            for (member, text) in [
                ("WinningTeamRole", rr.winning_team_role.map(|r| r.as_str())),
                ("RoundResult", rr.round_result.map(|o| o.as_str())),
            ] {
                if let Some(text) = text {
                    self.emit_struct_sub_field(
                        |out| put(out, format_args!("RoundResults[{index}].{member}")),
                        None,
                        Some(text.to_owned()),
                    );
                }
            }
        }

        !results.is_empty()
    }

    /// Decode TeamEconomy blob and emit sub-field rows.
    fn decode_team_economy_blob(&mut self, raw: &[u8], bit_count: u32) -> bool {
        use vrf_decode::structs::decode_team_economy;

        let mut reader = BitReader::with_bit_len(raw, u64::from(bit_count));
        let results = match decode_team_economy(&mut reader) {
            Ok(results) => results,
            Err(err) => return self.record_blob_failure(&err),
        };

        for te in &results {
            let index = te.index;
            self.emit_struct_sub_field(
                |out| put(out, format_args!("TeamEconomy[{index}].Index")),
                Some(i64::from(te.index)),
                None,
            );
            // Widened to i64 here rather than in the loop body: the members
            // are a mix of u32 and i32 and the array has to be one type.
            for (member, value) in [
                ("ReplicationId", te.replication_id.map(i64::from)),
                ("LoadoutValue", te.loadout_value.map(i64::from)),
                (
                    "AverageLoadoutValue",
                    te.average_loadout_value.map(i64::from),
                ),
            ] {
                if let Some(v) = value {
                    self.emit_struct_sub_field(
                        |out| put(out, format_args!("TeamEconomy[{index}].{member}")),
                        Some(v),
                        None,
                    );
                }
            }
        }

        !results.is_empty()
    }

    /// Decode RoundInfos blob and emit sub-field rows.
    fn decode_round_infos_blob(&mut self, raw: &[u8], bit_count: u32) -> bool {
        use vrf_decode::structs::decode_round_infos;

        let mut reader = BitReader::with_bit_len(raw, u64::from(bit_count));
        let decoded = {
            let declared = Self::declared_handle_names(self.cache, &self.current_group_path);
            decode_round_infos(&mut reader, &declared)
        };
        let results = match decoded {
            Ok(results) => results,
            Err(err) => return self.record_blob_failure(&err),
        };

        for ri in &results {
            let index = ri.index;
            for (member, value) in [
                ("RoundNumber", ri.round_number),
                ("StartOfRoundMoney", ri.start_of_round_money),
                ("StartOfRoundLoadoutValue", ri.start_of_round_loadout_value),
                ("EndOfRoundMoney", ri.end_of_round_money),
                ("EndOfRoundLoadoutValue", ri.end_of_round_loadout_value),
            ] {
                if let Some(v) = value {
                    self.emit_struct_sub_field(
                        |out| put(out, format_args!("RoundInfos[{index}].{member}")),
                        Some(i64::from(v)),
                        None,
                    );
                }
            }
        }

        !results.is_empty()
    }

    /// Emit a single sub-field row for a decoded struct blob element.
    ///
    /// The name is built by a closure straight into the interner's scratch
    /// buffer rather than passed as a `&str`, so a row costs no allocation for
    /// its name at all -- the callers used to build a prefix `String` and then
    /// a second `String` per member.
    ///
    /// Only the i64 and str columns are reachable: no struct-blob member
    /// decodes to a float or a bool today, and a parameter for a column no
    /// caller can fill reads as if the shape were open when it is not.
    fn emit_struct_sub_field(
        &mut self,
        name: impl FnOnce(&mut String),
        value_i64: Option<i64>,
        value_str: Option<String>,
    ) {
        let field_name = self.channel_state.names.intern_fmt(name);
        self.push_field(FieldValues {
            handle: 0,
            field_name: Some(field_name),
            bit_count: 0,
            raw_bits: None,
            value_i64,
            value_str,
            ..FieldValues::default()
        });
        self.stats.fields_emitted += 1;
    }
}

/// Decode one array leaf with a type the caller already resolved.
///
/// Split out so the overlay-driven path and the hardcoded-handle fallback share
/// one decode-and-widen, rather than each growing its own copy of the match on
/// `DecodedValue`.
fn decode_leaf_with(field_type: FieldType, raw: &[u8], bit_count: u32) -> DecodedColumns {
    use vrf_decode::{DecodedValue, decode_field};

    match decode_field(field_type, raw, bit_count) {
        Ok(DecodedValue::I64(v)) => (Some(v), None, None, None),
        Ok(DecodedValue::F64(v)) => (None, Some(v), None, None),
        Ok(DecodedValue::Bool(v)) => (None, None, Some(v), None),
        Ok(DecodedValue::Str(v)) => (None, None, None, Some(v)),
        Err(_) => (None, None, None, None),
    }
}

/// Fallback leaf typing for handles the overlay table cannot name.
///
/// The caller asks the table first, keyed on the name the replay declares for
/// the handle. This map only sees what that misses, so it is a floor rather
/// than the source of truth it used to be.
///
/// Returns (value_i64, value_f64, value_bool, value_str). All None if the
/// handle is not recognized or decoding fails.
///
/// Handle->type mapping derived from `CombatRoundReportsDecoder`:
/// - Int32 handles: 3, 5, 19, 21, 46, 81, 96
/// - Float handles: 18, 20, 47, 82
/// - Bool handles: 22, 25, 48, 49, 83, 84, 103
/// - EnumByte handles: 23, 45, 80
/// - ObjectNetGuid handles: 13, 24, 50, 85, 98
/// - FString handles: 11
/// - FName handles: 12
fn decode_array_leaf(handle: u32, raw: &[u8], bit_count: u32) -> DecodedColumns {
    let field_type = match handle {
        3 | 5 | 19 | 21 | 46 | 81 | 96 => FieldType::Int32,
        18 | 20 | 47 | 82 => FieldType::Float,
        22 | 25 | 48 | 49 | 83 | 84 | 103 => FieldType::Bool,
        23 | 45 | 80 => FieldType::EnumByte,
        13 | 24 | 50 | 85 | 98 => FieldType::ObjectNetGuid,
        11 => FieldType::FString,
        12 => FieldType::FName,
        _ => return (None, None, None, None),
    };

    decode_leaf_with(field_type, raw, bit_count)
}
