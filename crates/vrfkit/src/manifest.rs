//! Hand-rolled JSON serialization for manifest.json.
//!
//! No external dependencies (serde_json, etc.) -- just plain formatting.
//! The manifest is small (~500 fields in the export groups array), so generating
//! it as a String is fine.

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
    let ver = &preamble.header.replay_version;
    let info = &preamble.info;

    let mut out = String::with_capacity(64 * 1024);
    out.push_str("{\n");

    // Top-level metadata
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

/// Write key-value pair with trailing comma.
fn wkv(out: &mut String, key: &str, value: &str, indent: usize) {
    for _ in 0..indent {
        out.push_str("  ");
    }
    out.push_str(&format!("\"{key}\": {value},\n"));
}

/// Write key-value pair WITHOUT trailing comma (last in object).
fn wkvl(out: &mut String, key: &str, value: &str, indent: usize) {
    for _ in 0..indent {
        out.push_str("  ");
    }
    out.push_str(&format!("\"{key}\": {value}\n"));
}

/// JSON-escape a string value.
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
