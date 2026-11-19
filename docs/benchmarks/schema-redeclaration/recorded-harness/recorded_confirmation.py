"""Narrow second full-match comparison and same-binary timing control."""

from pathlib import Path
import subprocess
import sys

ROOT = Path('/workspace/scratch/d2a245f4874f')
tool = ROOT / 'worktrees/benchmark/tools/benchmark_resources.py'
baseline = ROOT / 'worktrees/schema-budget/target/release/vrfkit'
candidate = ROOT / 'worktrees/schema-reuse/target/release/vrfkit'
for label, selected in (('confirm', candidate), ('control', baseline)):
    name = 'schema-reuse-1205-' + label
    result = subprocess.run([
        sys.executable, str(tool), '--baseline-exe', str(baseline),
        '--candidate-exe', str(selected),
        '--replay', str(ROOT / 'evidence/corpus-links/12.05/fixture.vrf'),
        '--work-dir', str(ROOT / 'evidence' / name),
        '--output', str(ROOT / 'evidence' / (name + '.json')),
        '--warmups', '2', '--repeats', '7', '--checkpoints',
    ], check=False)
    if result.returncode:
        raise SystemExit(result.returncode)
