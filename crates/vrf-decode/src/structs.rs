//! Decoders for Valorant struct-array blobs that arrive as opaque raw bits.
//!
//! # Covered blobs
//!
//! | Field | Export group | Purpose |
//! |-------|-------------|---------|
//! | `RoundResults` | `BombGameState` | Per-round winning team and outcome |
//! | `TeamEconomy` | `BombGameState` | Team loadout value per round |
//! | `RoundInfos` | `OwnerExclusivePlayerInfo` | Per-player per-round credit |
//!
//! # Wire layout (established from C# reference parser + corpus validation)
//!
//! All three use the same UE RepLayout dynamic-array serialization:
//!
//! ```text
//! [IntPacked: declared_count]
//! repeat {
//!     [IntPacked: encoded_index]  // 0 = end, otherwise index = encoded - 1
//!     repeat {
//!         [IntPacked: encoded_handle]  // 0 = end, otherwise handle = encoded - 1
//!         [IntPacked: bit_count]       // payload length in bits
//!         [bits: payload]              // field-specific content
//!     }
//! }
//! ```
//!
//! Field handles and payload types per blob:
//!
//! ## RoundResults (handles from `AresRoundResults.cs`)
//! - 93: WinningTeam (FName)
//! - 94: WinningTeamRole (enum byte, variable bit width)
//! - 95: RoundResult (enum byte, variable bit width)
//! - 96: EliminatedTeams (skipped - opaque nested array)
//!
//! ## TeamEconomy (handles from `AresTeamEconomy.cs`)
//! - 56: ReplicationId (IntPacked)
//! - 57: LoadoutValue (Int32)
//! - 58: AverageLoadoutValue (Int32)
//!
//! ## RoundInfos (handles from `OwnerExclusivePlayerInfoDescriptor.cs`)
//! - 40: RoundNumber (Int32)
//! - 41: StartOfRoundMoney (Int32)
//! - 42: StartOfRoundLoadoutValue (Int32)
//! - 43: EndOfRoundMoney (Int32)
//! - 44: EndOfRoundLoadoutValue (Int32)
//!
//! # FName wire format (from `FArchive.ReadFNameCore`)
//! ```text
//! [1 bit: is_hardcoded]
//! if hardcoded: [IntPacked: name_index]  -> returned as decimal string
//! else:        [FString: name] [Int32: number_suffix (ignored)]
//! ```

use vrf_bitio::BitReader;

// ─── Error type ──────────────────────────────────────────────────────────────

