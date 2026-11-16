//! Main-stream identity evidence with explicit packet boundaries.

use std::collections::BTreeMap;

use thiserror::Error;
use vrf_bitio::BitReader;

use crate::{ActorRecord, FieldRecord, MovementProvenance, MovementRecord, ReplayRecords};

use super::ActorReference;

const BOMB_PLAYER_STATE: &str = "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C";
const SPAWNED_CHARACTER_CHECKSUM: u32 = 1_413_303_948;

/// A validated body reference, borrowing the complete unmodified field row.
/// This does not resolve an account or establish a kill credit.
#[derive(Debug, Clone, Copy)]
pub struct SpawnedCharacterObservation<'a> {
    pub player_state_net_guid: u32,
    pub character: ActorReference,
    pub source: &'a FieldRecord,
}

/// Project the proven PlayerState `SpawnedCharacter` relation.
///
/// The existing Python `player_identity` tool establishes this body criterion;
/// `Owner`, `PossessedCharacter` and a pawn's `PlayerState` do not establish it.
/// This helper accepts BombPlayerState and its canonical Swiftplay alias, the
/// measured checksum, an actor field, one u32 integer overlay and a completely
/// consumed packed GUID equal to that overlay. Other names return `Ok(None)`.
///
/// # Errors
///
/// A recognized name with unsupported identity or invalid raw/typed evidence
/// returns an error. Raw data is untouched. Use main-stream rows only.
pub fn spawned_character_observation(
    field: &FieldRecord,
) -> Result<Option<SpawnedCharacterObservation<'_>>, IdentityError> {
    if field.field_name.as_deref() != Some("SpawnedCharacter") {
        return Ok(None);
    }
    if vrf_decode::canonical_group(&field.group_path) != BOMB_PLAYER_STATE
        || field.object_net_guid.is_some()
        || field.actor_net_guid == 0
        || field.compatible_checksum != Some(SPAWNED_CHARACTER_CHECKSUM)
    {
        return Err(IdentityError::FieldIdentity);
    }
    let value = field
        .value_i64
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(IdentityError::FieldValue)?;
    if field.value_f64.is_some() || field.value_bool.is_some() || field.value_str.is_some() {
        return Err(IdentityError::FieldValue);
    }
    let raw = field
        .raw_bits
        .as_deref()
        .ok_or(IdentityError::FieldRawValue)?;
    if field.bit_count == 0 || field.bit_count % 8 != 0 || raw.len() != field.bit_count as usize / 8
    {
        return Err(IdentityError::FieldRawValue);
    }
    let mut reader = BitReader::new(raw);
    if reader.read_int_packed().ok() != Some(value)
        || reader.position() != u64::from(field.bit_count)
    {
        return Err(IdentityError::FieldRawValue);
    }
    Ok(Some(SpawnedCharacterObservation {
        player_state_net_guid: field.actor_net_guid,
        character: ActorReference { net_guid: value },
        source: field,
    }))
}

/// Why a main-stream identity observation could not safely update the timeline.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum IdentityError {
    #[error("SpawnedCharacter has an unsupported group, subobject, actor or checksum")]
    FieldIdentity,
    #[error("SpawnedCharacter lacks one unambiguous u32 integer overlay")]
    FieldValue,
    #[error("SpawnedCharacter raw bits do not exactly encode its typed GUID")]
    FieldRawValue,
    #[error("identity packet contains a row from a different packet")]
    PacketMismatch,
    #[error("identity packets must be observed once in strictly increasing order")]
    PacketOrder,
    #[error("actor lifecycle has an unsupported event or null actor")]
    ActorLifecycle,
    #[error("identity timeline is invalid after an earlier observation error")]
    InvalidTimeline,
    #[error("movement row does not have its exact aligned transport provenance")]
    MovementProvenance,
}

/// An explicit absence of sufficient identity evidence; zero is never guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum IdentityUnresolved {
    /// No preceding nonzero current SpawnedCharacter declaration names the actor.
    NoReference,
    /// The requested packet boundary lies beyond the observed stream frontier.
    UnobservedPacket,
    /// An observation failed; reset and replay before resolving any more rows.
    InvalidTimeline,
    /// A matching actor/PlayerState open is missing, closed or contradictory.
    Lifecycle,
    /// A close/replacement makes the prior reference belong to an older lifetime.
    PreviousLifetime,
    /// Separate record vectors do not establish order within this packet/range.
    ConcurrentChange,
}

