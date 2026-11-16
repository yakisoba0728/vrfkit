"""Fresh baseline on four added public inputs using the frozen Phase B CLI.

This is an attributable old-executable baseline, not current-source acceptance.
Only aggregate receipts may be published; detailed exports/logs stay local.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

if not __debug__:
    raise RuntimeError("QA requires assertions")


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def canonical(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                    allow_nan=False).encode()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    for name in ("source", "exe", "corpus", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    sys.path.insert(0, str(args.source / "tools"))
    import summary_counters
    import verify_build_corpus
    pins = json.loads((args.corpus / "receipt.json").read_text())
    assert len(pins) == 4 and len({pin["sha256"] for pin in pins}) == 4
    old = json.loads((args.source / "docs/evidence/phase_b/independent_qa/build.json").read_text())
    executable_sha = sha(args.exe)
    assert executable_sha == "29fa495b19b7dbce2bfe7bbedc1dc9be2492a4c6a5aca2a9d1468d3d74c657d4"
    assert old["source_commit"] == "2fbead4f716f12f7c5d41ff067ff4ee63a55dc90"
    args.output.mkdir(parents=True, exist_ok=False)
    specifications = json.loads((args.source / "tools/fixtures/public_fixture_type_evidence.json").read_text())
    runs = []
    for pin in pins:
        replay = args.corpus / pin["name"]
        data = replay.read_bytes()
        assert len(data) == pin["bytes"] and hashlib.sha256(data).hexdigest() == pin["sha256"]
        blob = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
        assert blob == pin["git_blob"]
        result = verify_build_corpus.audit_one((pin["sha256"], replay), args.exe, args.output, specifications)
        assert not result["failures"], result["failures"]
        export = args.output / pin["sha256"] / "export"
        manifest = json.loads((export / "manifest.json").read_text())
        tables = {path.name: {"bytes": path.stat().st_size, "sha256": sha(path)}
                  for path in sorted(export.glob("*.parquet"))}
        assert len(tables) == 13
        stable = {key: value for key, value in manifest.items() if key not in ("source_file", "elapsed_ms")}
        counters = summary_counters.read((export.parent / "export.log").read_text(), summary_counters.WHERE)
        runs.append({"build": manifest["replay_build"], "input": pin,
                     "output": {"tables": tables, "manifest_sha256": canonical(stable), "counters": counters},
                     "audit": result})
        (args.output / "baseline.json").write_text(json.dumps({
            "scope": "Fresh four-input public export by frozen accepted Phase B executable",
            "executable_source_commit": old["source_commit"], "executable_sha256": executable_sha,
            "runner_source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=args.source, text=True).strip(),
            "runner_sha256": sha(Path(__file__)), "runs": runs,
            "complete": len(runs) == 4, "performance_claim": None,
        }, indent=2) + "\n")
        print(manifest["replay_build"] + ": baseline validated/exported", flush=True)
    assert sha(args.exe) == executable_sha


if __name__ == "__main__":
    main()
