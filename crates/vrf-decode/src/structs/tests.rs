//! Pinned rows from replay `02d4d478`, with the values an independent parser
//! produced for the same bytes.

use super::*;
use crate::test_bits::hex;
use vrf_bitio::BitReader;
use vrf_testkit::{BitWrite, BitWriter};

// -- Declarations ---------------------------------------------------------
//
// Every test supplies the declaration its bytes were recorded under: the same
// members sit at different handles on 13.01 and 13.02.

/// `BombGameState_C` on build 13.01: RoundResults members at 93..=96.
fn bomb_game_state_1301() -> Vec<Option<&'static str>> {
    let mut d = vec![None; 98];
    d[92] = Some("RoundResults");
    d[93] = Some("WinningTeam");
    d[94] = Some("WinningTeamRole");
    d[95] = Some("RoundResult");
    d[96] = Some("EliminatedTeams");
    d[97] = Some("EliminatedTeams");
    // Handle 50 is the match-winner scalar, NOT a RoundResults member; it
    // shares the name, so a search by name could land here.
    d[50] = Some("WinningTeam");
    d
}

/// `BombGameState_C` on build 13.02: the same members, eight handles lower.
///
/// 13.02 dropped `TeamEconomy` and `TeamComponents` and added `TeamStates`.
fn bomb_game_state_1302() -> Vec<Option<&'static str>> {
    let mut d = vec![None; 86];
    d[80] = Some("RoundResults");
    d[81] = Some("WinningTeam");
    d[82] = Some("WinningTeamRole");
    d[83] = Some("RoundResult");
    d[84] = Some("EliminatedTeams");
    d[85] = Some("EliminatedTeams");
    d[50] = Some("WinningTeam");
    d[52] = Some("TeamStates");
    d[53] = Some("TeamStates");
    d
}

/// `BombGameState_C` on build 13.09: `WinningTeamMode` and the nested
/// `TeamResults` (86..=89 its members) join the RoundResults members.
fn bomb_game_state_1309() -> Vec<Option<&'static str>> {
    let mut d = vec![None; 94];
    d[81] = Some("RoundResults");
    d[82] = Some("WinningTeamMode");
    d[83] = Some("WinningTeam");
    d[84] = Some("WinningTeamRole");
    d[85] = Some("TeamResults");
    d[86] = Some("TeamName");
    d[87] = Some("TeamNumber");
    d[88] = Some("TeamRole");
    d[89] = Some("TeamOutcome");
    d[91] = Some("RoundResult");
    d[92] = Some("EliminatedTeams");
    d[93] = Some("EliminatedTeams");
    d
}

/// `OwnerExclusivePlayerInfo`, identical on both builds measured.
fn owner_exclusive_player_info() -> Vec<Option<&'static str>> {
    let mut d = vec![None; 45];
    d[39] = Some("RoundInfos");
    d[40] = Some("RoundNumber");
    d[41] = Some("StartOfRoundMoney");
    d[42] = Some("StartOfRoundLoadoutValue");
    d[43] = Some("EndOfRoundMoney");
    d[44] = Some("EndOfRoundLoadoutValue");
    d
}

/// A reader over a whole pinned blob: every one is a whole number of bytes.
fn reader(data: &[u8]) -> BitReader<'_> {
    BitReader::with_bit_len(data, data.len() as u64 * 8).unwrap()
}

// -- RoundResults ---------------------------------------------------------

/// Round 0 of `02d4d478` on 13.01 (t=84942ms) and of `f1110ea5` on 13.02
/// (t=72684ms), where the same members sit at 81..=84.
const ROUND_RESULTS_1301: &str =
    "0202bcc208000000a4cac800000000007c0d028c00c2800202c420250400000000";
const ROUND_RESULTS_1302: &str =
    "0202a4d20a00000084d8eaca00000000004c0d848a00aa800202ac20f50200000000";

