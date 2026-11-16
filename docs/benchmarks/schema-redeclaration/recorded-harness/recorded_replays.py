"""Paired complete checkpoint exports on the six public validation fixtures."""

import json
from pathlib import Path
import subprocess
import sys

ROOT = Path('/workspace/scratch/d2a245f4874f')
TOOL = ROOT / 'worktrees/benchmark/tools/benchmark_resources.py'
baseline = ROOT / 'worktrees/schema-budget/target/release/vrfkit'
candidate = ROOT / 'worktrees/schema-reuse/target/release/vrfkit'
for build in ('12.05', '12.08', '12.09', '12.10', '12.11', '13.00'):
    name = 'schema-reuse-' + build.replace('.', '') + '-checkpoints'
    output = ROOT / 'evidence' / (name + '.json')
    command = [sys.executable, str(TOOL), '--baseline-exe', str(baseline),
               '--candidate-exe', str(candidate),
               '--replay', str(ROOT / 'evidence/corpus-links' / build / 'fixture.vrf'),
               '--work-dir', str(ROOT / 'evidence' / name),
               '--output', str(output), '--warmups', '1', '--repeats', '5',
               '--checkpoints']
    result = subprocess.run(command, check=False)
    if result.returncode:
        raise SystemExit(result.returncode)
    report = json.loads(output.read_text())
    print(json.dumps({'build': build, 'report': output.name,
                      'top_level_keys': list(report)}), flush=True)
