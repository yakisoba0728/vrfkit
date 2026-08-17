//! The ClassNetCache RPC payload walker.
//!
//! A ClassNetCache block carries function calls, not properties. Each call's
//! payload is a sub-archive following the RepLayout `FunctionParameters`
//! grammar, and walking it turns one opaque blob into one row per named
//! parameter -- 559,346 of the reference replay's 1,246,812 field rows.

use std::sync::Arc;

use smallvec::{SmallVec, smallvec};
use vrf_bitio::BitReader;
use vrf_decode::{
    ArrayFieldSchema, DecodeErrorKind, EffectArrayKind, EffectBlobError, FieldType,
    LIFE_CHANGE_BY_SECTION_SCHEMA, LIFE_CHANGE_DAMAGE_SCHEMA, LIFE_CHANGE_SECTION_SCHEMA,
    apply_overlay_with_checksum, decode_effect_blob_json, decode_struct_array, group_hash_state,
};
use vrf_schema::{FxHashMap, NetGuidCache};

use super::intern::put;
use super::{ExportSink, FieldValues, TABLE};

/// Memo for [`ExportSink::find_rpc_param_group_path`].
///
/// That lookup is a pure function of (content-block group path, function name,
/// the set of declared group paths). The third input is what
/// `NetGuidCache::schema_generation` tracks, so stamping the memo with it and
/// clearing on a change makes the memo exactly equivalent to recomputing.
///
/// Why it matters: the fallback branch scans every declared group (475 on
/// 02d4d478) with `ends_with`, once per RPC, and a replay has 342,735 RPCs.
/// The distinct (group path, function name) pairs number in the hundreds.
///
/// Two levels rather than a tuple key so that a hit costs no allocation: the
/// outer level is keyed by the same interned `Arc<str>` the sink already holds,
/// so even a miss allocates nothing for the key.
#[derive(Debug, Clone, Default)]
pub(super) struct RpcParamGroupMemo {
    generation: u64,
    by_group: FxHashMap<Arc<str>, FxHashMap<String, Option<Arc<str>>>>,
}

