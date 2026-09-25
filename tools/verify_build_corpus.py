"""Apply one validation, checkpoint export and typed/raw check to every replay.

Recursively scan each --corpus root, deduplicate by SHA-256, and retain private
logs/exports under a new --work-dir. The JSON report contains aggregate build
counts and content hashes, never source filenames or player identifiers.
Missing counters and failed checks are errors, not implicit zeros. Absent
evidence fields are reported separately from mismatching observed values.
"""
from __future__ import annotations

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import subprocess

import pyarrow.parquet as pq

from atomic_io import atomic_write_text, sha256_file
import check_decode_errors_corpus as overlay
import check_export_baseline as baseline
from corpus_scan import find_replays
import validate_type_evidence as evidence

REPO = Path(__file__).resolve().parents[1]
DEFAULT_EVIDENCE = REPO / "tools/fixtures/public_fixture_type_evidence.json"
NET_ZERO = (
    "malformed_packets", "partial_errors", "unfinished_partials",
    "unfinished_partial_bits", "bunch_header_failures",
    "content_block_framing_failures", "malformed_content_blocks",
    "transform_failures", "field_stream_failures", "channel_reopens_while_open",
    "actor_opens_missing_spawn", "channel_state_limit_failures",
    "partial_resource_limit_failures",
)
SINK_ZERO = (
    "overlay_decoded_err", "struct_blobs_failed", "movement_rpc_errors",
    "array_truncations", "array_errors", "array_unconsumed_nested_bits",
    "array_unconsumed_root_bits", "array_implicit_terminations",
    "array_leaf_decode_errors", "truncated_rpcs",
)


def require_count(obj, key):
    value = obj[key]
    if type(value) is not int or value < 0:
        raise ValueError(f"invalid counter: {key}")
    return value


def manifest_counts(manifest):
    """Measure both passes, preserving raw RPC and suffix counts as limitations."""
    quality = manifest["quality"]
    if quality["checkpoints_enabled"] is not True:
        raise ValueError("checkpoint decoding was not enabled")
    counts, failures = {}, []
    for key in ("content_blocks_lost", "event_trailing_bytes",
                "replay_data_trailing_bytes", "event_layout_mismatches",
                "overlay_error_buckets", "overlay_errors_reported"):
        counts[key] = require_count(quality, key)
        if counts[key]:
            failures.append(f"{key}={counts[key]}")
    for prefix, scope in (("main", quality), ("checkpoint", quality["checkpoints"])):
        for category, zero_keys in (("net", NET_ZERO), ("sink", SINK_ZERO)):
            source = scope[category]
            keys = set(zero_keys)
            if category == "net":
                keys.update(("content_blocks", "skipped_bits", "rpc_stream_failures",
                             "unresolved_rpc_payloads_preserved"))
            else:
                keys.update(("overlay_decoded_ok", "overlay_raw_or_skip",
                             "overlay_not_in_table", "overlay_no_field_name",
                             "struct_blobs_decoded", "rpc_suffix_bits_dropped",
                             "overlay_handle_conflicts_refused"))
            for key in sorted(keys):
                name = f"{prefix}_{key}"
                counts[name] = require_count(source, key)
                if key in zero_keys and counts[name]:
                    failures.append(f"{name}={counts[name]}")
        lost = counts[f"{prefix}_rpc_stream_failures"] - counts[f"{prefix}_unresolved_rpc_payloads_preserved"]
        counts[f"{prefix}_rpc_loss"] = lost
        if lost != 0:
            failures.append(f"{prefix}_rpc_loss={lost}")
        counts[f"{prefix}_overlay_offered"] = sum(
            counts[f"{prefix}_{key}"] for key in
            ("overlay_decoded_ok", "overlay_decoded_err", "overlay_raw_or_skip",
             "overlay_not_in_table", "overlay_no_field_name"))
    cp = quality["checkpoints"]
    counts["checkpoint_chunks"] = require_count(cp, "checkpoint_chunks")
    if not counts["main_content_blocks"]:
        failures.append("no main content blocks")
    return counts, failures