/// One current body relation and the original evidence visible at its boundary.
#[derive(Debug, Clone, Copy)]
pub struct BodyIdentity<'a> {
    pub character_net_guid: u32,
    pub player_state_net_guid: u32,
    /// Exact reference write, including raw bytes, packet, time and schema.
    pub declaration: &'a FieldRecord,
    pub player_state_open: &'a ActorRecord,
    pub character_open: &'a ActorRecord,
}

/// Resolution of a character GUID to its currently observed PlayerState relation.
/// Resolved means this relation only, not an account, display name or kill credit.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum IdentityResolution<'a> {
    Null,
    Unresolved(IdentityUnresolved),
    /// Conflicting non-retired declarations, including indeterminate same-packet
    /// lifetime boundaries, retained in PlayerState GUID order.
    Ambiguous {
        declarations: Vec<&'a FieldRecord>,
    },
    Resolved(BodyIdentity<'a>),
}

/// An unchanged movement row and its aligned wire source, with optional identity.
#[derive(Debug)]
pub struct MovementIdentity<'a> {
    pub movement: &'a MovementRecord,
    pub provenance: &'a MovementProvenance,
    pub identity: IdentityResolution<'a>,
}

#[derive(Debug)]
struct Lifetime {
    opened: ActorRecord,
    ended: Option<ActorRecord>,
    previous_end: Option<u32>,
    conflicting_open: Option<ActorRecord>,
}

/// Optional main-stream timeline; construct a fresh value for each replay.
///
/// Feed every main [`crate::ReplayEvent::Records`] batch once and in order to
/// [`Self::observe_records`]. Checkpoint records are independent snapshots and
/// must never be fed to this timeline. Only identity fields and actor lifecycle
/// evidence are retained, not every field or movement row. Storage grows with
/// that evidence; the caller chooses whether to build and retain the timeline.
///
/// Queries use packet boundaries, never sorted wall-clock times or final GUID
/// paths. A later write cannot resolve an earlier boundary. Current null writes
/// retire the relation; earlier queries still retain the old body. This differs
/// intentionally from Python's whole-export body-history/account join.
///
/// All evidence is provisional until Reader completion. An observation error
/// invalidates the timeline so a skipped bad write cannot leave stale identity.
#[derive(Debug, Default)]
pub struct MainIdentityTimeline {
    declarations: BTreeMap<u32, Vec<FieldRecord>>,
    lifetimes: BTreeMap<u32, Vec<Lifetime>>,
    channels: BTreeMap<u32, u32>,
    last_end: BTreeMap<u32, u32>,
    latest_packet: Option<u32>,
    invalid: bool,
}

impl MainIdentityTimeline {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Observe a main-stream Reader batch. Partial-only flushes add no evidence.
    ///
    /// # Errors
    ///
    /// Invalid evidence or packet ordering invalidates the timeline. No partial
    /// update from the rejected batch is retained; earlier evidence stays owned.
    pub fn observe_records(&mut self, records: &ReplayRecords<'_>) -> Result<(), IdentityError> {
        let packet = records
            .fields
            .first()
            .map(|row| row.packet_id)
            .or_else(|| records.actors.first().map(|row| row.packet_id))
            .or_else(|| records.movement.first().map(|row| row.packet_id));
        let Some(packet) = packet else {
            return if self.invalid {
                Err(IdentityError::InvalidTimeline)
            } else {
                Ok(())
            };
        };
        if records.movement.iter().any(|row| row.packet_id != packet) {
            self.invalid = true;
            return Err(IdentityError::PacketMismatch);
        }
        self.observe_packet(packet, records.actors, records.fields)
    }

    /// Observe one complete main packet's actor and field vectors.
    ///
    /// Use [`Self::observe_records`] with ReplayReader. This entry point also
    /// supports already owned main rows. Pass all actor lifecycle and field
    /// rows in their original per-vector order. No ordering between the two
    /// vectors is invented.
    ///
    /// # Errors
    ///
    /// Invalid evidence or non-increasing packet IDs invalidate the timeline.
    pub fn observe_packet(
        &mut self,
        packet: u32,
        actors: &[ActorRecord],
        fields: &[FieldRecord],
    ) -> Result<(), IdentityError> {
        if let Err(error) = self.validate_packet(packet, actors, fields) {
            self.invalid = true;
            return Err(error);
        }
        for actor in actors {
            self.observe_actor(actor);
        }
        for field in fields {
            if field.field_name.as_deref() == Some("SpawnedCharacter") {
                self.declarations
                    .entry(field.actor_net_guid)
                    .or_default()
                    .push(field.clone());
            }
        }
        self.latest_packet = Some(packet);
        Ok(())
    }

