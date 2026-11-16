"""Independently inspect retained fresh Phase D exports; does not run the CLI.

Only aggregate counts, public input hashes and content digests are published.
Detailed replay/player values remain in the export directories.
"""
from __future__ import annotations

import argparse
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def sha(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def canonical(value) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()).hexdigest()


def no_duplicates(pairs):
    result = {}
    for key, value in pairs:
        assert key not in result, "duplicate JSON key"
        result[key] = value
    return result


def strict_json(path: Path):
    def invalid(value):
        raise ValueError("non-finite JSON constant")
    return json.loads(path.read_text(), object_pairs_hook=no_duplicates, parse_constant=invalid)


def focus_current(export, pq, pc, focus):
    result = []
    for table in ("fields", "checkpoint_fields"):
        pending = []
        parents = children = trailers = 0
        raw_digest = hashlib.sha256()
        with pq.ParquetFile(export / f"{table}.parquet", pre_buffer=False) as file:
            for batch in file.iter_batches(batch_size=4096, use_threads=False):
                mask = pc.and_(pc.equal(batch["group_path"], focus.GROUP),
                               pc.starts_with(pc.cast(batch["field_name"], "string"), "FocusProjectiles"))
                for row in batch.filter(mask).to_pylist():
                    if row["field_name"] != focus.PARENT[1]:
                        pending.append(row)
                        continue
                    assert (row["handle"], row["field_name"], row["compatible_checksum"]) == focus.PARENT
                    assert row["raw_bits"] is not None
                    _, values, trailer = focus.decode(row["raw_bits"], row["bit_count"])
                    expected = []
                    for index, value, width in values:
                        raw = bytearray()
                        remaining = value
                        while True:
                            chunk = (remaining & 127) << 1
                            remaining >>= 7
                            raw.append(chunk | bool(remaining))
                            if not remaining:
                                break
                        assert len(raw) * 8 == width
                        expected.append({**row, "field_name": f"FocusProjectiles[{index}].FocusProjectiles",
                            "handle": focus.MEMBER[0], "compatible_checksum": None,
                            "bit_count": width, "raw_bits": bytes(raw), "value_i64": value,
                            "value_f64": None, "value_bool": None, "value_str": None})
                    assert pending == expected, "Focus children differ from original raw parent/order"
                    pending = []
                    parents += 1
                    children += len(expected)
                    trailers += trailer
                    raw_digest.update(row["bit_count"].to_bytes(4, "little") + row["raw_bits"])
        assert not pending, "orphaned Focus children"
        assert (parents, children, trailers) == ((102, 53, 3) if table == "fields" else (18, 0, 0))
        result.append({"table": table, "parents": parents, "children": children,
                       "empty_trailers": trailers, "parent_raw_sequence_sha256": raw_digest.hexdigest()})
    return result


