"""Build and freeze exact combined-source functional verification binaries.

This uses the repository's optimized test profile, with debug information and
incremental compilation disabled. It is not a release-performance build.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

if not __debug__:
    raise RuntimeError('This verification harness requires Python assertions; remove -O/PYTHONOPTIMIZE.')

parser = argparse.ArgumentParser()
for name in ('source', 'target', 'probe-source', 'output'):
    parser.add_argument('--' + name, type=Path, required=True)
args = parser.parse_args()
args.source = args.source.resolve()
args.target = args.target.resolve()
args.output = args.output.resolve()
args.output.mkdir(parents=True, exist_ok=False)
env = {**os.environ, 'CARGO_TARGET_DIR': str(args.target), 'CARGO_BUILD_JOBS': '1',
       'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_PROFILE_TEST_DEBUG': '0', 'CARGO_INCREMENTAL': '0',
       'RUSTUP_TOOLCHAIN': '1.86.0'}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def runtime_sources():
    tracked = subprocess.check_output(['git', 'ls-files'], cwd=args.source, text=True).splitlines()
    paths = [path for path in tracked if (path.startswith('crates/') and '/src/' in path and path.endswith('.rs'))
             or path in ('Cargo.lock', 'Cargo.toml') or (path.startswith('crates/') and path.endswith('/Cargo.toml'))]
    return {path: sha(args.source / path) for path in paths}


def build(label, options, expected):
    command = ['cargo', '+1.86.0', 'build', '--profile', 'test', '-p', 'vrfkit', '--locked',
               '--message-format=json', *options]
    stdout = args.output / (label + '.jsonl')
    stderr = args.output / (label + '.log')
    with stdout.open('w') as output, stderr.open('w') as errors:
        code = subprocess.run(command, cwd=args.source, env=env, stdout=output, stderr=errors).returncode
    assert code == 0, label
    artifacts = {}
    for line in stdout.read_text().splitlines():
        message = json.loads(line)
        if message.get('reason') == 'compiler-artifact' and message.get('executable'):
            name = message['target']['name']
            if name in expected:
                executable = Path(message['executable'])
                assert Path(message['target']['src_path']).is_relative_to(args.source)
                assert executable.is_relative_to(args.target)
                destination = args.output / name
                shutil.copy2(executable, destination)
                dependency_file = executable.with_suffix('.d')
                dependencies = dependency_file.read_text().splitlines()[0].split(': ', 1)[1].split()
                workspace_files = [Path(path) for path in dependencies if '/crates/' in path and '/registry/' not in path]
                assert workspace_files and all(path.is_relative_to(args.source) and path.is_file() for path in workspace_files)
                artifacts[name] = {'sha256': sha(destination), 'bytes': destination.stat().st_size,
                    'target_source': str(Path(message['target']['src_path']).relative_to(args.source)),
                    'profile_from_compiler': message['profile'], 'cargo_fresh_flag': message['fresh'],
                    'workspace_dependency_count': len(workspace_files),
                    'workspace_dependencies_sha256': {str(path.relative_to(args.source)): sha(path) for path in workspace_files}}
    assert set(artifacts) == set(expected), label
    print(label + ': explicit build succeeded; exact artifacts frozen', flush=True)
    return {'command': command, 'exit_code': code, 'stdout_sha256': sha(stdout), 'stderr_sha256': sha(stderr), 'artifacts': artifacts}


assert subprocess.run(['git', 'diff', 'HEAD', '--quiet'], cwd=args.source).returncode == 0
source_commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=args.source, text=True).strip()
before = runtime_sources()
temporary = args.source / 'crates/vrfkit/examples/qa_gameplay_probe.rs'
assert not temporary.exists()
probe_bytes = args.probe_source.read_bytes()
temporary.write_bytes(probe_bytes)
try:
    cli = build('cli', ['--bin', 'vrfkit'], ['vrfkit'])
    probes = build('probes', ['--no-default-features', '--example', 'qa_gameplay_probe',
        '--example', 'movement_provenance', '--example', 'gameplay_events'],
        ['qa_gameplay_probe', 'movement_provenance', 'gameplay_events'])
finally:
    assert temporary.read_bytes() == probe_bytes
    temporary.unlink()
assert runtime_sources() == before
receipt = {'source_commit': source_commit,
    'source_tree': subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], cwd=args.source, text=True).strip(),
    'profile': 'Cargo test profile: opt-level 2, no debug information, no incremental compilation; debug assertions remain enabled',
    'profile_is_default_release': False, 'performance_claim': None,
    'rustc': subprocess.check_output(['rustc', '+1.86.0', '-Vv'], cwd=args.source, env=env, text=True).strip(),
    'runtime_sources_sha256': before, 'temporary_probe_sha256': hashlib.sha256(probe_bytes).hexdigest(),
    'source_unchanged_after_build': True, 'cli': cli, 'probes': probes}
(args.output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
