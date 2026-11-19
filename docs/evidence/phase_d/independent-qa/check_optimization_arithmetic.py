"""Independent verification and disposition of the completed, rejected experiment."""
from pathlib import Path
import hashlib
import argparse
import json
import math
import statistics
import subprocess

REPO = EVIDENCE = BENCH = OUT = None


def read(path):
    return json.loads(path.read_text())


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def canonical(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()).hexdigest()


def same(a, b):
    assert math.isclose(a, b, rel_tol=1e-12, abs_tol=1e-12), (a, b)


def main():
    global REPO, EVIDENCE, BENCH, OUT
    parser = argparse.ArgumentParser()
    parser.add_argument('--repo', type=Path, required=True)
    parser.add_argument('--evidence-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    REPO, EVIDENCE, OUT = args.repo.resolve(), args.evidence_root.resolve(), args.output
    BENCH = EVIDENCE / 'optimization-measurement'
    assert __debug__ and not OUT.exists()
    summary = read(BENCH / 'summary.json')
    protocol = read(BENCH / 'protocol.json')
    preflight = read(BENCH / 'preflight.json')
    accepted_preflight = read(EVIDENCE / 'benchmark/preflight.json')
    assert summary['completed_series'] == 8 and summary['timed_exports'] == 112
    assert summary['preflight_exports'] == preflight['exports_executed'] == 12
    assert preflight['cases_passed'] == 6
    assert summary['all_samples_retained'] and summary['all_strict_output_gates_passed']
    assert summary['failed_attempts'] == 0
    order = [(phase, i, role) for phase, count in [('warmup', 1), ('measured', 6)]
             for i in range(count) for role in (('baseline', 'candidate') if i % 2 == 0 else ('candidate', 'baseline'))]
    for case in preflight['cases']:
        accepted = next(c for c in accepted_preflight['cases'] if (c['build'], c['mode']) == (case['build'], case['mode']))
        for key in ['tables', 'counters', 'manifest_sha256']:
            assert case['baseline'][key] == case['candidate'][key] == accepted['candidate'][key]
    rows = []
    count = 0
    for row in summary['rows']:
        derived = {}
        for label in ['control', 'comparison']:
            name = f"{label}-{row['build']}-{row['mode']}"
            path = BENCH / (name + '.json')
            series = read(path)
            assert sha(path) == row['series_sha256'][label]
            assert series['status'] == 'completed' and series['equivalent'] and not series['failures']
            assert series['created_utc'] > protocol['predeclared_utc']
            assert series['provenance_before'] == series['provenance_after']
            for role in ['baseline', 'candidate']:
                expected = protocol['executables']['baseline' if label == 'control' else role]
                assert series['provenance_before'][role] == {k: expected[k] for k in ['sha256', 'bytes']}
            assert [(s['phase'], s['round'], s['variant']) for s in series['samples']] == order
            expected_output = next(c['candidate'] for c in preflight['cases'] if (c['build'], c['mode']) == (row['build'], row['mode']))
            for key in ['tables', 'counters', 'manifest_sha256']:
                assert series['reference_output'][key] == expected_output[key]
            fingerprint = canonical({k: expected_output[k] for k in ['tables', 'counters', 'manifest_sha256']})
            for sample in series['samples']:
                assert sample['valid'] and sample['status'] == 'completed' and sample['exit_code'] == 0
                assert sample['output_fingerprint'] == fingerprint
                directory = BENCH / (name + '-work') / f"{sample['phase']}-{sample['round']:03d}-{sample['variant']}"
                worker = read(directory / 'measurement.json')
                assert worker['exit_code'] == 0 and worker['status'] == 'completed'
                values = sample['measurements']
                assert all(values[k] == v for k, v in worker['measurements'].items())
                same(values['cpu_total_seconds'], values['cpu_user_seconds'] + values['cpu_system_seconds'])
                same(values['cpu_utilization_percent'], values['cpu_total_seconds'] * 100 / values['wall_seconds'])
                same(values['input_mib_per_second'], row['input']['bytes'] / 1024**2 / values['wall_seconds'])
                request = read(directory / 'request.json')
                assert ('--checkpoints' in request['command']) == (row['mode'] == 'checkpoints')
                assert request['timeout'] == protocol['timeout_per_export_seconds']
                assert not (directory / 'worker.log').read_text()
                count += 1
            for role in ['baseline', 'candidate']:
                values = [s['measurements'] for s in series['samples'] if s['phase'] == 'measured' and s['variant'] == role]
                aggregate = row['control_medians'][role] if label == 'control' else row[role]
                for metric, declared in series['summary'][role].items():
                    v = [x[metric] for x in values]
                    median = statistics.median(v)
                    expected = {'samples': 6, 'median': median, 'min': min(v), 'max': max(v),
                        'sample_stdev': statistics.stdev(v), 'median_absolute_deviation': statistics.median(abs(x - median) for x in v)}
                    for k, x in expected.items():
                        same(declared[k], x)
                    same(aggregate[metric], median)
                same(aggregate['replays_per_second'], statistics.median(1/x['wall_seconds'] for x in values))
            derived[label] = {}
            for metric in ['wall_seconds', 'cpu_total_seconds', 'peak_rss_bytes']:
                ratios = []
                for number in range(6):
                    pair = {s['variant']: s['measurements'][metric] for s in series['samples'] if s['phase'] == 'measured' and s['round'] == number}
                    ratios.append(pair['candidate'] / pair['baseline'])
                aggregate = row['control'][metric] if label == 'control' else row['paired_changes'][metric]
                assert aggregate['ratios'] == ratios and aggregate['range'] == [min(ratios), max(ratios)]
                same(aggregate['median'], statistics.median(ratios))
                assert series['comparison'][metric]['candidate_over_baseline'] == ratios
                if label == 'comparison':
                    same(aggregate['median_percent_change'], 100*(statistics.median(ratios)-1))
                derived[label][metric] = {'paired_median_ratio': statistics.median(ratios), 'range': [min(ratios), max(ratios)]}
        conclusions = {}
        for metric in derived['control']:
            a = derived['control'][metric]['range']
            b = derived['comparison'][metric]['range']
            overlap = max(a[0],b[0]) <= min(a[1],b[1])
            reversal = b[0] < 1 < b[1]
            conclusions[metric] = {'aa_comparison_ranges_overlap': overlap, 'direction_reversal': reversal}
        assert conclusions['wall_seconds']['aa_comparison_ranges_overlap']
        assert conclusions['wall_seconds']['direction_reversal']
        rows.append({'build': row['build'], 'mode': row['mode'], 'recomputed': derived, 'conclusions': conclusions})
    assert len(rows) == 4 and count == 112
    frozen_path = EVIDENCE / 'optimization/frozen-optimization/build.json'
    frozen = read(frozen_path)
    assert frozen['source']['commit'] == protocol['actual_candidate_build_source'] == summary['candidate_actual_build_source']
    assert frozen['source']['tree'] == protocol['local_remote_tree'] == summary['candidate_tree']
    assert frozen['frozen_executable'] == protocol['executables']['candidate']
    assert sha(frozen_path.parent / 'vrfkit') == frozen['frozen_executable']['sha256']
    assert sha(EVIDENCE / 'optimization/build.cargo.jsonl') == frozen['cargo_messages_sha256']
    source_files = frozen['source']['files']
    for name, expected in source_files.items():
        raw = subprocess.check_output(['git', '-C', str(REPO), 'show', frozen['source']['commit']+':'+name])
        assert hashlib.sha256(raw).hexdigest() == expected
    assert canonical(source_files) == frozen['source']['files_sha256']
    assert all(x['fresh'] is False for x in frozen['runtime_artifacts']) and len(frozen['runtime_artifacts']) == 11
    out = {'schema_version':1,'decision':'WITHDRAW_DO_NOT_INTEGRATE', 'runner_sha256':sha(Path(__file__)),
        'summary_sha256':sha(BENCH/'summary.json'), 'protocol_sha256':sha(BENCH/'protocol.json'),
        'worker_records_crosschecked':count, 'series':8, 'preflight_exports':12, 'preflight_cases':6,
        'all_6_preflight_outputs_equal_accepted_fresh_outputs':True,
        'frozen_candidate_receipt_sha256':sha(frozen_path), 'source_files_checked':len(source_files),
        'independent_correctness':'PASSED (candidate-mutation-review.json and optimization-fresh-output-review.json)',
        'rows':rows, 'accepted_runtime_optimizations':0,
        'reason':['Every wall-time comparison reverses direction and overlaps same-binary variation.',
                  'Positive allocation-mechanism observations do not establish useful end-to-end benefit.',
                  'RSS paired medians all increase; 12.09 checkpoint RSS increases in every paired observation.',
                  'Preserve the isolated experiment and stop repeats according to the declared budget.'],
        'new_exports_or_builds_by_this_arithmetic_audit':0}
    OUT.write_text(json.dumps(out,indent=2)+'\n')
    print(json.dumps({k:out[k] for k in ['decision','worker_records_crosschecked','series','preflight_cases','source_files_checked']},indent=2))


if __name__=='__main__':
    main()