def main() -> None:
    parser = argparse.ArgumentParser()
    for name in ("source", "preflight", "exports", "build-receipt", "executable", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    assert __debug__, "Assertions must remain enabled"
    assert not args.output.exists()
    sys.path.insert(0, str(args.source / "tools"))
    sys.path.insert(0, str(args.source / "docs/evidence/phase_c/independent_qa"))
    import pyarrow.parquet as pq
    import pyarrow.compute as pc
    import summary_counters as sc
    import verify_build_corpus as corpus
    import validate_type_evidence as evidence
    import focus_array_evidence as focus

    build = strict_json(args.build_receipt)
    assert sha(args.executable) == build["frozen_executable"]["sha256"]
    preflight = strict_json(args.preflight)
    assert preflight["status"] == "passed" and preflight["cases_passed"] == 6
    assert preflight["exports_executed"] == 12
    specs = evidence.load_specifications(args.source / "tools/fixtures/public_fixture_type_evidence.json")
    mandatory = ["movement_component_" + suffix for suffix in
                 ("streams", "complete", "errors", "empty", "envelopes", "payload_bytes",
                  "unused_bits", "input_headers", "discrepancy_headers")]
    mandatory += ["route_children_focus_projectiles", "focus_projectiles_empty_trailers"]
    results = []
    controls = []
    fingerprints = {}
    for case in preflight["cases"]:
        assert case["equivalent"] is True
        for key in ("tables", "counters", "manifest_sha256"):
            assert case["baseline"][key] == case["candidate"][key]
        directory = args.exports / f"{case['build']}-{case['mode']}-candidate"
        export = directory / "export"
        expected = case["candidate"]
        assert {path.name for path in export.iterdir()} == set(expected["tables"]) | {"manifest.json"}
        assert all(path.is_file() and not path.is_symlink() for path in export.iterdir())
        actual = {path.name: {"bytes": path.stat().st_size, "sha256": sha(path)}
                  for path in sorted(export.glob("*.parquet"))}
        assert actual == expected["tables"], "retained table names/bytes/hash changed"
        fingerprints[(case["build"], case["mode"])] = actual
        manifest = strict_json(export / "manifest.json")
        assert canonical({key: value for key, value in manifest.items()
                          if key not in ("source_file", "elapsed_ms")}) == expected["manifest_sha256"]
        assert manifest["source_size_bytes"] == case["input_bytes"]
        wanted = [key for key in sc.WHERE if case["mode"] == "checkpoints" or not key.startswith("cp_")]
        actual_counters = sc.read((directory / "parser.log").read_text(), wanted)
        assert actual_counters == expected["counters"]
        quality = manifest["quality"]
        scopes = [quality]
        if case["mode"] == "checkpoints":
            scopes.append(quality["checkpoints"])
        for scope in scopes:
            assert all(type(scope["sink"][key]) is int and scope["sink"][key] >= 0 for key in mandatory)
        rows = {}
        for name in actual:
            with pq.ParquetFile(export / name, pre_buffer=False) as file:
                rows[name] = file.metadata.num_rows
                assert file.schema_arrow.names
        result = {"build": case["build"], "mode": case["mode"], "input_sha256": case["input_sha256"],
                  "tables_verified": len(actual), "table_rows": rows,
                  "manifest_sha256": expected["manifest_sha256"], "summary_counters_verified": len(actual_counters),
                  "native_and_focus_keys_verified": len(scopes) * len(mandatory), "failures": []}
        if case["mode"] == "checkpoints":
            counts, failures = corpus.manifest_counts(manifest)
            assert not failures, "manifest quality gate failed"
            _, guid = corpus.check_export((directory / "parser.log").read_text(), export)
            typed = evidence.validate(export, specs, compare_typed=True)
            assert not typed["failure_count"] and not typed["typed_mismatch_count"]
            result.update(checkpoints=counts["checkpoint_chunks"],
                          typed_values_compared=sum(field["rows"] for field in typed["fields"].values()),
                          typed_width_failures=typed["failure_count"], typed_mismatches=typed["typed_mismatch_count"],
                          absent_evidence_fields=typed["missing"], guid_crosscheck=guid,
                          raw_rpc_loss_main=counts["main_rpc_loss"], raw_rpc_loss_checkpoint=counts["checkpoint_rpc_loss"])
            assert result["typed_values_compared"] > 0
            if case["build"] == "12.05":
                result["focus_parent_child_oracle"] = focus_current(export, pq, pc, focus)
                for scope_name in ("main", "checkpoint"):
                    for key in mandatory:
                        mutant = deepcopy(manifest)
                        scope = mutant["quality"] if scope_name == "main" else mutant["quality"]["checkpoints"]
                        del scope["sink"][key]
                        detected = False
                        try:
                            _, errors = corpus.manifest_counts(mutant)
                            detected = bool(errors)
                        except (KeyError, TypeError, ValueError):
                            detected = True
                        assert detected, "missing mandatory counter not detected"
                        controls.append({"scope": scope_name, "removed_key": key, "detected": True})
        results.append(result)
        print(f"verified {case['build']} {case['mode']}", flush=True)
    for build_name in ("12.05", "12.09", "13.00"):
        main, cp = (fingerprints[(build_name, mode)] for mode in ("main", "checkpoints"))
        assert all(cp[name] == item for name, item in main.items()), "checkpoint enablement changed a main table"
    assert sha(args.executable) == build["frozen_executable"]["sha256"]
    receipt = {"schema_version": 1, "decision": "ACCEPT_FRESH_RETAINED_OUTPUTS",
        "executing_binary_source": build["source"]["commit"], "executing_binary_sha256": build["frozen_executable"]["sha256"],
        "audit_source_context": subprocess.check_output(["git", "-C", str(args.source), "rev-parse", "HEAD"], text=True).strip(),
        "runner_sha256": sha(Path(__file__)), "preflight_receipt_sha256": sha(args.preflight),
        "source_of_exports": "Fresh Phase D A preflight exports, independently reread by D; no additional CLI exports by this script",
        "new_cli_exports_by_this_script": 0, "independently_inspected_exports": len(results),
        "distinct_inputs": len({row["input_sha256"] for row in results}),
        "parquet_files_independently_rehashed_and_footer_read": sum(row["tables_verified"] for row in results),
        "typed_values_compared": sum(row.get("typed_values_compared", 0) for row in results),
        "checkpoint_chunks": sum(row.get("checkpoints", 0) for row in results),
        "missing_counter_negative_controls": controls, "results": results,
        "limitations": ["Comparison uses the same pinned public inputs; private corpus and complete game semantics are outside this scope.",
                        "Raw/typed decoder is the established independent Python wire implementation; this phase adds a current-output Focus ordering oracle."]}
    args.output.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({key: receipt[key] for key in ("decision", "distinct_inputs", "parquet_files_independently_rehashed_and_footer_read", "typed_values_compared", "checkpoint_chunks")}, indent=2))


if __name__ == "__main__":
    main()