impl ExportSink<'_> {
    /// Try to parse an RPC payload as a RepLayout field stream using the
    /// RPC parameter group from the schema.
    ///
    /// # Wire format (confirmed against C# `ParseClassNetCachePayload`)
    ///
    /// The RPC payload is a sub-archive whose contents follow RepLayout
    /// `FunctionParameters` grammar:
    /// ```text
    ///   propertyChecksum : 1 bit (ignored)
    ///   loop:
    ///     encodedHandle  : IntPacked
    ///     [if 0 -> break]
    ///     handle = encodedHandle - 1
    ///     payloadBits    : IntPacked
    ///     fieldPayload   : sub-reader of payloadBits bits
    /// ```
    /// Additionally, if exactly 1 bit remains after reading all fields, it is
    /// a trailing alignment bit that must be consumed (C# grammar check:
    /// `FunctionParameters && BitsRemaining == 1 -> SkipBits(1)`).
    ///
    /// # Group path resolution
    ///
    /// RPC parameter groups are registered with paths like
    /// `/Script/ShooterGame.ShooterCharacter:MulticastNotifyKilledEnemy`.
    /// The content block's CNC group might be an agent-specific path like
    /// `Wushu_PC_C_ClassNetCache`. We cannot simply strip the suffix and
    /// append `:FunctionName` because the parent class differs.
    ///
    /// Strategy: look up the group by function name as a unique leaf suffix
    /// (the part after `:`). Most RPC parameter groups have a unique function
    /// name across all 84 groups. For the rare case where the name is not
    /// unique, we fall back to emitting unnamed handle-indexed rows.
    ///
    /// Returns `true` if parameters were emitted (even if just handle-indexed),
    /// `false` if the payload could not be walked at all (caller should emit
    /// raw_bits row).
    ///
    /// A walk that starts but breaks on a malformed read (truncated handle,
    /// truncated payload length, or a length exceeding the remaining bits) is
    /// neither a clean parse nor an unparseable blob: whatever rows were already
    /// emitted stay, and `self.stats.truncated_rpcs` is bumped so the summary
    /// can distinguish "completed" from "ran out of bits".
    pub(super) fn try_parse_rpc_params(
        &mut self,
        rpc_handle: u32,
        reader: BitReader<'_>,
        function_name: Option<&str>,
    ) -> bool {
        let Some(func_name) = function_name else {
            return false;
        };
        let whole_reader = reader.clone();
        let Ok(whole_bit_count) = u32::try_from(reader.bits_remaining()) else {
            return false;
        };

        // Find the RPC parameter group. Try direct path construction first,
        // then fall back to function-name leaf match.
        let param_group_path = self.find_rpc_param_group_path(func_name);

        // Parse the RepLayout stream inside the RPC payload.
        let mut rpc_reader = reader;

        // Property checksum bit (1 bit) -- always present for FunctionParameters.
        if rpc_reader.read_bit().is_err() {
            return false;
        }

        let mut emitted_any = false;
        // Set on the four malformed-read break paths below, so the caller can
        // tell a completed walk from one that ran out of bits. The normal exits
        // (clean end-of-stream, the trailing alignment bit, the zero-handle
        // terminator) leave it `false`.
        let mut truncated = false;
        let mut unexplained_suffix = false;
        let param_group_path_ref = param_group_path.as_deref();

        loop {
            if rpc_reader.at_end() {
                break;
            }

            // FunctionParameters grammar: if exactly 1 bit remains, skip it.
            if rpc_reader.bits_remaining() == 1 {
                let _ = rpc_reader.read_bit();
                break;
            }

            let Ok(encoded_handle) = rpc_reader.read_int_packed() else {
                truncated = true;
                break;
            };
            if encoded_handle == 0 {
                // The wire said "no more parameters". Ask what is still in the
                // payload before believing it: one trailing alignment bit is
                // grammar, anything more is a suffix this walk cannot explain.
                // See `ExportStats::rpc_suffix_bits_dropped` for why it is
                // counted rather than rejected.
                let leftover = rpc_reader.bits_remaining();
                if leftover > 1 {
                    self.stats.rpc_suffix_bits_dropped =
                        self.stats.rpc_suffix_bits_dropped.saturating_add(leftover);
                    unexplained_suffix = true;
                }
                break;
            }

            let param_handle = encoded_handle - 1;
            let Ok(payload_bits) = rpc_reader.read_int_packed() else {
                truncated = true;
                break;
            };

            if u64::from(payload_bits) > rpc_reader.bits_remaining() {
                // Malformed: more bits declared than available. Stop parsing
                // but keep what we have (emitted_any may be true).
                truncated = true;
                break;
            }

            let Ok(sub) = rpc_reader.sub_reader(u64::from(payload_bits)) else {
                truncated = true;
                break;
            };

            // Resolve parameter field name from the group. Borrowed, not
            // cloned: it is only needed to build `full_field_name` and to key
            // the overlay, both of which end before the next mutation of self.
            //
            // The same `NetFieldExport` carries `compatible_checksum`, so the
            // overlay's last-resort lookup costs nothing extra here -- one
            // schema walk yields both.
            let param_export = param_group_path_ref.and_then(|gp| {
                self.cache
                    .get_group_by_path(gp)
                    .and_then(|g| g.get_field(param_handle))
            });
            let param_name: Option<&str> = param_export.map(|f| f.name.as_str());
            let param_checksum: Option<u32> = param_export.map(|f| f.compatible_checksum);

            // Build field_name: "FunctionName.ParamName" or "FunctionName._h{N}".
            //
            // Interned rather than `format!`-ed per row: the distinct set is
            // bounded by the schema (a few hundred), while this loop body runs
            // 559,346 times on the reference replay.
            let full_field_name = match param_name {
                Some(pn) => self.channel_state.names.intern_join(func_name, '.', pn),
                None => self.channel_state.names.intern_fmt(|out| {
                    put(out, format_args!("{func_name}._h{param_handle}"));
                }),
            };

            // Extract raw bits for this parameter field.
            let raw_bits = copy_raw_bits(sub, payload_bits);

            // Apply type overlay using the parameter group path as group_path.
            // The group hash is cached for the current_group_path fallback; a
            // resolved parameter group is a different path, so its hash is
            // computed fresh.
            let overlay_group = param_group_path_ref.unwrap_or(&self.current_group_path);
            let group_state = match param_group_path_ref {
                Some(gp) => group_hash_state(gp),
                None => self.current_group_hash,
            };
            let overlay_field = param_name.unwrap_or(&full_field_name);
            let (value_i64, value_f64, value_bool, mut value_str) =
                match apply_overlay_with_checksum(
                    &TABLE,
                    overlay_group,
                    group_state,
                    Some(overlay_field),
                    param_handle,
                    param_checksum,
                    raw_bits.as_deref(),
                    payload_bits,
                    &mut self.stats.overlay,
                ) {
                    Some(result) => (
                        result.value_i64,
                        result.value_f64,
                        result.value_bool,
                        result.value_str,
                    ),
                    None => (None, None, None, None),
                };

            // Second, additive pass: the EffectContainer arrays.
            //
            // The static overlay types these as `Raw` (or does not know them at
            // all), so they reach here with every `value_*` null and only the
            // bits to show for themselves -- 45.8% of everything still untyped
            // in `02d4d478`. `decode_effect_blob_json` turns one into a JSON
            // array; `raw_bits` is left in place, so this is an overlay, not a
            // replacement, and a consumer that wants to reinterpret the bits
            // still can.
            //
            // Only attempted when the overlay produced nothing: a declared type
            // that decoded is the more specific answer and outranks a decode
            // driven by the parameter's name.
            if value_i64.is_none()
                && value_f64.is_none()
                && value_bool.is_none()
                && value_str.is_none()
            {
                if let (Some(kind), Some(raw)) = (
                    effect_array_kind_for_param(func_name, param_name),
                    raw_bits.as_deref(),
                ) {
                    // `payload_bits`, not `raw.len() * 8`: the last byte is
                    // padded, and handing the padding to the decoder as data is
                    // the latent bug docs/archive/PROJECT_STATUS.md 12-D pins
                    // on the Python side of this same format.
                    match decode_effect_blob_json(kind, raw, payload_bits) {
                        Ok(json) => {
                            value_str = Some(json);
                            self.stats.effect_blobs_decoded += 1;
                        }
                        Err(err) => {
                            // Loud, not silent: `value_str` stays null, the bits
                            // stay, and the row lands in the export summary's
                            // "Decode errors" line and its per-field breakdown.
                            //
                            // `stats.overlay` is the only channel that survives
                            // here -- the sink is rebuilt per packet and the
                            // driver aggregates nothing else. The cost is that a
                            // failing row is counted in two buckets, so "Rows
                            // offered" over-reports by the failure count. That
                            // denominator is a diagnostic, not an invariant, and
                            // the count was 0 across all 61,617 blobs measured.
                            self.stats.overlay.decoded_err += 1;
                            self.stats.overlay.error_report.record(
                                overlay_group,
                                &full_field_name,
                                FieldType::Raw,
                                payload_bits,
                                effect_error_kind(&err),
                            );
                        }
                    }
                }
            }

            // Third, additive pass: the life-change arrays.
            //
            // Outside the `value_*.is_none()` gate above, not inside it. That
            // gate exists so a declared type outranks a name-driven decode of
            // the same bits; this pass emits *additional rows* and takes the
            // parent's bits away from nobody. Nesting it would mean that giving
            // the parent a type one day silently stops the children being
            // emitted -- the shape of failure this file has been chasing all
            // week.
            if let (Some(schema), Some(raw)) = (
                life_change_schema_for_param(func_name, param_name),
                raw_bits.as_deref(),
            ) {
                self.emit_life_change_array(
                    schema,
                    &full_field_name,
                    rpc_handle,
                    raw,
                    payload_bits,
                );
            }

            self.push_field(FieldValues {
                handle: rpc_handle,
                field_name: Some(full_field_name),
                compatible_checksum: param_checksum,
                bit_count: payload_bits,
                raw_bits,
                value_i64,
                value_f64,
                value_bool,
                value_str,
            });
            self.stats.fields_emitted += 1;

            emitted_any = true;
        }

        if truncated {
            self.stats.truncated_rpcs = self.stats.truncated_rpcs.saturating_add(1);
        }

        // Parameter rows already emitted remain useful, but they cannot stand
        // in for bytes the walk did not explain. Add the original payload as a
        // fallback row whenever a partial walk would otherwise suppress the
        // caller's ordinary whole-payload fallback. This is additive: typed
        // parameter rows stay, and the exact raw input stays available for a
        // future decoder.
        if emitted_any && (truncated || unexplained_suffix) {
            let field_name = self.channel_state.names.intern(func_name);
            self.push_field(FieldValues {
                handle: rpc_handle,
                field_name: Some(field_name),
                bit_count: whole_bit_count,
                raw_bits: copy_raw_bits(whole_reader, whole_bit_count),
                ..FieldValues::default()
            });
            self.stats.fields_emitted += 1;
        }

        emitted_any
    }

    /// Emit one row per member of a life-change array element.
    ///
    /// Purely additive: the parent row keeps its `raw_bits` and is pushed by
    /// the caller either way, so a consumer that was reading the blob still
    /// can.
    ///
    /// The rows carry `rpc_handle`, not the member's handle inside the struct.
    /// That is not cosmetic -- `tools/to_valplay_bundle.py` groups a call's
    /// parameters by `(packet, actor, group, handle)`, so member handles would
    /// split one RPC into as many fake calls as it has members. Measured, not
    /// reasoned about: injecting two child rows under their struct handles
    /// produced two bundle events where the RPC handle produced one.
    fn emit_life_change_array(
        &mut self,
        schema: &'static ArrayFieldSchema,
        prefix: &str,
        rpc_handle: u32,
        raw: &[u8],
        bit_count: u32,
    ) {
        let flattened =
            decode_struct_array(raw, bit_count, Some(schema), &[], &mut self.stats.array);
        for field in &flattened {
            let (value_i64, value_f64, value_bool, value_str) =
                match life_change_member_type(&field.path) {
                    Some(ft) => super::blobs::decode_leaf_with_stats(
                        ft,
                        &field.raw_bits,
                        field.bit_count,
                        &mut self.stats.array_leaf_decode_errors,
                    ),
                    None => (None, None, None, None),
                };
            let full_name = self.channel_state.names.intern_fmt(|out| {
                out.push_str(prefix);
                out.push_str(&field.path);
            });
            self.push_field(FieldValues {
                handle: rpc_handle,
                field_name: Some(full_name),
                // A member is addressed inside the array payload, so the
                // replay declares no checksum for it. See `FieldRecord`.
                compatible_checksum: None,
                bit_count: field.bit_count,
                raw_bits: Some(SmallVec::from_slice(&field.raw_bits)),
                value_i64,
                value_f64,
                value_bool,
                value_str,
            });
            self.stats.fields_emitted += 1;
        }
    }

    /// Find the RPC parameter group path for a given function name.
    ///
    /// Memoised wrapper around [`Self::compute_rpc_param_group_path`].
    ///
    /// The computation is a pure function of the current group path, the
    /// function name, and the set of declared group paths; only the third can
    /// change while a replay is being read, and `schema_generation` tracks
    /// exactly that. A generation change discards the whole memo, so a hit is
    /// indistinguishable from a recomputation.
    ///
    /// Strategy 2 in the computation is O(groups) per call. Without this memo a
    /// replay pays 342,735 x 475 `ends_with` probes.
    fn find_rpc_param_group_path(&mut self, function_name: &str) -> Option<Arc<str>> {
        let Self {
            cache,
            channel_state,
            current_group_path,
            ..
        } = self;
        let memo = &mut channel_state.rpc_param_groups;
        let generation = cache.schema_generation();
        if memo.generation != generation {
            memo.by_group.clear();
            memo.generation = generation;
        }

        if let Some(by_function) = memo.by_group.get(&**current_group_path) {
            if let Some(hit) = by_function.get(function_name) {
                return hit.clone();
            }
        }

        let resolved = Self::compute_rpc_param_group_path(cache, current_group_path, function_name);
        memo.by_group
            // Cloning the interned handle, not the string.
            .entry(Arc::clone(current_group_path))
            .or_default()
            .insert(function_name.to_owned(), resolved.clone());
        resolved
    }

    /// Strategy:
    /// 1. Try stripping `_ClassNetCache` from current_group_path and appending
    ///    `:<function_name>` -- this works when the CNC group matches the
    ///    parameter group's class (e.g. DamageableComponent).
    /// 2. Search all registered groups for one whose path ends with
    ///    `:<function_name>`. If exactly one matches, use it (unique leaf match).
    ///    If multiple match, return None (ambiguous).
    ///
    /// The second strategy handles inheritance: `Wushu_PC_C_ClassNetCache` has
    /// `MulticastNotifyKilledEnemy` but the parameter group is under
    /// `ShooterCharacter:MulticastNotifyKilledEnemy`.
    fn compute_rpc_param_group_path(
        cache: &NetGuidCache,
        current_group_path: &str,
        function_name: &str,
    ) -> Option<Arc<str>> {
        // Strategy 1: direct path construction from CNC group.
        if let Some(base) = current_group_path.strip_suffix(vrf_schema::CLASS_NET_CACHE_SUFFIX) {
            let candidate = format!("{base}:{function_name}");
            if cache.get_group_by_path(&candidate).is_some() {
                return Some(Arc::from(candidate));
            }
        }

        // Strategy 2: search for unique group with `:<function_name>` suffix.
        let suffix = format!(":{function_name}");
        let mut found: Option<&str> = None;
        for group in cache.groups() {
            if group.path.ends_with(&suffix) && group.path.contains(':') {
                if found.is_some() {
                    // Ambiguous: multiple groups match this function name.
                    return None;
                }
                found = Some(&group.path);
            }
        }
        found.map(Arc::from)
    }
}

