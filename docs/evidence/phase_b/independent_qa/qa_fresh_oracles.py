"""Independent Phase B functional checks on fresh combined-source artifacts.

No benchmark or resource measurement is performed. Detailed output stays
local; receipts contain hashes, counts and explicit comparison boundaries.
"""
import argparse
from collections import Counter
import gzip
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys

if not __debug__:
    raise RuntimeError('This verification harness requires Python assertions; remove -O/PYTHONOPTIMIZE.')

parser = argparse.ArgumentParser()
for name in ('source', 'exe', 'gameplay-probe', 'movement-probe', 'corpus', 'reference-root', 'output'):
    parser.add_argument('--' + name, type=Path, required=True)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=False)
sys.path.insert(0, str(args.source / 'tools'))
import pyarrow.compute as pc
import pyarrow.parquet as pq
import summary_counters
from extract_kill_ledger import validate_event_payload


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def canonical(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()).hexdigest()


def save(name, value):
    (args.output / name).write_text(json.dumps(value, indent=2) + '\n')


source_commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=args.source, text=True).strip()
binaries = {key: sha(path) for key, path in [('cli', args.exe), ('gameplay_probe', args.gameplay_probe), ('movement_probe', args.movement_probe)]}
steps = []


def run(name, command):
    log = args.output / (name + '.log')
    with log.open('w') as stream:
        code = subprocess.run(command, cwd=args.source, stdout=stream, stderr=subprocess.STDOUT).returncode
    steps.append({'name': name, 'exit_code': code, 'log_sha256': sha(log)})
    save('steps.json', steps)
    print(name + ': exit ' + str(code), flush=True)
    if code:
        raise RuntimeError(name + ' failed; inspect the local log')


audit = args.output / 'corpus-work'
baseline = json.loads((args.source / 'docs/evidence/phase-b-cli-public-parity.json').read_text())
expected = {row['input']['sha256']: row for row in baseline['runs']}
discovered = [path.resolve() for path in args.corpus.rglob('*')
              if path.is_file() and path.suffix.casefold() in ('.vrf', '.replay')]
inventory = {sha(path): path for path in discovered}
assert len(inventory) == 6 and set(inventory) == set(expected)
# The repository's generic corpus scanner intentionally accepts .vrf only.
# Public full-match fixtures use .replay; local read-only aliases preserve
# their actual bytes and let the common auditor validate all six inputs.
aliases = args.output / 'corpus-aliases'
aliases.mkdir()
for digest, replay in inventory.items():
    (aliases / (digest + '.vrf')).symlink_to(replay)
run('public-corpus', [sys.executable, '-W', 'error', str(args.source / 'tools/verify_build_corpus.py'),
    '--exe', str(args.exe), '--corpus', str(aliases), '--work-dir', str(audit),
    '--output', str(args.output / 'public-corpus.json'), '--jobs', '1'])
run('all-typed-identities', [sys.executable, '-W', 'error', str(args.source / 'tools/validate_type_evidence.py'),
    str(audit), str(args.source / 'tools/fixtures/public_fixture_type_evidence.json'),
    '--compare-typed', '--output', str(args.output / 'typed-evidence.json')])