#[test]
fn round_results_rows_decode_to_the_pinned_values() {
    use AresRoundOutcome::{Defuse, Elimination, TimeExpired};
    use AresTeamRole::{Attacker, Defender};
    let (d1301, d1302) = (bomb_game_state_1301(), bomb_game_state_1302());
    // Rounds 4 and 6 of 02d4d478, t=580448ms and t=796414ms.
    let row4 = "0a0abcd20a00000084d8eaca00000000007c0d048c300000";
    let row6 = "0e0ebcd20a00000084d8eaca00000000007c0d048c100000";
    for (digits, declared, (round_number, team, role, outcome)) in [
        (
            ROUND_RESULTS_1301,
            &d1301,
            (0, "Red", Attacker, Elimination),
        ),
        (row4, &d1301, (4, "Blue", Defender, TimeExpired)),
        (row6, &d1301, (6, "Blue", Defender, Defuse)),
        (
            ROUND_RESULTS_1302,
            &d1302,
            (0, "Blue", Defender, Elimination),
        ),
    ] {
        let data = hex(digits);
        let results = decode_round_results(&mut reader(&data), declared).unwrap();
        let want = RoundResult {
            round_number,
            winning_team: Some(team.to_owned()),
            winning_team_role: Some(role),
            round_result: Some(outcome),
            winning_team_mode: None,
            team_results: None,
        };
        assert_eq!(results, [want], "{digits}");
    }
}

/// Round 0 of a recorded 13.09 RoundResults update: 761 bits, every member
/// present, TeamResults holding Blue and Red.
#[test]
fn round_results_1309_decodes_nested_team_results_and_rejects_renamed_member() {
    let data = hex(
        "0202a60641955600000020c456570600000000a06a20d646030201576905000000426c7565         0000000000b04001000000b206a235080081ab3002000000a9323200000000005820010000         005983d01a080000170174e104047821200100000000",
    );
    let mut reader = BitReader::with_bit_len(&data, 761).unwrap();
    let results = decode_round_results(&mut reader, &bomb_game_state_1309()).unwrap();
    let want = RoundResult {
        round_number: 0,
        winning_team: Some("Blue".to_owned()),
        winning_team_role: Some(AresTeamRole::Defender),
        round_result: Some(AresRoundOutcome::Elimination),
        winning_team_mode: Some(1),
        team_results: Some(vec![
            TeamRoundResult {
                index: 0,
                team_name: Some("Blue".to_owned()),
                team_number: Some(1),
                team_role: Some(AresTeamRole::Defender),
                team_outcome: Some(1),
            },
            TeamRoundResult {
                index: 1,
                team_name: Some("Red".to_owned()),
                team_number: Some(2),
                team_role: Some(AresTeamRole::Attacker),
                team_outcome: Some(2),
            },
        ]),
    };
    assert_eq!(results, [want]);
    assert_eq!(reader.bits_remaining(), 0);

    // A name drift inside the nested array must not silently emit plausible
    // team outcomes. The raw parent survives the error in the export sink.
    let mut renamed = bomb_game_state_1309();
    renamed[88] = Some("UnexpectedTeamRole");
    let mut reader = BitReader::with_bit_len(&data, 761).unwrap();
    assert!(matches!(
        decode_round_results(&mut reader, &renamed),
        Err(StructBlobError::UnsupportedMember {
            handle: 88,
            context: "TeamResults",
            ..
        })
    ));
}

/// Bytes under the other build's declaration, or under none, FAIL by the
/// first member's handle rather than return an empty vector: that is how a
/// whole build's missing match scores looked like a clean export. The decoder
/// is keyed on the declaration, and there is no safe fallback set of handle
/// numbers to guess with.
#[test]
fn round_results_under_another_declaration_name_the_undeclared_handle() {
    for (digits, declared, handle) in [
        (ROUND_RESULTS_1302, bomb_game_state_1301(), 81),
        (ROUND_RESULTS_1301, bomb_game_state_1302(), 93),
        (ROUND_RESULTS_1301, Vec::new(), 93),
    ] {
        let data = hex(digits);
        let err = decode_round_results(&mut reader(&data), &declared).unwrap_err();
        assert!(
            matches!(
                err,
                StructBlobError::UndeclaredHandle { handle: h, context: "RoundResults" } if h == handle
            ),
            "expected an undeclared-handle error naming handle {handle}, got {err:?}"
        );
    }
}

/// A handle that IS declared, under a name with no arm, names itself in the
/// error. This is the shape a renamed or added member takes.
#[test]
fn round_results_unknown_member_name_is_reported_by_name() {
    let data = hex(ROUND_RESULTS_1301);
    let mut declared = bomb_game_state_1301();
    declared[93] = Some("WinningTeamV2");
    let err = decode_round_results(&mut reader(&data), &declared).unwrap_err();
    match err {
        StructBlobError::UnsupportedMember { name, handle, .. } => {
            assert_eq!(name, "WinningTeamV2");
            assert_eq!(handle, 93);
        }
        other => panic!("expected UnsupportedMember, got {other:?}"),
    }
}