/// Copy `bit_count` bits out of `reader` into a fresh byte buffer.
///
/// `None` for a zero-bit field: an empty blob and "no payload" are different
/// things downstream, and `raw_bits` is the column the valplay adapter's
/// capture predicate keys on.
pub(super) fn copy_raw_bits(reader: BitReader<'_>, bit_count: u32) -> Option<SmallVec<[u8; 16]>> {
    if bit_count == 0 {
        return None;
    }
    let mut buf = smallvec![0u8; (bit_count as usize).div_ceil(8)];
    let mut reader = reader;
    let _ = reader.copy_bits_to(&mut buf, u64::from(bit_count));
    Some(buf)
}

/// The one RPC whose effect blobs must keep reaching the downstream adapter as
/// raw bits.
///
/// `tools/to_valplay_bundle.py` builds `valorant_shot_received` -- and with it
/// the `weapons`, `shot_rays`, `spray_control` and `posture` metric sections --
/// from this RPC's blobs, which it captures at line 1744 under a predicate it
/// calls `is_raw`. That predicate is `_get_value` at line 1096, and it returns
/// `is_raw = False` the moment `value_str` is non-null: the `row_str` test at
/// lines 1104-1105 runs *before* the `row_raw` test at line 1110. So filling
/// `value_str` on these rows would not merely change their shape, it would
/// stop the adapter capturing them at all, silently, and the shot sections
/// would go with them.
///
/// The exclusion is therefore a property of the consumer, not of the wire
/// format -- which is why it lives here and not in `vrf_decode::effect`. It can
/// go away once the adapter reads the decoded JSON instead of the bits.
const EFFECT_BLOB_RPC_LEFT_RAW_FOR_ADAPTER: &str = "ReplayPlayContinuousEffectAtLocation";

