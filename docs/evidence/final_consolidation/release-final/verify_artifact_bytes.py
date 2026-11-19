"""Independent local checks of downloaded GitHub artifacts; never executes an EXE."""
import argparse
import hashlib
import io
import json
from pathlib import Path, PureWindowsPath
import re
import struct
import subprocess
import tarfile
import zipfile


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--repo', type=Path, required=True)
    ap.add_argument('--sha', required=True)
    ap.add_argument('--run-head-sha', help='run head SHA when --sha is the PR synthetic checkout')
    ap.add_argument('--artifacts', type=Path, required=True)
    ap.add_argument('--directory', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    args = ap.parse_args()
    def git(*extra):
        return subprocess.check_output(['git', '-C', str(args.repo), *extra])
    tree = git('rev-parse', args.sha + '^{tree}').decode().strip()
    entries = {}
    for row in git('ls-tree', '-r', '-z', args.sha).split(b'\0'):
        if row:
            header, name = row.split(b'\t', 1)
            mode, kind, blob = header.decode().split()
            assert kind == 'blob', kind
            entries[name.decode()] = {'mode': mode, 'blob': blob}
    files = {name: git('cat-file', 'blob', e['blob']) for name, e in entries.items()}
    metadata = json.loads(args.artifacts.read_text())
    archive_checks = []
    bundles = {}
    for a in metadata:
        assert a['workflow_run']['head_sha'] == (args.run_head_sha or args.sha)
        p = args.directory / (a['name'] + '.zip')
        b = p.read_bytes()
        assert len(b) == a['size_in_bytes'], (a['name'], 'size')
        assert 'sha256:' + digest(b) == a['digest'], (a['name'], 'digest')
        archive_checks.append({'name': a['name'], 'id': a['id'], 'size': len(b),
                               'sha256': digest(b), 'url': a['url'],
                               'expires_at': a['expires_at']})
        with zipfile.ZipFile(io.BytesIO(b)) as z:
            assert len(set(z.namelist())) == len(z.namelist()), 'duplicate artifact entry'
            bundles[a['name']] = {n: z.read(n) for n in z.namelist()}
    package = bundles['windows-release']
    tag = 'v0.0.0-ci.' + args.sha
    inner_name = 'vrfkit-' + tag + '-windows-x64.zip'
    assert set(package) == {inner_name, inner_name + '.sha256'}
    inner = package[inner_name]
    assert package[inner_name + '.sha256'].decode('ascii') == digest(inner) + '  ' + inner_name + '\n'
    with zipfile.ZipFile(io.BytesIO(inner)) as z:
        assert len(z.namelist()) == 4 and set(z.namelist()) == {'vrfkit.exe', 'LICENSE', 'NOTICE.md', 'build-info.json'}
        payload = {n: z.read(n) for n in z.namelist()}
    exe = payload['vrfkit.exe']
    info = json.loads(payload['build-info.json'])
    assert info == {'tag': tag, 'commit': args.sha, 'target': 'x86_64-pc-windows-msvc', 'executable_sha256': digest(exe)}
    assert exe[:2] == b'MZ'
    pe_offset = struct.unpack_from('<I', exe, 60)[0]
    assert exe[pe_offset:pe_offset + 4] == b'PE\0\0'
    assert struct.unpack_from('<H', exe, pe_offset + 4)[0] == 0x8664
    assert struct.unpack_from('<H', exe, pe_offset + 24)[0] == 0x20b
    legal_checks = {}
    for name in ['LICENSE', 'NOTICE.md']:
        assert payload[name].replace(b'\r\n', b'\n') == files[name].replace(b'\r\n', b'\n')
        legal_checks[name] = {'archive_sha256': digest(payload[name]),
                              'git_sha256': digest(files[name]),
                              'normalized_eol_equal': True}
    report_bundle = bundles['release-replay-verification']
    report = json.loads(report_bundle['release-report.json'])
    prov = report['provenance']
    assert prov['parser_commit'] == args.sha
    assert prov['exe_sha256'] == digest(exe)
    assert report['executable_changed'] is False
    assert not report['build_errors'] and not report['failures']
    paths = sorted((n for n in files if n.startswith('crates/') and n.endswith('.rs')), key=PureWindowsPath)
    paths += sorted((n for n in files if n.startswith('crates/') and n.endswith('/Cargo.toml')), key=PureWindowsPath)
    paths += ['Cargo.lock', 'Cargo.toml']
    source_digests = {}
    for eol in ['LF', 'CRLF']:
        normalized = {p: files[p].replace(b'\r\n', b'\n') for p in paths}
        if eol == 'CRLF':
            normalized = {p: b.replace(b'\n', b'\r\n') for p, b in normalized.items()}
        source_digests[eol] = digest('\n'.join(p + ' ' + digest(normalized[p]) for p in paths).encode())
    assert prov['parser_sources_sha256'] in source_digests.values(), (prov['parser_sources_sha256'], source_digests)
    matched_eol = next(eol for eol, d in source_digests.items() if d == prov['parser_sources_sha256'])
    def expected_file_hash(name):
        b = files[name].replace(b'\r\n', b'\n')
        return digest(b if matched_eol == 'LF' else b.replace(b'\n', b'\r\n'))
    assert prov['runner_sha256'] == expected_file_hash('tools/verify_build_corpus.py')
    assert prov['evidence_sha256'] == expected_file_hash('tools/fixtures/public_fixture_type_evidence.json')
    assert prov['unique_replays'] == 1 and prov['duplicates'] == 0
    assert set(report['builds']) == {'++Ares-Core+release-12.10'}
    fixture_match = re.search(rb'^  FIXTURE_1210_SHA256: ([a-f0-9]{64})$', files['.github/workflows/ci.yml'], re.M)
    assert fixture_match, 'cannot recover the pinned public fixture hash'
    builds = []
    for name, row in report['builds'].items():
        assert row['failed'] == 0 and row['passed'] == row['replays'] == 1
        assert row['input_sha256'] == [fixture_match[1].decode('ascii')]
        assert row['checkpoint_evidence'] == 'observed'
        counts = row['counts']
        assert counts['checkpoint_chunks'] > 0 and counts['typed_values_compared'] > 0
        assert counts['typed_mismatches'] == 0 and counts['type_width_failures'] == 0
        builds.append({'branch': name, 'replays': row['replays'], 'passed': row['passed'],
                       'failed': row['failed'], 'input_sha256': row['input_sha256'],
                       'checkpoints': counts['checkpoint_chunks'],
                       'typed_values_compared': counts['typed_values_compared'],
                       'typed_mismatches': counts['typed_mismatches'],
                       'type_width_failures': counts['type_width_failures'],
                       'table_count': len(row['tables'])})
    source_check = None
    if 'vrfkit-source' in bundles:
        b = bundles['vrfkit-source']
        assert b['source-commit.txt'].decode().strip() == args.sha
        tar = b['vrfkit-source.tar']
        assert b['source-sha256.txt'].decode().split()[0] == digest(tar)
        expected = {n for n in files if not n.lower().endswith(('.vrf', '.replay', '.parquet'))}
        seen = set()
        with tarfile.open(fileobj=io.BytesIO(tar)) as archive:
            for member in archive:
                if member.isdir():
                    continue
                n = member.name
                assert n in expected and n not in seen, n
                seen.add(n)
                assert member.isfile(), (n, 'non-file')
                assert archive.extractfile(member).read() == files[n], (n, 'content')
                assert bool(member.mode & 0o111) == (entries[n]['mode'] == '100755'), (n, 'executable mode')
        assert seen == expected, sorted(expected - seen)
        source_check = {'tar_sha256': digest(tar), 'files_matched': len(seen),
                        'all_included_git_bytes_and_executable_bits_match': True,
                        'excluded_extensions': ['.vrf', '.replay', '.parquet']}
    receipt = {'source_sha': args.sha, 'tree_sha': tree, 'artifact_archives': archive_checks,
               'package': {'name': inner_name, 'zip_sha256': digest(inner), 'size': len(inner),
                            'executable_size': len(exe), 'build_info': info,
                            'pe_machine': '0x8664 AMD64', 'optional_header': '0x020b PE32+',
                            'sidecar_verified': True, 'legal_files': legal_checks},
               'hosted_replay': {'report_sha256': digest(report_bundle['release-report.json']),
                                 'provenance': prov, 'builds': builds, 'failures': report['failures'],
                                 'executable_unchanged': True, 'source_files': len(paths),
                                 'git_source_digest_by_eol': source_digests,
                                 'matching_checkout_eol': matched_eol},
               'source_archive': source_check,
               'local_windows_execution': 'NOT_EXECUTED: PE bytes checked locally; actual Windows execution is in hosted job logs'}
    args.output.write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps({'source_sha': args.sha, 'artifact_count': len(archive_checks),
                      'zip_sha256': digest(inner), 'exe_sha256': digest(exe),
                      'builds': builds, 'source_archive': source_check, 'receipt': str(args.output)}, indent=2))


if __name__ == '__main__':
    main()