inputs = {digest: Path(path) for digest, path in json.loads((audit / 'inputs.json').read_text()).items()}
assert len(inputs) == 6 and set(inputs) == set(inventory)
parity = []
gameplay = []
supported = {
    'roundStarted': (1, 2, 'EReplayEventGroup::RoundStart'),
    'characterDeath': (2, 8, 'EReplayEventGroup::CharacterDeath'),
    'spikePlanted': (0, 4, 'EReplayEventGroup::SpikePlanted'),
    'spikeDefused': (0, 5, 'EReplayEventGroup::SpikeDefused'),
    'spikeExploded': (0, 6, 'EReplayEventGroup::SpikeExploded'),
}
for digest, replay in inputs.items():
    export = audit / digest / 'export'
    previous = expected[digest]['output']
    tables = {p.name: {'bytes': p.stat().st_size, 'sha256': sha(p)} for p in sorted(export.glob('*.parquet'))}
    manifest = json.loads((export / 'manifest.json').read_text())
    manifest_hash = canonical({key: value for key, value in manifest.items() if key not in ('source_file', 'elapsed_ms')})
    counters = summary_counters.read((audit / digest / 'export.log').read_text(), summary_counters.WHERE)
    assert len(tables) == 13 and tables == previous['tables'], digest
    assert manifest_hash == previous['manifest_sha256'], digest
    assert counters == previous['counters'], digest
    parity.append({'build': replay.parent.name, 'input_sha256': digest,
                   'parquet_tables': len(tables), 'exact_parquet_bytes_equal': True,
                   'all_nonvolatile_manifest_values_equal': True, 'all_summary_counters_equal': True,
                   'tables': tables, 'manifest_sha256': manifest_hash})
    result = subprocess.run([str(args.gameplay_probe), str(replay)], text=True, capture_output=True)
    (args.output / (digest + '.gameplay.tsv')).write_text(result.stdout)
    (args.output / (digest + '.gameplay.stderr')).write_text(result.stderr)
    assert result.returncode == 0, digest
    lines = result.stdout.splitlines()
    actual_native = [line for line in lines if line.startswith('N\t')]
    actual_phase = [line for line in lines if line.startswith('P\t')]
    actual_ignored = {line.split('\t')[1]: int(line.split('\t')[2]) for line in lines if line.startswith('U\t')}
    expected_native, expected_phase = [], []
    native_counts, phase_counts, ignored = Counter(), Counter(), Counter()
    events = pq.read_table(export / 'events.parquet').to_pylist()
    for row in events:
        group = row['group']
        if group not in supported:
            ignored[group] += 1
            continue
        word_count, tag, name = supported[group]
        assert validate_event_payload(row, word_count, tag, name) is None, digest
        raw = row['raw_payload']
        words = [struct.unpack_from('<I', raw, 4 + 4 * index)[0] for index in range(word_count)]
        words += ['null'] * (2 - word_count)
        seconds_bits = struct.unpack('<I', raw[-4:])[0]
        expected_native.append(f"N\t{group}\t{row['time1']}\t{row['time2']}\t{seconds_bits}\t{words[0]}\t{words[1]}")
        native_counts[group] += 1
    with pq.ParquetFile(export / 'fields.parquet', pre_buffer=False) as fields:
        names = ['field_name','time_ms','packet_id','channel_index','actor_net_guid','object_net_guid',
                 'group_path','compatible_checksum','bit_count','raw_bits','value_i64','value_f64','value_bool','value_str']
        for batch in fields.iter_batches(columns=names):
            selected = batch.filter(pc.equal(pc.cast(batch['field_name'], 'string'), 'MulticastSetPhase.NewPhase'))
            for row in selected.to_pylist():
                assert row['group_path'] == '/Game/GameModes/Bomb/BombGameState.BombGameState_C_ClassNetCache'
                assert row['compatible_checksum'] == 3954716686 and row['object_net_guid'] is None
                assert row['bit_count'] == 8 and row['raw_bits'] is not None and len(row['raw_bits']) == 1
                value = row['raw_bits'][0]
                assert row['value_i64'] == value and all(row[key] is None for key in ('value_f64','value_bool','value_str'))
                expected_phase.append(f"P\t{row['time_ms']}\t{row['packet_id']}\t{row['channel_index']}\t{row['actor_net_guid']}\t{value}")
                phase_counts[value] += 1
    assert actual_native == expected_native, (digest, 'native values, multiplicity or order')
    assert actual_phase == expected_phase, (digest, 'phase values, multiplicity or order')
    assert actual_ignored == dict(ignored), digest
    summaries = [line.split('\t') for line in lines if line.startswith('S\t')]
    assert len(summaries) == 1 and int(summaries[0][1]) == len(events), digest
    assert int(summaries[0][2]) == manifest['quality']['checkpoints']['checkpoint_chunks'], digest
    assert sha(replay) == digest
    gameplay.append({'build': replay.parent.name, 'input_sha256': digest,
        'native_counts': dict(native_counts), 'phase_counts': dict(phase_counts), 'unprojected_native_counts': dict(ignored),
        'native_observations_compared': len(expected_native), 'phase_observations_compared': len(expected_phase),
        'skipped_checkpoints': int(summaries[0][2]), 'mismatches': 0, 'probe_output_sha256': hashlib.sha256(result.stdout.encode()).hexdigest()})
    print(replay.parent.name + ': exact export and fresh gameplay/raw equivalence', flush=True)
save('export-equivalence.json', {'source_commit': source_commit, 'cli_sha256': binaries['cli'],
    'baseline_source': baseline['baseline_commit'], 'baseline_executable_sha256': baseline['baseline_executable_sha256'],
    'baseline_evidence_sha256': sha(args.source / 'docs/evidence/phase-b-cli-public-parity.json'),
    'scope': 'Fresh combined-source exports vs accepted Phase A baseline fingerprints; all 13 tables, all nonvolatile manifest values and every required summary counter.', 'reports': parity})
