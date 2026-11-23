"""Apply one validation, checkpoint export and typed/raw check to every replay.

Recursively scan each --corpus root, deduplicate by SHA-256, and retain private
logs/exports under a new --work-dir. The JSON report contains aggregate build
counts and content hashes, never source filenames or player identifiers.
Missing counters and failed checks are errors, not implicit zeros. Absent
evidence fields are reported separately from mismatching observed values.
Each export also runs `check_export_baseline.checkpoint_guid_crosscheck`, and
its counts reach the report as `guid_crosscheck_*`.
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


from atomic_io import atomic_write_text, sha256_file
import check_decode_errors_corpus as overlay
import check_export_baseline as baseline
from corpus_scan import find_replays
import summary_counters
import validate_type_evidence as evidence

REPO = Path(__file__).resolve().parents[1]
DEFAULT_EVIDENCE = REPO / "tools/fixtures/public_fixture_type_evidence.json"
NET_ZERO = (
    "malformed_packets", "partial_errors", "unfinished_partials",
    "unfinished_partial_bits", "bunch_header_failures",
    "content_block_framing_failures", "malformed_content_blocks",
    "transform_failures", "field_stream_failures", "channel_reopens_while_open",
    "actor_opens_missing_spawn", "channel_state_limit_failures",
    "partial_resource_limit_failures", "failed_reopens_while_open",
    "bunches_on_unopened_channel", "unopened_channel_bits",
    # Nothing after the exports is read: 0 in both passes of all 1,018 replays.
    "package_map_exports", "rep_layout_export_bunches",
)
SINK_ZERO = (
    "overlay_decoded_err", "struct_blobs_failed", "movement_rpc_errors",
    "array_truncations", "array_errors", "array_unconsumed_nested_bits",
    "array_unconsumed_root_bits", "array_implicit_terminations",
    "array_leaf_decode_errors", "truncated_rpcs",
    # The brute force rests on one empirical constant (fc=34). Native
    # component errors include malformed envelopes and unrepresented bodies.
    "cnc_bruteforce_payloads_unwalked",
    "movement_component_errors",
)
#: Zero in the checkpoint pass only: main-pass suffix bits are the measured
#: ActiveGameplayEffects delta (nonzero on 24 of 27 builds), but no checkpoint
#: RPC has carried one (0 on all 27 builds, 11.06-13.10).
CHECKPOINT_SINK_ZERO = ("rpc_suffix_bits_dropped",)
#: Each sink event tally and the NetStats counter it must equal: vrf-net calls
#: the sink beside its own increment, so a difference is broken bookkeeping.
#: `fields_emitted` pairs with fields.parquet's rows instead (`check_export`).
SINK_NET_EQUAL = (
    ("sink_rpcs_emitted", "rpcs"),
    ("sink_actor_opens", "actor_opens"),
    ("sink_actor_closes", "actor_closes"),
    ("sink_content_blocks", "content_blocks"),
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
                "overlay_error_buckets", "overlay_errors_reported", "unknown_chunks"):
        counts[key] = require_count(quality, key)
        if counts[key]:
            failures.append(f"{key}={counts[key]}")
    for prefix, scope in (("main", quality), ("checkpoint", quality["checkpoints"])):
        failures.extend(f"{prefix}: {error}" for error in summary_counters.movement_contract_errors(
            scope["sink"], require_work=prefix == "main"))
        if summary_counters.obsolete_movement_contract(scope["sink"]):
            raise ValueError(summary_counters.OBSOLETE_MOVEMENT_CONTRACT)
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
                             "overlay_handle_conflicts_refused",
                             "cnc_bruteforce_payloads_attempted",
                             *summary_counters.MOVEMENT_COUNTERS,
                             "active_blinds_empty_trailers",
                             *summary_counters.FOCUS_PROJECTILES_COUNTERS))
            if category == "sink" and prefix == "checkpoint":
                zero_keys = zero_keys + CHECKPOINT_SINK_ZERO
            for key in sorted(keys):
                name = f"{prefix}_{key}"
                counts[name] = require_count(source, key)
                if key in zero_keys and counts[name]:
                    failures.append(f"{name}={counts[name]}")
        for sink_key, net_key in SINK_NET_EQUAL:
            tally = counts[f"{prefix}_{sink_key}"] = require_count(scope["sink"], sink_key)
            framed = counts[f"{prefix}_{net_key}"] = require_count(scope["net"], net_key)
            if tally != framed:
                failures.append(f"{prefix}_{sink_key}={tally} != {prefix}_{net_key}={framed}")
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
    # The checkpoint twin of replay_data_trailing_bytes: 0 in all 19,166
    # checkpoint archives of 1,014 replays, 11.06-13.06.
    counts["checkpoint_trailing_bytes"] = require_count(cp, "checkpoint_trailing_bytes")
    if counts["checkpoint_trailing_bytes"]:
        failures.append(f"checkpoint_trailing_bytes={counts['checkpoint_trailing_bytes']}")
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
    printed = summary_counters.read(text, summary_counters.WHERE)
    missing = [key for key, value in printed.items() if value is None]
    if missing:
        raise ValueError(f"export omits counter {missing[0]}")
    if mismatch := overlay.reconcile(printed):
        raise ValueError(mismatch)
    tables = baseline.parquet_shape(
        directory, baseline.PARQUET_FILES + baseline.CHECKPOINT_PARQUET_FILES)
    errors, guid_counts = baseline.export_errors(directory, printed, True)
    errors = baseline.cross_checks(printed, tables) + errors
    if errors:
        raise ValueError("; ".join(errors + [baseline.format_guid_crosscheck(guid_counts)]))
    return tables, guid_counts


def audit_one(entry, exe, work, specifications):
    digest, replay = entry
    directory = work / digest
    directory.mkdir()
    result = {"sha256": digest, "bytes": replay.stat().st_size,
              "branch": None, "failures": [], "counts": {}}
    try:
        code, text = summary_counters.vrfkit(exe, "validate", replay, timeout=1800)
        (directory / "validate.log").write_text(text, encoding="utf-8")
        branch, counts = validation_counts(text, code)
        result.update(branch=branch, counts=counts)
        export = directory / "export"
        code, text = summary_counters.vrfkit(exe, "export", replay, export, True, timeout=1800)
        (directory / "export.log").write_text(text, encoding="utf-8")
        if code:
            raise ValueError(f"export exit {code}")
        manifest = json.loads((export / "manifest.json").read_text(encoding="utf-8"))
        if manifest["replay_build"] != branch:
            raise ValueError("validate/export branches disagree")
        counts, failures = manifest_counts(manifest)
        result["counts"].update(counts)
        result["failures"].extend(failures)
        result["tables"], guid_counts = check_export(text, export)
        # Every count, zeros included, so the report shows how many entries
        # each build actually compared rather than only that nothing failed.
        result["counts"].update({f"guid_crosscheck_{key}": value
                                 for key, value in guid_counts.items()})
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
                  # Strict: parsed into the report, and a commit hash is ASCII.
                  "parser_commit": subprocess.check_output(
                      ["git", "rev-parse", "HEAD"], cwd=REPO, text=True,
                      encoding="utf-8", errors="strict").strip(),
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
