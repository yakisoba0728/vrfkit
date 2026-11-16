"""Compile broken properties, observe independent failures, and restore source.

Run only after the clean binaries and public comparison have been frozen.
Raw replay exports remain local; the JSON receipt is aggregate-only.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from decoder_delta import compare_fields


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser()
    for name in ("source", "target", "frozen", "output", "focus-replay", "focus-baseline", "public-receipt"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    build = json.loads((args.frozen / "receipt.json").read_text())
    completed = json.loads(args.public_receipt.read_text())
    assert completed["complete"] and completed["mismatches"] == 0 and completed["public_replays"] == 10
    assert completed["executable_build_commit"] == build["source_commit"]
    originals = build["runtime_sources_sha256"]
    assert all(sha(args.source / path) == digest for path, digest in originals.items())
    clean_binaries = {name: sha(args.frozen / name) for name in build["binaries"]["artifacts"]}
    assert all(clean_binaries[name] == item["sha256"] for name, item in build["binaries"]["artifacts"].items())
    env = {**os.environ, "CARGO_TARGET_DIR": str(args.target), "CARGO_BUILD_JOBS": "1",
           "CARGO_PROFILE_TEST_OPT_LEVEL": "2", "CARGO_PROFILE_TEST_DEBUG": "0",
           "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_INCREMENTAL": "0", "RUSTUP_TOOLCHAIN": "1.86.0"}
    temporary = args.source / "crates/vrfkit/tests/qa_phase_c_contracts.rs"
    contracts = args.source / "docs/evidence/phase_c/independent_qa/contracts.rs"
    assert sha(contracts) == build["temporary_sources_sha256"][str(contracts.relative_to(args.source))]
    assert not temporary.exists()
    shutil.copyfile(contracts, temporary)
    results = []

    def compile_target(label, cli=False):
        command = ["cargo", "+1.86.0", "build" if cli else "test", "-p", "vrfkit", "--locked", "--all-features", "--message-format=json"]
        command += ["--profile", "test", "--bin", "vrfkit"] if cli else ["--no-run", "--test", "qa_phase_c_contracts", "--test", "gameplay_lifecycle"]
        stdout, stderr = args.output / (label + ".jsonl"), args.output / (label + ".compile.log")
        with stdout.open("w") as out, stderr.open("w") as errors:
            code = subprocess.run(command, cwd=args.source, env=env, stdout=out, stderr=errors).returncode
        assert code == 0, label + " did not compile; not a killed mutation"
        artifacts = {}
        expected = {"vrfkit"} if cli else {"qa_phase_c_contracts", "gameplay_lifecycle"}
        for line in stdout.read_text().splitlines():
            record = json.loads(line)
            if record.get("reason") == "compiler-artifact" and record.get("executable"):
                name = record["target"]["name"]
                if name in expected:
                    path = Path(record["executable"])
                    assert path.is_relative_to(args.target)
                    destination = args.output / (label + "-" + name)
                    shutil.copy2(path, destination)
                    artifacts[name] = destination
        assert set(artifacts) == expected
        return artifacts, {"command": command, "compile_exit": code, "compile_stdout_sha256": sha(stdout),
                           "compile_stderr_sha256": sha(stderr), "binaries_sha256": {key: sha(path) for key, path in artifacts.items()}}

    cases = [
        ("checkpoint_summary_omitted", "crates/vrfkit/src/reader.rs",
         "consumer(ReplayEvent::Checkpoint(CheckpointEvent::Summary(&summary)))?;", "let _ = &summary;",
         "qa_checkpoint_consumer_error_at_each_phase_prevents_later_work"),
        ("same_packet_identity_admitted", "crates/vrfkit/src/gameplay/identity.rs",
         "history.partition_point(|row| row.packet_id < packet)", "history.partition_point(|row| row.packet_id <= packet)",
         "qa_identity_strict_packet_boundary_retains_owned_evidence"),
        ("bounded_eof_probe_omitted", "crates/vrfkit/src/reader.rs",
         "let request = remaining.min(buffer.len()).max(1);", "if remaining == 0 { break; }\n            let request = remaining.min(buffer.len()).max(1);",
         "qa_bounded_checkpoint_callbacks_wait_for_eof_and_caps_consume_one_probe"),
        ("focus_route_suppressed", "crates/vrfkit/src/sink/blobs.rs",
         "if parent_handle != 45", "if parent_handle == 45", None),
    ]
    try:
        for label, relative, old, new, test in cases:
            path = args.source / relative
            before = path.read_bytes()
            assert before.count(old.encode()) == 1
            path.write_bytes(before.replace(old.encode(), new.encode()))
            try:
                artifacts, receipt = compile_target(label, cli=test is None)
                receipt.update(name=label, source_path=relative, clean_source_sha256=hashlib.sha256(before).hexdigest(),
                               mutant_source_sha256=sha(path), replacement={"from": old, "to": new})
                if test is not None:
                    run = subprocess.run([str(artifacts["qa_phase_c_contracts"]), test, "--exact", "--nocapture"],
                                         text=True, capture_output=True)
                    log = args.output / (label + ".run.log")
                    log.write_text(run.stdout + run.stderr)
                    assert run.returncode != 0 and "1 failed" in run.stdout and test + " ... FAILED" in run.stdout
                    receipt.update(detector=test, test_exit=run.returncode, observed_failed_tests=1, run_log_sha256=sha(log))
                else:
                    export = args.output / "focus-mutant-export"
                    log = args.output / "focus-mutant-export.log"
                    command = [str(artifacts["vrfkit"]), "export", str(args.focus_replay), "--out", str(export), "--checkpoints"]
                    with log.open("w") as out:
                        code = subprocess.run(command, stdout=out, stderr=subprocess.STDOUT).returncode
                    assert code == 0, "mutant export crashed; not an oracle rejection"
                    assert sha(export / "fields.parquet") == sha(args.focus_baseline / "fields.parquet")
                    try:
                        compare_fields(args.focus_baseline / "fields.parquet", export / "fields.parquet")
                    except AssertionError as error:
                        rejection = str(error)
                    else:
                        raise AssertionError("independent exact field oracle accepted suppressed route")
                    receipt.update(detector="independent predicted child-row stream", export_exit=code,
                                   rejection=rejection, export_log_sha256=sha(log))
                receipt["detected"] = True
                results.append(receipt)
                (args.output / "progress.json").write_text(json.dumps(results, indent=2) + "\n")
                print(label + ": compiled defect detected", flush=True)
            finally:
                path.write_bytes(before)
            assert all(sha(args.source / path) == digest for path, digest in originals.items())
        artifacts, restoration = compile_target("restored")
        for name, count in (("qa_phase_c_contracts", 9), ("gameplay_lifecycle", 3)):
            run = subprocess.run([str(artifacts[name]), "--nocapture"], text=True, capture_output=True)
            log = args.output / (name + "-restored.log")
            log.write_text(run.stdout + run.stderr)
            assert run.returncode == 0 and f"test result: ok. {count} passed; 0 failed; 0 ignored" in run.stdout
            restoration[name] = {"exit_code": 0, "passed": count, "log_sha256": sha(log)}
    finally:
        assert temporary.read_bytes() == contracts.read_bytes()
        temporary.unlink()
    assert all(sha(args.source / path) == digest for path, digest in originals.items())
    assert clean_binaries == {name: sha(args.frozen / name) for name in clean_binaries}
    receipt = {"source_commit": build["source_commit"], "source_tree": build["source_tree"],
        "mutations_detected": len(results), "mutations": results, "restoration": restoration,
        "runtime_sources_restored": True, "clean_frozen_binaries_unchanged": True,
        "performance_claim": None}
    (args.output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
