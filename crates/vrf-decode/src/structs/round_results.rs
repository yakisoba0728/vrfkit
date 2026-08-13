//! `BombGameState.RoundResults` -- per-round winning team and outcome.

use vrf_bitio::BitReader;

use super::framing::{
    MAX_FIELDS_PER_ELEMENT, ensure_consumed, ensure_member_consumed, member_name, read_array_count,
    read_element_index, read_field_header, read_fname, read_narrow_byte,
};
use super::{Result, StructBlobError};

/// Names this blob in error messages.
const CONTEXT: &str = "RoundResults";

/// The role a team played during the round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AresTeamRole {
    None = 0,
    Attacker = 1,
    Defender = 2,
    FreeForAll = 3,
    Any = 4,
    RoleCount = 5,
}

impl AresTeamRole {
    fn from_byte(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::None),
            1 => Some(Self::Attacker),
            2 => Some(Self::Defender),
            3 => Some(Self::FreeForAll),
            4 => Some(Self::Any),
            5 => Some(Self::RoleCount),
            _ => Option::None,
        }
    }

    /// The snake_case spelling exporters write.
    ///
    /// Here rather than at the export site so the exhaustive match sits with
    /// the enum it enumerates: a new variant then fails to compile one file
    /// away from where it was added, instead of in a crate that does not own
    /// the type.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Attacker => "attacker",
            Self::Defender => "defender",
            Self::FreeForAll => "free_for_all",
            Self::Any => "any",
            Self::RoleCount => "role_count",
        }
    }
}

/// How the round ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AresRoundOutcome {
    Elimination = 0,
    Defuse = 1,
    Detonate = 2,
    TimeExpired = 3,
    Cheat = 4,
    Surrendered = 5,
    RoundOutcomeCount = 6,
    Invalid = 7,
}

impl AresRoundOutcome {
    fn from_byte(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Elimination),
            1 => Some(Self::Defuse),
            2 => Some(Self::Detonate),
            3 => Some(Self::TimeExpired),
            4 => Some(Self::Cheat),
            5 => Some(Self::Surrendered),
            6 => Some(Self::RoundOutcomeCount),
            7 => Some(Self::Invalid),
            _ => Option::None,
        }
    }

    /// The snake_case spelling exporters write. See [`AresTeamRole::as_str`].
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Elimination => "elimination",
            Self::Defuse => "defuse",
            Self::Detonate => "detonate",
            Self::TimeExpired => "time_expired",
            Self::Cheat => "cheat",
            Self::Surrendered => "surrendered",
            Self::RoundOutcomeCount => "round_outcome_count",
            Self::Invalid => "invalid",
        }
    }
}

/// A single round result entry.
#[derive(Debug, Clone, PartialEq)]
pub struct RoundResult {
    /// Zero-based round number.
    pub round_number: u32,
    /// The team name that won (e.g. "Blue", "Red"). `None` if not present.
    pub winning_team: Option<String>,
    /// The role of the winning team. `None` if not present in this update.
    pub winning_team_role: Option<AresTeamRole>,
    /// How the round ended. `None` if not present in this update.
    pub round_result: Option<AresRoundOutcome>,
}

/// Decode a `BombGameState.RoundResults` blob.
///
/// # Wire layout
///
/// Standard UE RepLayout dynamic-array framing (see module docs). Members are
/// selected by the name the replay declares for each handle, because the
/// numbers move between builds -- 93..=96 on 13.01, 81..=84 on 13.02.
///
/// # Arguments
///
/// * `reader` - A `BitReader` positioned at the start of the blob, with
///   `len_bits()` equal to the declared bit count.
/// * `declared` - The enclosing group's net field export names indexed by
///   handle. An empty slice makes every member undeclared and the blob an
///   error, which is the intended outcome: there is no safe fallback, only a
///   set of build-specific numbers that would be wrong without warning.
pub fn decode_round_results(
    reader: &mut BitReader<'_>,
    declared: &[Option<&str>],
) -> Result<Vec<RoundResult>> {
    if reader.at_end() {
        return Ok(Vec::new());
    }

    let count = read_array_count(reader)?;
    let mut results = Vec::new();

    while let Some(round_number) = read_element_index(reader, count)? {
        let mut winning_team: Option<String> = Option::None;
        let mut winning_team_role: Option<AresTeamRole> = Option::None;
        let mut round_result: Option<AresRoundOutcome> = Option::None;

        for field_idx in 0..=MAX_FIELDS_PER_ELEMENT {
            let Some((handle, bit_count)) = read_field_header(reader)? else {
                break;
            };
            if field_idx == MAX_FIELDS_PER_ELEMENT {
                return Err(StructBlobError::TooManyFields { context: CONTEXT });
            }

            // The sub-reader consumes the bits from the parent, so a field we
            // do not interpret still advances the stream correctly.
            let mut sub = reader.sub_reader(u64::from(bit_count))?;
            let member = member_name(declared, handle, CONTEXT)?;
            match member {
                "WinningTeam" => winning_team = Some(read_fname(&mut sub)?),
                // A payload of no usable width is an error. Returning `None`
                // would make a member explicitly sent by the wire look exactly
                // like one absent from this update.
                // A value the enum has no variant for is REPORTED, not folded
                // back into `None`. `None` already means "this update did not
                // send the member", so reusing it for "sent, unrecognised" made
                // a newly added game variant indistinguishable from an absent
                // field -- the column simply starts going null after a patch,
                // with no counter moving.
                "WinningTeamRole" => {
                    let v = read_narrow_byte(&mut sub, member, CONTEXT)?;
                    winning_team_role = Some(AresTeamRole::from_byte(v).ok_or(
                        StructBlobError::UnknownEnumValue {
                            enum_name: "AresTeamRole",
                            value: v,
                            context: CONTEXT,
                        },
                    )?);
                }
                "RoundResult" => {
                    let v = read_narrow_byte(&mut sub, member, CONTEXT)?;
                    round_result = Some(AresRoundOutcome::from_byte(v).ok_or(
                        StructBlobError::UnknownEnumValue {
                            enum_name: "AresRoundOutcome",
                            value: v,
                            context: CONTEXT,
                        },
                    )?);
                }
                // Opaque nested array, skipped. Declared at two consecutive
                // handles in both builds; one arm covers both because the
                // match is on the name.
                //
                // Skipped EXPLICITLY rather than by falling out of the match:
                // the consumption check below would otherwise read this as a
                // member that left its window unread, which is precisely the
                // distinction being drawn -- these bits are deliberately not
                // interpreted, they are not accidentally dropped.
                "EliminatedTeams" => sub.skip_remaining(),
                name => {
                    return Err(StructBlobError::UnsupportedMember {
                        name: name.to_owned(),
                        handle,
                        context: CONTEXT,
                    });
                }
            }
            // `read_fname` is self-delimiting and `read_narrow_byte` takes the
            // window's full width, so an interpreted member here consumes
            // exactly. A leftover means the payload was not the shape the name
            // promised; invalid enum widths are rejected at their reader.
            ensure_member_consumed(&sub, member, handle, bit_count, CONTEXT)?;
        }

        results.push(RoundResult {
            round_number,
            winning_team,
            winning_team_role,
            round_result,
        });
    }

    ensure_consumed(reader)?;
    Ok(results)
}