/// A 0-bit blob is an error for all three decoders: not even the element
/// count is there. (The export path never hands a decoder zero bits.)
#[test]
fn a_zero_bit_blob_is_an_error_for_every_decoder() {
    let eof = |err: StructBlobError| {
        matches!(
            err,
            StructBlobError::BitIo(vrf_bitio::BitError::Eof {
                position: 0,
                length: 0,
                requested: 8
            })
        )
    };
    let err = decode_round_results(&mut reader(&[]), &bomb_game_state_1301()).unwrap_err();
    assert!(eof(err.clone()), "RoundResults: {err:?}");
    let err = decode_round_infos(&mut reader(&[]), &owner_exclusive_player_info()).unwrap_err();
    assert!(eof(err.clone()), "RoundInfos: {err:?}");
    let err = decode_team_economy(&mut reader(&[])).unwrap_err();
    assert!(eof(err.clone()), "TeamEconomy: {err:?}");
}

// -- TeamEconomy and RoundInfos -------------------------------------------

/// `02d4d478` at t=7ms (the initial spawn, with replication IDs), t=62ms and
/// t=92033ms; per element: replication ID, loadout value, average loadout.
#[test]
fn team_economy_rows_decode_to_the_pinned_values() {
    let spawn = "0402722021047440000000007640000000000004722025047440000000007640000000000000";
    let start = "04027440fe100000764066030000000474403610000076403e0300000000";
    let midgame = "04027440d052000076409010000000047440502d00007640100900000000";
    for (digits, elements) in [
        (spawn, [(Some(272), 0, 0), (Some(274), 0, 0)]),
        (start, [(None, 4350, 870), (None, 4150, 830)]),
        (midgame, [(None, 21200, 4240), (None, 11600, 2320)]),
    ] {
        let want: Vec<TeamEconomyUpdate> = (0..)
            .zip(elements)
            .map(
                |(index, (replication_id, loadout, average))| TeamEconomyUpdate {
                    index,
                    replication_id,
                    loadout_value: Some(loadout),
                    average_loadout_value: Some(average),
                },
            )
            .collect();
        let data = hex(digits);
        assert_eq!(
            decode_team_economy(&mut reader(&data)).unwrap(),
            want,
            "{digits}"
        );
    }
}

/// Three players' first RoundInfos row, all at t=91927ms (actors 196, 184 and
/// 240): round 0, nothing at its start, then end-of-round money and loadout.
const ROUND_INFOS: [(&str, i32, i32); 3] = [
    (
        "020252400000000054400000000056400000000058406c0700005a40000000000000",
        1900,
        0,
    ),
    (
        "02025240000000005440000000005640000000005840d00700005a40c80000000000",
        2000,
        200,
    ),
    (
        "02025240000000005440000000005640000000005840340800005a40580200000000",
        2100,
        600,
    ),
];

#[test]
fn round_infos_rows_decode_to_the_pinned_values() {
    for (digits, end_money, end_loadout) in ROUND_INFOS {
        let want = PlayerRoundInfo {
            index: 0,
            round_number: Some(0),
            start_of_round_money: Some(0),
            start_of_round_loadout_value: Some(0),
            end_of_round_money: Some(end_money),
            end_of_round_loadout_value: Some(end_loadout),
        };
        let data = hex(digits);
        let results = decode_round_infos(&mut reader(&data), &owner_exclusive_player_info());
        assert_eq!(results.unwrap(), [want], "{digits}");
    }
}

// -- The fields-per-element cap -------------------------------------------

/// The framing module's `MAX_FIELDS_PER_ELEMENT`, repeated rather than
/// imported on purpose: a changed cap must fail the boundary tests.
const MAX_FIELDS_PER_ELEMENT: u32 = 8;

/// A `RoundInfos` blob whose single, correctly terminated element carries `n`
/// 32-bit zero `RoundNumber` fields (handle 40).
fn round_infos_with_n_fields(n: u32) -> (Vec<u8>, u64) {
    let mut bits = BitWriter::new();
    bits.int_packed(1).int_packed(1); // one element, index 0
    for _ in 0..n {
        bits.int_packed(41).int_packed(32).u32(0);
    }
    bits.int_packed(0).int_packed(0); // field and element terminators
    let (data, bit_len) = bits.finish();
    (data, u64::from(bit_len))
}

