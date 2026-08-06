//! Channel lifecycle: open, close, and the two bunch-level GUID preambles.
//!
//! A bunch that opens a channel carries the actor GUID (and, for dynamic
//! actors, the spawn block in [`super::spawn`]) ahead of its content blocks. A
//! bunch that closes one carries nothing but the header flag. Everything here
//! runs at most a few thousand times per replay; the per-bunch hot path is in
//! [`super::framing`].

use vrf_bitio::BitReader;

use crate::bunch::RawBunchHeader;
use crate::error::{NetError, Result};
use crate::net_guid;
use crate::stats::NetStats;
use crate::types::{MAX_GUID_COUNT, NetworkGuid};

use super::spawn;
use super::{ActorChannelState, ChannelTable, PLAYER_CONTROLLER_LEAF, ReplicationSink};

/// Whether `path` names the replay controller, in any of the spellings Unreal
/// uses for the same asset.
///
/// The same class arrives under at least four different strings depending on
/// where in the stream it was written:
///
/// | source | string |
/// |---|---|
/// | net field export group path | `/Game/Characters/_Core/BaseReplayController.BaseReplayController_C` |
/// | NetGUID path (package-map export) | `/Game/Characters/_Core/BaseReplayController` |
/// | archetype GUID path (class default object) | `Default__BaseReplayController_C` |
/// | `/_Core/` elided alias | `/Game/Characters/BaseReplayController` |
///
/// So this normalises instead of comparing: take the last `/`-separated
/// segment, drop anything before a `.` (the `Asset.Class_C` form), strip a
/// `Default__` prefix and a `_C` suffix, then compare the bare name.
///
/// Getting this wrong is silent. The index byte below is simply not consumed,
/// and every content block after it in that bunch is shifted by 8 bits -- which
/// surfaces only as one malformed block and a few hundred skipped bits.
pub(super) fn is_player_controller_path(path: &str) -> bool {
    let segment = path.rsplit('/').next().unwrap_or(path);
    // `Asset.Class_C` -> `Class_C`; a bare segment is unchanged.
    let class = segment.rsplit('.').next().unwrap_or(segment);
    let class = class.strip_prefix("Default__").unwrap_or(class);
    let class = class.strip_suffix("_C").unwrap_or(class);
    class == PLAYER_CONTROLLER_LEAF
}

/// Whether this channel's actor or archetype resolves to the replay
/// controller, which is what decides the net-player-index byte.
///
/// Unreal writes a 1-byte "player index" between the actor-open spawn data and
/// the first content block when the newly opened actor is a dynamic
/// PlayerController. Without consuming that byte, all subsequent content blocks
/// in the bunch are shifted by 8 bits.
///
/// C# reference: `ReadNetPlayerIndexStage.cs` -- checks `OpenedDynamicActor &&
/// IsPlayerController(channel archetype/class/actor path)`.
///
/// The path comes from the sink's cache. An earlier version of this crate also
/// maintained its own `HashSet` of controller GUIDs, filled by intercepting
/// `register_path`; it was never consulted, because `vrf-schema`'s reader
/// writes chunk-level GUID exports straight into the cache and so the cache
/// knows the archetype path when the set does not. An instrumented run
/// confirmed the set stayed empty for the whole reference replay while this
/// lookup answered true 2 028 times. The set is gone; the cache lookup that
/// always decided this is what remains.
///
/// Missing the byte does not desync visibly. Combined with the spawn velocity
/// bit in [`super::spawn`] it started the controller's opening bunch nine bits
/// early, and the misframed header happened to re-synchronise: it consumed 11
/// bits where the real header consumes 2, so the content-bit count was read at
/// the same offset and every later block framed identically. The controller's
/// own property block was simply routed to the ClassNetCache path and never
/// walked. See docs/archive/PROJECT_STATUS.md 17-A.
pub(super) fn is_player_controller_channel(
    actor_net_guid: NetworkGuid,
    archetype_net_guid: NetworkGuid,
    sink: &dyn ReplicationSink,
) -> bool {
    [archetype_net_guid.0, actor_net_guid.0]
        .iter()
        .filter_map(|&g| sink.path_for_guid(g))
        .any(is_player_controller_path)
}

/// Read a package-map export bunch: a run of GUID declarations with paths.
///
/// # Failure policy
///
/// The two skip paths below drop the whole bunch, and they are not the same
/// kind of drop:
///
/// - a RepLayout export is a *limitation* -- this parser does not implement
///   that variant -- so it is counted on its own line and reported as `Ok`;
/// - an impossible GUID count is a *failure*: every path declaration that
///   followed is lost, and returning `Ok` for it used to increment
///   `package_map_exports` as though the bunch had been read, leaving actors
///   that later failed to resolve their path or class with no counter pointing
///   anywhere. It is now an error, which the caller counts and whose abandoned
///   bits the caller tallies.
pub(super) fn read_package_map_exports(
    payload: &mut BitReader<'_>,
    stats: &mut NetStats,
    sink: &mut dyn ReplicationSink,
) -> Result<()> {
    let has_rep_layout_export = payload.read_bit()?;
    if has_rep_layout_export {
        // Unsupported variant: skip the bunch, but say so.
        stats.rep_layout_export_bunches += 1;
        payload.skip_remaining();
        return Ok(());
    }

    let num_guids = payload.read_i32()?;
    if num_guids < 0 || num_guids as u32 > MAX_GUID_COUNT {
        return Err(NetError::InvalidGuidCount {
            count: num_guids,
            max: MAX_GUID_COUNT,
        });
    }

    for _ in 0..num_guids {
        let _ = net_guid::internal_load_object(payload, true, 0, sink)?;
        stats.exported_guids += 1;
    }
    Ok(())
}

