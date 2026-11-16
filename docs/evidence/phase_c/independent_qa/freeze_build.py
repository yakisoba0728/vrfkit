"""Build and freeze exact combined-source functional verification artifacts.

The Cargo test profile retains debug assertions at opt-level 2. This records
source and executable correspondence; it does not measure performance.
"""
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
    for name in ("source", "target", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    args.source, args.target, args.output = (path.resolve() for path in (args.source, args.target, args.output))
    args.output.mkdir(parents=True, exist_ok=False)
    env = {**os.environ, "CARGO_TARGET_DIR": str(args.target), "CARGO_BUILD_JOBS": "1",
        "CARGO_PROFILE_TEST_OPT_LEVEL": "2", "CARGO_PROFILE_TEST_DEBUG": "0", "CARGO_INCREMENTAL": "0",
        "CARGO_PROFILE_DEV_DEBUG": "0", "RUSTUP_TOOLCHAIN": "1.86.0"}

    def git(*command):
        return subprocess.check_output(["git", *command], cwd=args.source, text=True).strip()

    def runtime_sources():
        return {name: sha(args.source / name) for name in git("ls-files").splitlines()
                if name in ("Cargo.toml", "Cargo.lock") or (name.startswith("crates/") and
                   (name.endswith("Cargo.toml") or ("/src/" in name and name.endswith(".rs"))))}

    def build(label, command, expected):
        stdout, stderr = args.output / (label + ".jsonl"), args.output / (label + ".log")
        with stdout.open("w") as out, stderr.open("w") as errors:
            code = subprocess.run(command, cwd=args.source, env=env, stdout=out, stderr=errors).returncode
        assert code == 0, label + " compile failure"
        found = {}
        for line in stdout.read_text().splitlines():
            record = json.loads(line)
            if record.get("reason") != "compiler-artifact" or not record.get("executable"):
                continue
            name = record["target"]["name"]
            if name not in expected:
                continue
            executable = Path(record["executable"])
            assert executable.is_relative_to(args.target)
            source = Path(record["target"]["src_path"])
            assert source.is_relative_to(args.source)
            destination = args.output / name
            shutil.copy2(executable, destination)
            dependency_file = executable.with_suffix(".d")
            dependencies = []
            if dependency_file.exists():
                for item in dependency_file.read_text().splitlines()[0].split(": ", 1)[1].split():
                    path = Path(item)
                    if not path.is_absolute():
                        path = args.source / path
                    if "/crates/" in str(path) and "/registry/" not in str(path):
                        dependencies.append(path)
                assert dependencies and all(path.is_relative_to(args.source) and path.is_file() for path in dependencies)
            found[name] = {"sha256": sha(destination), "bytes": destination.stat().st_size,
                "target_source": str(source.relative_to(args.source)), "profile": record["profile"],
                "cargo_fresh": record["fresh"], "source_sha256": sha(source),
                "workspace_dependencies_sha256": {str(path.relative_to(args.source)): sha(path) for path in dependencies}}
        assert set(found) == set(expected), (label, set(found))
        print(label + ": exact artifacts frozen", flush=True)
        return {"command": command, "exit_code": code, "stdout_sha256": sha(stdout), "stderr_sha256": sha(stderr), "artifacts": found}

    assert subprocess.run(["git", "diff", "HEAD", "--quiet"], cwd=args.source).returncode == 0
    source_commit, source_tree, before = git("rev-parse", "HEAD"), git("rev-parse", "HEAD^{tree}"), runtime_sources()
    qa = args.source / "docs/evidence/phase_c/independent_qa"
    temporary = {
        "crates/vrfkit/examples/qa_checkpoint_probe.rs": qa / "checkpoint_probe.rs",
        "crates/vrfkit/examples/qa_identity_probe.rs": qa / "identity_probe.rs",
        "crates/vrfkit/examples/qa_gameplay_probe.rs": args.source / "docs/evidence/phase_b/independent_qa/gameplay_probe.rs",
        "crates/vrfkit/tests/qa_phase_c_contracts.rs": qa / "contracts.rs",
    }
    for destination, origin in temporary.items():
        assert not (args.source / destination).exists()
        shutil.copyfile(origin, args.source / destination)
    try:
        command = ["cargo", "+1.86.0", "build", "--profile", "test", "-p", "vrfkit", "--all-features", "--locked", "--message-format=json", "--bin", "vrfkit"]
        examples = ["qa_checkpoint_probe", "qa_identity_probe", "qa_gameplay_probe", "movement_provenance"]
        for name in examples:
            command += ["--example", name]
        binaries = build("binaries", command, ["vrfkit", *examples])
        suites = {"qa_phase_c_contracts": (9, "independently authored by E"),
                  "gameplay_lifecycle": (3, "external public API regression tests independently rerun"),
                  "reader_checkpoints": (11, "A author tests independently rerun"),
                  "reader_bounded": (11, "D author tests independently rerun")}
        command = ["cargo", "+1.86.0", "test", "-p", "vrfkit", "--all-features", "--locked", "--no-run", "--message-format=json"]
        for name in suites:
            command += ["--test", name]
        contracts = build("contracts", command, list(suites))
        test_counts = {}
        for name, (expected, origin) in suites.items():
            result = subprocess.run([str(args.output / name), "--nocapture"], cwd=args.source,
                                    env=env, text=True, capture_output=True)
            (args.output / (name + "-run.log")).write_text(result.stdout + result.stderr)
            assert result.returncode == 0, "independent contract failure"
            counts = re.search(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out", result.stdout)
            assert counts is not None and int(counts[1]) == expected and all(int(counts[i]) == 0 for i in range(2, 6))
            test_counts[name] = {"passed": int(counts[1]), "failed": 0, "ignored": 0, "origin": origin,
                                 "run_log_sha256": sha(args.output / (name + "-run.log"))}
    finally:
        for destination, origin in temporary.items():
            assert (args.source / destination).read_bytes() == origin.read_bytes()
            (args.source / destination).unlink()
    assert runtime_sources() == before
    receipt = {"source_commit": source_commit, "source_tree": source_tree, "runtime_sources_sha256": before,
        "rustc": subprocess.check_output(["rustc", "+1.86.0", "-Vv"], env=env, text=True).strip(),
        "profile": "Cargo test profile opt-level2, debug assertions enabled, no debuginfo/incremental; not release-performance build",
        "temporary_sources_sha256": {str(origin.relative_to(args.source)): sha(origin) for origin in temporary.values()},
        "binaries": binaries, "contracts": contracts, "independent_contracts": test_counts,
        "independently_authored_contracts_passed": test_counts["qa_phase_c_contracts"]["passed"],
        "total_focused_contracts_passed": sum(row["passed"] for row in test_counts.values()), "tracked_runtime_unchanged": True,
        "performance_claim": None}
    (args.output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print("focused contracts passed: " + str(receipt["total_focused_contracts_passed"]), flush=True)


if __name__ == "__main__":
    main()