/// Decide whether an RPC parameter should be decoded as an effect-array blob.
///
/// Eleven functions on `02d4d478` declare a parameter named `FloatValues`,
/// `ObjectValues` or `VectorValues`, and all 61,617 of those payloads decode
/// as this format and consume their window exactly. No other parameter name
/// does, which is why the match is on the name and not on the function.
/// Which life-change schema an RPC parameter takes, if any.
///
/// Keyed on the function as well as the parameter, because the local handles
/// differ per function -- each struct array gets its own handle space, so the
/// same four members sit at 10-13, 1-4 or 2-5 depending on where they arrive.
///
/// `MulticastNotifyHeal` and `MulticastNotifyOverhealDecay` name their
/// parameter `LifeChangeBySection`, not `LifeChangeEvents`. Dispatching on the
/// array's name alone would silently miss both, which between them are more
/// than half the calls.
///
/// `MulticastReceivePlayerTemporaryDeathEvent_Point` and
/// `MulticastReceivePlayerDownedEvent_Point` send the same array at handles
/// 12-15 and are deliberately absent: 9 and 2 calls across twenty replays is
/// not enough to check a schema against anything.
fn life_change_schema_for_param(
    function_name: &str,
    param_name: Option<&str>,
) -> Option<&'static ArrayFieldSchema> {
    match (function_name, param_name?) {
        ("MulticastNotifyDamage_Point" | "MulticastNotifyDamage_Base", "LifeChangeEvents") => {
            Some(&LIFE_CHANGE_DAMAGE_SCHEMA)
        }
        ("MulticastSectionLifeChange", "LifeChangeEvents") => Some(&LIFE_CHANGE_SECTION_SCHEMA),
        ("MulticastNotifyHeal" | "MulticastNotifyOverhealDecay", "LifeChangeBySection") => {
            Some(&LIFE_CHANGE_BY_SECTION_SCHEMA)
        }
        _ => None,
    }
}