    fn validate_packet(
        &self,
        packet: u32,
        actors: &[ActorRecord],
        fields: &[FieldRecord],
    ) -> Result<(), IdentityError> {
        if self.invalid {
            return Err(IdentityError::InvalidTimeline);
        }
        if self
            .latest_packet
            .is_some_and(|previous| previous >= packet)
        {
            return Err(IdentityError::PacketOrder);
        }
        if actors.iter().any(|row| row.packet_id != packet)
            || fields.iter().any(|row| row.packet_id != packet)
        {
            return Err(IdentityError::PacketMismatch);
        }
        for actor in actors {
            if actor.actor_net_guid == 0 || !matches!(actor.event, "open" | "close" | "dormant") {
                return Err(IdentityError::ActorLifecycle);
            }
        }
        for field in fields {
            spawned_character_observation(field)?;
        }
        Ok(())
    }

    fn end_actor(&mut self, guid: u32, evidence: &ActorRecord) {
        if let Some(last) = self
            .lifetimes
            .get_mut(&guid)
            .and_then(|history| history.last_mut())
        {
            if last.ended.is_none() {
                last.ended = Some(evidence.clone());
            }
        }
        self.last_end.insert(guid, evidence.packet_id);
        self.channels.retain(|_, occupant| *occupant != guid);
    }

    fn observe_actor(&mut self, row: &ActorRecord) {
        let guid = row.actor_net_guid;
        match row.event {
            "close" => self.end_actor(guid, row),
            "dormant" => {
                if self.channels.get(&row.channel_index) == Some(&guid) {
                    self.channels.remove(&row.channel_index);
                }
            }
            "open" => {
                if let Some(old) = self.channels.get(&row.channel_index).copied() {
                    if old != guid {
                        self.end_actor(old, row);
                    }
                }
                let already_open = self.channels.values().any(|&actor| actor == guid);
                let history = self.lifetimes.entry(guid).or_default();
                if let Some(life) = history.last_mut().filter(|life| life.ended.is_none()) {
                    if already_open && life.conflicting_open.is_none() {
                        life.conflicting_open = Some(row.clone());
                    }
                } else {
                    history.push(Lifetime {
                        opened: row.clone(),
                        ended: None,
                        previous_end: self.last_end.get(&guid).copied(),
                        conflicting_open: None,
                    });
                }
                self.channels.insert(row.channel_index, guid);
            }
            _ => unreachable!("lifecycle events were validated"),
        }
    }

    fn lifetime_before(&self, guid: u32, packet: u32) -> Option<&Lifetime> {
        let history = self.lifetimes.get(&guid)?;
        let index = history.partition_point(|life| life.opened.packet_id < packet);
        history.get(index.checked_sub(1)?)
    }

    fn live_before(&self, guid: u32, packet: u32) -> Option<&Lifetime> {
        let life = self.lifetime_before(guid, packet)?;
        if life
            .ended
            .as_ref()
            .is_some_and(|end| end.packet_id < packet)
            || life
                .conflicting_open
                .as_ref()
                .is_some_and(|event| event.packet_id < packet)
        {
            None
        } else {
            Some(life)
        }
    }

