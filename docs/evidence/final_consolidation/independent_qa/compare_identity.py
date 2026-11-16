"""Fresh public GameplayIdentity verification with the retained interval oracle.

The Rust adapter's per-row output stays in a pipe. Only aggregate counts and
digests are published. This script does not interpret player account identity.
"""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import signal
import subprocess
import sys


def sha(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def deadline(_signal, _frame):
    raise TimeoutError("public identity comparison exceeded 600 seconds")


def main():
    parser = argparse.ArgumentParser()
    for name in ("source", "probe", "corpus", "corpus-report", "work", "output"):
        parser.add_argument("--" + name, required=True, type=Path)
    args = parser.parse_args()
    assert __debug__
    args.work.mkdir(parents=True, exist_ok=False)
    assert not args.output.exists()
    source_commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=args.source, text=True).strip()
    corpus_report = json.loads(args.corpus_report.read_text())
    assert corpus_report["provenance"]["parser_commit"] == source_commit
    assert not corpus_report["failures"] and not corpus_report["build_errors"]
    sys.path.insert(0, str(args.source / "docs/evidence/phase_c/independent_qa"))
    import identity_oracle
    probe_source = args.source / "docs/evidence/phase_c/independent_qa/identity_probe.rs"
    installed = args.source / "crates/vrfkit/examples/qa_final_identity.rs"
    assert sha(installed) == sha(probe_source)
    subprocess.run(["git", "diff", "--exit-code", "HEAD", "--", "Cargo.toml", "Cargo.lock", "crates"],
                   cwd=args.source, check=True, stdout=subprocess.DEVNULL)
    executable_sha = sha(args.probe)
    inputs = json.loads((args.corpus / "inputs.json").read_text())
    assert len(inputs) == 5
    signal.signal(signal.SIGALRM, deadline)
    results = []
    for digest, replay_name in sorted(inputs.items()):
        replay = Path(replay_name)
        assert sha(replay) == digest
        export = args.corpus / digest / "export"
        manifest = json.loads((export / "manifest.json").read_text())
        signal.alarm(600)
        try:
            compared = identity_oracle.compare(args.probe, replay, export, args.work / (digest + ".stderr"))
        finally:
            signal.alarm(0)
        assert compared["mismatches"] == 0
        assert sha(replay) == digest
        row = {"input_sha256": digest, "build": manifest["replay_build"], **compared}
        results.append(row)
        print(row["build"] + ": " + str(row["movement_rows_compared"]) + " rows checked", flush=True)
    assert sha(args.probe) == executable_sha
    resolutions = Counter()
    for row in results:
        resolutions.update(row["resolution_counts"])
    receipt = {"schema_version": 1, "verdict": "PASS", "source_commit": source_commit,
               "probe_executable_sha256": executable_sha, "runner_sha256": sha(Path(__file__)),
               "retained_probe_source_sha256": sha(probe_source),
               "retained_oracle_sha256": sha(Path(identity_oracle.__file__)),
               "corpus_report_sha256": sha(args.corpus_report), "distinct_inputs": len(results),
               "movement_rows_compared": sum(r["movement_rows_compared"] for r in results),
               "exact_proof_tuples_compared": sum(r["exact_proof_tuples_compared"] for r in results),
               "movement_components": sum(r["movement_components"] for r in results),
               "fragmented_movement_rows": sum(r["fragmented_movement_rows"] for r in results),
               "spawned_character_raw_values_compared": sum(r["spawned_character_raw_values_compared"] for r in results),
               "resolution_counts": dict(resolutions), "mismatches": 0, "results": results,
               "limitations": ["Fresh Linux execution; retained Rust adapter and independent Python interval oracle.",
                               "Main packet intervals only; checkpoints are explicitly skipped by this identity API.",
                               "No account identity, event timestamp, final path join, or unobserved wire semantics inferred."]}
    args.output.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({key: value for key, value in receipt.items() if key != "results"}, indent=2))


if __name__ == "__main__":
    main()
