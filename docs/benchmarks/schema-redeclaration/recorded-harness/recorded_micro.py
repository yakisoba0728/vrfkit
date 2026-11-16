"""Reproduce paired schema-registration micro measurements in this checkout."""

from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import platform
import statistics
import subprocess
import sys

ROOT = Path('/workspace/scratch/d2a245f4874f')
sys.path.insert(0, str(ROOT / 'worktrees/benchmark/tools'))
import benchmark_resources as br


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def distribution(values):
    middle = statistics.median(values)
    return {'samples': len(values), 'median': middle, 'min': min(values),
            'max': max(values), 'sample_stdev': statistics.stdev(values),
            'median_absolute_deviation': statistics.median(abs(v - middle) for v in values)}


trees = {'baseline': ROOT / 'worktrees/schema-reuse-baseline',
         'candidate': ROOT / 'worktrees/schema-reuse'}
destination = ROOT / 'evidence/schema-reuse-micro'
destination.mkdir()
provenance = {}
for variant, tree in trees.items():
    executable = tree / 'target/release/examples/registration_bench'
    provenance[variant] = {
        'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=tree, text=True).strip(),
        'executable_sha256': digest(executable),
        'cache_source_sha256': digest(tree / 'crates/vrf-schema/src/cache.rs'),
        'example_sha256': digest(tree / 'crates/vrf-schema/examples/registration_bench.rs'),
        'cargo_lock_sha256': digest(tree / 'Cargo.lock'),
    }
assert provenance['baseline']['example_sha256'] == provenance['candidate']['example_sha256']
assert provenance['baseline']['cargo_lock_sha256'] == provenance['candidate']['cargo_lock_sha256']
report = {
    'scope': '512 groups; 4096 identical path/index redeclarations; 32 slots per group',
    'timing_scope': 'Rust Instant measures registration loop only; process wall/CPU/RSS include initialization, validation and startup',
    'limitations': ['Not whole-replay throughput', 'Allocation counts and allocated bytes not measured',
                    'Short candidate process CPU/RSS measurements include substantial startup overhead'],
    'date_utc': datetime.now(timezone.utc).isoformat(),
    'rust': subprocess.check_output(['rustc', '+1.86.0', '--version'], text=True).strip(),
    'platform': platform.platform(),
    'profile': 'workspace release: opt-level=3, lto=fat, codegen-units=1, debug=1; --locked',
    'runner_sha256': digest(Path(__file__)),
    'measurement_tool_sha256': digest(Path(br.__file__)),
    'provenance': provenance,
    'modes': {},
}
for mode in ('add', 'declare'):
    samples = []
    for phase, number, variant in br.plan(warmups=1, repeats=9):
        directory = destination / f'{mode}-{phase}-{number:02d}-{variant}'
        directory.mkdir()
        measured = br.run_worker([str(trees[variant] / 'target/release/examples/registration_bench'), mode], directory, 30)
        assert measured['status'] == 'completed' and measured['exit_code'] == 0, measured
        output = json.loads((directory / 'parser.log').read_text())
        assert output['mode'] == mode and output['groups'] == 512
        assert output['redeclarations'] == 4096 and output['slots_per_group'] == 32
        sample = {'phase': phase, 'round': number, 'variant': variant,
                  'loop_seconds': output['seconds'], 'measurements': measured['measurements']}
        samples.append(sample)
    loop_distributions = {variant: distribution([sample['loop_seconds'] for sample in samples
                            if sample['phase'] == 'measured' and sample['variant'] == variant])
                          for variant in trees}
    paired_loop_ratios = []
    for number in range(9):
        pair = {sample['variant']: sample['loop_seconds'] for sample in samples
                if sample['phase'] == 'measured' and sample['round'] == number}
        paired_loop_ratios.append(pair['candidate'] / pair['baseline'])
    report['modes'][mode] = {'samples': samples, 'loop_distributions': loop_distributions,
                           'paired_loop_candidate_over_baseline': distribution(paired_loop_ratios),
                           'process_distributions': br.summarize(samples),
                           'paired_process_ratios': br.paired_ratios(samples)}
output = ROOT / 'evidence/schema-reuse-measurements.json'
output.write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({mode: {'loop': values['loop_distributions'],
                        'paired_loop_ratio': values['paired_loop_candidate_over_baseline'],
                        'process': {variant: {metric: values['process_distributions'][variant][metric]
                                              for metric in ('wall_seconds', 'cpu_total_seconds', 'peak_rss_bytes')}
                                    for variant in trees}}
                  for mode, values in report['modes'].items()}, indent=2))