    /// Resolve the current relation strictly before a main packet starts.
    ///
    /// This boundary excludes every row in `packet`, including same-time rows.
    /// It is not a claim about an entity at a native Event timestamp. Queries
    /// beyond the latest observed packet's following boundary are unresolved.
    #[must_use]
    pub fn resolve_before_packet(
        &self,
        character: ActorReference,
        packet: u32,
    ) -> IdentityResolution<'_> {
        use IdentityResolution::{Null, Unresolved};
        if character.net_guid == 0 {
            return Null;
        }
        if self.invalid {
            return Unresolved(IdentityUnresolved::InvalidTimeline);
        }
        if self
            .latest_packet
            .is_none_or(|latest| packet > latest.saturating_add(1))
        {
            return Unresolved(IdentityUnresolved::UnobservedPacket);
        }
        let mut candidates = Vec::new();
        let mut retired_reference = false;
        let character_previous_end = self
            .lifetime_before(character.net_guid, packet)
            .and_then(|life| life.previous_end);
        for history in self.declarations.values() {
            let index = history.partition_point(|row| row.packet_id < packet);
            let Some(row) = index.checked_sub(1).and_then(|index| history.get(index)) else {
                continue;
            };
            if row.value_i64 == Some(i64::from(character.net_guid)) {
                let state_life = self.lifetime_before(row.actor_net_guid, packet);
                let ended = state_life.is_some_and(|life| {
                    life.ended
                        .as_ref()
                        .is_some_and(|end| end.packet_id < packet)
                });
                if ended {
                    continue;
                }
                // Only a strictly later end proves this declaration retired.
                // Equal-packet field/lifecycle order remains indeterminate.
                let retired = [
                    state_life.and_then(|life| life.previous_end),
                    character_previous_end,
                ]
                .into_iter()
                .flatten()
                .any(|end| row.packet_id < end);
                if retired {
                    retired_reference = true;
                    continue;
                }
                candidates.push(row);
            }
        }
        if candidates.len() > 1 {
            return IdentityResolution::Ambiguous {
                declarations: candidates,
            };
        }
        let Some(declaration) = candidates.first().copied() else {
            return Unresolved(if retired_reference {
                IdentityUnresolved::PreviousLifetime
            } else {
                IdentityUnresolved::NoReference
            });
        };
        let Some(state_life) = self.live_before(declaration.actor_net_guid, packet) else {
            return Unresolved(IdentityUnresolved::Lifecycle);
        };
        let Some(character_life) = self.live_before(character.net_guid, packet) else {
            return Unresolved(IdentityUnresolved::Lifecycle);
        };
        if declaration.packet_id < state_life.opened.packet_id
            || state_life
                .previous_end
                .is_some_and(|end| declaration.packet_id <= end)
            || character_life
                .previous_end
                .is_some_and(|end| declaration.packet_id <= end)
        {
            return Unresolved(IdentityUnresolved::PreviousLifetime);
        }
        IdentityResolution::Resolved(BodyIdentity {
            character_net_guid: character.net_guid,
            player_state_net_guid: declaration.actor_net_guid,
            declaration,
            player_state_open: &state_life.opened,
            character_open: &character_life.opened,
        })
    }

    /// Associate one movement by aligned wire provenance, without time or
    /// coordinate proximity. First observe this complete batch, then call here.
    ///
    /// Identity must already be known before the initial fragment packet and
    /// remain unchanged through completion. A relevant relation write or
    /// lifetime boundary in that range is `ConcurrentChange`: the separate
    /// vectors do not establish its order with the movement RPC. The transport
    /// actor can be a replay controller, not the movement row's character.
    /// This is an association at the observed transport window; it is not
    /// backdated to the move's server tick or a native Event timestamp.
    ///
    /// # Errors
    ///
    /// Missing, misaligned or impossible transport provenance is rejected.
    pub fn movement_identity<'a>(
        &'a self,
        records: &'a ReplayRecords<'_>,
        index: usize,
    ) -> Result<MovementIdentity<'a>, IdentityError> {
        if records.movement.len() != records.movement_provenance.len() {
            return Err(IdentityError::MovementProvenance);
        }
        let movement = records
            .movement
            .get(index)
            .ok_or(IdentityError::MovementProvenance)?;
        let provenance = &records.movement_provenance[index];
        let source = provenance.source.ok_or(IdentityError::MovementProvenance)?;
        if source.completion_packet_id != movement.packet_id
            || source.initial_packet_id > source.completion_packet_id
            || self.latest_packet != Some(movement.packet_id)
        {
            return Err(IdentityError::MovementProvenance);
        }
        let mut identity = self.resolve_before_packet(
            ActorReference {
                net_guid: movement.character_net_guid,
            },
            source.initial_packet_id,
        );
        if let IdentityResolution::Resolved(body) = &identity {
            let in_range = |packet: u32| {
                source.initial_packet_id <= packet && packet <= source.completion_packet_id
            };
            let reference_change = self.declarations.values().any(|rows| {
                rows.iter()
                    .rev()
                    .take_while(|row| row.packet_id >= source.initial_packet_id)
                    .any(|row| {
                        in_range(row.packet_id)
                            && (row.actor_net_guid == body.player_state_net_guid
                                || row.value_i64 == Some(i64::from(body.character_net_guid)))
                    })
            });
            let lifecycle_change = [body.character_net_guid, body.player_state_net_guid]
                .iter()
                .any(|guid| {
                    self.lifetimes.get(guid).is_some_and(|lives| {
                        lives.iter().any(|life| {
                            in_range(life.opened.packet_id)
                                || life
                                    .ended
                                    .as_ref()
                                    .is_some_and(|event| in_range(event.packet_id))
                                || life
                                    .conflicting_open
                                    .as_ref()
                                    .is_some_and(|event| in_range(event.packet_id))
                        })
                    })
                });
            if reference_change || lifecycle_change {
                identity = IdentityResolution::Unresolved(IdentityUnresolved::ConcurrentChange);
            }
        }
        Ok(MovementIdentity {
            movement,
            provenance,
            identity,
        })
    }
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