/// MAX fields in one element must parse: the guard rejects only the MAX+1-th.
#[test]
fn round_infos_accepts_max_fields_per_element() {
    let (bytes, bits) = round_infos_with_n_fields(MAX_FIELDS_PER_ELEMENT);
    let mut r = BitReader::with_bit_len(&bytes, bits).unwrap();
    let results = decode_round_infos(&mut r, &owner_exclusive_player_info()).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].round_number, Some(0));
}

/// The MAX+1-th field is rejected as `TooManyFields`.
#[test]
fn round_infos_rejects_one_more_than_max_fields() {
    let (bytes, bits) = round_infos_with_n_fields(MAX_FIELDS_PER_ELEMENT + 1);
    let mut r = BitReader::with_bit_len(&bytes, bits).unwrap();
    let err = decode_round_infos(&mut r, &owner_exclusive_player_info()).unwrap_err();
    assert!(
        matches!(
            err,
            StructBlobError::TooManyFields {
                context: "RoundInfos"
            }
        ),
        "expected TooManyFields, got {err:?}"
    );
}

/// A bit left after the array terminator is `NotFullyConsumed`, not dropped;
/// the same blob without it decodes.
#[test]
fn a_bit_after_the_array_terminator_is_not_fully_consumed() {
    for extra in [false, true] {
        let mut bits = BitWriter::new();
        bits.int_packed(1).int_packed(1); // one element, index 0
        bits.int_packed(41).int_packed(32).u32(0);
        bits.int_packed(0).int_packed(0); // field and element terminators
        if extra {
            bits.bit(false);
        }
        let (data, bit_len) = bits.finish();
        let mut r = BitReader::with_bit_len(&data, u64::from(bit_len)).unwrap();
        let result = decode_round_infos(&mut r, &owner_exclusive_player_info());
        if extra {
            let err = result.unwrap_err();
            assert!(
                matches!(err, StructBlobError::NotFullyConsumed { remaining: 1 }),
                "expected NotFullyConsumed, got {err:?}"
            );
        } else {
            assert_eq!(result.unwrap().len(), 1);
        }
    }
}

/// Each shared framing failure keeps its name and fields (cap 128).
#[test]
fn framing_failures_reach_the_caller_as_their_own_variant() {
    for (packed, expected) in [
        (&[129][..], "ArrayCountTooLarge { count: 129, max: 128 }"),
        (&[1, 4], "IndexOutOfBounds { index: 3, count: 1 }"),
        (
            &[1, 1, 41, 32],
            "PayloadTooLarge { bits: 32, remaining: 0 }",
        ),
    ] {
        let mut bits = BitWriter::new();
        for &v in packed {
            bits.int_packed(v);
        }
        let (data, bit_len) = bits.finish();
        let mut r = BitReader::with_bit_len(&data, u64::from(bit_len)).unwrap();
        let err = decode_round_infos(&mut r, &owner_exclusive_player_info()).unwrap_err();
        assert_eq!(format!("{err:?}"), expected);
    }
}

// -- Unknown enum values --------------------------------------------------

/// One element carrying a single member at `handle`, whose payload window is
/// `payload`.
fn one_member_bits(handle: u32, payload: &[bool]) -> (Vec<u8>, u64) {
    let mut bits = BitWriter::new();
    bits.int_packed(1).int_packed(1); // one element, index 0
    bits.int_packed(handle + 1).int_packed(payload.len() as u32);
    bits.extend_bits(payload);
    bits.int_packed(0).int_packed(0); // end of element, end of array
    let (data, bit_len) = bits.finish();
    (data, u64::from(bit_len))
}

/// [`one_member_bits`] with a `width`-bit window holding `value` (zero past its
/// 32 bits).
fn one_member(handle: u32, value: u32, width: u32) -> (Vec<u8>, u64) {
    one_member_bits(handle, BitWriter::new().bits(u64::from(value), width))
}

/// An `AresTeamRole` value outside the declared variants is `UnknownEnumValue`,
/// not an absent field.
#[test]
fn round_results_unknown_team_role_is_an_error_not_an_absent_field() {
    // Handle 94 is WinningTeamRole on 13.01. Role 7 is past RoleCount (5).
    let (data, bit_len) = one_member(94, 7, 3);
    let mut r = BitReader::with_bit_len(&data, bit_len).unwrap();
    let err = decode_round_results(&mut r, &bomb_game_state_1301())
        .expect_err("an unknown role must not decode as an absent field");
    assert!(
        matches!(
            err,
            StructBlobError::UnknownEnumValue {
                enum_name: "AresTeamRole",
                value: 7,
                ..
            }
        ),
        "got {err:?}"
    );
}

