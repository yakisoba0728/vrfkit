"""Independent final-candidate preservation checks on freshly audited outputs.

The CLI audits run separately with tools/verify_build_corpus.py. This script
checks complete byte/manifest parity, independently decoded typed evidence,
Focus parent/child order, required-counter negative controls and JSONL values.
Inputs, detailed CLI logs and row values remain in the private work directory.
"""
from __future__ import annotations

import argparse
from copy import deepcopy
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys

import pyarrow.compute as pc
import pyarrow.parquet as pq


def sha(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def pairs_unique(pairs):
    result = {}
    for key, value in pairs:
        assert key not in result, "duplicate JSON key"
        result[key] = value
    return result


def strict_json(path):
    def invalid(value):
        raise ValueError("non-finite JSON constant")
    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=pairs_unique,
                      parse_constant=invalid)


def canonical(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                    allow_nan=False).encode()).hexdigest()


def tables(directory):
    result = {}
    for path in sorted(directory.glob("*.parquet")):
        with pq.ParquetFile(path, pre_buffer=False) as parquet:
            result[path.name] = {"sha256": sha(path), "bytes": path.stat().st_size,
                                 "rows": parquet.metadata.num_rows}
    assert len(result) == 13, "expected all main and checkpoint tables"
    assert {p.name for p in directory.iterdir()} == set(result) | {"manifest.json"}
    return result


def normalized(manifest):
    return {key: value for key, value in manifest.items()
            if key not in ("source_file", "elapsed_ms")}


def checkpoint_path_bytes(export):
    """Replay the path-index table using lengths only, including repeated GUIDs."""
    snapshots = {}
    with pq.ParquetFile(export / "checkpoint_guid_entries.parquet", pre_buffer=False) as parquet:
        for batch in parquet.iter_batches(batch_size=4096, use_threads=False):
            for row in batch.to_pylist():
                state = snapshots.setdefault(row["checkpoint_index"],
                    {"literals": [], "resolved_bytes": 0, "entries": 0, "max_path_bytes": 0})
                if row["path_is_string"]:
                    length = len(row["literal_path"].encode("utf-8"))
                    state["literals"].append(length)
                else:
                    length = state["literals"][row["name_index"]]
                state["resolved_bytes"] += length
                state["entries"] += 1
                state["max_path_bytes"] = max(state["max_path_bytes"], length)
    return {"checkpoints_with_guid_entries": len(snapshots),
            "guid_entries": sum(s["entries"] for s in snapshots.values()),
            "max_resolved_path_bytes_per_checkpoint": max((s["resolved_bytes"] for s in snapshots.values()), default=0),
            "max_single_path_utf8_bytes": max((s["max_path_bytes"] for s in snapshots.values()), default=0),
            "budget_bytes": 256 * 1024 * 1024,
            "all_observed_within_budget": all(s["resolved_bytes"] <= 256 * 1024 * 1024 for s in snapshots.values())}


def logical_jsonl(export, output, module):
    results = []
    output.mkdir()
    for path in sorted(export.glob("*.parquet")):
        destination = output / (path.stem + ".jsonl")
        published = module.export_jsonl(export, destination, table=path.name, batch_size=137)
        rows = binary_bytes = 0
        with pq.ParquetFile(path, pre_buffer=False) as parquet, destination.open(encoding="utf-8") as text:
            header = json.loads(next(text), object_pairs_hook=pairs_unique)
            assert header["type"] == "schema" and header["format"] == "vrfkit.parquet-jsonl"
            assert header["version"] == 1 and header["table"] == path.name
            assert header["rows"] == parquet.metadata.num_rows
            assert [field["name"] for field in header["columns"]] == parquet.schema_arrow.names
            assert [field["arrow_type"] for field in header["columns"]] == [str(f.type) for f in parquet.schema_arrow]
            assert [field["nullable"] for field in header["columns"]] == [f.nullable for f in parquet.schema_arrow]
            for batch in parquet.iter_batches(batch_size=193, use_threads=False):
                for row in batch.to_pylist():
                    observed = json.loads(next(text), object_pairs_hook=pairs_unique)
                    expected = {}
                    for key, value in row.items():
                        if isinstance(value, bytes):
                            binary_bytes += len(value)
                            value = value.hex()
                        expected[key] = value
                    assert observed == {"type": "row", "values": expected}, "JSONL logical-value mismatch"
                    rows += 1
            assert text.read() == "", "extra JSONL rows"
            assert rows == parquet.metadata.num_rows == published["rows"]
        results.append({"table": path.name, "rows": rows, "binary_bytes": binary_bytes,
                        "jsonl_sha256": sha(destination)})
    return results


