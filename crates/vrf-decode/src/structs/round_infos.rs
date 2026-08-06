//! `OwnerExclusivePlayerInfo.RoundInfos` -- per-player per-round credit.

use vrf_bitio::BitReader;

use super::framing::{
    MAX_FIELDS_PER_ELEMENT, ensure_consumed, ensure_member_consumed, member_name, read_array_count,
    read_element_index, read_field_header,
};
use super::{Result, StructBlobError};

/// Names this blob in error messages.
const CONTEXT: &str = "RoundInfos";

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
/// Standard UE RepLayout dynamic-array framing (see module docs). Members are
/// selected by declared name. These five sit at 40..=44 in both builds
/// measured so far, which is luck rather than a guarantee -- 13.02 shifted
/// `RoundResults` by eight in the group next door, and this group simply had
/// no property removed above it.
///
/// # Arguments
///
/// * `reader` - A `BitReader` positioned at the start of the blob, with
///   `len_bits()` equal to the declared bit count.
/// * `declared` - The enclosing group's net field export names indexed by
///   handle. See [`super::round_results::decode_round_results`] on why an
///   empty slice is an error rather than a fallback.
pub fn decode_round_infos(
    reader: &mut BitReader<'_>,
    declared: &[Option<&str>],
) -> Result<Vec<PlayerRoundInfo>> {
    let count = read_array_count(reader)?;
    let mut results = Vec::new();

    while let Some(index) = read_element_index(reader, count)? {
        let mut round_number: Option<i32> = None;
        let mut start_of_round_money: Option<i32> = None;
        let mut start_of_round_loadout_value: Option<i32> = None;
        let mut end_of_round_money: Option<i32> = None;
        let mut end_of_round_loadout_value: Option<i32> = None;

        for field_idx in 0..=MAX_FIELDS_PER_ELEMENT {
            let Some((handle, bit_count)) = read_field_header(reader)? else {
                break;
            };
            if field_idx == MAX_FIELDS_PER_ELEMENT {
                return Err(StructBlobError::TooManyFields { context: CONTEXT });
            }

            let mut sub = reader.sub_reader(u64::from(bit_count))?;
            let member = member_name(declared, handle, CONTEXT)?;
            match member {
                "RoundNumber" => round_number = Some(sub.read_i32()?),
                "StartOfRoundMoney" => start_of_round_money = Some(sub.read_i32()?),
                "StartOfRoundLoadoutValue" => {
                    start_of_round_loadout_value = Some(sub.read_i32()?);
                }
                "EndOfRoundMoney" => end_of_round_money = Some(sub.read_i32()?),
                "EndOfRoundLoadoutValue" => end_of_round_loadout_value = Some(sub.read_i32()?),
                name => {
                    return Err(StructBlobError::UnsupportedMember {
                        name: name.to_owned(),
                        handle,
                        context: CONTEXT,
                    });
                }
            }
            // Every member here is a fixed 32-bit Int32, so its window is fully
            // spoken for. A wider one means the field is not the Int32 this
            // decoder believes it is, and the half it read is not a value.
            ensure_member_consumed(&sub, member, handle, bit_count, CONTEXT)?;
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
