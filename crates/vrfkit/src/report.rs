//! Counter text printed on more than one line: `validate` and the export
//! summary, or its main and checkpoint blocks. Each call site keeps its own
//! label and padding, which the corpus tools anchor on; only the text after
//! the label lives here, so the reports cannot drift.

use vrf_frame::FrameSkips;
use vrf_net::stats::NetStats;

/// `N external blobs / N external bytes / N game-specific bytes`.
pub fn frame_skips(skips: &FrameSkips) -> String {
    format!(
        "{} external blobs / {} external bytes / {} game-specific bytes",
        skips.external_data_blobs, skips.external_data_bytes, skips.game_specific_bytes
    )
}

/// Every measured array route's child rows, zeros included.
#[cfg(feature = "export")]
pub fn route_children(sink: &crate::sink::ExportStats) -> String {
    format!(
        "{} player info / {} rewards / {} selected / {} kills / {} active effects / {} ignore actors / {} blinds / {} projectile path / {} focus projectiles",
        sink.route_children_player_information,
        sink.route_children_tracked_rewards,
        sink.route_children_selected_v2,
        sink.route_children_kill_data,
        sink.route_children_server_active_effects,
        sink.route_children_requested_ignore_actors,
        sink.route_children_active_blinds,
        sink.route_children_projectile_path,
        sink.route_children_focus_projectiles
    )
}

/// `N attempted / N errors / N accepted fragments / N completed`.
pub fn partial_bunches(stats: &NetStats) -> String {
    format!(
        "{} attempted / {} errors / {} accepted fragments / {} completed",
        stats.partial_bunches,
        stats.partial_errors,
        stats.partial_fragments,
        stats.partial_completed
    )
}

/// The partial-reassembly error causes, then what they leave of
/// `partial_errors` unexplained and what they count beyond it.
pub fn partial_causes(stats: &NetStats) -> String {
    format!(
        "{} missing initial / {} overlapping initial / {} mismatched continuation / {} unaligned / {} channel close / {} resource limit / {} unclassified / {} overclassified",
        stats.partial_missing_initial,
        stats.partial_overlapping_initial,
        stats.partial_mismatched_continuation,
        stats.partial_non_byte_aligned,
        stats.partial_channel_close,
        stats.partial_resource_limit_failures,
        stats.partial_unclassified_errors(),
        stats.partial_overclassified_errors()
    )
}
