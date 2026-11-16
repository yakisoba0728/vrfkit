"""Install isolated QA adapters and verify an immutable candidate checkout.

Only untracked QA tests/examples are installed. Product files must remain
identical to HEAD. The 43 selected tests include 28 maintained tests, 9 retained
independent contracts, and 6 newly authored independent regressions.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser()
    for name in ("source", "target", "work", "output"):
        parser.add_argument("--" + name, required=True, type=Path)
    args = parser.parse_args()
    assert __debug__
    args.work.mkdir(parents=True, exist_ok=False)
    assert not args.output.exists()

    def git(*command):
        return subprocess.check_output(["git", *command], cwd=args.source, text=True).strip()

    assert not git("diff", "HEAD", "--", "Cargo.lock", "Cargo.toml", "crates")
    source_commit = git("rev-parse", "HEAD")
    tracked = git("ls-files", "Cargo.lock", "Cargo.toml", "crates").splitlines()
    fingerprints = {p: sha(args.source / p) for p in tracked if p.endswith((".rs", "Cargo.toml", "Cargo.lock"))}
    sources = {
        "docs/evidence/final_consolidation/independent_qa/reader_malformed_guid.rs": "crates/vrfkit/tests/qa_final_guid.rs",
        "docs/evidence/final_consolidation/independent_qa/net_payload_bounds.rs": "crates/vrf-net/tests/qa_final_payload_bounds.rs",
        "docs/evidence/final_consolidation/independent_qa/schema_public_budget.rs": "crates/vrf-schema/tests/qa_final_path_budget.rs",
        "docs/evidence/phase_c/independent_qa/contracts.rs": "crates/vrfkit/tests/qa_phase_c_contracts.rs",
        "docs/evidence/phase_c/independent_qa/identity_probe.rs": "crates/vrfkit/examples/qa_final_identity.rs",
    }
    adapter_hashes = {}
    for origin, target in sources.items():
        src, dest = args.source / origin, args.source / target
        assert not dest.exists() or sha(dest) == sha(src)
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src, dest)
        adapter_hashes[target] = {"origin": origin, "sha256": sha(dest)}
    env = dict(os.environ, CARGO_TARGET_DIR=str(args.target), CARGO_BUILD_JOBS="1",
               CARGO_PROFILE_TEST_DEBUG="0", CARGO_PROFILE_DEV_DEBUG="0", CARGO_INCREMENTAL="0")
    env["PATH"] = "/home/agent/.cargo/bin:" + env["PATH"]
    test_targets = {"qa_final_payload_bounds": 3, "qa_final_path_budget": 2,
                    "gameplay_lifecycle": 3, "movement_reader": 3, "qa_final_guid": 1,
                    "qa_phase_c_contracts": 9, "reader_bounded": 11, "reader_checkpoints": 11}
    tests = ["cargo", "+1.86.0", "test", "-p", "vrf-net", "-p", "vrf-schema", "-p", "vrfkit",
             "--no-default-features", "--locked"]
    for target in test_targets:
        tests += ["--test", target]
    tests += ["--", "--test-threads=1"]
    build = ["cargo", "+1.86.0", "build", "-p", "vrfkit", "--no-default-features",
             "--locked", "--example", "qa_final_identity"]
    runs = []
    for name, command in (("contracts", tests), ("identity-build", build)):
        log = args.work / (name + ".log")
        with log.open("wb") as out:
            code = subprocess.run(command, cwd=args.source, env=env, stdout=out,
                                  stderr=subprocess.STDOUT, timeout=900).returncode
        assert code == 0, name + " failed; inspect recorded log"
        runs.append({"name": name, "command": command, "exit_code": code, "log_sha256": sha(log)})
    text = (args.work / "contracts.log").read_text()
    observed = {}
    current = None
    for line in text.splitlines():
        match = re.search(r"Running tests/([^.]+)\.rs", line)
        if match:
            current = match[1]
        match = re.search(r"test result: ok\. (\d+) passed; 0 failed; 0 ignored;", line)
        if match:
            assert current is not None and current not in observed
            observed[current] = int(match[1])
    assert observed == test_targets
    probe = args.work / "identity-probe"
    shutil.copyfile(args.target / "debug/examples/qa_final_identity", probe)
    probe.chmod(0o555)
    assert fingerprints == {p: sha(args.source / p) for p in fingerprints}
    assert not git("diff", "HEAD", "--", "Cargo.lock", "Cargo.toml", "crates")
    receipt = {"schema_version": 1, "verdict": "PASS", "source_commit": source_commit,
               "source_tree": git("rev-parse", "HEAD^{tree}"), "runner_sha256": sha(Path(__file__)),
               "rustc": subprocess.check_output(["rustc", "+1.86.0", "-Vv"], env=env, text=True),
               "environment": {key: env[key] for key in ("CARGO_BUILD_JOBS", "CARGO_PROFILE_TEST_DEBUG",
                                "CARGO_PROFILE_DEV_DEBUG", "CARGO_INCREMENTAL")},
               "source_file_sha256": fingerprints, "installed_adapters": adapter_hashes,
               "runs": runs, "tests_passed": sum(observed.values()), "test_targets": observed,
               "maintained_tests": 28, "retained_independent_tests": 9, "new_independent_tests": 6,
               "identity_probe_sha256": sha(probe), "tracked_source_unchanged": True,
               "limitations": ["Linux Rust1.86 no-default-features selected regression execution.",
                               "Full regular workspace/feature/Windows gates are separate and these28maintained tests overlap.",
                               "Only untracked QA adapter files are installed; original tracked files remain unchanged."]}
    args.output.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({key: value for key, value in receipt.items()
                      if key not in ("source_file_sha256", "installed_adapters", "runs")}, indent=2))


if __name__ == "__main__":
    main()