/// The member type for a leaf of a life-change element.
///
/// Matched on the name the schema gave it rather than on its handle, because
/// the handles move between functions and the names do not. Evidence for each:
/// `ChangedComponent` resolves through `net_guids` to a `*DamageSection` actor
/// on better than 99.98% of rows, `sum(DeltaLife)` matches the RPC's own
/// scalar total on 69,818 of 69,818 calls, and `bAliveAfterChange` is one bit
/// on every row and agrees with the sibling `bAliveAfterDamage` 17,550/17,550.
fn life_change_member_type(path: &str) -> Option<FieldType> {
    if path.ends_with("ChangedComponent") {
        Some(FieldType::ObjectNetGuid)
    } else if path.ends_with("LifeResult") || path.ends_with("DeltaLife") {
        Some(FieldType::Float)
    } else if path.ends_with("bAliveAfterChange") {
        Some(FieldType::Bool)
    } else {
        None
    }
}

fn effect_array_kind_for_param(
    function_name: &str,
    param_name: Option<&str>,
) -> Option<EffectArrayKind> {
    if function_name == EFFECT_BLOB_RPC_LEFT_RAW_FOR_ADAPTER {
        return None;
    }
    // A parameter whose name the group did not resolve is emitted as `_h{N}`,
    // and a handle does not identify the element type across functions.
    EffectArrayKind::from_param_name(param_name?)
}

/// Map an effect-blob failure onto the overlay report's error kinds.
///
/// `DecodeErrorKind` has three variants and this decoder has seven failures,
/// so the mapping is lossy by construction; the report's `field_name` column
/// carries the identification. `Residual` is the bucket for "the payload was
/// not consumable as this format", which is what every structural failure
/// means here.
fn effect_error_kind(err: &EffectBlobError) -> DecodeErrorKind {
    match err {
        EffectBlobError::BitIo(_) => DecodeErrorKind::Eof,
        _ => DecodeErrorKind::Residual,
    }
}
