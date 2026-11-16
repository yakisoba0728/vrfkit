import hashlib
import json
import re
import shutil
import statistics
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

import argparse

parser = argparse.ArgumentParser(description='Diagnostic stage attribution on equivalent full exports')
parser.add_argument('--baseline', required=True, type=Path)
parser.add_argument('--profile', required=True, type=Path)
parser.add_argument('--replay', required=True, type=Path)
parser.add_argument('--harness-dir', required=True, type=Path)
parser.add_argument('--work-dir', required=True, type=Path)
parser.add_argument('--report', required=True, type=Path)
args = parser.parse_args()
if not all(p.is_file() for p in (args.baseline, args.profile, args.replay)):
    parser.error('Executables and replay must be existing files')
if args.work_dir.exists() or args.report.exists():
    parser.error('Work and report paths must be new')
sys.path.insert(0, str(args.harness_dir.resolve()))
import benchmark_resources as br

work = args.work_dir.resolve()
work.mkdir(mode=0o700)
replay = args.replay.resolve()
binaries = {'baseline': args.baseline.resolve(), 'profile': args.profile.resolve()}
report_path = args.report.resolve()
report = {'schema_version': 1, 'started_utc': datetime.now(timezone.utc).isoformat(),
          'status': 'running', 'purpose': 'diagnostic stage attribution, not an optimization comparison',
          'provenance': br.provenance({**binaries, 'input': replay}), 'samples': [],
          'unmeasured': ['allocation counts', 'allocated bytes', 'isolated disk I/O time']}
report_path.write_text(json.dumps(report, indent=2)+'\n')
reference = None
for number, variant in enumerate(['baseline', 'profile', 'profile', 'baseline', 'baseline', 'profile']):
    d = work/f'{number:02d}-{variant}'
    d.mkdir()
    out = d/'export'
    measured = br.run_worker([str(binaries[variant]), 'export', str(replay), '--out', str(out), '--checkpoints'], d, 120)
    assert measured['status'] == 'completed' and measured['exit_code'] == 0, measured
    fingerprint = br.fingerprint(out, d/'parser.log', True, replay.stat().st_size)
    if reference is None:
        reference = fingerprint
    assert br.same_output(reference, fingerprint), (number, variant)
    values = measured['measurements']
    values.update(input_mib_per_second=replay.stat().st_size/1048576/values['wall_seconds'],
                  parquet_bytes=fingerprint['parquet_bytes'], output_bytes=fingerprint['output_bytes'])
    profile_lines = [l for l in (d/'parser.log').read_text().splitlines() if l.startswith('PHASE_PROFILE ')]
    stages = []
    for line in profile_lines:
        parts = line.split()
        first_value = next(i for i,p in enumerate(parts) if '=' in p)
        stages.append({'label': ' '.join(parts[1:first_value]),
                       **{k:int(v) for k,v in (p.split('=') for p in parts[first_value:])}})
    report['samples'].append({'order': number, 'variant': variant, 'measurements': values,
                              'profile': stages, 'output_fingerprint': br.json_hash(fingerprint),
                              'equivalent': True})
    shutil.rmtree(out)
    report_path.write_text(json.dumps(report, indent=2)+'\n')
report['provenance_after'] = br.provenance({**binaries, 'input': replay})
assert report['provenance_after'] == report['provenance']
report['status'] = 'completed'
report['equivalent'] = True
report['output'] = reference
report['summary'] = {
    variant: {metric: statistics.median(s['measurements'][metric] for s in report['samples'] if s['variant']==variant) if all(s['measurements'][metric] is not None for s in report['samples'] if s['variant']==variant) else None
              for metric in br.METRICS}
    for variant in binaries
}
report['finished_utc'] = datetime.now(timezone.utc).isoformat()
report_path.write_text(json.dumps(report, indent=2)+'\n')
print(json.dumps({'status':report['status'],'summary':report['summary']}))
