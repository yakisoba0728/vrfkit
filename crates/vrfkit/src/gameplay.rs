//! Optional, evidence-backed projections of existing reader observations.
//!
//! Call [`native_event`] or [`round_phase_event`] from a [`crate::ReplayReader`]
//! callback. The reader never calls these helpers itself, so a consumer that
//! does not request gameplay interpretation pays no projection cost. Raw rows
//! and Event chunks stay untouched and available even when projection fails.
//!
//! A native declaration is not a reconstructed match result. Derived phase
//! observations are not deduplicated transitions or an assembled round table.
//! Identity resolution, movement joins and complete round analysis remain
//! separate operations. All observations are provisional until the read ends
//! successfully; file order need not be timestamp order.

use thiserror::Error;
use vrf_container::{
    EventChunk, event_payload_seconds_matches_time, known_event_word_count,
    parse_known_event_payload,
};
use vrf_export::FieldRecord;

const PHASE_FIELD: &str = "MulticastSetPhase.NewPhase";
const PHASE_GROUP: &str = "/Game/GameModes/Bomb/BombGameState.BombGameState_C_ClassNetCache";
const PHASE_CHECKSUM: u32 = 3_954_716_686;

/// The boundary of the evidence, not a numeric confidence estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GameplayEvidence {
    /// Exact native payload layout, tag, name and timestamp were validated.
    /// Actor references have not been resolved to player identities.
    ValidatedNativePayload,
    /// The declared field identity and raw byte match the typed phase value.
    /// No corroborating RPC or complete-round join has been performed.
    ValidatedReplicationValue,
}

/// An Event word interpreted as an actor reference, without guessing a player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorReference {
    /// The complete wire value. Zero is the engine's null reference.
    pub net_guid: u32,
}

impl ActorReference {
    /// Resolution remains explicit even for a nonzero reference.
    #[must_use]
    pub const fn status(self) -> ActorReferenceStatus {
        if self.net_guid == 0 {
            ActorReferenceStatus::Null
        } else {
            ActorReferenceStatus::Unresolved
        }
    }
}

/// What is known about an [`ActorReference`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActorReferenceStatus {
    /// The wire sent zero. There is no actor reference to resolve.
    Null,
    /// A GUID is present; no lifecycle or PlayerState lookup was attempted.
    Unresolved,
}

/// Observed `MulticastSetPhase.NewPhase` values, matching the existing Python
/// round extractor. These labels do not imply that every round phase occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RoundPhase {
    Reset,
    RoundStart,
    BuyPhaseEnd,
    /// Value 5 is observed after a round; no RPC gives it a semantic name.
    /// In particular it is not asserted to be a validated round-end result.
    UnnamedPostRound,
    SideSwitch,
    /// Preserve an unrecognized byte without assigning a game meaning.
    Unknown(u8),
}

impl RoundPhase {
    /// The original byte, including unknown values.
    #[must_use]
    pub const fn wire_value(self) -> u8 {
        match self {
            Self::Reset => 2,
            Self::RoundStart => 3,
            Self::BuyPhaseEnd => 4,
            Self::UnnamedPostRound => 5,
            Self::SideSwitch => 6,
            Self::Unknown(value) => value,
        }
    }
}

/// Supported observations. Native and derived origins are distinct in
/// [`GameplayEvent::source`] and [`GameplayEvent::evidence`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GameplayKind {
    /// Server-declared round number, without an ordinal or score inference.
    RoundStarted {
        round_number: u32,
    },
    /// Native character-death declaration. GUIDs refer to actors, not accounts
    /// or PlayerStates; neither a kill credit nor an identity join is implied.
    CharacterDeath {
        killer: ActorReference,
        victim: ActorReference,
    },
    /// This native payload does not identify a planter or site.
    SpikePlanted,
    /// This native payload does not identify a defuser.
    SpikeDefused,
    SpikeExploded,
    /// One replication observation, not an inferred transition. Repeated
    /// observations stay repeated, and unknown values retain their byte.
    RoundPhaseObserved(RoundPhase),
}