def validation_counts(text, returncode):
    if returncode:
        raise ValueError(f"validate exit {returncode}")
    branch = re.search(r"Branch:\s+(\+\+Ares-Core\+release-[\d.]+)", text)
    match = re.search(r"ORACLE PASS RATE:\s+[\d.]+% \((\d+) / (\d+) blocks passed\)", text)
    if not branch or not match:
        raise ValueError("validation omits branch or oracle counters")
    passed, total = map(int, match.groups())
    if total == 0 or passed != total:
        raise ValueError(f"oracle passed {passed}/{total}")
    return branch[1], {"oracle_passed_blocks": passed, "oracle_scored_blocks": total}


def check_export(text, directory):
    counters, error = overlay.read_counters(text, 0, require_checkpoints=True)
    if error:
        raise ValueError(error)
    if mismatch := overlay.reconcile(counters):
        raise ValueError(mismatch)
    patterns = dict(baseline.PATTERNS)
    patterns.update({k: re.compile(v) for k, v in baseline.CHECKPOINT_COUNTERS.items()})
    printed = {}
    for key, pattern in patterns.items():
        match = pattern.search(text)
        if match is None:
            raise ValueError(f"export omits counter {key}")
        printed[key] = int(match[1])
    tables = {}
    for name in baseline.PARQUET_FILES + baseline.CHECKPOINT_PARQUET_FILES:
        path = directory / f"{name}.parquet"
        tables[name] = {"rows": pq.ParquetFile(path).metadata.num_rows,
                        "bytes": path.stat().st_size}
    errors = baseline.cross_checks(printed, tables)
    errors += baseline.checkpoint_manifest_errors(directory, printed)
    errors += baseline.reward_opaque_manifest_errors(directory, printed, True)
    errors += baseline.targeting_manifest_errors(directory, printed, True)
    if errors:
        raise ValueError("; ".join(errors))
    return tables


def audit_one(entry, exe, work, specifications):
    digest, replay = entry
    directory = work / digest
    directory.mkdir()
    result = {"sha256": digest, "bytes": replay.stat().st_size,
              "branch": None, "failures": [], "counts": {}}
    try:
        run = subprocess.run([str(exe), "validate", str(replay)], capture_output=True,
                             text=True, encoding="utf-8", errors="replace", timeout=1800)
        text = run.stdout + run.stderr
        (directory / "validate.log").write_text(text, encoding="utf-8")
        branch, counts = validation_counts(text, run.returncode)
        result.update(branch=branch, counts=counts)
        export = directory / "export"
        run = subprocess.run([str(exe), "export", str(replay), "--out", str(export),
                              "--checkpoints"], capture_output=True, text=True,
                             encoding="utf-8", errors="replace", timeout=1800)
        text = run.stdout + run.stderr
        (directory / "export.log").write_text(text, encoding="utf-8")
        if run.returncode:
            raise ValueError(f"export exit {run.returncode}")
        manifest = json.loads((export / "manifest.json").read_text(encoding="utf-8"))
        if manifest["replay_build"] != branch:
            raise ValueError("validate/export branches disagree")
        counts, failures = manifest_counts(manifest)
        result["counts"].update(counts)
        result["failures"].extend(failures)
        result["tables"] = check_export(text, export)
        checked = evidence.validate(export, specifications, compare_typed=True)
        (directory / "type-evidence.json").write_text(json.dumps(checked, indent=2), encoding="utf-8")
        result["evidence_fields"] = {name: value["rows"] for name, value in checked["fields"].items()}
        result["counts"]["typed_values_compared"] = sum(result["evidence_fields"].values())
        result["counts"]["type_width_failures"] = checked["failure_count"]
        result["counts"]["typed_mismatches"] = checked["typed_mismatch_count"]
        result["absent_evidence_fields"] = checked["missing"]
        if checked["failure_count"] or checked["typed_mismatch_count"]:
            result["failures"].append("typed/raw comparison failed")
        if not result["counts"]["typed_values_compared"]:
            result["failures"].append("no observed evidence values to compare")
        if sha256_file(replay) != digest:
            result["failures"].append("input changed during verification")
    except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as exc:
        # Details and paths remain private. Public reports identify this input by digest.
        (directory / "error.txt").write_text(str(exc), encoding="utf-8")
        result["failures"].append(f"{type(exc).__name__}: see private error.txt")
    atomic_write_text(directory / "result.json", json.dumps(result, indent=2) + "\n")
    return result