/// Consume the must-be-mapped GUID list that precedes the content blocks.
pub(super) fn read_must_be_mapped_guids(
    payload: &mut BitReader<'_>,
    stats: &mut NetStats,
) -> Result<()> {
    let count = payload.read_u16()?;
    for _ in 0..count {
        let _guid = payload.read_int_packed()?;
        stats.must_be_mapped_guids += 1;
    }
    Ok(())
}

/// Read the actor GUID (and spawn block, when dynamic) that opens a channel.
pub(super) fn handle_channel_open(
    header: &RawBunchHeader,
    payload: &mut BitReader<'_>,
    channels: &mut ChannelTable,
    stats: &mut NetStats,
    sink: &mut dyn ReplicationSink,
) -> Result<()> {
    let ch_index = header.ch_index;

    let actor_net_guid = net_guid::internal_load_object(payload, false, 0, sink)?;

    let mut state = ActorChannelState {
        channel_index: ch_index,
        is_open: true,
        is_dormant: false,
        actor_net_guid,
        archetype_net_guid: NetworkGuid(0),
        level_guid: NetworkGuid(0),
        spawn_location: None,
        spawn_rotation: None,
        spawn_scale: None,
        spawn_velocity: None,
        open_packet_id: header.packet_id,
    };

    // Dynamic actors have spawn data. It is mandatory, not optional: the
    // reference (`NewActorSerializer.cs`) reads archetype, level, location,
    // rotation, scale and velocity unconditionally. This used to be guarded by
    // `!payload.at_end()`, which turned "the spawn block is missing" into a
    // successful open carrying archetype and level GUID 0 and no transforms --
    // an actor row invented from a payload that ended. A payload that stops one
    // bit later already failed here; stopping exactly at the boundary now fails
    // the same way, and the shape is counted so a corpus run can say whether it
    // ever occurs.
    if actor_net_guid.is_dynamic() {
        if payload.at_end() {
            stats.actor_opens_missing_spawn += 1;
        }
        spawn::read_dynamic_spawn_data(payload, &mut state, sink)?;
    }

    stats.actor_opens += 1;
    sink.on_actor_open(&state);
    // The row already exists -- this channel's bunch counter was bumped before
    // the payload reached here -- so this must not overwrite it.
    let slot = channels.entry(ch_index).or_default();
    // Replacing a channel that is still open loses the previous actor: every
    // later block on this channel is attributed to the new one, and the old one
    // gets no close. The wire says the new actor owns the channel, so the
    // replacement stands, but a fabricated close for the old actor would be a
    // row the replay never sent. Count it instead -- nothing else moves when
    // this happens. A channel that was properly closed first is the ordinary
    // reuse and is not counted.
    if slot.state.as_ref().is_some_and(|s| s.is_open) {
        stats.channel_reopens_while_open += 1;
    }
    slot.state = Some(state);
    Ok(())
}

/// Mark a channel closed and notify the sink. A channel that is already closed
/// is left alone so a repeated close does not double-count.
pub(super) fn handle_channel_close(
    header: &RawBunchHeader,
    channels: &mut ChannelTable,
    stats: &mut NetStats,
    sink: &mut dyn ReplicationSink,
) {
    let Some(ch) = channels
        .get_mut(&header.ch_index)
        .and_then(|s| s.state.as_mut())
    else {
        return;
    };
    if !ch.is_open {
        return;
    }
    ch.is_open = false;
    ch.is_dormant = header.b_dormant;
    stats.actor_closes += 1;
    sink.on_actor_close(header.ch_index, ch.actor_net_guid, header.b_dormant);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every spelling of the controller asset in the corpus must normalise to
    /// the same leaf, and nothing else may.
    #[test]
    fn controller_path_spellings_all_normalise() {
        for path in [
            "/Game/Characters/_Core/BaseReplayController.BaseReplayController_C",
            "/Game/Characters/_Core/BaseReplayController",
            "Default__BaseReplayController_C",
            "/Game/Characters/BaseReplayController",
            "BaseReplayController",
        ] {
            assert!(is_player_controller_path(path), "{path}");
        }
        for path in [
            "",
            "/Game/Characters/_Core/BaseReplayControllerExtra",
            "/Game/Characters/_Core/PlayerController.PlayerController_C",
            "Default__BaseReplayController_D",
        ] {
            assert!(!is_player_controller_path(path), "{path}");
        }
    }
}
