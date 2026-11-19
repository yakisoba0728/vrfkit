"""Exercise CLI input preservation and bounded malformed-file failures.

Only disposable copies of one verified public replay are used. No production
replay is modified. Detailed diagnostic logs stay in the requested work root.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def snapshot(path):
    return {str(p.relative_to(path)): sha(p) for p in sorted(path.rglob("*")) if p.is_file()}


def main():
    parser = argparse.ArgumentParser()
    for name in ("exe", "replay", "work", "output"):
        parser.add_argument("--" + name, required=True, type=Path)
    parser.add_argument("--observe-baseline", action="store_true")
    args = parser.parse_args()
    assert __debug__
    args.work.mkdir(parents=True, exist_ok=False)
    initial_input_hash, initial_binary_hash = sha(args.replay), sha(args.exe)
    raw = args.replay.read_bytes()
    rows = []

    def run(label, command):
        process = subprocess.run([str(args.exe), *map(str, command)], capture_output=True,
                                 timeout=60)
        log = args.work / (label + ".log")
        log.write_bytes(process.stdout + process.stderr)
        assert b"panicked at" not in process.stderr
        return process.returncode, sha(log)

    def protected(label, filename="fields.parquet", *, alias=None, checkpoints=False):
        root = args.work / label
        root.mkdir()
        out = root / "export"
        out.mkdir()
        source = out / filename
        source.write_bytes(raw)
        before = snapshot(out)
        argument_source, argument_out = source, out
        if alias == "source-parent":
            link = root / "source-alias"
            link.symlink_to(out, target_is_directory=True)
            argument_source = link / filename
        elif alias == "destination":
            link = root / "destination-alias"
            link.symlink_to(out, target_is_directory=True)
            argument_out = link
        command = ["export", argument_source, "--out", argument_out]
        if checkpoints:
            command.append("--checkpoints")
        code, log = run(label, command)
        unchanged = snapshot(out) == before
        leftovers = [p for p in root.iterdir() if ".vrfkit-" in p.name]
        row = {"case": label, "exit_code": code, "input_and_destination_unchanged": unchanged,
               "no_staging_or_backup_leftovers": not leftovers, "log_sha256": log,
               "accepted": code == 1 and unchanged and not leftovers}
        rows.append(row)
        if not args.observe_baseline:
            assert row["accepted"], label

    protected("input-as-main-table")
    protected("input-as-main-table-checkpoints", checkpoints=True)
    protected("input-as-manifest", "manifest.json")
    protected("input-as-checkpoint-table", "checkpoint_fields.parquet", checkpoints=True)
    protected("input-via-source-parent-alias", alias="source-parent")
    protected("input-via-destination-alias", alias="destination")
    if not args.observe_baseline:
        for hardlink in (False, True):
            label = "legal-outside-hardlink" if hardlink else "legal-sibling-output"
            root = args.work / label
            root.mkdir()
            source = root / "fields.parquet"
            source.write_bytes(raw)
            out = root / "export"
            out.mkdir()
            if hardlink:
                os.link(source, out / "fields.parquet")
            code, log = run(label, ["export", source, "--out", out, "--checkpoints"])
            assert code == 0 and sha(source) == initial_input_hash
            assert (out / "manifest.json").is_file()
            rows.append({"case": label, "exit_code": code, "input_unchanged": True,
                         "accepted": True, "log_sha256": log})

        prior = args.work / "rollback-output"
        code, _ = run("rollback-initial", ["export", args.replay, "--out", prior, "--checkpoints"])
        assert code == 0
        before = snapshot(prior)
        malformed = [("truncated-" + str(length), raw[:length])
                     for length in (0, 1, 4, 16, 128, len(raw) - 1)]
        malformed += [("oversized-chunk", raw + struct.pack("<Ii", 99, 2147483647)),
                      ("negative-chunk", raw + struct.pack("<Ii", 99, -1))]
        for label, payload in malformed:
            path = args.work / (label + ".vrf")
            path.write_bytes(payload)
            digest = sha(path)
            for command in ("validate", "export"):
                cmd = [command, path]
                if command == "export":
                    cmd += ["--out", prior, "--checkpoints"]
                code, log = run(label + "-" + command, cmd)
                assert code == 1, (label, command, code)
                assert snapshot(prior) == before and sha(path) == digest
                assert not [p for p in args.work.iterdir() if ".vrfkit-" in p.name]
                rows.append({"case": label + "-" + command, "exit_code": code,
                             "prior_output_and_input_unchanged": True,
                             "no_staging_or_backup_leftovers": True,
                             "accepted": True, "log_sha256": log})

        diagnostics = args.work / "diagnostics.json"
        code, log = run("diag-file", ["diag", args.replay, "--json", diagnostics])
        assert code == 0 and json.loads(diagnostics.read_text())
        first = diagnostics.read_bytes()
        code, log = run("diag-replace", ["diag", args.replay, "--json", diagnostics])
        assert code == 0 and diagnostics.read_bytes() == first
        rows.append({"case": "diagnostic-replacement", "exit_code": code,
                     "byte_identical_json": True, "accepted": True, "log_sha256": log})
    assert sha(args.replay) == initial_input_hash and sha(args.exe) == initial_binary_hash
    report = {"schema_version": 1, "mode": "baseline-observation" if args.observe_baseline else "candidate-acceptance",
              "runner_sha256": sha(Path(__file__)), "executable_sha256": initial_binary_hash,
              "input_sha256": initial_input_hash, "platform": os.name,
              "cases": rows, "failed_expectations": sum(not r["accepted"] for r in rows),
              "original_input_and_executable_unchanged": True,
              "limitations": ["Linux execution; Windows runtime gates belong to hosted CI.",
                              "No hostile concurrent filesystem mutation or OS crash simulation.",
                              "Each CLI invocation has a 60-second ceiling; no total process-memory assertion."]}
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"cases": len(rows), "failed_expectations": report["failed_expectations"]}))


if __name__ == "__main__":
    main()
