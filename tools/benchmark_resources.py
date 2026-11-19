#!/usr/bin/env python3
"""Compare complete exports with paired timings and checked output equivalence.

Each export runs in a fresh measurement worker. Private command lines, parser
logs and manifests stay in --work-dir; the JSON report contains only hashes,
fixed counter names and aggregate measurements. See docs/RESOURCE_BENCHMARKS.md.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import threading
import time

from atomic_io import atomic_write_text, sha256_file
import summary_counters as sc


MAIN_TABLES = ("actors", "events", "fields", "movement", "net_guids", "partials")
CHECKPOINT_TABLES = ("checkpoint_actors", "checkpoint_blocks", "checkpoint_export_fields",
                     "checkpoint_export_groups", "checkpoint_fields",
                     "checkpoint_guid_entries", "checkpoint_net_guids")
# Same zero-error contract as verify_build_corpus, without its PyArrow dependency.
NET_ZERO = (
    "malformed_packets", "partial_errors", "unfinished_partials", "unfinished_partial_bits",
    "bunch_header_failures", "content_block_framing_failures", "malformed_content_blocks",
    "transform_failures", "field_stream_failures", "channel_reopens_while_open",
    "actor_opens_missing_spawn", "channel_state_limit_failures", "partial_resource_limit_failures",
    "failed_reopens_while_open", "bunches_on_unopened_channel", "unopened_channel_bits",
    "package_map_exports", "rep_layout_export_bunches",
)
SINK_ZERO = (
    "overlay_decoded_err", "struct_blobs_failed", "movement_rpc_errors", "array_truncations",
    "array_errors", "array_unconsumed_nested_bits", "array_unconsumed_root_bits",
    "array_implicit_terminations", "array_leaf_decode_errors", "truncated_rpcs",
    "cnc_bruteforce_payloads_unwalked", "movement_component_errors",
)
QUALITY_ZERO = ("content_blocks_lost", "event_trailing_bytes", "replay_data_trailing_bytes",
                "event_layout_mismatches", "overlay_error_buckets", "overlay_errors_reported",
                "unknown_chunks", "frame_non_finite_times")
TALLIES = (("sink_rpcs_emitted", "rpcs"), ("sink_actor_opens", "actor_opens"),
           ("sink_actor_closes", "actor_closes"), ("sink_content_blocks", "content_blocks"))
METRICS = ("wall_seconds", "cpu_user_seconds", "cpu_system_seconds", "cpu_total_seconds",
           "cpu_utilization_percent", "peak_rss_bytes", "input_mib_per_second",
           "io_input_blocks", "io_output_blocks", "output_bytes", "parquet_bytes")


def strict_json(text: str):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON key")
            result[key] = value
        return result

    def nonfinite(_):
        raise ValueError("nonfinite JSON value")

    return json.loads(text, object_pairs_hook=unique, parse_constant=nonfinite)


def json_hash(value) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                    allow_nan=False).encode("utf-8")).hexdigest()


def count(obj, key: str) -> int:
    value = obj[key]
    if type(value) is not int or value < 0:
        raise ValueError("invalid required counter")
    return value


def quality_check(manifest: dict, checkpoints: bool) -> None:
    """Require observed work and the existing zero-error preservation contract."""
    quality = manifest["quality"]
    if quality["checkpoints_enabled"] is not checkpoints:
        raise ValueError("checkpoint mode differs")
    if any(count(quality, key) for key in QUALITY_ZERO):
        raise ValueError("quality error counter is nonzero")
    scopes = [quality]
    if checkpoints:
        cp = quality["checkpoints"]
        if count(cp, "checkpoint_trailing_bytes") or count(cp, "checkpoint_frame_non_finite_times"):
            raise ValueError("checkpoint error counter is nonzero")
        if not count(cp, "checkpoint_chunks"):
            raise ValueError("no checkpoint work observed")
        scopes.append(cp)
    elif quality["checkpoints"] is not None:
        raise ValueError("unexpected checkpoint scope")
    for scope in scopes:
        net, sink = scope["net"], scope["sink"]
        movement_errors = sc.movement_contract_errors(sink, require_work=scope is quality)
        if movement_errors:
            raise ValueError("; ".join(movement_errors))
        if any(count(net, key) for key in NET_ZERO) or any(count(sink, key) for key in SINK_ZERO):
            raise ValueError("decoder error counter is nonzero")
        if count(net, "rpc_stream_failures") != count(net, "unresolved_rpc_payloads_preserved"):
            raise ValueError("RPC preservation counters differ")
        if any(count(sink, a) != count(net, b) for a, b in TALLIES):
            raise ValueError("sink and parser counters differ")
        if not count(net, "content_blocks") or not count(sink, "overlay_decoded_ok"):
            raise ValueError("no positive decoding evidence")


def fingerprint(directory: Path, log: Path, checkpoints: bool, input_size: int) -> dict:
    """Hash tables and all nonvolatile manifest data outside the timed interval."""
    names = tuple(name + ".parquet" for name in MAIN_TABLES + (CHECKPOINT_TABLES if checkpoints else ()))
    expected = set(names) | {"manifest.json"}
    entries = list(directory.iterdir())
    if ({p.name for p in entries} != expected
            or any(p.is_symlink() or not p.is_file() for p in entries)):
        raise ValueError("export table set differs")
    manifest = strict_json((directory / "manifest.json").read_text(encoding="utf-8"))
    if count(manifest, "source_size_bytes") != input_size:
        raise ValueError("manifest input size differs")
    quality_check(manifest, checkpoints)
    wanted = [key for key in sc.WHERE if checkpoints or not key.startswith("cp_")]
    text = log.read_text(encoding="utf-8", errors="replace")
    lines = {sc.WHERE[key][0] for key in wanted}
    if any(len(list(line.pattern.finditer(text))) != 1 for line in lines):
        raise ValueError("summary counter is missing or duplicated")
    counters = sc.read(text, wanted)
    if any(value is None for value in counters.values()):
        raise ValueError("summary counter is missing")
    if counters["overlay_rows_offered"] != sum(counters[key] for key in (
            "overlay_decoded_ok", "overlay_decode_errors", "overlay_raw_skip",
            "overlay_not_in_table", "overlay_no_field_name")):
        raise ValueError("summary overlay counters do not reconcile")
    zero_summary = ("overlay_decode_errors", "malformed_packets", "unknown_chunks",
                    "frame_non_finite_times", "event_layout_mismatches",
                    *(key for key in SINK_ZERO if key != "overlay_decoded_err"))
    if any(counters[key] for prefix in ("", "cp_") for name in zero_summary
           if (key := prefix + name) in counters):
        raise ValueError("summary error counter is nonzero")
    # Source path is private, elapsed time is inherently different. Everything
    # else, including players and raw header metadata, participates in equality
    # without its values being copied into the shareable report.
    normalized = {key: value for key, value in manifest.items()
                  if key not in ("source_file", "elapsed_ms")}
    tables = {}
    for name in names:
        path = directory / name
        with path.open("rb") as stream:
            if path.stat().st_size < 12 or stream.read(4) != b"PAR1":
                raise ValueError("invalid Parquet envelope")
            stream.seek(-4, os.SEEK_END)
            if stream.read(4) != b"PAR1":
                raise ValueError("invalid Parquet envelope")
        tables[name] = {"bytes": path.stat().st_size, "sha256": sha256_file(path)}
    return {"tables": tables, "manifest_sha256": json_hash(normalized),
            "counters": counters, "output_bytes": sum(p.stat().st_size for p in entries),
            "parquet_bytes": sum(row["bytes"] for row in tables.values())}


def resource_metrics(usage, wall: float, system: str) -> dict:
    """Normalize known getrusage units; unsupported metrics remain null."""
    result = {key: None for key in ("cpu_user_seconds", "cpu_system_seconds", "cpu_total_seconds",
                                  "cpu_utilization_percent", "peak_rss_bytes",
                                  "io_input_blocks", "io_output_blocks")}
    if usage is None:
        return result
    result.update(cpu_user_seconds=usage.ru_utime, cpu_system_seconds=usage.ru_stime,
                  cpu_total_seconds=usage.ru_utime + usage.ru_stime,
                  cpu_utilization_percent=100 * (usage.ru_utime + usage.ru_stime) / wall,
                  io_input_blocks=usage.ru_inblock, io_output_blocks=usage.ru_oublock)
    scale = {"Linux": 1024, "Darwin": 1}.get(system)
    if scale is not None and usage.ru_maxrss > 0:
        result["peak_rss_bytes"] = int(usage.ru_maxrss * scale)
    return result


def measure(command: list[str], log: Path, timeout: float) -> dict:
    """Called only in a fresh worker: its only child is the measured command."""
    try:
        import resource
    except ImportError:
        resource = None
    start = time.perf_counter()
    with log.open("xb") as stream:
        try:
            with subprocess.Popen(command, stdout=stream, stderr=subprocess.STDOUT) as process:
                expired = threading.Event()

                def stop():
                    if process.poll() is None:
                        expired.set()
                        try:
                            process.kill()
                        except ProcessLookupError:
                            pass

                # wait(timeout) polls with increasing sleeps on POSIX, adding up
                # to 50 ms to short runs. A watchdog permits a blocking wait.
                timer = threading.Timer(timeout, stop)
                timer.daemon = True
                timer.start()
                try:
                    code = process.wait()
                    elapsed = time.perf_counter() - start
                finally:
                    timer.cancel()
                    timer.join()
                status = "timeout" if expired.is_set() else "completed"
        except OSError:
            status, code = "launch_error", None
            elapsed = time.perf_counter() - start
    usage = resource.getrusage(resource.RUSAGE_CHILDREN) if resource else None
    values = {"wall_seconds": elapsed, **resource_metrics(usage, elapsed, platform.system())}
    return {"status": status, "exit_code": code, "measurements": values}


def run_worker(command: list[str], directory: Path, timeout: float) -> dict:
    request, result, log = (directory / name for name in ("request.json", "measurement.json", "parser.log"))
    request.write_text(json.dumps({"command": command, "timeout": timeout}), encoding="utf-8")
    with (directory / "worker.log").open("xb") as errors:
        process = subprocess.run([sys.executable, str(Path(__file__).resolve()), "--worker",
                                  str(request), str(result), str(log)],
                                 stdout=errors, stderr=subprocess.STDOUT, check=False)
    if process.returncode or not result.is_file():
        return {"status": "worker_error", "exit_code": None, "measurements": {}}
    return strict_json(result.read_text(encoding="utf-8"))


def validate_measurements(values: dict) -> None:
    required = METRICS[:6] + ("io_input_blocks", "io_output_blocks")
    for name in required:
        value = values[name]
        if value is None and name != "wall_seconds":
            continue
        if (isinstance(value, bool) or not isinstance(value, (int, float))
                or not math.isfinite(value) or value < 0
                or (name in ("wall_seconds", "peak_rss_bytes") and value == 0)):
            raise ValueError("invalid resource measurement")
    cpu = [values[key] for key in ("cpu_user_seconds", "cpu_system_seconds", "cpu_total_seconds",
                                   "cpu_utilization_percent")]
    if any(value is None for value in cpu) and not all(value is None for value in cpu):
        raise ValueError("partial CPU measurement")


def plan(warmups: int, repeats: int):
    for phase, rounds in (("warmup", warmups), ("measured", repeats)):
        for round_number in range(rounds):
            order = ("baseline", "candidate") if round_number % 2 == 0 else ("candidate", "baseline")
            for variant in order:
                yield phase, round_number, variant


def summarize(samples: list[dict]) -> dict:
    result = {}
    for variant in ("baseline", "candidate"):
        measured = [s["measurements"] for s in samples if s["phase"] == "measured" and s["variant"] == variant]
        result[variant] = {}
        for metric in METRICS:
            values = [s.get(metric) for s in measured]
            if not values or any(value is None for value in values):
                result[variant][metric] = None
                continue
            median = statistics.median(values)
            result[variant][metric] = {"samples": len(values), "median": median,
                                       "min": min(values), "max": max(values),
                                       "sample_stdev": statistics.stdev(values) if len(values) > 1 else None,
                                       "median_absolute_deviation": statistics.median(abs(x - median) for x in values)}
    return result


def paired_ratios(samples: list[dict]) -> dict:
    pairs = {}
    for sample in samples:
        if sample["phase"] == "measured":
            pairs.setdefault(sample["round"], {})[sample["variant"]] = sample["measurements"]
    result = {}
    for metric in ("wall_seconds", "cpu_total_seconds", "peak_rss_bytes"):
        ratios = []
        for pair in pairs.values():
            baseline, candidate = pair["baseline"].get(metric), pair["candidate"].get(metric)
            if baseline is None or candidate is None or baseline <= 0:
                ratios = []
                break
            ratios.append(candidate / baseline)
        result[metric] = ({"candidate_over_baseline": ratios,
                           "median_candidate_over_baseline": statistics.median(ratios)} if ratios else None)
    return result


def same_output(left: dict, right: dict) -> bool:
    return all(left[key] == right[key] for key in ("tables", "manifest_sha256", "counters"))


def provenance(paths: dict[str, Path]) -> dict:
    return {role: {"sha256": sha256_file(path), "bytes": path.stat().st_size} for role, path in paths.items()}


def benchmark(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-exe", type=Path, required=True)
    parser.add_argument("--candidate-exe", type=Path, required=True)
    parser.add_argument("--replay", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True, help="new private directory for per-run logs")
    parser.add_argument("--output", type=Path, required=True, help="new JSON report, without private paths or raw values")
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--timeout", type=float, default=1800)
    parser.add_argument("--checkpoints", action="store_true")
    parser.add_argument("--keep-exports", action="store_true", help="retain private Parquet and manifests for independent checks")
    args = parser.parse_args(argv)
    if args.warmups < 0 or args.repeats < 1 or not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("nonnegative warmups, positive repeats and a positive finite timeout are required")
    paths = {"baseline": args.baseline_exe.resolve(), "candidate": args.candidate_exe.resolve(),
             "input": args.replay.resolve(), "harness": Path(__file__).resolve(),
             "summary_schema": Path(sc.__file__).resolve(),
             "file_helpers": Path(__file__).resolve().with_name("atomic_io.py")}
    if any(not path.is_file() for path in paths.values()):
        parser.error("input and both executables must be existing files")
    if (args.work_dir.exists() or args.work_dir.is_symlink() or args.output.exists()
            or args.output.is_symlink() or args.output.resolve() in paths.values()
            or args.output.resolve().is_relative_to(args.work_dir.resolve())
            or args.work_dir.resolve().is_relative_to(args.output.resolve())):
        parser.error("work directory and report must be new paths, outside each other, and must not alias inputs")
    args.work_dir.mkdir(parents=True, mode=0o700)
    before = provenance(paths)
    report = {
        "schema_version": 1, "status": "running", "equivalent": False, "comparison": None,
        "created_utc": datetime.now(timezone.utc).isoformat(), "provenance_before": before,
        "environment": {"system": platform.system(), "release": platform.release(),
                        "machine": platform.machine(), "python": platform.python_version(),
                        "logical_cpu_count": os.cpu_count()},
        "configuration": {"warmup_pairs": args.warmups, "measured_pairs": args.repeats,
                          "checkpoints": args.checkpoints, "timeout_seconds": args.timeout},
        "equivalence_scope": "exact Parquet bytes, all nonvolatile manifest values and required summary counters",
        "unmeasured": ["allocation_count", "allocated_bytes", "io_time_seconds", "independent_semantic_correctness"],
        "resource_scope": "child CPU includes waited descendants; RSS is the largest reaped process peak, not summed process-tree memory",
        "samples": [], "failures": [],
    }
    expected = None
    for phase, number, variant in plan(args.warmups, args.repeats):
        directory = args.work_dir / f"{phase}-{number:03d}-{variant}"
        directory.mkdir(mode=0o700)
        export = directory / "export"
        command = [str(paths[variant]), "export", str(paths["input"]), "--out", str(export)]
        if args.checkpoints:
            command.append("--checkpoints")
        sample = {"phase": phase, "round": number, "variant": variant, "valid": False,
                  "measurements": {}}
        try:
            measurement = run_worker(command, directory, args.timeout)
            sample.update(status=measurement["status"], exit_code=measurement["exit_code"])
            values = measurement["measurements"]
            if values:
                validate_measurements(values)
                sample["measurements"] = values
            if measurement["status"] != "completed" or measurement["exit_code"] != 0:
                raise ValueError("export process failed")
            validate_measurements(values)
            output = fingerprint(export, directory / "parser.log", args.checkpoints, before["input"]["bytes"])
            values.update(output_bytes=output["output_bytes"], parquet_bytes=output["parquet_bytes"],
                          input_mib_per_second=before["input"]["bytes"] / (1024 ** 2) / values["wall_seconds"])
            sample["output_fingerprint"] = json_hash({key: output[key] for key in ("tables", "manifest_sha256", "counters")})
            if expected is not None and not same_output(expected, output):
                raise ValueError("export outputs or counters differ")
            expected = output
            sample["valid"] = True
        except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as exc:
            # Raw exception messages can contain source paths or decoder output.
            (directory / "error.txt").write_text(str(exc), encoding="utf-8")
            report["failures"].append({"phase": phase, "round": number, "variant": variant,
                                       "reason": "export failed validation; inspect private run logs"})
        report["samples"].append(sample)
        if not args.keep_exports and export.is_dir() and not export.is_symlink():
            shutil.rmtree(export)
        atomic_write_text(args.output, json.dumps(report, indent=2, allow_nan=False) + "\n")
        print(f"{phase} {number + 1} {variant}: {'valid' if sample['valid'] else 'INVALID'}", flush=True)
        if not sample["valid"]:
            break
    try:
        after = provenance(paths)
        report["provenance_after"] = after
        if before != after:
            report["failures"].append({"reason": "input, executable or harness changed during measurement"})
    except OSError:
        report["failures"].append({"reason": "input, executable or harness unavailable after measurement"})
    report["equivalent"] = not report["failures"]
    report["status"] = "completed" if report["equivalent"] else "invalid"
    report["summary"] = summarize([s for s in report["samples"] if s["valid"]])
    if report["equivalent"]:
        report["comparison"] = paired_ratios(report["samples"])
        report["reference_output"] = expected
    atomic_write_text(args.output, json.dumps(report, indent=2, allow_nan=False) + "\n")
    print("COMPLETE: equivalent exports" if report["equivalent"] else "INVALID: no performance comparison")
    return 0 if report["equivalent"] else 1


def main(argv=None) -> int:
    try:
        return benchmark(argv)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError):
        # Filesystem and JSON exceptions can embed private paths. Any earlier
        # checkpoint still says running/invalid and never offers a comparison.
        print("FAILED: benchmark could not finish; no completed comparison", file=sys.stderr)
        return 1


if __name__ == "__main__":
    if len(sys.argv) == 5 and sys.argv[1] == "--worker":
        request = strict_json(Path(sys.argv[2]).read_text(encoding="utf-8"))
        result = measure(request["command"], Path(sys.argv[4]), request["timeout"])
        atomic_write_text(Path(sys.argv[3]), json.dumps(result, allow_nan=False) + "\n")
    else:
        raise SystemExit(main())
