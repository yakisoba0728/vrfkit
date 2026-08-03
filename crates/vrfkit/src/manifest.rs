//! Hand-rolled JSON serialization for manifest.json.
//!
//! No external dependencies (serde_json, etc.) -- just plain formatting.
//! Generating the whole document as a String is fine: the export groups array
//! is a few hundred entries, and the one genuinely large member is
//! `game_specific_data`, which is copied through verbatim rather than parsed.
//!
//! # game_specific_data
//!
//! The header's game-specific data entries are themselves JSON documents (on
//! the reference replay entry 1 is a ~219 KB blob holding the match roster).
//! They are emitted as JSON *strings*, escaped like any other string. Parsing
//! and re-serialising a document this crate did not author would risk silently
//! altering it -- number formatting, key order, duplicate keys -- for no gain,
//! since a consumer recovers the object with a single nested parse.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use vrf_container::Preamble;
use vrf_net::stats::NetStats;
use vrf_schema::NetGuidCache;

use crate::error::CliError;

#[allow(clippy::too_many_arguments)]
pub fn write_manifest(
    path: &Path,
    source_file: &str,
    file_size: usize,
    preamble: &Preamble,
    cache: &NetGuidCache,
    stats: &NetStats,
    total_packets: u32,
    elapsed: Duration,
) -> Result<(), CliError> {
    let header = &preamble.header;
    let ver = &header.replay_version;
    let info = &preamble.info;

    // The game-specific data blob dominates the document size; reserve for it
    // up front rather than growing a quarter-megabyte String by doubling.
    let gsd_bytes: usize = header.game_specific_data.iter().map(String::len).sum();
    let mut out = String::with_capacity(64 * 1024 + gsd_bytes * 2);
    out.push_str("{\n");

    // Top-level metadata.
    //
    // The first five keys are read by tools/to_valplay_bundle.py; keep their
    // names and types stable.
    wkv(&mut out, "source_file", &json_str(source_file), 1);
    wkv(&mut out, "source_size_bytes", &file_size.to_string(), 1);
    wkv(&mut out, "replay_build", &json_str(&ver.branch), 1);
    wkv(
        &mut out,
        "replay_version",
        &json_str(&format!("{}.{}.{}", ver.major, ver.minor, ver.patch)),
        1,
    );
    wkv(
        &mut out,
        "replay_changelist",
        &ver.changelist.to_string(),
        1,
    );
    wkv(&mut out, "duration_ms", &info.length_in_ms.to_string(), 1);
    wkv(&mut out, "elapsed_ms", &elapsed.as_millis().to_string(), 1);

    // --- Replay info section ----------------------------------------------
    wkv(&mut out, "friendly_name", &json_str(&info.friendly_name), 1);
    wkv(&mut out, "is_live", json_bool(info.is_live), 1);
    wkv(&mut out, "compressed", json_bool(info.compressed), 1);
    wkv(&mut out, "encrypted", json_bool(info.encrypted), 1);
    // Unreal FDateTime ticks: 100-nanosecond intervals since 0001-01-01, NOT
    // the Windows FILETIME epoch of 1601-01-01. On the reference replay the
    // 1601 reading lands in the year 3626, so the epoch is not a guess.
    // Emitted raw: the wire records no timezone, so any derived calendar
    // string would have to assert a UTC-vs-local fact this file cannot know.
    wkv(&mut out, "timestamp_ticks", &info.timestamp.to_string(), 1);
    // Named for its source, not "network_version": this is the info section's
    // copy, which nothing validates and which does not hold the protocol
    // version 19 on real replays. The header's validated copy is a different
    // number; see `game_network_protocol_version` below.
    wkv(
        &mut out,
        "info_network_version",
        &info.network_version.to_string(),
        1,
    );

    // --- Replay header ----------------------------------------------------
    wkv(
        &mut out,
        "network_checksum",
        &header.network_checksum.to_string(),
        1,
    );
    wkv(
        &mut out,
        "game_network_protocol_version",
        &header.game_network_protocol_version.to_string(),
        1,
    );
    // Four u32 words in Unreal serialisation order, exactly as stored. Not
    // rendered as a canonical GUID string: that formatting imposes a byte
    // order this crate has no way to confirm.
    wkv(
        &mut out,
        "guid",
        &format!(
            "[{}, {}, {}, {}]",
            header.guid[0], header.guid[1], header.guid[2], header.guid[3]
        ),
        1,
    );
    wkv(&mut out, "ue4_version", &header.ue4_version.to_string(), 1);
    wkv(&mut out, "ue5_version", &header.ue5_version.to_string(), 1);
    wkv(
        &mut out,
        "package_version_license",
        &header.package_version_license.to_string(),
        1,
    );
    wkv(&mut out, "flags", &header.flags.to_string(), 1);
    wkv(&mut out, "platform", &json_str(&header.platform), 1);
    wkv(
        &mut out,
        "build_config",
        &header.build_config.to_string(),
        1,
    );
    wkv(
        &mut out,
        "build_target_type",
        &header.build_target_type.to_string(),
        1,
    );
    wkv(
        &mut out,
        "min_record_hz",
        &json_f32(header.min_record_hz),
        1,
    );
    wkv(
        &mut out,
        "max_record_hz",
        &json_f32(header.max_record_hz),
        1,
    );
    wkv(
        &mut out,
        "frame_limit_in_ms",
        &json_f32(header.frame_limit_in_ms),
        1,
    );
    wkv(
        &mut out,
        "checkpoint_limit_in_ms",
        &json_f32(header.checkpoint_limit_in_ms),
        1,
    );

    let levels: Vec<String> = header
        .level_names_and_times
        .iter()
        .map(|(name, time)| format!("{{ \"name\": {}, \"time_ms\": {time} }}", json_str(name)))
        .collect();
    wkv_array(&mut out, "level_names_and_times", &levels, 1);

    // Placed after the small scalars so the readable metadata precedes the
    // quarter-megabyte blob rather than following it.
    let gsd: Vec<String> = header
        .game_specific_data
        .iter()
        .map(|s| json_str(s))
        .collect();
    wkv_array(&mut out, "game_specific_data", &gsd, 1);

    // Stats
    out.push_str("  \"stats\": {\n");
    wkv(&mut out, "packet_count", &total_packets.to_string(), 2);
    wkv(&mut out, "bunch_count", &stats.bunches.to_string(), 2);
    wkv(
        &mut out,
        "malformed_packet_count",
        &stats.malformed_packets.to_string(),
        2,
    );
    wkv(
        &mut out,
        "partial_error_count",
        &stats.partial_errors.to_string(),
        2,
    );
    wkv(
        &mut out,
        "partial_fragments",
        &stats.partial_fragments.to_string(),
        2,
    );
    wkvl(
        &mut out,
        "partial_completed",
        &stats.partial_completed.to_string(),
        2,
    );
    out.push_str("  },\n");

    // Counts
    out.push_str("  \"counts\": {\n");
    wkv(
        &mut out,
        "content_blocks",
        &stats.content_blocks.to_string(),
        2,
    );
    wkv(
        &mut out,
        "rep_layout_blocks",
        &stats.rep_layout_blocks.to_string(),
        2,
    );
    wkv(
        &mut out,
        "class_net_cache_blocks",
        &stats.class_net_cache_blocks.to_string(),
        2,
    );
    wkv(
        &mut out,
        "deleted_blocks",
        &stats.deleted_blocks.to_string(),
        2,
    );
    wkv(&mut out, "fields", &stats.fields.to_string(), 2);
    wkv(&mut out, "rpcs", &stats.rpcs.to_string(), 2);
    wkv(&mut out, "actor_opens", &stats.actor_opens.to_string(), 2);
    wkv(&mut out, "actor_closes", &stats.actor_closes.to_string(), 2);
    wkv(
        &mut out,
        "exported_guids",
        &stats.exported_guids.to_string(),
        2,
    );
    wkv(&mut out, "skipped_bits", &stats.skipped_bits.to_string(), 2);
    wkvl(
        &mut out,
        "malformed_content_blocks",
        &stats.malformed_content_blocks.to_string(),
        2,
    );
    out.push_str("  },\n");

    // Export groups
    out.push_str("  \"net_field_export_groups\": [\n");
    let groups = cache.groups();
    for (gi, group) in groups.iter().enumerate() {
        out.push_str("    {\n");
        wkv(&mut out, "path", &json_str(&group.path), 3);
        wkv(
            &mut out,
            "path_name_index",
            &group.path_name_index.to_string(),
            3,
        );
        out.push_str("      \"fields\": [");
        let populated: Vec<_> = group.populated_fields().collect();
        if populated.is_empty() {
            out.push(']');
        } else {
            out.push('\n');
            for (fi, field) in populated.iter().enumerate() {
                out.push_str("        { ");
                out.push_str(&format!(
                    "\"handle\": {}, \"name\": {}, \"compatible_checksum\": {}",
                    field.handle,
                    json_str(&field.name),
                    field.compatible_checksum
                ));
                out.push_str(" }");
                if fi + 1 < populated.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("      ]");
        }
        out.push('\n');
        out.push_str("    }");
        if gi + 1 < groups.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n");
    out.push_str("}\n");

    let mut file = fs::File::create(path)?;
    file.write_all(out.as_bytes())?;
    Ok(())
}

/// Push `indent` levels of two-space indentation.
fn push_indent(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push_str("  ");
    }
}

/// Write key-value pair with trailing comma.
fn wkv(out: &mut String, key: &str, value: &str, indent: usize) {
    push_indent(out, indent);
    out.push_str(&format!("\"{key}\": {value},\n"));
}

/// Write key-value pair WITHOUT trailing comma (last in object).
fn wkvl(out: &mut String, key: &str, value: &str, indent: usize) {
    push_indent(out, indent);
    out.push_str(&format!("\"{key}\": {value}\n"));
}

/// Write an array whose elements are already rendered JSON values, one per
/// line, with a trailing comma after the closing bracket.
///
/// Like [`wkv`] and unlike [`wkvl`], this always emits the trailing comma, so
/// the array must not be the last member of its object. There is deliberately
/// no comma-less variant: add one before moving an array to the end of an
/// object, rather than dropping the comma by hand.
fn wkv_array(out: &mut String, key: &str, values: &[String], indent: usize) {
    push_indent(out, indent);
    out.push_str(&format!("\"{key}\": ["));
    if values.is_empty() {
        out.push_str("],\n");
        return;
    }
    out.push('\n');
    for (i, value) in values.iter().enumerate() {
        push_indent(out, indent + 1);
        out.push_str(value);
        if i + 1 < values.len() {
            out.push(',');
        }
        out.push('\n');
    }
    push_indent(out, indent);
    out.push_str("],\n");
}

/// JSON literal for a boolean.
fn json_bool(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

/// JSON number for an `f32`, or `null` when the value is not finite.
///
/// This is the one place where "emit what the wire says" collides with "the
/// output must be valid JSON": Rust renders NaN as `NaN` and the infinities as
/// `inf`/`-inf`, none of which JSON admits. The recording-rate fields are
/// floats read straight off the wire, so a corrupt or unusual header could
/// carry any of the three. `null` says "the wire held a value JSON cannot
/// represent" instead of producing a file no parser will accept.
fn json_f32(value: f32) -> String {
    if value.is_finite() {
        format!("{value}")
    } else {
        "null".to_string()
    }
}

/// JSON-escape a string value.
///
/// RFC 8259 requires escaping exactly `"`, `\`, and U+0000..U+001F; every
/// other code point may appear literally in a UTF-8 document, so the remaining
/// characters are passed through and the manifest is written as UTF-8.
///
/// That is sufficient even for the game-specific data blob, which arrives as
/// UTF-16 on the wire: `BitReader::read_fstring` decodes it with the strict
/// `String::from_utf16`, so an unpaired surrogate is a parse error rather than
/// something that reaches this function. Every `&str` here is therefore
/// well-formed Unicode, and no code point above U+001F needs special handling.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < '\x20' => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_str_escapes_only_what_rfc_8259_requires() {
        assert_eq!(json_str("plain"), "\"plain\"");
        assert_eq!(json_str("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_str("a\\b"), "\"a\\\\b\"");
        assert_eq!(json_str("a\nb\rc\td"), "\"a\\nb\\rc\\td\"");
        // Control characters without a short form take the \u escape.
        assert_eq!(json_str("\u{0}\u{1}\u{1f}"), "\"\\u0000\\u0001\\u001f\"");
    }

    #[test]
    fn json_str_passes_non_ascii_through_verbatim() {
        // Source stays ASCII (check_ascii.py) while the runtime values are not.
        // DEL and the C1 range sit above U+001F, so JSON admits them literally;
        // escaping them would be wrong, not merely redundant.
        for s in [
            "\u{7f}",           // DEL
            "\u{80}\u{9f}",     // C1 controls
            "\u{e9}\u{440}",    // Latin-1 supplement, Cyrillic
            "\u{d55c}\u{ad6d}", // Hangul, as a Valorant player name may carry
            "\u{1f600}",        // astral plane (surrogate pair on the wire)
            "\u{2028}\u{2029}", // line/paragraph separator: legal JSON
            "\u{feff}",         // BOM as an interior character
        ] {
            assert_eq!(json_str(s), format!("\"{s}\""), "mangled {s:?}");
        }
    }

    #[test]
    fn json_f32_replaces_non_finite_with_null() {
        assert_eq!(json_f32(30.0), "30");
        assert_eq!(json_f32(-0.5), "-0.5");
        assert_eq!(json_f32(f32::NAN), "null");
        assert_eq!(json_f32(f32::INFINITY), "null");
        assert_eq!(json_f32(f32::NEG_INFINITY), "null");
    }

    #[test]
    fn wkv_array_renders_empty_and_populated_forms() {
        let mut out = String::new();
        wkv_array(&mut out, "empty", &[], 1);
        assert_eq!(out, "  \"empty\": [],\n");

        let mut out = String::new();
        let values = vec!["1".to_string(), "2".to_string()];
        wkv_array(&mut out, "pair", &values, 1);
        assert_eq!(out, "  \"pair\": [\n    1,\n    2\n  ],\n");
    }
}
