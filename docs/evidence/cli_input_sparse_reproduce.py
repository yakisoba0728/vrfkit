"""Linux sparse-input proof; arguments: baseline candidate new-work-directory.

Creates a synthetic 2 GiB sparse input and runs both CLIs with a 512 MiB
address-space limit. This proves skipped input retention, not decoded memory.
"""

from pathlib import Path
import argparse
import hashlib
import json
import resource
import struct
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('baseline', type=Path)
parser.add_argument('candidate', type=Path)
parser.add_argument('work', type=Path)
args = parser.parse_args()
baseline = args.baseline.resolve(strict=True)
candidate = args.candidate.resolve(strict=True)
root = args.work.resolve()
root.mkdir(mode=0o700)
work = root / 'sparse-proof'
work.mkdir(mode=0o700)

def pack(fmt, *values):
    return struct.pack('<' + fmt, *values)

def string(text):
    value = text.encode('ascii') + b'\0'
    return pack('i', len(value)) + value

info = pack('IIi', 0x43F4EFDD, 7, 1)
info += pack('IIIIi', 0x95A4F03E, 0x7E0B49E4, 0xBA43D356, 0x94FF87D9, 7)
info += pack('iII', 60000, 19, 1234) + string('Match')
info += pack('IqIIi', 0, 42, 0, 0, 0)
header = pack('IIiIII', 0x2CF5A13D, 19, 0, 0x11223344, 32, 0x55667788)
header += pack('IIIIHHHI', 0x00112233, 0x44556677, 0x8899AABB, 0xCCDDEEFF, 12, 10, 1, 123456)
header += string('++Ares-Core+release-12.10') + pack('i', 3) + b'18\0'
header += pack('IIIi', 1001, 1002, 1003, 1) + string('Ascent')
header += pack('IIi', 42, 0b1010, 2) + string('valorant') + string('competitive')
header += pack('ffff', 15.0, 30.0, 33.3, 250.0) + string('Windows') + bytes([7, 3])
replay = work / 'skipped-2gib.vrf'
with replay.open('wb') as f:
    f.write(info + pack('Ii', 0, len(header)) + header)
    for _ in range(2):
        f.write(pack('Ii', 99, 1024 * 1024 * 1024))
        f.seek(1024 * 1024 * 1024, 1)
    f.truncate()

def limit():
    bound = 512 * 1024 * 1024
    resource.setrlimit(resource.RLIMIT_AS, (bound, bound))

rows = []
for label, executable, args, expected in [
    ('baseline-validate', baseline, ['validate', str(replay)], 1),
    ('candidate-validate', candidate, ['validate', str(replay)], 2),
    ('candidate-export-checkpoints', candidate, ['export', str(replay), '--out', str(work/'export'), '--checkpoints'], 0),
]:
    run = subprocess.run([str(executable), *args], capture_output=True, preexec_fn=limit, timeout=60)
    (work/(label+'.log')).write_bytes(run.stdout + run.stderr)
    text = (run.stdout + run.stderr).decode(errors='replace')
    if run.returncode != expected:
        raise RuntimeError(label + ': unexpected exit ' + str(run.returncode) + ': ' + text)
    if label == 'baseline-validate' and not ('memory' in text.lower() or 'allocat' in text.lower()):
        raise RuntimeError('baseline failure is not an allocation failure: ' + text)
    if label == 'candidate-validate' and ('No content blocks found' not in text or 'Unknown chunks:       2' not in text):
        raise RuntimeError('candidate did not walk both sparse chunks: ' + text)
    rows.append({'case': label, 'exit_code': run.returncode, 'expected_exit': expected,
                 'log_sha256': hashlib.sha256(run.stdout+run.stderr).hexdigest()})
    print(label, 'passed', flush=True)
report = {'status': 'passed', 'address_space_limit_bytes': 512*1024*1024,
          'input_bytes': replay.stat().st_size,
          'scope': 'Two 1GiB sparse unknown chunks, no decoded data. Validates skipped-input retention only, not a decoded chunk memory bound.',
          'baseline_binary_sha256': hashlib.sha256(baseline.read_bytes()).hexdigest(),
          'candidate_binary_sha256': hashlib.sha256(candidate.read_bytes()).hexdigest(),
          'checks': rows}
(root/'cli-sparse-proof.json').write_text(json.dumps(report, indent=2)+'\n')
replay.unlink()