/// Native source data, borrowed without copying the raw payload.
#[derive(Debug, Clone, Copy)]
pub struct NativeEventSource<'a> {
    pub id: &'a str,
    pub group: &'a str,
    pub metadata: &'a str,
    /// First header timestamp in milliseconds. Zero is a valid wire value.
    pub time1_ms: u32,
    /// Second header timestamp, preserved without assuming it equals time1.
    pub time2_ms: u32,
    /// Finite inner timestamp, validated against time1 by the shared validator.
    pub payload_seconds: f32,
    pub raw_payload: &'a [u8],
}

/// Source stream and original evidence. Checkpoints are separate streams:
/// this API never relabels a checkpoint row as a main-stream observation.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum GameplaySource<'a> {
    /// An outer native Event chunk; it does not have a replication packet ID.
    NativeContainer(NativeEventSource<'a>),
    /// A field from `ReplayEvent::Records`, which currently contains only the
    /// main stream. Packet, channel, actor, raw bits and schema stay available.
    MainField(&'a FieldRecord),
}

/// A borrowed, optional interpretation of an unchanged raw observation.
#[derive(Debug, Clone, Copy)]
pub struct GameplayEvent<'a> {
    pub kind: GameplayKind,
    pub source: GameplaySource<'a>,
}

impl GameplayEvent<'_> {
    /// The observed millisecond timestamp. No missing time is replaced by zero.
    #[must_use]
    pub fn time_ms(&self) -> u32 {
        match self.source {
            GameplaySource::NativeContainer(source) => source.time1_ms,
            GameplaySource::MainField(field) => field.time_ms,
        }
    }

    #[must_use]
    pub fn evidence(&self) -> GameplayEvidence {
        match self.source {
            GameplaySource::NativeContainer(_) => GameplayEvidence::ValidatedNativePayload,
            GameplaySource::MainField(_) => GameplayEvidence::ValidatedReplicationValue,
        }
    }
}

/// Why a recognized observation was not assigned a gameplay interpretation.
/// The original reader data remains usable and is never changed by this error.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum GameplayError {
    #[error("native Event payload size does not match its declaration")]
    PayloadSize,
    #[error("native Event has unexplained bytes after its declared payload")]
    TrailingBytes,
    #[error("native Event payload does not match the measured layout, tag or name")]
    PayloadLayout,
    #[error("native Event payload timestamp is non-finite or disagrees with its header")]
    PayloadTime,
    #[error("round-phase field has an unsupported group, subobject or checksum")]
    PhaseIdentity,
    #[error("round-phase field lacks one unambiguous byte-valued integer overlay")]
    PhaseValue,
    #[error("round-phase field raw byte disagrees with its typed value")]
    PhaseRawValue,
}

/// Project five supported native Event groups without changing the chunk.
///
/// `Ok(None)` means this group is outside the supported gameplay projection;
/// callers can still inspect its raw payload through the reader. `Err` means
/// a supported group failed validation. Besides the shared container payload
/// checks, the measured name must be NUL-terminated and no trailing chunk
/// bytes may be unexplained. This deliberately does not tighten raw parsing.
///
/// # Errors
///
/// [`GameplayError`] for malformed or changed supported payloads. A successful
/// projection establishes a native declaration, not independently corroborated
/// player identities, kill credit or complete round results.
pub fn native_event<'a>(
    event: &'a EventChunk<'_>,
) -> Result<Option<GameplayEvent<'a>>, GameplayError> {
    match event.group.as_str() {
        "roundStarted" | "characterDeath" | "spikePlanted" | "spikeDefused" | "spikeExploded" => {}
        _ => return Ok(None),
    }
    if usize::try_from(event.size_in_bytes).ok() != Some(event.payload.len()) {
        return Err(GameplayError::PayloadSize);
    }
    if event.trailing_bytes != 0 {
        return Err(GameplayError::TrailingBytes);
    }
    let word_count = known_event_word_count(&event.group).ok_or(GameplayError::PayloadLayout)?;
    if !terminated_name(event.payload, word_count) {
        return Err(GameplayError::PayloadLayout);
    }
    let payload = parse_known_event_payload(&event.group, event.payload)
        .ok_or(GameplayError::PayloadLayout)?;
    if !event_payload_seconds_matches_time(event.time1, payload.seconds) {
        return Err(GameplayError::PayloadTime);
    }
    let kind = match event.group.as_str() {
        "roundStarted" => GameplayKind::RoundStarted {
            round_number: payload.words[0],
        },
        "characterDeath" => GameplayKind::CharacterDeath {
            killer: ActorReference {
                net_guid: payload.words[0],
            },
            victim: ActorReference {
                net_guid: payload.words[1],
            },
        },
        "spikePlanted" => GameplayKind::SpikePlanted,
        "spikeDefused" => GameplayKind::SpikeDefused,
        "spikeExploded" => GameplayKind::SpikeExploded,
        _ => unreachable!("supported groups were selected above"),
    };
    Ok(Some(GameplayEvent {
        kind,
        source: GameplaySource::NativeContainer(NativeEventSource {
            id: &event.id,
            group: &event.group,
            metadata: &event.metadata,
            time1_ms: event.time1,
            time2_ms: event.time2,
            payload_seconds: payload.seconds,
            raw_payload: event.payload,
        }),
    }))
}

