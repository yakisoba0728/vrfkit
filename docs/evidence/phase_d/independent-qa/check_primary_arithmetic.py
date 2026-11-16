"""Independent recomputation from every fresh worker record; no benchmark reruns."""
from __future__ import annotations

import hashlib
import argparse
import json
import math
from pathlib import Path
import statistics

ROOT = Path('.')  # Assigned from --evidence-root before reading any evidence.
BENCH = ROOT / 'benchmark'
OUTPUT = ROOT / 'independent-qa/primary-arithmetic-review.json'


def read(path):
    return json.loads(path.read_text())


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def canonical(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()).hexdigest()


def same(left, right):
    assert math.isclose(left, right, rel_tol=1e-12, abs_tol=1e-12), (left, right)


def main():
    parser = argparse.ArgumentParser()
    global ROOT, BENCH
    parser.add_argument('--evidence-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    ROOT = args.evidence_root.resolve()
    BENCH = ROOT / 'benchmark'
    output = args.output
    assert __debug__ and not output.exists()
    report = read(BENCH / 'primary-summary.json')
    protocol = read(BENCH / 'protocol.json')
    preflight = read(BENCH / 'preflight.json')
    assert report['completed_series'] == 12 and report['completed_exports'] == 168
    assert report['preflight_exports'] == preflight['exports_executed'] == 12
    assert report['all_samples_retained'] and report['all_strict_gates_passed']
    assert protocol['warmup_pairs'] == 1 and protocol['measured_pairs'] == 6
    expected_order = [(phase, number, role)
        for phase, rounds in [('warmup', 1), ('measured', 6)]
        for number in range(rounds)
        for role in (('baseline', 'candidate') if number % 2 == 0 else ('candidate', 'baseline'))]
    evidence_rows = []
    record_count = 0
    for row in report['rows']:
        derived = {}
        pre = next(x for x in preflight['cases'] if (x['build'], x['mode']) == (row['build'], row['mode']))
        for label in ['control', 'comparison']:
            name = f"{label}-{row['build']}-{row['mode']}"
            path = BENCH / (name + '.json')
            d = read(path)
            assert digest(path) == row['series_sha256'][label]
            assert d['status'] == 'completed' and d['equivalent'] and not d['failures']
            assert d['created_utc'] > protocol['predeclared_utc']
            assert d['provenance_before'] == d['provenance_after']
            assert [(s['phase'], s['round'], s['variant']) for s in d['samples']] == expected_order
            if label == 'control':
                assert d['provenance_before']['baseline'] == d['provenance_before']['candidate']
            else:
                assert d['provenance_before']['baseline']['sha256'] == read(BENCH / 'frozen-baseline/build.json')['frozen_executable']['sha256']
            assert d['provenance_before']['candidate']['sha256'] == read(BENCH / 'frozen-candidate/build.json')['frozen_executable']['sha256']
            fingerprint = canonical({k: d['reference_output'][k] for k in ['tables', 'manifest_sha256', 'counters']})
            assert fingerprint == row['output_fingerprint']
            for key in ['tables', 'manifest_sha256', 'counters']:
                assert d['reference_output'][key] == pre['candidate'][key] == pre['baseline'][key]
            for sample in d['samples']:
                assert sample['valid'] and sample['status'] == 'completed' and sample['exit_code'] == 0
                assert sample['output_fingerprint'] == fingerprint
                directory = BENCH / (name + '-work') / f"{sample['phase']}-{sample['round']:03d}-{sample['variant']}"
                worker = read(directory / 'measurement.json')
                assert worker['status'] == 'completed' and worker['exit_code'] == 0
                values = sample['measurements']
                assert all(values[k] == v for k, v in worker['measurements'].items())
                same(values['cpu_total_seconds'], values['cpu_user_seconds'] + values['cpu_system_seconds'])
                same(values['cpu_utilization_percent'], 100 * values['cpu_total_seconds'] / values['wall_seconds'])
                same(values['input_mib_per_second'], row['input']['bytes'] / 1024**2 / values['wall_seconds'])
                request = read(directory / 'request.json')
                command = request['command']
                assert command[1] == 'export' and command[3] == '--out'
                assert ('--checkpoints' in command) == (row['mode'] == 'checkpoints')
                expected_exe = 'candidate' if label == 'control' else sample['variant']
                assert Path(command[0]).parent.name == 'frozen-' + expected_exe
                assert request['timeout'] == protocol['timeout_per_export_seconds']
                assert (directory / 'worker.log').read_text() == ''
                record_count += 1
            medians = {}
            for variant in ['baseline', 'candidate']:
                values = [s['measurements'] for s in d['samples'] if s['phase'] == 'measured' and s['variant'] == variant]
                assert len(values) == 6
                medians[variant] = {}
                for metric, declared in d['summary'][variant].items():
                    raw = [v[metric] for v in values]
                    median = statistics.median(raw)
                    expected = {'samples': 6, 'median': median, 'min': min(raw), 'max': max(raw),
                                'sample_stdev': statistics.stdev(raw),
                                'median_absolute_deviation': statistics.median(abs(x - median) for x in raw)}
                    for key, value in expected.items():
                        same(declared[key], value)
                    medians[variant][metric] = median
                    aggregate = row['control_medians'][variant] if label == 'control' else row[variant]
                    same(aggregate[metric], median)
                aggregate = row['control_medians'][variant] if label == 'control' else row[variant]
                same(aggregate['replays_per_second'], statistics.median(1 / v['wall_seconds'] for v in values))
            derived[label] = {}
            for metric in ['wall_seconds', 'cpu_total_seconds', 'peak_rss_bytes']:
                ratios = []
                for number in range(6):
                    pair = {s['variant']: s['measurements'][metric] for s in d['samples']
                            if s['phase'] == 'measured' and s['round'] == number}
                    ratios.append(pair['candidate'] / pair['baseline'])
                median = statistics.median(ratios)
                assert ratios == d['comparison'][metric]['candidate_over_baseline']
                same(median, d['comparison'][metric]['median_candidate_over_baseline'])
                aggregate = row['control'][metric] if label == 'control' else row['paired_changes'][metric]
                assert ratios == aggregate['ratios'] and [min(ratios), max(ratios)] == aggregate['range']
                same(median, aggregate['median'])
                if label == 'control':
                    same(max(abs(x - 1) for x in ratios), aggregate['max_absolute_fraction_from_one'])
                else:
                    same(100 * (median - 1), aggregate['median_percent_change'])
                derived[label][metric] = {'paired_median_ratio': median, 'ratio_of_medians': medians['candidate'][metric] / medians['baseline'][metric], 'range': [min(ratios), max(ratios)]}
        conclusions = {}
        for metric in derived['control']:
            control = derived['control'][metric]['range']
            comparison = derived['comparison'][metric]['range']
            overlap = max(control[0], comparison[0]) <= min(control[1], comparison[1])
            changes_direction = comparison[0] < 1 < comparison[1]
            assert overlap or changes_direction
            conclusions[metric] = {'control_range_overlaps_comparison_range': overlap,
                'comparison_changes_direction': changes_direction, 'classification': 'INCONCLUSIVE'}
        evidence_rows.append({'build': row['build'], 'mode': row['mode'], 'recomputed': derived, 'conclusions': conclusions})
    assert record_count == 168 and len(evidence_rows) == 6
    receipt = {'schema_version': 1, 'decision': 'ACCEPT_ARITHMETIC_AND_INCONCLUSIVE_CLASSIFICATION',
        'reviewer': 'D independent QA', 'runner_sha256': digest(Path(__file__)),
        'primary_summary_sha256': digest(BENCH / 'primary-summary.json'), 'worker_records_crosschecked': record_count,
        'series': 12, 'cases': 6, 'new_export_executions': 0, 'rows': evidence_rows,
        'notes': ['Median paired ratios and ratios of independently computed medians are different estimands; both recomputed.',
                  'All samples retained, no outlier removal, each per-worker measurement matches aggregate input.',
                  'A/A control uses the same accepted-runtime executable in both slots.',
                  'Modes were measured in separate serial windows; absolute times do not establish relative checkpoint overhead.',
                  'This audit verifies calculations and retained records, not exclusive-host or cache conditions.']}
    output.write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps({k: receipt[k] for k in ['decision', 'worker_records_crosschecked', 'series', 'cases']}, indent=2))


if __name__ == '__main__':
    main()
