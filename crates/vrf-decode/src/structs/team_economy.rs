//! `BombGameState.TeamEconomy` -- team loadout value per round.

use vrf_bitio::BitReader;

use super::framing::{
    MAX_FIELDS_PER_ELEMENT, ensure_consumed, read_array_count, read_element_index,
    read_field_header,
};
use super::{Result, StructBlobError};

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
/// # Why this one still matches on handle numbers
///
/// Its two siblings select members by the name the replay declares, because
/// handle numbers move between builds. This decoder cannot: the replay
/// declares handle 56 as `"241"` -- a hardcoded FName index, not a name -- so
/// `ReplicationId` has nothing to match on. Nor is there anything to
/// generalise toward. `TeamEconomy` does not exist in build 13.02: it and
/// `TeamComponents` were replaced by `TeamStates`, and the values moved into a
/// separately replicated `/Script/ShooterGame.BaseTeamState` actor that
/// nothing here decodes yet. So this stays a 13.01 decoder pinned to 13.01
/// numbers, and the struct-blob failure counter is what reports it if those
/// numbers ever move under it.
///
/// # Arguments
///
/// * `reader` - A `BitReader` positioned at the start of the blob, with
///   `len_bits()` equal to the declared bit count.
pub fn decode_team_economy(reader: &mut BitReader<'_>) -> Result<Vec<TeamEconomyUpdate>> {
    let count = read_array_count(reader)?;
    let mut results = Vec::new();

    while let Some(index) = read_element_index(reader, count)? {
        let mut replication_id: Option<u32> = None;
        let mut loadout_value: Option<i32> = None;
        let mut average_loadout_value: Option<i32> = None;

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
                56 => replication_id = Some(sub.read_int_packed()?),
                57 => loadout_value = Some(sub.read_i32()?),
                58 => average_loadout_value = Some(sub.read_i32()?),
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
