"""Sample a direct child and its threads with Linux software CPU-clock events.

Diagnostic only. Standard library; no privilege or kernel policy changes.
The raw report contains local executable mappings and must remain private.
"""
import argparse
import ctypes
import fcntl
import json
import mmap
import os
from pathlib import Path
import platform
import signal
import struct
import time

p = argparse.ArgumentParser()
p.add_argument('--report', required=True, type=Path)
p.add_argument('--log', required=True, type=Path)
p.add_argument('command', nargs=argparse.REMAINDER)
a = p.parse_args()
if platform.system() != 'Linux' or platform.machine() not in ('x86_64', 'amd64'):
    p.error('This diagnostic uses the Linux x86-64 perf_event_open syscall number')
if not a.command:
    p.error('An executable command is required after --')
command = a.command[1:] if a.command[0] == '--' else a.command
if not command or not Path(command[0]).is_file():
    p.error('An existing executable path is required')
if a.report.exists() or a.log.exists():
    p.error('Refusing existing output paths')
libc = ctypes.CDLL(None, use_errno=True)
attr = ctypes.create_string_buffer(128)
# disabled, inherit, exclude_kernel, exclude_hv, mmap, freq, enable_on_exec.
flags = 1 | (1 << 1) | (1 << 5) | (1 << 6) | (1 << 8) | (1 << 10) | (1 << 12)
struct.pack_into('IIQQQQQ', attr, 0, 1, 128, 0, 1999, 3, 0, flags)
pid = os.fork()
if pid == 0:
    os.kill(os.getpid(), signal.SIGSTOP)
    fd = os.open(a.log, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    os.dup2(fd, 1)
    os.dup2(fd, 2)
    os.close(fd)
    os.execv(command[0], command)
os.waitpid(pid, os.WUNTRACED)
page = mmap.PAGESIZE
ring_size = page * 128
rings = []
try:
    for cpu in sorted(os.sched_getaffinity(0)):
        fd = libc.syscall(298, ctypes.byref(attr), pid, cpu, -1, 0)
        if fd < 0:
            err = ctypes.get_errno()
            raise OSError(err, os.strerror(err))
        ring = mmap.mmap(fd, page + ring_size, mmap.MAP_SHARED, mmap.PROT_READ | mmap.PROT_WRITE)
        rings.append([fd, ring, 0])
except BaseException:
    os.kill(pid, signal.SIGKILL)
    os.waitpid(pid, 0)
    for fd, ring, _ in rings:
        ring.close()
        os.close(fd)
    raise
samples = []
maps = []
lost = 0
other = {}

def drain_one(entry):
    global lost
    fd, ring, tail = entry
    head = struct.unpack_from('Q', ring, 1024)[0]
    while tail < head:
        offset = tail % ring_size
        header = ring[page + offset:page + offset + 8]
        if len(header) < 8:
            header += ring[page:page + 8 - len(header)]
        typ, misc, size = struct.unpack('IHH', header)
        data = ring[page + offset:page + min(offset + size, ring_size)]
        if offset + size > ring_size:
            data += ring[page:page + offset + size - ring_size]
        if typ == 9:
            ip, sample_pid, tid = struct.unpack_from('QII', data, 8)
            samples.append([ip, sample_pid, tid])
        elif typ == 1:
            m_pid, tid, addr, length, pgoff = struct.unpack_from('IIQQQ', data, 8)
            filename = data[40:].split(b'\0', 1)[0].decode(errors='replace')
            maps.append([m_pid, tid, addr, length, pgoff, filename])
        elif typ == 2:
            lost += struct.unpack_from('Q', data, 16)[0]
        else:
            other[typ] = other.get(typ, 0) + 1
        tail += size
    struct.pack_into('Q', ring, 1032, tail)
    entry[2] = tail

def drain():
    for entry in rings:
        drain_one(entry)

started = time.monotonic()
os.kill(pid, signal.SIGCONT)
while True:
    drain()
    done, status = os.waitpid(pid, os.WNOHANG)
    if done:
        break
    if time.monotonic() - started > 120:
        os.kill(pid, signal.SIGKILL)
        _, status = os.waitpid(pid, 0)
        break
    time.sleep(.002)
for fd, _, _ in rings:
    fcntl.ioctl(fd, 0x2401, 0)  # PERF_EVENT_IOC_DISABLE
drain()
for fd, ring, _ in rings:
    ring.close()
    os.close(fd)
report = {'event': 'PERF_COUNT_SW_CPU_CLOCK', 'frequency_hz': 1999,
          'inherit_threads': True, 'exclude_kernel': True,
          'exit_code': os.waitstatus_to_exitcode(status),
          'wall_seconds_instrumented': time.monotonic() - started,
          'lost': lost, 'samples': samples, 'maps': maps, 'other_records': other}
a.report.write_text(json.dumps(report) + '\n')
print(json.dumps({k: v for k, v in report.items() if k not in ('samples', 'maps')}, indent=2))
print(f'Samples: {len(samples)}, maps: {len(maps)}, thread IDs: {len(set(s[2] for s in samples))}')
