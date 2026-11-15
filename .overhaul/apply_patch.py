"""Transport reviewed text patches; never execute patched code or touch main.

This operational helper stays on its dedicated branch, outside integration.
Only source/docs text can be changed. No secrets, workflow edits, force pushes,
releases or PR merges are supported. Validation runs separately with read-only
permissions after the interactive controller opens the resulting PR.
"""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parent
REPO = 'yakisoba0728/vrfkit'
assert os.environ['GITHUB_REPOSITORY'] == REPO
assert os.environ['GITHUB_REF'] == 'refs/heads/chore/overhaul-review-environment'
request = json.loads((ROOT / 'request.json').read_text())
branch, base = request['branch'], request['base']
assert re.fullmatch(r'(fix|feat|perf|docs)/overhaul-[a-z0-9-]+', branch), 'forbidden branch'
assert re.fullmatch(r'[0-9a-f]{40}', base), 'full base commit required'
message = request['message']
assert isinstance(message, str) and 0 < len(message) < 4000
patch = ROOT / 'change.patch'
assert patch.stat().st_size < 2_000_000
assert hashlib.sha256(patch.read_bytes()).hexdigest() == request['patch_sha256']
token = os.environ.pop('GH_TOKEN')
child_env = dict(os.environ)


def git(*args, cwd=None):
    return subprocess.check_output(['git', *args], cwd=cwd, env=child_env).decode().strip()


def api(path, data=None, method=None):
    req = urllib.request.Request(
        'https://api.github.com/repos/' + REPO + path,
        data=None if data is None else json.dumps(data).encode(),
        headers={'Authorization': 'Bearer ' + token, 'Accept': 'application/vnd.github+json',
                 'Content-Type': 'application/json', 'X-GitHub-Api-Version': '2022-11-28'},
        method=method)
    with urllib.request.urlopen(req, timeout=30) as response:
        return json.load(response)


def remote_head():
    try:
        return api('/git/ref/heads/' + branch)['object']['sha']
    except urllib.error.HTTPError as exc:
        if exc.code == 404:
            return None
        raise


initial = remote_head()
assert initial in (None, base), 'branch moved; rebuild the patch, never overwrite'
work = Path(os.environ['RUNNER_TEMP']) / 'reviewed-patch-work'
git('clone', '--quiet', '--no-checkout', 'https://github.com/' + REPO + '.git', str(work))
git('checkout', '--quiet', '--detach', base, cwd=work)
# Validate paths before git apply; reject binary and symlink/submodule changes.
patch_text = patch.read_text(encoding='utf-8')
assert 'GIT binary patch' not in patch_text
assert not re.search(r'(?:new|old) (?:file )?mode (?:120000|160000)', patch_text)
rows = git('apply', '--numstat', str(patch), cwd=work).splitlines()
assert 0 < len(rows) <= 80
for row in rows:
    added, removed, name = row.split('\t', 2)
    p = PurePosixPath(name)
    assert added.isdecimal() and removed.isdecimal(), 'text patches only'
    assert not p.is_absolute() and '..' not in p.parts and '\\' not in name
    assert p.parts[0] in ('crates', 'dev', 'tools', 'docs') or name in (
        'README.md', 'CONTRIBUTING.md', 'Cargo.toml', 'Cargo.lock', 'requirements.txt')
git('apply', '--check', '--index', str(patch), cwd=work)
git('apply', '--index', str(patch), cwd=work)
names = git('diff', '--cached', '--name-only', '--no-renames', cwd=work).splitlines()
assert len(names) == len(rows), 'unexpected changed-file set'
entries = []
for name in names:
    path = work / name
    if not path.exists():
        entries.append({'path': name, 'mode': '100644', 'type': 'blob', 'sha': None})
        continue
    assert not path.is_symlink() and path.is_file()
    content = path.read_bytes()
    assert len(content) < 2_000_000
    mode = git('ls-files', '-s', '--', name, cwd=work).split()[0]
    assert mode in ('100644', '100755')
    entries.append({'path': name, 'mode': mode, 'type': 'blob', 'content': content.decode('utf-8')})
parent = api('/git/commits/' + base)
tree = api('/git/trees', {'base_tree': parent['tree']['sha'], 'tree': entries})
commit = api('/git/commits', {'message': message, 'tree': tree['sha'], 'parents': [base]})
assert remote_head() == initial, 'branch moved during preparation'
if initial is None:
    api('/git/refs', {'ref': 'refs/heads/' + branch, 'sha': commit['sha']})
else:
    api('/git/refs/heads/' + branch, {'sha': commit['sha'], 'force': False}, 'PATCH')
print(json.dumps({'branch': branch, 'base': base, 'head': commit['sha'], 'files': names}))
