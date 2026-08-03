//! `OwnerExclusivePlayerInfo.RoundInfos` -- per-player per-round credit.

use vrf_bitio::BitReader;

use super::framing::{
    MAX_FIELDS_PER_ELEMENT, ensure_consumed, read_array_count, read_element_index,
    read_field_header,
};
use super::{Result, StructBlobError};

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
        let mut round_number: Option<i32> = None;
        let mut start_of_round_money: Option<i32> = None;
        let mut start_of_round_loadout_value: Option<i32> = None;
        let mut end_of_round_money: Option<i32> = None;
        let mut end_of_round_loadout_value: Option<i32> = None;

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
                40 => round_number = Some(sub.read_i32()?),
                41 => start_of_round_money = Some(sub.read_i32()?),
                42 => start_of_round_loadout_value = Some(sub.read_i32()?),
                43 => end_of_round_money = Some(sub.read_i32()?),
                44 => end_of_round_loadout_value = Some(sub.read_i32()?),
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
