"""Independent native raw/phase-byte oracle for a freshly executed Rust probe.

Inputs are local aggregate output roots; detailed GUID-containing rows are
retained locally, never included in the committed aggregate receipt.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys

import pyarrow.compute as pc
import pyarrow.parquet as pq


def file_sha256(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--probe', type=Path, required=True)
    ap.add_argument('--inputs', type=Path, required=True)
    ap.add_argument('--exports', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--source', type=Path, required=True)
    args = ap.parse_args()
    sys.path.insert(0, str(args.source / 'tools'))
    from extract_kill_ledger import validate_event_payload
    from extract_rounds import SET_PHASE, PHASE_COLUMNS
    supported = {
        'roundStarted': (1, 2, 'EReplayEventGroup::RoundStart'),
        'characterDeath': (2, 8, 'EReplayEventGroup::CharacterDeath'),
        'spikePlanted': (0, 4, 'EReplayEventGroup::SpikePlanted'),
        'spikeDefused': (0, 5, 'EReplayEventGroup::SpikeDefused'),
        'spikeExploded': (0, 6, 'EReplayEventGroup::SpikeExploded'),
    }
    args.out.mkdir(parents=True, exist_ok=True)
    reports = []
    for digest, path in json.loads(args.inputs.read_text()).items():
        replay = Path(path)
        assert file_sha256(replay) == digest
        run = subprocess.run([str(args.probe), str(replay)], text=True, capture_output=True)
        (args.out / (digest + '.probe.tsv')).write_text(run.stdout)
        (args.out / (digest + '.stderr.txt')).write_text(run.stderr)
        assert run.returncode == 0, (digest, run.returncode, run.stderr[-1000:])
        actual = Counter(x for x in run.stdout.splitlines() if x.startswith(('N\t','P\t')))
        actual_ignored = {x.split('\t')[1]: int(x.split('\t')[2]) for x in run.stdout.splitlines() if x.startswith('U\t')}
        expected = Counter()
        expected_native_rows = []
        expected_phase_rows = []
        groups = Counter()
        phases = Counter()
        ignored = Counter()
        export = args.exports / digest / 'export'
        events = pq.read_table(export / 'events.parquet').to_pylist()
        for row in events:
            group = row['group']
            if group not in supported:
                ignored[group] += 1
                continue
            count, tag, name = supported[group]
            assert validate_event_payload(row, count, tag, name) is None, (digest, group)
            raw = row['raw_payload']
            words = [struct.unpack_from('<I', raw, 4 + 4*i)[0] for i in range(count)]
            bits = struct.unpack('<I', raw[-4:])[0]
            words += ['null'] * (2 - count)
            line = f"N\t{group}\t{row['time1']}\t{row['time2']}\t{bits}\t{words[0]}\t{words[1]}"
            expected[line] += 1
            expected_native_rows.append(line)
            groups[group] += 1
        with pq.ParquetFile(export / 'fields.parquet', pre_buffer=False) as pf:
            columns = ['field_name','time_ms','packet_id','channel_index','actor_net_guid','object_net_guid',
                       'group_path','compatible_checksum','value_i64','value_f64','value_bool','value_str','bit_count','raw_bits']
            for batch in pf.iter_batches(columns=columns):
                selected = batch.filter(pc.equal(pc.cast(batch['field_name'], 'string'), SET_PHASE))
                for row in selected.to_pylist():
                    assert row['group_path'] == '/Game/GameModes/Bomb/BombGameState.BombGameState_C_ClassNetCache'
                    assert row['compatible_checksum'] == 3954716686 and row['object_net_guid'] is None
                    assert row['bit_count'] == 8 and row['raw_bits'] is not None and len(row['raw_bits']) == 1
                    value = row['raw_bits'][0]
                    assert value == row['value_i64'] and all(row[c] is None for c in ['value_f64','value_bool','value_str'])
                    assert value in PHASE_COLUMNS
                    line = f"P\t{row['time_ms']}\t{row['packet_id']}\t{row['channel_index']}\t{row['actor_net_guid']}\t{value}"
                    expected[line] += 1
                    expected_phase_rows.append(line)
                    phases[value] += 1
        assert expected == actual, (digest, len(expected-actual), len(actual-expected))
        assert [x for x in run.stdout.splitlines() if x.startswith('N\t')] == expected_native_rows, (digest, 'native order')
        assert [x for x in run.stdout.splitlines() if x.startswith('P\t')] == expected_phase_rows, (digest, 'phase order')
        assert dict(ignored) == actual_ignored, (digest, ignored, actual_ignored)
        summary = [x for x in run.stdout.splitlines() if x.startswith('S\t')]
        assert len(summary) == 1 and int(summary[0].split('\t')[1]) == len(events)
        reports.append(dict(replay_sha256=digest, build=replay.parent.name,
            native_counts=dict(groups), phase_counts=dict(phases),
            compared_observations=sum(expected.values()), unknown_native_counts=dict(ignored),
            checkpoints_skipped=int(summary[0].split('\t')[2]),
            mismatches=0, probe_output_sha256=hashlib.sha256(run.stdout.encode()).hexdigest()))
    receipt = dict(schema_version=1, purpose='fresh Rust optional gameplay projection vs existing independent Python raw validator',
        oracle_exports='Phase A final public exports: source 16345d94c2a35e50aeb1c1a98012e19cbd38b448, tree ed7a5a7afe39231a9624d5deebb1eed2e2977acd; 134/135 recorded runtime files match, only lib.rs adds the optional module',
        replay_count=len(reports), reports=reports, total_observations=sum(r['compared_observations'] for r in reports),
        mismatches=0, probe_sha256=file_sha256(args.probe))
    (args.out/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
    print(json.dumps(receipt,indent=2))


if __name__ == '__main__':
    main()