save('gameplay.json', {'source_commit': source_commit, 'probe_sha256': binaries['gameplay_probe'],
    'oracle_exports': {'fresh': True, 'source_commit': source_commit, 'cli_sha256': binaries['cli']},
    'native_decoder': 'Existing independent Python extract_kill_ledger.validate_event_payload plus raw struct decoding',
    'phase_decoder': 'Exact schema identity and raw eight-bit byte compared to the sole typed value',
    'reports': gameplay, 'mismatches': 0,
    'limits': ['Native GUIDs remain unresolved; no player identity, kill-credit, damage or movement joins are certified.',
               'Ordering is checked within each source stream, not between native Event and replication timelines.',
               'Reader checkpoints are counted as skipped; gameplay does not decode checkpoints.']})
movement = []
for build in ('12.10', '12.11', '13.00'):
    matching = [(digest, path) for digest, path in inputs.items() if path.parent.name == build]
    assert len(matching) == 1
    digest, replay = matching[0]
    reference_dir = args.reference_root / build / 'reference'
    reference = reference_dir / 'movement.ndjson'
    reference_manifest = json.loads((reference_dir / 'manifest.json').read_text())
    assert reference_manifest['source_sha256'] == digest and reference_manifest['parse_status'] == 'completed'
    reference_source_commit = reference_manifest['parser_version'].rsplit('+', 1)[-1]
    assert reference_source_commit == 'fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8', build
    historical = json.loads((args.source / f'docs/evidence/movement-comparison-{build}.json').read_text())
    reference_hash = sha(reference)
    assert reference_hash == historical['reference']['logical_content_sha256'], build
    rust = args.output / (build + '.movement.jsonl.gz')
    stderr_path = args.output / (build + '.movement.stderr')
    with stderr_path.open('wb') as errors, rust.open('wb') as raw:
        with gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=0) as compressed:
            with subprocess.Popen([str(args.movement_probe), str(replay)], stdout=subprocess.PIPE, stderr=errors) as process:
                while data := process.stdout.read(1024 * 1024):
                    compressed.write(data)
                process.stdout.close()
                assert process.wait() == 0, build
    output = args.output / (build + '.movement-comparison.json')
    run(build + '-movement', [sys.executable, '-W', 'error', str(args.source / 'tools/compare_movement_provenance.py'),
        str(reference), str(rust), '--out', str(output)])
    comparison = json.loads(output.read_text())
    assert comparison['status'] == 'pass' and sha(reference) == reference_hash
    movement.append({'build': build, 'input_sha256': digest, 'status': comparison['status'],
        'reference_sha256': reference_hash, 'reference_manifest_sha256': sha(reference_dir / 'manifest.json'),
        'reference_source_commit': reference_source_commit,
        'reference_regenerated': False, 'historical_accepted_report_sha256': sha(args.source / f'docs/evidence/movement-comparison-{build}.json'),
        'comparison': comparison})
save('movement.json', {'source_commit': source_commit, 'probe_sha256': binaries['movement_probe'],
    'scope': 'Fresh combined-source Reader on three public small fixtures vs frozen C# reference outputs; wire identities, complete latest-move coverage, exact f32 bits.',
    'reports': movement})
example_inputs = [(digest, path) for digest, path in inputs.items() if path.parent.name == '12.10']
assert len(example_inputs) == 1
example_digest, _ = example_inputs[0]
adoption = []
for table in ('fields', 'checkpoint_fields'):
    table_path = audit / example_digest / 'export' / (table + '.parquet')
    label = 'adoption-' + table
    run(label, [sys.executable, '-W', 'error', str(args.source / 'docs/examples/summarize_fields.py'),
        str(table_path), '--batch-size', '2048'])
    counts = json.loads((args.output / (label + '.log')).read_text())
    with pq.ParquetFile(table_path, pre_buffer=False) as file:
        assert counts['field_rows'] == file.metadata.num_rows
    assert counts['rows_with_overlay'] + counts['rows_without_overlay'] == counts['field_rows']
    adoption.append({'table': table, 'counts': counts})
save('adoption.json', {'source_commit': source_commit, 'input_sha256': example_digest,
    'scope': 'Documented aggregate PyArrow example on fresh public main and checkpoint fields',
    'reports': adoption})
assert binaries == {key: sha(path) for key, path in [('cli', args.exe), ('gameplay_probe', args.gameplay_probe), ('movement_probe', args.movement_probe)]}
save('receipt.json', {'complete': True, 'source_commit': source_commit, 'binaries_sha256': binaries,
    'private_replays_executed': 0, 'public_replays': len(inputs), 'movement_fixtures': len(movement),
    'native_observations': sum(row['native_observations_compared'] for row in gameplay),
    'phase_observations': sum(row['phase_observations_compared'] for row in gameplay),
    'exact_parquet_tables': sum(row['parquet_tables'] for row in parity), 'performance_claim': None,
    'steps': steps})
print('fresh independent functional verification complete', flush=True)
