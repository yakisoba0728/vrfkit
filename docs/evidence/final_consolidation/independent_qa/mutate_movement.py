"""Compile a stale-identity mutant, then restore and retest the exact source.

Run only in an isolated worktree with reader_malformed_guid.rs installed as
crates/vrfkit/tests/qa_final_guid.rs. No runtime file is changed on completion.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    for name in ("source", "target", "work", "output"):
        parser.add_argument("--" + name, required=True, type=Path)
    args = parser.parse_args()
    assert __debug__
    args.work.mkdir(parents=True, exist_ok=False)
    assert not args.output.exists()
    source = args.source / "crates/vrf-movement/src/rpc.rs"
    test = args.source / "crates/vrfkit/tests/qa_final_guid.rs"
    assert sha(test) == sha(Path(__file__).with_name("reader_malformed_guid.rs"))
    original = source.read_bytes()
    marker = b"                    shooter_guid = None;\n"
    assert original.count(marker) == 1
    original_sha = sha(source)
    command = ["cargo", "+1.86.0", "test", "-p", "vrfkit", "--no-default-features",
               "--locked", "--test", "qa_final_guid"]
    env = dict(os.environ, CARGO_TARGET_DIR=str(args.target), CARGO_BUILD_JOBS="1",
               CARGO_PROFILE_TEST_DEBUG="0", CARGO_PROFILE_DEV_DEBUG="0", CARGO_INCREMENTAL="0")
    env["PATH"] = "/home/agent/.cargo/bin:" + env["PATH"]

    def run(name):
        log = args.work / (name + ".log")
        with log.open("wb") as out:
            code = subprocess.run(command, cwd=args.source, env=env, stdout=out,
                                  stderr=subprocess.STDOUT, timeout=180).returncode
        text = log.read_text()
        assert "Finished `test` profile" in text and "running 1 test" in text, "mutant must compile"
        executables = [p for p in (args.target / "debug/deps").glob("qa_final_guid-*")
                       if p.is_file() and p.suffix != ".d" and os.access(p, os.X_OK)]
        executable = max(executables, key=lambda p: p.stat().st_mtime_ns)
        return {"exit_code": code, "source_sha256": sha(source),
                "log_sha256": sha(log), "test_executable_sha256": sha(executable)}

    baseline = run("before-mutation")
    assert baseline["exit_code"] == 0
    try:
        source.write_bytes(original.replace(marker, b""))
        mutant = run("compiled-mutant")
        assert mutant["exit_code"] == 101
        log = (args.work / "compiled-mutant.log").read_text()
        assert "left: 12" in log and "right: 8" in log
        assert "test result: FAILED. 0 passed; 1 failed" in log
        assert mutant["test_executable_sha256"] != baseline["test_executable_sha256"]
    finally:
        source.write_bytes(original)
        assert sha(source) == original_sha
        restored = run("restored")
        assert restored["exit_code"] == 0
        assert baseline["source_sha256"] == restored["source_sha256"]
        assert baseline["test_executable_sha256"] == restored["test_executable_sha256"]
    result = {"schema_version": 1, "verdict": "PASS",
              "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"],
                  cwd=args.source, text=True).strip(),
              "runner_sha256": sha(Path(__file__)), "test_source_sha256": sha(test),
              "command": command,
              "environment": {key: env[key] for key in ("CARGO_BUILD_JOBS", "CARGO_PROFILE_TEST_DEBUG",
                              "CARGO_PROFILE_DEV_DEBUG", "CARGO_INCREMENTAL")},
              "mutation": "remove only shooter_guid = None from the malformed repeated GUID branch",
              "before": baseline, "compiled_mutant": mutant, "restored": restored,
              "source_and_test_executable_restored_exactly": True,
              "limitations": ["Synthetic public Reader replay tests; no mutation of public corpus inputs.",
                              "Only stale movement identity mutation is measured here."]}
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