def summarize(rows):
    builds = {}
    for row in rows:
        branch = row["branch"] or "unidentified"
        build = builds.setdefault(branch, {"replays": 0, "passed": 0, "failed": 0,
                                          "counts": Counter(), "evidence_fields": Counter(),
                                          "tables": {}, "input_sha256": []})
        build["replays"] += 1
        build["failed" if row["failures"] else "passed"] += 1
        build["counts"].update(row["counts"])
        build["evidence_fields"].update(row.get("evidence_fields", {}))
        build["input_sha256"].append(row["sha256"])
        for name, table in row.get("tables", {}).items():
            aggregate = build["tables"].setdefault(name, Counter())
            aggregate.update(table)
    for build in builds.values():
        build["input_sha256"].sort()
        if (not build["counts"].get("checkpoint_content_blocks")
                or not build["counts"].get("checkpoint_overlay_decoded_ok")):
            build["checkpoint_evidence"] = "absent"
        else:
            build["checkpoint_evidence"] = "observed"
    return dict(sorted(builds.items()))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, action="append", required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, default=DEFAULT_EVIDENCE)
    parser.add_argument("--jobs", type=int, default=4)
    args = parser.parse_args(argv)
    if args.jobs < 1 or not args.exe.is_file() or any(not p.is_dir() for p in args.corpus):
        parser.error("positive jobs, an executable and existing corpus directories are required")
    if args.work_dir.exists() or args.output.exists():
        parser.error("work directory and report must be new paths")
    args.work_dir.mkdir(parents=True)
    files = sorted({p.resolve() for root in args.corpus for p in find_replays(root, True)})
    if not files:
        parser.error("no replay files found")
    specifications = json.loads(args.evidence.read_text(encoding="utf-8"))
    unique = {}
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for path, digest in zip(files, pool.map(sha256_file, files)):
            unique.setdefault(digest, path)
    print(f"{len(files)} paths; {len(unique)} unique replays; {len(files)-len(unique)} duplicates", flush=True)
    atomic_write_text(args.work_dir / "inputs.json", json.dumps(
        {digest: str(path) for digest, path in unique.items()}, indent=2) + "\n")
    tracked_sources = sorted((REPO / "crates").rglob("*.rs")) + sorted((REPO / "crates").rglob("Cargo.toml")) + [REPO / "Cargo.lock", REPO / "Cargo.toml"]
    source_digest = hashlib.sha256("\n".join(
        f"{p.relative_to(REPO).as_posix()} {sha256_file(p)}" for p in tracked_sources).encode()).hexdigest()
    exe_hash = sha256_file(args.exe)
    provenance = {"date_utc": datetime.now(timezone.utc).isoformat(),
                  "parser_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
                  "parser_sources_sha256": source_digest, "exe_sha256": exe_hash,
                  "runner_sha256": sha256_file(Path(__file__)),
                  "evidence_sha256": sha256_file(args.evidence),
                  "discovered_paths": len(files), "unique_replays": len(unique),
                  "duplicates": len(files)-len(unique),
                  "corpus_sha256": hashlib.sha256("\n".join(sorted(unique)).encode()).hexdigest()}
    rows = []
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        tasks = [pool.submit(audit_one, entry, args.exe.resolve(), args.work_dir.resolve(), specifications)
                 for entry in sorted(unique.items())]
        for task in as_completed(tasks):
            rows.append(task.result())
            if len(rows) % 10 == 0 or len(rows) == len(unique):
                print(f"{len(rows)}/{len(unique)} checked; {sum(bool(r['failures']) for r in rows)} failed", flush=True)
            atomic_write_text(args.work_dir / "progress.json", json.dumps(summarize(rows), indent=2) + "\n")
    changed = sha256_file(args.exe) != exe_hash
    builds = summarize(rows)
    build_errors = [f"{branch}: no observed checkpoint decoding"
                    for branch, build in builds.items()
                    if build["checkpoint_evidence"] != "observed"]
    report = {"provenance": provenance, "executable_changed": changed,
              "builds": builds, "build_errors": build_errors,
              "failures": [{"sha256": row["sha256"], "branch": row["branch"], "errors": row["failures"]}
                           for row in rows if row["failures"]]}
    atomic_write_text(args.output, json.dumps(report, indent=2) + "\n")
    return int(changed or bool(report["failures"]) or bool(build_errors))


if __name__ == "__main__":
    raise SystemExit(main())
