"""Rerun the existing bounded three-fixture movement provenance oracle.

Fresh Rust observations are compared with pinned historical C# reference
outputs. The reference is not regenerated; only aggregate results are saved.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser()
    for name in ("source", "probe", "corpus", "reference", "output", "build-receipt"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    sys.path.insert(0, str(args.source / "tools"))
    from compare_movement_provenance import compare
    build = json.loads(args.build_receipt.read_text())
    probe_sha = sha(args.probe)
    assert probe_sha == build["binaries"]["artifacts"]["movement_provenance"]["sha256"]
    results = []
    for label in ("12.10", "12.11", "13.00"):
        replay, = (args.corpus / label).glob("*.vrf")
        reference = args.reference / label / "reference/movement.ndjson"
        manifest = json.loads((reference.parent / "manifest.json").read_text())
        historical = json.loads((args.source / ("docs/evidence/movement-comparison-" + label + ".json")).read_text())
        assert manifest["source_sha256"] == sha(replay) and manifest["parse_status"] == "completed"
        assert manifest["parser_version"].rsplit("+", 1)[-1] == "fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8"
        assert sha(reference) == historical["reference"]["logical_content_sha256"]
        output = args.output / (label + ".jsonl")
        with output.open("wb") as out, (args.output / (label + ".stderr")).open("wb") as errors:
            code = subprocess.run([str(args.probe), str(replay)], stdout=out, stderr=errors).returncode
        assert code == 0
        result = compare(reference, output, work_dir=args.output)
        assert result["status"] == "pass"
        results.append({"build": label, "input_sha256": sha(replay), "reference_regenerated": False,
                        "reference_manifest_sha256": sha(reference.parent / "manifest.json"), "comparison": result})
        print(label + ": exact movement provenance correspondence passed", flush=True)
    assert sha(args.probe) == probe_sha
    (args.output / "receipt.json").write_text(json.dumps({"executable_build_commit": build["source_commit"],
        "probe_sha256": probe_sha, "scope": "Fresh Rust output versus pinned historical C# latest-move reference",
        "public_inputs": 3, "reports": results, "performance_claim": None}, indent=2) + "\n")


if __name__ == "__main__":
    main()
