"""Prove selected Phase B guards fail under actual implementation mutations.

Run only after a passing unmodified suite. Changes are temporary, never
committed; every source file is restored byte-for-byte in a finally block.
Detailed compiler/test logs remain in the local output directory.
Run replay oracles on frozen clean binaries before these controls. Restoring
source bytes does not restore target artifacts: compiled mutant executables
must never be accepted or used until an explicit clean-source rebuild.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

if not __debug__:
    raise RuntimeError('This verification harness requires Python assertions; remove -O/PYTHONOPTIMIZE.')

parser = argparse.ArgumentParser()
parser.add_argument('--source', type=Path, required=True)
parser.add_argument('--target', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=False)
env = {**os.environ, 'CARGO_TARGET_DIR': str(args.target), 'CARGO_BUILD_JOBS': '1',
       'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_PROFILE_TEST_DEBUG': '0', 'CARGO_INCREMENTAL': '0'}
cases = [
    ('native-time', 'crates/vrfkit/src/gameplay.rs',
     'if !event_payload_seconds_matches_time(event.time1, payload.seconds) {',
     'if false && !event_payload_seconds_matches_time(event.time1, payload.seconds) {',
     'gameplay', 'native_declarations_and_time_are_checked_separately', True),
    ('phase-raw', 'crates/vrfkit/src/gameplay.rs',
     'if field.bit_count != 8 || field.raw_bits.as_deref() != Some(&[value][..]) {',
     'if false && (field.bit_count != 8 || field.raw_bits.as_deref() != Some(&[value][..])) {',
     'gameplay', 'phase_raw_window_must_equal_the_overlay_exactly', True),
    ('skipped-extent', 'crates/vrfkit/src/reader_input.rs',
     'if size as u64 > available {',
     'if false && size as u64 > available {',
     'cli_input', 'validate_rejects_a_late_invalid_skipped_payload', True),
    ('checkpoint-omission', 'crates/vrfkit/src/reader_input.rs',
     'if kind == ChunkType::Checkpoint {',
     'if false && kind == ChunkType::Checkpoint {',
     'cli_input', 'sparse_unknown_chunks_do_not_change_main_or_checkpoint_tables', False),
    ('checkpoint-error-suppression', 'crates/vrfkit/src/driver/mod.rs',
     'let mut checkpoints = checkpoints?;',
     'let mut checkpoints = checkpoints.unwrap_or(None);',
     'cli_input', 'late_input_and_checkpoint_errors_leave_prior_export_unchanged', False),
]
cases = [(name, 'vrfkit', *rest) for name, *rest in cases]
cases.extend([
    ('dictionary-count', 'vrf-export', 'crates/vrf-export/src/schema.rs',
     'count += 1;', 'count += 2;', None,
     'schema::tests::dictionary_runs_keep_keys_values_and_first_occurrence_order', False),
    ('parquet-run-count', 'vrf-export', 'crates/vrf-export/src/schema.rs',
     'count += 1;', 'count += 2;', 'roundtrip',
     'group_path_runs_keep_exact_parquet_bytes_across_batches', False),
    ('parquet-run-value-order', 'vrf-export', 'crates/vrf-export/src/schema.rs',
     '} else {\n                builder.append_values(previous.as_ref(), count);',
     '} else {\n                builder.append_values(value.as_ref(), count);',
     'roundtrip', 'group_path_runs_keep_exact_parquet_bytes_across_batches', False),
])
receipt = {'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=args.source, text=True).strip(),
           'purpose': 'Implementation mutation controls, not input-only mutations',
           'target_artifacts': 'May contain compiled mutants after this run; source restoration is not binary restoration. Use frozen pre-mutation binaries or explicitly rebuild clean source.',
           'cases': []}
for name, package, relative, original, replacement, suite, test, core_only in cases:
    path = args.source / relative
    before = path.read_bytes()
    content = before.decode()
    assert content.count(original) == 1, name
    mutant = content.replace(original, replacement, 1).encode()
    command = ['cargo', '+1.86.0', 'test', '-p', package, '--locked']
    if core_only:
        command.append('--no-default-features')
    command.extend(['--test', suite] if suite else ['--lib'])
    command.extend([test, '--', '--exact'])
    try:
        path.write_bytes(mutant)
        with (args.output / (name + '.log')).open('w') as log:
            process = subprocess.run(command, cwd=args.source, env=env,
                                     stdout=log, stderr=subprocess.STDOUT, timeout=300)
        observed = (args.output / (name + '.log')).read_text()
        killed = process.returncode == 101 and f'test {test} ... FAILED' in observed
        if not killed:
            raise RuntimeError(name + ': mutation did not produce the required test assertion failure')
    finally:
        path.write_bytes(before)
        assert path.read_bytes() == before, name
    row = {'name': name, 'source_file': relative, 'package': package, 'test_suite': suite or 'lib', 'test': test,
           'mutated_from': original, 'mutated_to': replacement,
           'original_sha256': hashlib.sha256(before).hexdigest(),
           'mutant_sha256': hashlib.sha256(mutant).hexdigest(),
           'restored_sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
           'exit_code': process.returncode, 'target_test_failed': killed,
           'log_sha256': hashlib.sha256(observed.encode()).hexdigest()}
    receipt['cases'].append(row)
    (args.output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(name + ': expected assertion failure; source restored', flush=True)
receipt['complete'] = True
(args.output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