fn terminated_name(payload: &[u8], word_count: usize) -> bool {
    let offset = 4 + 4 * word_count;
    let Some(length_bytes) = payload.get(offset..offset + 4) else {
        return false;
    };
    let length = i32::from_le_bytes(length_bytes.try_into().expect("four-byte slice"));
    let Some(width) = (length.unsigned_abs() as usize).checked_mul(if length < 0 { 2 } else { 1 })
    else {
        return false;
    };
    let Some(end) = (offset + 4).checked_add(width) else {
        return false;
    };
    let Some(name) = payload.get(offset + 4..end) else {
        return false;
    };
    if length < 0 {
        name.ends_with(&[0, 0])
    } else {
        name.last() == Some(&0)
    }
}

/// Project one main-stream phase field using the measured schema identity.
///
/// Pass only fields from [`crate::ReplayEvent::Records`]. This stateless helper
/// does not deduplicate repetitions, invent missing phases, use future state,
/// or assemble rounds. The existing Python `extract_rounds.py` performs those
/// higher-level joins and RPC cross-checks. Unknown phase bytes stay explicit.
/// Do not pass checkpoint rows: their scope is outside this API contract.
///
/// # Errors
///
/// A recognized field with a different source identity, absent/conflicting
/// typed overlay, or raw-byte mismatch returns [`GameplayError`]. Other field
/// names return `Ok(None)`.
pub fn round_phase_event(field: &FieldRecord) -> Result<Option<GameplayEvent<'_>>, GameplayError> {
    if field.field_name.as_deref() != Some(PHASE_FIELD) {
        return Ok(None);
    }
    if field.group_path.as_ref() != PHASE_GROUP
        || field.object_net_guid.is_some()
        || field.compatible_checksum != Some(PHASE_CHECKSUM)
    {
        return Err(GameplayError::PhaseIdentity);
    }
    let value = field
        .value_i64
        .and_then(|value| u8::try_from(value).ok())
        .ok_or(GameplayError::PhaseValue)?;
    if field.value_f64.is_some() || field.value_bool.is_some() || field.value_str.is_some() {
        return Err(GameplayError::PhaseValue);
    }
    if field.bit_count != 8 || field.raw_bits.as_deref() != Some(&[value][..]) {
        return Err(GameplayError::PhaseRawValue);
    }
    let phase = match value {
        2 => RoundPhase::Reset,
        3 => RoundPhase::RoundStart,
        4 => RoundPhase::BuyPhaseEnd,
        5 => RoundPhase::UnnamedPostRound,
        6 => RoundPhase::SideSwitch,
        other => RoundPhase::Unknown(other),
    };
    Ok(Some(GameplayEvent {
        kind: GameplayKind::RoundPhaseObserved(phase),
        source: GameplaySource::MainField(field),
    }))
}