/// Errors that can occur while decoding a struct-array blob.
#[derive(Debug, Clone, thiserror::Error)]
pub enum StructBlobError {
    /// The underlying bit reader hit EOF or produced a malformed primitive.
    #[error("bit read: {0}")]
    BitIo(#[from] vrf_bitio::BitError),

    /// The declared array element count exceeds a sane maximum.
    #[error("array count {count} exceeds maximum {max}")]
    ArrayCountTooLarge { count: u32, max: u32 },

    /// An element index is out of bounds relative to the declared count.
    #[error("element index {index} >= declared count {count}")]
    IndexOutOfBounds { index: u32, count: u32 },

    /// A field payload declared more bits than remain in the stream.
    #[error("field payload {bits} bits exceeds remaining {remaining}")]
    PayloadTooLarge { bits: u32, remaining: u64 },

    /// An unexpected field handle was encountered.
    #[error("unsupported field handle {handle} in {context}")]
    UnsupportedHandle { handle: u32, context: &'static str },

    /// Too many fields in a single element (guard against infinite loops).
    #[error("too many fields in element ({context})")]
    TooManyFields { context: &'static str },

    /// Bits remain after the blob should have been fully consumed.
    #[error("not fully consumed: {remaining} bits left")]
    NotFullyConsumed { remaining: u64 },
}

/// Convenience alias.
pub type Result<T> = core::result::Result<T, StructBlobError>;

// ─── Constants ───────────────────────────────────────────────────────────────

const MAX_ARRAY_COUNT: u32 = 128;
const MAX_FIELDS_PER_ELEMENT: u32 = 8;
const MAX_FIELD_PAYLOAD_BITS: u32 = 64 * 1024;

// ─── Helper: read array framing ──────────────────────────────────────────────

/// Read the declared element count from the stream.
fn read_array_count(reader: &mut BitReader<'_>) -> Result<u32> {
    let count = reader.read_int_packed()?;
    if count > MAX_ARRAY_COUNT {
        return Err(StructBlobError::ArrayCountTooLarge {
            count,
            max: MAX_ARRAY_COUNT,
        });
    }
    Ok(count)
}

/// Read the next element index. Returns `None` if the terminator (0) is read.
fn read_element_index(reader: &mut BitReader<'_>, declared_count: u32) -> Result<Option<u32>> {
    let encoded = reader.read_int_packed()?;
    if encoded == 0 {
        return Ok(None);
    }
    let index = encoded - 1;
    if index >= declared_count {
        return Err(StructBlobError::IndexOutOfBounds {
            index,
            count: declared_count,
        });
    }
    Ok(Some(index))
}

/// Read the next field handle. Returns `None` if the terminator (0) is read.
/// Also reads the bit_count of the field payload.
fn read_field_header(reader: &mut BitReader<'_>) -> Result<Option<(u32, u32)>> {
    let encoded = reader.read_int_packed()?;
    if encoded == 0 {
        return Ok(None);
    }
    let handle = encoded - 1;
    let bit_count = reader.read_int_packed()?;
    if bit_count > MAX_FIELD_PAYLOAD_BITS {
        return Err(StructBlobError::PayloadTooLarge {
            bits: bit_count,
            remaining: reader.bits_remaining(),
        });
    }
    if u64::from(bit_count) > reader.bits_remaining() {
        return Err(StructBlobError::PayloadTooLarge {
            bits: bit_count,
            remaining: reader.bits_remaining(),
        });
    }
    Ok(Some((handle, bit_count)))
}

/// Read an FName from a sub-reader (1 bit hardcoded flag, then either IntPacked
/// or FString + Int32).
fn read_fname(reader: &mut BitReader<'_>) -> Result<String> {
    let is_hardcoded = reader.read_bit()?;
    if is_hardcoded {
        let index = reader.read_int_packed()?;
        Ok(index.to_string())
    } else {
        let name = reader.read_fstring(1024)?;
        let _number = reader.read_i32()?;
        Ok(name)
    }
}

/// Ensure the reader is fully consumed.
fn ensure_consumed(reader: &BitReader<'_>) -> Result<()> {
    if reader.bits_remaining() > 0 {
        return Err(StructBlobError::NotFullyConsumed {
            remaining: reader.bits_remaining(),
        });
    }
    Ok(())
}

// ─── RoundResults ────────────────────────────────────────────────────────────

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
/// Standard UE RepLayout dynamic-array framing (see module docs).
/// Field handles: 93=WinningTeam(FName), 94=WinningTeamRole(enum),
/// 95=RoundResult(enum), 96=EliminatedTeams(skip).
///
/// # Arguments
///
/// * `reader` - A `BitReader` positioned at the start of the blob, with
///   `len_bits()` equal to the declared bit count.
pub fn decode_round_results(reader: &mut BitReader<'_>) -> Result<Vec<RoundResult>> {
    if reader.at_end() {
        return Ok(Vec::new());
    }

    let count = read_array_count(reader)?;
    let mut results = Vec::new();

    while let Some(round_number) = read_element_index(reader, count)? {
        let mut winning_team: Option<String> = Option::None;
        let mut winning_team_role: Option<AresTeamRole> = Option::None;
        let mut round_result: Option<AresRoundOutcome> = Option::None;

        for field_idx in 0..MAX_FIELDS_PER_ELEMENT {
            let Some((handle, bit_count)) = read_field_header(reader)? else {
                break;
            };
            if field_idx == MAX_FIELDS_PER_ELEMENT - 1 {
                return Err(StructBlobError::TooManyFields {
                    context: "RoundResults",
                });
            }

            let mut sub = reader.sub_reader(u64::from(bit_count))?;
            match handle {
                93 => {
                    winning_team = Some(read_fname(&mut sub)?);
                }
                94 => {
                    let bits = sub.bits_remaining();
                    if bits > 0 && bits <= 8 {
                        let v = sub.read_bits(bits as u32)? as u8;
                        winning_team_role = AresTeamRole::from_byte(v);
                    }
                }
                95 => {
                    let bits = sub.bits_remaining();
                    if bits > 0 && bits <= 8 {
                        let v = sub.read_bits(bits as u32)? as u8;
                        round_result = AresRoundOutcome::from_byte(v);
                    }
                }
                96 => {
                    // EliminatedTeams: opaque nested array, skip.
                }
                _ => {
                    return Err(StructBlobError::UnsupportedHandle {
                        handle,
                        context: "RoundResults",
                    });
                }
            }
            // sub_reader already consumed the bits from the parent
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

// ─── TeamEconomy ─────────────────────────────────────────────────────────────

/// A single team economy update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamEconomyUpdate {
    /// Zero-based team index (0 = first team, 1 = second team).
    pub index: u32,
    /// The actor replication ID for this team (only present on initial spawn).
    pub replication_id: Option<u32>,
    /// Total loadout value for the team this round.
    pub loadout_value: Option<i32>,
    /// Average loadout value per player for the team this round.
    pub average_loadout_value: Option<i32>,
}

/// Decode a `BombGameState.TeamEconomy` blob.
///
/// # Wire layout
///
/// Standard UE RepLayout dynamic-array framing (see module docs).
/// Field handles: 56=ReplicationId(IntPacked), 57=LoadoutValue(Int32),
/// 58=AverageLoadoutValue(Int32).
///
/// # Arguments
///
/// * `reader` - A `BitReader` positioned at the start of the blob, with
///   `len_bits()` equal to the declared bit count.
pub fn decode_team_economy(reader: &mut BitReader<'_>) -> Result<Vec<TeamEconomyUpdate>> {
    let count = read_array_count(reader)?;
    let mut results = Vec::new();

    while let Some(index) = read_element_index(reader, count)? {
        let mut replication_id: Option<u32> = Option::None;
        let mut loadout_value: Option<i32> = Option::None;
        let mut average_loadout_value: Option<i32> = Option::None;

        for field_idx in 0..MAX_FIELDS_PER_ELEMENT {
            let Some((handle, bit_count)) = read_field_header(reader)? else {
                break;
            };
            if field_idx == MAX_FIELDS_PER_ELEMENT - 1 {
                return Err(StructBlobError::TooManyFields {
                    context: "TeamEconomy",
                });
            }

            let mut sub = reader.sub_reader(u64::from(bit_count))?;
            match handle {
                56 => {
                    replication_id = Some(sub.read_int_packed()?);
                }
                57 => {
                    loadout_value = Some(sub.read_i32()?);
                }
                58 => {
                    average_loadout_value = Some(sub.read_i32()?);
                }
                _ => {
                    return Err(StructBlobError::UnsupportedHandle {
                        handle,
                        context: "TeamEconomy",
                    });
                }
            }
        }

        results.push(TeamEconomyUpdate {
            index,
            replication_id,
            loadout_value,
            average_loadout_value,
        });
    }

    ensure_consumed(reader)?;
    Ok(results)
}

// ─── RoundInfos ──────────────────────────────────────────────────────────────

/// A single player round-info entry (per-round economy for one player).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerRoundInfo {
    /// Zero-based index within the array (round index within this update).
    pub index: u32,
    /// Round number. `None` if not present in this update.
    pub round_number: Option<i32>,
    /// Money at the start of the round.
    pub start_of_round_money: Option<i32>,
    /// Loadout value at the start of the round.
    pub start_of_round_loadout_value: Option<i32>,
    /// Money at the end of the round (the analytically critical field).
    pub end_of_round_money: Option<i32>,
    /// Loadout value at the end of the round.
    pub end_of_round_loadout_value: Option<i32>,
}

/// Decode an `OwnerExclusivePlayerInfo.RoundInfos` blob.
///
/// # Wire layout
///
/// Standard UE RepLayout dynamic-array framing (see module docs).
/// Field handles: 40=RoundNumber(Int32), 41=StartOfRoundMoney(Int32),
/// 42=StartOfRoundLoadoutValue(Int32), 43=EndOfRoundMoney(Int32),
/// 44=EndOfRoundLoadoutValue(Int32).
///
/// # Arguments
///
/// * `reader` - A `BitReader` positioned at the start of the blob, with
///   `len_bits()` equal to the declared bit count.
pub fn decode_round_infos(reader: &mut BitReader<'_>) -> Result<Vec<PlayerRoundInfo>> {
    let count = read_array_count(reader)?;
    let mut results = Vec::new();

    while let Some(index) = read_element_index(reader, count)? {
        let mut round_number: Option<i32> = Option::None;
        let mut start_of_round_money: Option<i32> = Option::None;
        let mut start_of_round_loadout_value: Option<i32> = Option::None;
        let mut end_of_round_money: Option<i32> = Option::None;
        let mut end_of_round_loadout_value: Option<i32> = Option::None;

        for field_idx in 0..MAX_FIELDS_PER_ELEMENT {
            let Some((handle, bit_count)) = read_field_header(reader)? else {
                break;
            };
            if field_idx == MAX_FIELDS_PER_ELEMENT - 1 {
                return Err(StructBlobError::TooManyFields {
                    context: "RoundInfos",
                });
            }

            let mut sub = reader.sub_reader(u64::from(bit_count))?;
            match handle {
                40 => {
                    round_number = Some(sub.read_i32()?);
                }
                41 => {
                    start_of_round_money = Some(sub.read_i32()?);
                }
                42 => {
                    start_of_round_loadout_value = Some(sub.read_i32()?);
                }
                43 => {
                    end_of_round_money = Some(sub.read_i32()?);
                }
                44 => {
                    end_of_round_loadout_value = Some(sub.read_i32()?);
                }
                _ => {
                    return Err(StructBlobError::UnsupportedHandle {
                        handle,
                        context: "RoundInfos",
                    });
                }
            }
        }

        results.push(PlayerRoundInfo {
            index,
            round_number,
            start_of_round_money,
            start_of_round_loadout_value,
            end_of_round_money,
            end_of_round_loadout_value,
        });
    }

    ensure_consumed(reader)?;
    Ok(results)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use vrf_bitio::BitReader;

    // ── RoundResults tests ───────────────────────────────────────────────────

    /// Row 0 from replay 02d4d478, t=84942ms.
    /// C# output: [{RoundNumber:0, WinningTeam:"Red", WinningTeamRole:attacker, RoundResult:elimination}]
    #[test]
    fn round_results_row0_red_attacker_elimination() {
        let data =
            hex_to_bytes("0202bcc208000000a4cac800000000007c0d028c00c2800202c420250400000000");
        let mut r = BitReader::with_bit_len(&data, 264);
        let results = decode_round_results(&mut r).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].round_number, 0);
        assert_eq!(results[0].winning_team.as_deref(), Some("Red"));
        assert_eq!(results[0].winning_team_role, Some(AresTeamRole::Attacker));
        assert_eq!(results[0].round_result, Some(AresRoundOutcome::Elimination));
    }

    /// Row 4 from replay 02d4d478, t=580448ms.
    /// C# output: [{RoundNumber:4, WinningTeam:"Blue", WinningTeamRole:defender, RoundResult:time_expired}]
    #[test]
    fn round_results_row4_blue_defender_time_expired() {
        let data = hex_to_bytes("0a0abcd20a00000084d8eaca00000000007c0d048c300000");
        let mut r = BitReader::with_bit_len(&data, 192);
        let results = decode_round_results(&mut r).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].round_number, 4);
        assert_eq!(results[0].winning_team.as_deref(), Some("Blue"));
        assert_eq!(results[0].winning_team_role, Some(AresTeamRole::Defender));
        assert_eq!(results[0].round_result, Some(AresRoundOutcome::TimeExpired));
    }