/// The same for `AresRoundOutcome`, whose declared range is 0..=7, so an
/// unknown value needs a four-bit payload.
#[test]
fn round_results_unknown_outcome_is_an_error_not_an_absent_field() {
    // Handle 95 is RoundResult on 13.01. Outcome 8 is past Invalid (7).
    let (data, bit_len) = one_member(95, 8, 4);
    let mut r = BitReader::with_bit_len(&data, bit_len).unwrap();
    let err = decode_round_results(&mut r, &bomb_game_state_1301())
        .expect_err("an unknown outcome must not decode as an absent field");
    assert!(
        matches!(
            err,
            StructBlobError::UnknownEnumValue {
                enum_name: "AresRoundOutcome",
                value: 8,
                ..
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn round_results_zero_width_enum_is_an_error_not_an_absent_field() {
    let (data, bit_len) = one_member(94, 0, 0);
    let mut r = BitReader::with_bit_len(&data, bit_len).unwrap();
    let err = decode_round_results(&mut r, &bomb_game_state_1301())
        .expect_err("a declared enum field with no payload cannot become absent");
    assert!(
        matches!(
            err,
            StructBlobError::InvalidEnumWidth {
                bits: 0,
                context: "RoundResults",
                ..
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn struct_fname_rejects_a_negative_instance_number() {
    // WinningTeam (handle 93 on 13.01) as an inline "Blue" with instance -1.
    let (data, bit_len) = one_member_bits(93, BitWriter::new().bit(false).fstring("Blue").i32(-1));
    let mut reader = BitReader::with_bit_len(&data, bit_len).unwrap();
    let err = decode_round_results(&mut reader, &bomb_game_state_1301())
        .expect_err("the struct decoder must propagate invalid FName numbers");
    assert!(
        matches!(
            err,
            StructBlobError::Decode(crate::DecodeError::InvalidFNameNumber { number: -1 })
        ),
        "got {err:?}"
    );
}

// -- Field windows the member did not consume -----------------------------

/// A member that reads less than its declared window fails
/// (`MemberNotFullyConsumed`) instead of exporting the part it read: the
/// 64-bit `EndOfRoundMoney` whose first 32 bits say 1900.
#[test]
fn round_infos_member_that_underreads_its_window_is_an_error() {
    // Handle 43 is EndOfRoundMoney; 64 declared bits against a 32-bit Int32.
    let (data, bit_len) = one_member(43, 1900, 64);
    let mut r = BitReader::with_bit_len(&data, bit_len).unwrap();
    let err = decode_round_infos(&mut r, &owner_exclusive_player_info())
        .expect_err("half-read money must not export as a value");
    match &err {
        StructBlobError::MemberNotFullyConsumed {
            name,
            declared,
            remaining,
            ..
        } => {
            assert_eq!(name, "EndOfRoundMoney");
            assert_eq!(*declared, 64);
            assert_eq!(*remaining, 32);
        }
        other => panic!("expected MemberNotFullyConsumed, got {other:?}"),
    }
}

/// TeamEconomy declares its replication ID as hardcoded FName `241` and
/// reports a leftover under `ReplicationId`, the name that reaches the
/// manifest's struct-blob error. Handles as 12.05 declares them (53).
#[test]
fn team_economy_replication_id_that_underreads_is_reported_by_its_label() {
    let mut names = [None; 56];
    names[53] = Some("241");
    // 16 declared bits; the one-byte IntPacked 2 reads 8 of them.
    let (data, bit_len) = one_member(53, 4, 16);
    let mut r = BitReader::with_bit_len(&data, bit_len).unwrap();
    match decode_team_economy_declared(&mut r, &names).unwrap_err() {
        StructBlobError::MemberNotFullyConsumed {
            name,
            handle,
            declared,
            remaining,
            ..
        } => assert_eq!(
            (name.as_str(), handle, declared, remaining),
            ("ReplicationId", 53, 16, 8)
        ),
        other => panic!("expected MemberNotFullyConsumed, got {other:?}"),
    }
}
