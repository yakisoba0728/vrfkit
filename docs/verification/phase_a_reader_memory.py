#!/usr/bin/env python3
"""Run the generated Reader sparse-input proof under a Linux address limit."""

import argparse
import hashlib
import json
import resource
import subprocess
import sys
from pathlib import Path


LIMIT = 512 * 1024 * 1024
EXPECTED = "callbacks=2 events=1 unknown_chunks=1 skipped_checkpoints=1 frames=0"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def constrain():
    resource.setrlimit(resource.RLIMIT_AS, (LIMIT, LIMIT))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path)
    parser.add_argument("fixture", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--worker", choices=("file", "buffered"))
    args = parser.parse_args()
    assert sys.platform == "linux", "this proof requires Linux resource limits"
    if args.worker:
        run = subprocess.run([str(args.executable), args.worker, str(args.fixture)],
                             capture_output=True, text=True, timeout=15,
                             preexec_fn=constrain)
        usage = resource.getrusage(resource.RUSAGE_CHILDREN)
        print(json.dumps({"mode": args.worker, "exit_code": run.returncode,
                          "stdout": run.stdout.strip(), "stderr": run.stderr.strip(),
                          "peak_rss_bytes": usage.ru_maxrss * 1024}))
        return
    stat = args.fixture.stat()
    assert stat.st_size > 2 * 1024 * 1024 * 1024
    assert stat.st_blocks * 512 < 32768, "input must be a generated sparse fixture"
    before = digest(args.executable)
    runs = []
    for mode in ("file", "buffered"):
        worker = subprocess.run([sys.executable, __file__, str(args.executable),
                                 str(args.fixture), str(args.output), "--worker", mode],
                                capture_output=True, text=True, check=True, timeout=20)
        runs.append(json.loads(worker.stdout))
    assert runs[0]["exit_code"] == 0 and runs[0]["stdout"] == EXPECTED
    assert runs[1]["exit_code"] != 0
    assert runs[1]["stderr"] == "Error: Kind(OutOfMemory)", runs[1]
    assert digest(args.executable) == before
    after = args.fixture.stat()
    assert (after.st_size, after.st_mtime_ns) == (stat.st_size, stat.st_mtime_ns)
    report = {"schema_version": 1, "status": "passed",
              "address_limit_bytes": LIMIT, "fixture_logical_bytes": stat.st_size,
              "fixture_allocated_bytes": stat.st_blocks * 512,
              "executable_sha256": before, "script_sha256": digest(Path(__file__)),
              "runs": runs,
              "scope": "Synthetic skipped-payload memory-bound proof on Linux; not an ordinary-replay speed benchmark or a global Reader memory cap."}
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
