"""Check the two fresh production builds against immutable Git and Cargo records."""
from __future__ import annotations

import hashlib
import argparse
import json
from pathlib import Path
import subprocess

REPO = BENCH = OUT = None


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def canonical(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def git(*args):
    return subprocess.check_output(['git', '-C', str(REPO), *args])


def main():
    global REPO, BENCH, OUT
    parser = argparse.ArgumentParser()
    parser.add_argument('--repo', type=Path, required=True)
    parser.add_argument('--benchmark-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    REPO, BENCH, OUT = args.repo.resolve(), args.benchmark_dir.resolve(), args.output
    assert __debug__ and not OUT.exists()
    builds = {}
    for label in ['baseline', 'candidate']:
        receipt_path = BENCH / f'frozen-{label}/build.json'
        d = json.loads(receipt_path.read_text())
        source = d['source']
        commit = source['commit']
        assert git('rev-parse', commit + '^{tree}').decode().strip() == source['tree']
        wanted = git('ls-tree', '-r', '--name-only', commit, '--', 'Cargo.toml', 'Cargo.lock', 'crates', 'dev', 'tools/extract_component_classes', '.cargo').decode().splitlines()
        assert sorted(wanted) == sorted(source['files'])
        request = ''.join(f'{commit}:{name}\n' for name in wanted).encode()
        records = subprocess.check_output(['git', '-C', str(REPO), 'cat-file', '--batch'], input=request)
        offset = 0
        actual = {}
        for name in wanted:
            end = records.index(b'\n', offset)
            header = records[offset:end].split()
            assert header[1] == b'blob'
            size = int(header[2])
            data = records[end + 1:end + 1 + size]
            assert len(data) == size and records[end + 1 + size:end + 2 + size] == b'\n'
            offset = end + 2 + size
            actual[name] = hashlib.sha256(data).hexdigest()
        assert offset == len(records)
        assert actual == source['files'] and canonical(actual) == source['files_sha256']
        assert d['declared_release_profile'] == {'opt-level': 3, 'lto': 'fat', 'codegen-units': 1, 'debug': 1}
        assert d['build_environment'] == {'RUSTFLAGS': '', 'CARGO_ENCODED_RUSTFLAGS': '', 'CARGO_BUILD_JOBS': '1', 'CARGO_INCREMENTAL': '0'}
        assert d['compiler']['rustc_verbose'] == (BENCH / 'rustc.txt').read_text()
        assert d['compiler']['cargo_version'] == (BENCH / 'cargo.txt').read_text()
        assert d['build_command'] == (BENCH / f'{label}.command.txt').read_text()
        cargo_path = BENCH / f'{label}.cargo.jsonl'
        assert sha(cargo_path) == d['cargo_messages_sha256']
        cargo = [json.loads(line) for line in cargo_path.read_text().splitlines()]
        assert [x for x in cargo if x['reason'] == 'build-finished'] == [{'reason': 'build-finished', 'success': True}]
        assert d['binary_artifact'] in cargo
        artifacts = d['runtime_artifacts']
        assert len(artifacts) == 11 and all(x in cargo for x in artifacts)
        names = {Path(x['manifest_path']).parent.name for x in artifacts}
        assert len(names) == 10
        feature_profiles = {}
        for artifact in artifacts:
            assert artifact['fresh'] is False and artifact['profile']['test'] is False
            assert artifact['profile']['opt_level'] == '3' and artifact['profile']['debuginfo'] == 1
            assert artifact['profile']['debug_assertions'] is False
            key = artifact['target']['name'] + '/' + ','.join(artifact['target']['kind'])
            feature_profiles[key] = {'features': artifact['features'], 'profile': artifact['profile']}
        frozen = BENCH / f'frozen-{label}/vrfkit'
        assert frozen.is_file() and not frozen.is_symlink()
        assert frozen.stat().st_size == d['frozen_executable']['bytes']
        assert sha(frozen) == d['frozen_executable']['sha256']
        builds[label] = {'receipt_sha256': sha(receipt_path), 'source_commit': commit, 'source_tree': source['tree'],
            'tracked_source_files_checked': len(actual), 'source_files_sha256': canonical(actual),
            'frozen_executable_sha256': sha(frozen), 'runtime_artifact_count': len(artifacts),
            'production_crate_count': len(names), 'feature_profiles': feature_profiles}
    assert builds['baseline']['feature_profiles'] == builds['candidate']['feature_profiles']
    out = {'schema_version': 1, 'decision': 'ACCEPT_FRESH_BUILD_CORRESPONDENCE', 'builds': builds,
        'runner_sha256': sha(Path(__file__)), 'new_builds_by_this_audit': 0,
        'reviewed_build_driver': 'evidence/benchmark/build_benchmarks.py',
        'build_driver_sha256': sha(BENCH / 'build_benchmarks.py'),
        'freeze_helper_sha256': sha(REPO / 'docs/examples/freeze_benchmark_binary.py'),
        'checked_conditions': ['Both recorded source maps and tree IDs equal immutable Git objects.',
            'All ten production crates plus the CLI artifact are newly compiled non-test Cargo artifacts.',
            'Compiler identity, Cargo JSON hashes, feature vectors and effective artifact profiles agree.',
            'Release fat LTO/codegen-units1 declaration is unchanged; build driver removes Cargo profile/Rust flag overrides and wrappers.',
            'Each build driver target directory had to be absent; frozen binaries match receipt bytes and SHA256.',
            'Freeze helper requires full revision, clean source, successful build-finished, checkout-owned source paths, isolated executable path and copy hash stability.'],
        'limits': ['Compiler invocations are ordinary Cargo records, not externally signed build attestations.',
            'The freeze helper validates artifact metadata and source declaration; effective LTO provenance also depends on the reviewed clean build environment.',
            'No new release compilation or performance export is attributed to D by this audit.']}
    OUT.write_text(json.dumps(out, indent=2) + '\n')
    print(json.dumps({'decision': out['decision'], 'source_files_each': [x['tracked_source_files_checked'] for x in builds.values()], 'runtime_artifacts_each': [x['runtime_artifact_count'] for x in builds.values()]}, indent=2))


if __name__ == '__main__':
    main()
