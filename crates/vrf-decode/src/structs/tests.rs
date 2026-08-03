//! Pinned rows from replay `02d4d478`, with the values the C# reference
//! produced for the same bytes.

use super::*;
use vrf_bitio::BitReader;

// -- RoundResults tests ---------------------------------------------------

/// Row 0 from replay 02d4d478, t=84942ms.
/// C# output: [{RoundNumber:0, WinningTeam:"Red", WinningTeamRole:attacker, RoundResult:elimination}]
#[test]
fn round_results_row0_red_attacker_elimination() {
    let data = hex_to_bytes("0202bcc208000000a4cac800000000007c0d028c00c2800202c420250400000000");
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

// -- TeamEconomy tests ----------------------------------------------------

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

// -- RoundInfos tests -----------------------------------------------------

/// First RoundInfos row from replay 02d4d478, t=91927ms, actor 196.
/// C# base64: "AgJSQAAAAABUQAAAAABWQAAAAABYQGwHAABaQAAAAAAAAA=="
/// Decoded: [{Index:0, RN:0, SM:0, SL:0, EM:1900, EL:0}]
#[test]
fn round_infos_row0_end_of_round1() {
    let data = hex_to_bytes("020252400000000054400000000056400000000058406c0700005a40000000000000");
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

// -- Helpers --------------------------------------------------------------

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