    /// Row 6 from replay 02d4d478, t=796414ms.
    /// C# output: [{RoundNumber:6, WinningTeam:"Blue", WinningTeamRole:defender, RoundResult:defuse}]
    #[test]
    fn round_results_row6_blue_defender_defuse() {
        let data = hex_to_bytes("0e0ebcd20a00000084d8eaca00000000007c0d048c100000");
        let mut r = BitReader::with_bit_len(&data, 192);
        let results = decode_round_results(&mut r).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].round_number, 6);
        assert_eq!(results[0].winning_team.as_deref(), Some("Blue"));
        assert_eq!(results[0].winning_team_role, Some(AresTeamRole::Defender));
        assert_eq!(results[0].round_result, Some(AresRoundOutcome::Defuse));
    }

    /// Empty blob (0 bits) should return empty vec.
    #[test]
    fn round_results_empty() {
        let data = [];
        let mut r = BitReader::with_bit_len(&data, 0);
        let results = decode_round_results(&mut r).unwrap();
        assert!(results.is_empty());
    }

    // ── TeamEconomy tests ────────────────────────────────────────────────────

    /// Row 0 from replay 02d4d478, t=7ms. Initial spawn with ReplicationIds.
    /// C# output: [{Index:0, LV:0, ALV:0, RepId:272}, {Index:1, LV:0, ALV:0, RepId:274}]
    #[test]
    fn team_economy_row0_initial_spawn() {
        let data = hex_to_bytes(
            "0402722021047440000000007640000000000004722025047440000000007640000000000000",
        );
        let mut r = BitReader::with_bit_len(&data, 304);
        let results = decode_team_economy(&mut r).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].index, 0);
        assert_eq!(results[0].replication_id, Some(272));
        assert_eq!(results[0].loadout_value, Some(0));
        assert_eq!(results[0].average_loadout_value, Some(0));
        assert_eq!(results[1].index, 1);
        assert_eq!(results[1].replication_id, Some(274));
        assert_eq!(results[1].loadout_value, Some(0));
        assert_eq!(results[1].average_loadout_value, Some(0));
    }

    /// Row 1 from replay 02d4d478, t=62ms.
    /// C# output: [{Index:0, LV:4350, ALV:870, RepId:null}, {Index:1, LV:4150, ALV:830, RepId:null}]
    #[test]
    fn team_economy_row1_round_start() {
        let data = hex_to_bytes("04027440fe100000764066030000000474403610000076403e0300000000");
        let mut r = BitReader::with_bit_len(&data, 240);
        let results = decode_team_economy(&mut r).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].index, 0);
        assert_eq!(results[0].replication_id, None);
        assert_eq!(results[0].loadout_value, Some(4350));
        assert_eq!(results[0].average_loadout_value, Some(870));
        assert_eq!(results[1].index, 1);
        assert_eq!(results[1].replication_id, None);
        assert_eq!(results[1].loadout_value, Some(4150));
        assert_eq!(results[1].average_loadout_value, Some(830));
    }

    /// Row 2 from replay 02d4d478, t=92033ms.
    /// C# output: [{Index:0, LV:21200, ALV:4240}, {Index:1, LV:11600, ALV:2320}]
    #[test]
    fn team_economy_row2_midgame() {
        let data = hex_to_bytes("04027440d052000076409010000000047440502d00007640100900000000");
        let mut r = BitReader::with_bit_len(&data, 240);
        let results = decode_team_economy(&mut r).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].index, 0);
        assert_eq!(results[0].loadout_value, Some(21200));
        assert_eq!(results[0].average_loadout_value, Some(4240));
        assert_eq!(results[1].index, 1);
        assert_eq!(results[1].loadout_value, Some(11600));
        assert_eq!(results[1].average_loadout_value, Some(2320));
    }

    // ── RoundInfos tests ─────────────────────────────────────────────────────

    /// First RoundInfos row from replay 02d4d478, t=91927ms, actor 196.
    /// C# base64: "AgJSQAAAAABUQAAAAABWQAAAAABYQGwHAABaQAAAAAAAAA=="
    /// Decoded: [{Index:0, RN:0, SM:0, SL:0, EM:1900, EL:0}]
    #[test]
    fn round_infos_row0_end_of_round1() {
        let data =
            hex_to_bytes("020252400000000054400000000056400000000058406c0700005a40000000000000");
        let mut r = BitReader::with_bit_len(&data, 272);
        let results = decode_round_infos(&mut r).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].index, 0);
        assert_eq!(results[0].round_number, Some(0));
        assert_eq!(results[0].start_of_round_money, Some(0));
        assert_eq!(results[0].start_of_round_loadout_value, Some(0));
        assert_eq!(results[0].end_of_round_money, Some(1900));
        assert_eq!(results[0].end_of_round_loadout_value, Some(0));
    }

    /// Second RoundInfos row from replay 02d4d478, t=91927ms, actor 184.
    /// C# base64: "AgJSQAAAAABUQAAAAABWQAAAAABYQNAHAABaQMgAAAAAAA=="
    /// Decoded: [{Index:0, RN:0, SM:0, SL:0, EM:2000, EL:200}]
    #[test]
    fn round_infos_row1_different_player() {
        let data = base64_to_bytes("AgJSQAAAAABUQAAAAABWQAAAAABYQNAHAABaQMgAAAAAAA==");
        let mut r = BitReader::with_bit_len(&data, 272);
        let results = decode_round_infos(&mut r).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].index, 0);
        assert_eq!(results[0].round_number, Some(0));
        assert_eq!(results[0].start_of_round_money, Some(0));
        assert_eq!(results[0].start_of_round_loadout_value, Some(0));
        assert_eq!(results[0].end_of_round_money, Some(2000));
        assert_eq!(results[0].end_of_round_loadout_value, Some(200));
    }

    /// Third RoundInfos row from replay 02d4d478, t=91927ms, actor 240.
    /// C# base64: "AgJSQAAAAABUQAAAAABWQAAAAABYQDQIAABaQFgCAAAAAA=="
    /// Decoded: [{Index:0, RN:0, SM:0, SL:0, EM:2100, EL:600}]
    #[test]
    fn round_infos_row2_another_player() {
        let data = base64_to_bytes("AgJSQAAAAABUQAAAAABWQAAAAABYQDQIAABaQFgCAAAAAA==");
        let mut r = BitReader::with_bit_len(&data, 272);
        let results = decode_round_infos(&mut r).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].index, 0);
        assert_eq!(results[0].end_of_round_money, Some(2100));
        assert_eq!(results[0].end_of_round_loadout_value, Some(600));
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn hex_to_bytes(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn base64_to_bytes(s: &str) -> Vec<u8> {
        // Minimal base64 decoder for tests (standard alphabet, with padding).
        const TABLE: [u8; 128] = {
            let mut t = [255u8; 128];
            let mut i = 0u8;
            while i < 26 {
                t[(b'A' + i) as usize] = i;
                t[(b'a' + i) as usize] = i + 26;
                i += 1;
            }
            let mut d = 0u8;
            while d < 10 {
                t[(b'0' + d) as usize] = d + 52;
                d += 1;
            }
            t[b'+' as usize] = 62;
            t[b'/' as usize] = 63;
            t
        };

        let input: Vec<u8> = s.bytes().filter(|&b| b != b'=').collect();
        let mut out = Vec::with_capacity(input.len() * 3 / 4);
        let chunks = input.chunks(4);
        for chunk in chunks {
            let mut buf = [0u8; 4];
            for (i, &b) in chunk.iter().enumerate() {
                buf[i] = TABLE[b as usize];
            }
            out.push((buf[0] << 2) | (buf[1] >> 4));
            if chunk.len() > 2 {
                out.push((buf[1] << 4) | (buf[2] >> 2));
            }
            if chunk.len() > 3 {
                out.push((buf[2] << 6) | buf[3]);
            }
        }
        out
    }
}