def main():
    parser = argparse.ArgumentParser()
    for name in ("source", "baseline", "candidate", "candidate-exe", "input-receipt", "work", "output"):
        parser.add_argument("--" + name, required=True, type=Path)
    args = parser.parse_args()
    assert __debug__, "Assertions must remain enabled"
    args.work.mkdir(parents=True, exist_ok=False)
    assert not args.output.exists()
    sys.path.insert(0, str(args.source / "tools"))
    sys.path.insert(0, str(args.source / "docs/evidence/phase_c/independent_qa"))
    import verify_build_corpus as corpus
    import summary_counters as sc
    import export_jsonl
    import focus_array_evidence as focus
    source = args.source / "docs/evidence/phase_d/independent-qa/check_fresh_outputs.py"
    spec = importlib.util.spec_from_file_location("retained_output_oracle", source)
    oracle = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(oracle)
    inputs = strict_json(args.input_receipt)["inputs"]
    assert len(inputs) == 5 and all(r["status"] == "PASS" for r in inputs)
    candidate_sha = sha(args.candidate_exe)
    mandatory = list(sc.MOVEMENT_COUNTERS) + list(sc.FOCUS_PROJECTILES_COUNTERS)
    assert len(mandatory) == 11
    results, negative_controls = [], []
    observed_fields, absent_in_every_input = set(), None
    for item in inputs:
        digest, build = item["sha256"], item["build"]
        previous = args.baseline / digest
        current = args.candidate / digest
        old_result, new_result = (strict_json(root / "result.json") for root in (previous, current))
        assert not old_result["failures"] and not new_result["failures"]
        assert old_result["counts"] == new_result["counts"], "audit count delta"
        old_tables, new_tables = (tables(root / "export") for root in (previous, current))
        assert old_tables == new_tables, "Parquet byte or row delta"
        manifests = [strict_json(root / "export/manifest.json") for root in (previous, current)]
        assert normalized(manifests[0]) == normalized(manifests[1]), "stable full-manifest delta"
        old_counters, new_counters = [sc.read((root / "export.log").read_text(), sc.WHERE)
                                      for root in (previous, current)]
        assert old_counters == new_counters and all(v is not None for v in new_counters.values())
        manifest = manifests[1]
        counts, failures = corpus.manifest_counts(manifest)
        assert not failures
        public_type = strict_json(current / "type-evidence.json")
        assert public_type["failure_count"] == public_type["typed_mismatch_count"] == 0
        observed_fields.update(public_type["fields"])
        missing = set(public_type["missing"])
        absent_in_every_input = missing if absent_in_every_input is None else absent_in_every_input & missing
        row = {"build": build, "input_sha256": digest, "tables": new_tables,
               "baseline_audit_result_sha256": sha(previous / "result.json"),
               "candidate_audit_result_sha256": sha(current / "result.json"),
               "candidate_type_evidence_sha256": sha(current / "type-evidence.json"),
               "stable_manifest_sha256": canonical(normalized(manifest)),
               "summary_counters_equal": len(new_counters),
               "checkpoints": counts["checkpoint_chunks"],
               "typed_values_compared": sum(v["rows"] for v in public_type["fields"].values()),
               "absent_evidence_fields": public_type["missing"],
               "raw_rpc_loss_main": counts["main_rpc_loss"],
               "raw_rpc_loss_checkpoint": counts["checkpoint_rpc_loss"],
               "checkpoint_path_resources": checkpoint_path_bytes(current / "export")}
        assert row["checkpoint_path_resources"]["all_observed_within_budget"]
        main = args.work / (build + "-main")
        replay = Path(strict_json(args.candidate / "inputs.json")[digest])
        assert sha(replay) == digest
        inspected = subprocess.run([str(args.candidate_exe), "inspect", str(replay)],
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=60)
        (args.work / (build + "-inspect.log")).write_text(inspected.stdout, encoding="utf-8")
        assert inspected.returncode == 0
        assert re.search(r"Branch:\s+\+\+Ares-Core\+release-" + re.escape(build) + r"\s", inspected.stdout)
        assert int(re.search(r"Checkpoint:\s+(\d+) chunks", inspected.stdout)[1]) == row["checkpoints"]
        assert int(re.search(r"File size:\s+(\d+) bytes", inspected.stdout)[1]) == replay.stat().st_size
        row["inspect"] = {"exit_code": 0, "branch_checkpoint_count_and_size_match": True,
                          "log_sha256": sha(args.work / (build + "-inspect.log"))}
        run = subprocess.run([str(args.candidate_exe), "export", str(replay), "--out", str(main)],
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=180)
        (args.work / (build + "-main.log")).write_text(run.stdout, encoding="utf-8")
        assert run.returncode == 0, "main-only export failed"
        main_tables = {p.name: sha(p) for p in main.glob("*.parquet")}
        assert len(main_tables) == 6
        assert all(value == new_tables[name]["sha256"] for name, value in main_tables.items())
        assert sha(replay) == digest
        row["main_only_tables_byte_equal_to_checkpoint_enabled"] = len(main_tables)
        if build == "12.05":
            row["focus_oracle"] = oracle.focus_current(current / "export", pq, pc, focus)
            for scope_name in ("main", "checkpoint"):
                for key in mandatory:
                    mutant = deepcopy(manifest)
                    scope = mutant["quality"] if scope_name == "main" else mutant["quality"]["checkpoints"]
                    del scope["sink"][key]
                    try:
                        _, errors = corpus.manifest_counts(mutant)
                        detected = bool(errors)
                    except (KeyError, ValueError, TypeError):
                        detected = True
                    assert detected, "missing counter not rejected"
                    negative_controls.append({"scope": scope_name, "removed_counter": key, "detected": True})
        if build in ("12.10", "12.11", "13.00"):
            row["jsonl"] = logical_jsonl(current / "export", args.work / (build + "-jsonl"), export_jsonl)
        results.append(row)
        print("verified " + build, flush=True)
    assert sha(args.candidate_exe) == candidate_sha
    assert not absent_in_every_input, "at least one evidence specification lacks all public observation"
    dependencies = ["tools/verify_build_corpus.py", "tools/summary_counters.py",
                    "tools/export_jsonl.py", "tools/atomic_io.py", "tools/validate_type_evidence.py",
                    "tools/wire_bits.py", "tools/fixtures/public_fixture_type_evidence.json",
                    "docs/evidence/phase_c/independent_qa/focus_array_evidence.py",
                    "docs/evidence/phase_d/independent-qa/check_fresh_outputs.py"]
    result = {"schema_version": 1, "verdict": "PASS", "runner_sha256": sha(Path(__file__)),
              "runner_source_context": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=args.source, text=True).strip(),
              "dependency_sources_sha256": {path: sha(args.source / path) for path in dependencies},
              "candidate_executable_sha256": candidate_sha, "distinct_inputs": 5,
              "checkpoint_chunks": sum(r["checkpoints"] for r in results),
              "parquet_tables_byte_equal": sum(len(r["tables"]) for r in results),
              "typed_values_compared": sum(r["typed_values_compared"] for r in results),
              "typed_evidence_specifications_observed": len(observed_fields),
              "evidence_absent_from_every_input": sorted(absent_in_every_input),
              "additional_main_only_exports": 5,
              "inspect_invocations": 5,
              "jsonl_tables": sum(len(r.get("jsonl", [])) for r in results),
              "jsonl_rows": sum(v["rows"] for r in results for v in r.get("jsonl", [])),
              "jsonl_binary_bytes": sum(v["binary_bytes"] for r in results for v in r.get("jsonl", [])),
              "missing_counter_negative_controls": negative_controls, "results": results,
              "limitations": ["Five public inputs; private bytes not downloaded or executed.",
                              "Independent Python wire oracles reused with fresh outputs; absence of a field is explicit.",
                              "No performance claim; byte equality does not establish untested game semantics."]}
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: value for key, value in result.items() if key not in ("results", "missing_counter_negative_controls")}, indent=2))


if __name__ == "__main__":
    main()
