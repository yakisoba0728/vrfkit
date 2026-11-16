"""Fresh Phase C functional acceptance; never executes a resource benchmark.

Inputs and verbose local outputs can contain player data. Only the sanitized
JSON receipts are eligible for a reviewed public evidence commit.
"""
from collections import Counter
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys

import pyarrow.compute as pc
import pyarrow.parquet as pq

from candidate_inventory import inventory as candidate_inventory
from identity_oracle import compare as compare_identity
import decoder_delta

if not __debug__:
    raise RuntimeError("QA requires assertions; remove Python optimization flags")


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def canonical(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                    allow_nan=False).encode()).hexdigest()


def compare_tables(export, expected):
    actual = {path.name: {"bytes": path.stat().st_size, "sha256": sha(path)}
              for path in sorted(export.glob("*.parquet"))}
    assert actual and actual == expected, "table names, bytes or content differ"
    return actual


def compare_checkpoints(log, export, callback_export, quality):
    observations = {}
    rows = {}
    diagnostics = {}
    for line in log.splitlines():
        words = line.split("\t")
        if words[0] in ("C", "M"):
            assert len(words) == 4
            key = tuple(words[:3])
            assert key not in observations, "duplicate counter"
            observations[key] = int(words[3])
        elif words[0] == "D":
            assert len(words) == 3 and words[1] not in diagnostics
            diagnostics[words[1]] = int(words[2])
        else:
            assert words[0] == "R" and len(words) == 3
            assert words[1] not in rows
            rows[words[1]] = int(words[2])
    observed = {}
    for scope, expected in (("M", quality), ("C", quality["checkpoints"])):
        for category in ("net", "sink"):
            actual = {key[2]: value for key, value in observations.items()
                      if key[:2] == (scope, category)}
            numeric = {key: value for key, value in expected[category].items() if type(value) is int}
            optional = {key: value for key, value in expected[category].items() if type(value) is not int}
            assert optional == ({} if category == "net" else {
                "struct_blob_first_error": None, "movement_first_error": None}), "unexpected diagnostic value or key"
            # The probe separately requires both Reader error options to be
            # absent in every checkpoint and in the final main summary.
            assert actual == numeric, (scope, category, "counter coverage/value")
            observed[scope + "_" + category] = len(actual)
    scalars = {key[2]: value for key, value in observations.items() if key[:2] == ("C", "summary")}
    expected_scalars = {key: value for key, value in quality["checkpoints"].items()
                        if type(value) is int}
    assert scalars == expected_scalars, "checkpoint scalar coverage/value"
    tables = {path.name: {"bytes": path.stat().st_size, "sha256": sha(path)}
              for path in sorted(export.glob("checkpoint_*.parquet"))}
    assert len(tables) == 7
    compare_tables(callback_export, tables)
    names = {"guid_entries": "checkpoint_guid_entries", "groups": "checkpoint_export_groups",
             "exports": "checkpoint_export_fields", "fields": "checkpoint_fields",
             "actors": "checkpoint_actors", "blocks": "checkpoint_blocks", "guids": "checkpoint_net_guids"}
    assert set(rows) == set(names)
    for key, table in names.items():
        with pq.ParquetFile(export / (table + ".parquet"), pre_buffer=False) as file:
            assert rows[key] == file.metadata.num_rows, key
    assert set(diagnostics) == {"main_stream_diagnostics", "checkpoint_stream_diagnostics"}
    return {"callback_tables_byte_equal": 7, "snapshot_chunks": scalars["checkpoint_chunks"],
            "scalar_counters_equal": len(scalars), "counter_maps_equal": observed,
            "first_error_options_checked": 4, "callback_rows": rows,
            "observed_diagnostic_list_counts": diagnostics, "mismatches": 0}


def compare_gameplay(log, export, validate_event_payload):
    supported = {"roundStarted": (1, 2, "EReplayEventGroup::RoundStart"),
        "characterDeath": (2, 8, "EReplayEventGroup::CharacterDeath"),
        "spikePlanted": (0, 4, "EReplayEventGroup::SpikePlanted"),
        "spikeDefused": (0, 5, "EReplayEventGroup::SpikeDefused"),
        "spikeExploded": (0, 6, "EReplayEventGroup::SpikeExploded")}
    native, phase = [], []
    counts, ignored, phases = Counter(), Counter(), Counter()
    with pq.ParquetFile(export / "events.parquet", pre_buffer=False) as file:
        events = file.read().to_pylist()
    for row in events:
        if row["group"] not in supported:
            ignored[row["group"]] += 1
            continue
        word_count, tag, name = supported[row["group"]]
        assert validate_event_payload(row, word_count, tag, name) is None
        raw = row["raw_payload"]
        words = [struct.unpack_from("<I", raw, 4 + 4 * index)[0] for index in range(word_count)]
        words += ["null"] * (2 - word_count)
        bits = struct.unpack("<I", raw[-4:])[0]
        native.append(f"N\t{row['group']}\t{row['time1']}\t{row['time2']}\t{bits}\t{words[0]}\t{words[1]}")
        counts[row["group"]] += 1
    columns = ["field_name", "time_ms", "packet_id", "channel_index", "actor_net_guid",
               "object_net_guid", "group_path", "compatible_checksum", "bit_count", "raw_bits",
               "value_i64", "value_f64", "value_bool", "value_str"]
    with pq.ParquetFile(export / "fields.parquet", pre_buffer=False) as file:
        for batch in file.iter_batches(batch_size=4096, columns=columns, use_threads=False):
            for row in batch.filter(pc.equal(batch.column("field_name"), "MulticastSetPhase.NewPhase")).to_pylist():
                assert row["group_path"] == "/Game/GameModes/Bomb/BombGameState.BombGameState_C_ClassNetCache"
                assert row["compatible_checksum"] == 3954716686 and row["object_net_guid"] is None
                assert row["bit_count"] == 8 and row["raw_bits"] is not None and len(row["raw_bits"]) == 1
                value = row["raw_bits"][0]
                assert row["value_i64"] == value
                assert all(row[name] is None for name in ("value_f64", "value_bool", "value_str"))
                phase.append(f"P\t{row['time_ms']}\t{row['packet_id']}\t{row['channel_index']}\t{row['actor_net_guid']}\t{value}")
                phases[value] += 1
    lines = log.splitlines()
    assert [line for line in lines if line.startswith("N\t")] == native
    assert [line for line in lines if line.startswith("P\t")] == phase
    actual_ignored = {parts[1]: int(parts[2]) for parts in map(lambda line: line.split("\t"), lines)
                      if parts[0] == "U"}
    assert actual_ignored == dict(ignored)
    summaries = [line.split("\t") for line in lines if line.startswith("S\t")]
    assert len(summaries) == 1 and int(summaries[0][1]) == len(events)
    return {"native_counts": dict(counts), "phase_counts": dict(phases),
            "native_compared": len(native), "phase_compared": len(phase),
            "skipped_checkpoints": int(summaries[0][2]), "mismatches": 0}


def main():
    parser = argparse.ArgumentParser()
    for name in ("source", "exe", "checkpoint-probe", "gameplay-probe", "identity-probe", "corpus", "output",
                 "extra-corpus", "extra-baseline", "historical-exports", "build-receipt"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--focus-route", action="store_true")
    parser.add_argument("--reuse-audit", type=Path,
                        help="reuse a completed, receipt-verified fresh audit from this same frozen CLI")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    sys.path.insert(0, str(args.source / "tools"))
    import summary_counters
    from extract_kill_ledger import validate_event_payload

    def save(name, value):
        (args.output / name).write_text(json.dumps(value, indent=2) + "\n")

    steps = []
    def run(label, command):
        log = args.output / (label + ".log")
        with log.open("w") as stream:
            code = subprocess.run(command, cwd=args.source, stdout=stream, stderr=subprocess.STDOUT).returncode
        steps.append({"name": label, "exit_code": code, "log_sha256": sha(log)})
        save("steps.json", steps)
        print(label + ": exit " + str(code), flush=True)
        if code:
            raise RuntimeError(label + " failed; inspect local log")
        return log.read_text()

    baseline = json.loads((args.source / "docs/evidence/phase-b-cli-public-parity.json").read_text())
    expected = {row["input"]["sha256"]: row for row in baseline["runs"]}
    historical = set(expected)
    extra = json.loads((args.extra_baseline / "baseline.json").read_text())
    assert extra["complete"] and len(extra["runs"]) == 4
    assert extra["executable_sha256"] == "29fa495b19b7dbce2bfe7bbedc1dc9be2492a4c6a5aca2a9d1468d3d74c657d4"
    for row in extra["runs"]:
        assert row["input"]["sha256"] not in expected
        expected[row["input"]["sha256"]] = row
    inputs = {}
    for root in (args.corpus, args.extra_corpus):
        for path in root.rglob("*"):
            if path.is_file() and path.suffix.casefold() in (".vrf", ".replay"):
                digest = sha(path)
                assert digest not in inputs, "duplicate input"
                inputs[digest] = path.resolve()
    assert len(inputs) == 10 and set(inputs) == set(expected), "ten-input scope mismatch"
    source_commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=args.source, text=True).strip()
    source_tree = subprocess.check_output(["git", "rev-parse", "HEAD^{tree}"], cwd=args.source, text=True).strip()
    binaries = {name: sha(path) for name, path in (("cli", args.exe),
        ("checkpoint_probe", args.checkpoint_probe), ("gameplay_probe", args.gameplay_probe), ("identity_probe", args.identity_probe))}
    build = json.loads(args.build_receipt.read_text())
    assert all(sha(args.source / name) == digest for name, digest in build["runtime_sources_sha256"].items())
    for name, artifact in (("cli", "vrfkit"), ("checkpoint_probe", "qa_checkpoint_probe"),
                           ("gameplay_probe", "qa_gameplay_probe"), ("identity_probe", "qa_identity_probe")):
        assert binaries[name] == build["binaries"]["artifacts"][artifact]["sha256"]
    aliases = args.output / "corpus-aliases"
    aliases.mkdir()
    for digest, path in inputs.items():
        (aliases / (digest + ".vrf")).symlink_to(path)
    if args.reuse_audit is not None:
        audit = args.reuse_audit / "corpus-work"
        prior = json.loads((args.reuse_audit / "public-corpus.json").read_text())
        typed = json.loads((args.reuse_audit / "typed-evidence.json").read_text())
        assert prior["provenance"]["exe_sha256"] == binaries["cli"]
        assert prior["provenance"]["unique_replays"] == 10
        assert not prior["executable_changed"] and not prior["failures"] and not prior["build_errors"]
        tracked_sources = sorted((args.source / "crates").rglob("*.rs")) + sorted((args.source / "crates").rglob("Cargo.toml")) + [args.source / "Cargo.lock", args.source / "Cargo.toml"]
        source_digest = hashlib.sha256("\n".join(f"{path.relative_to(args.source).as_posix()} {sha(path)}" for path in tracked_sources).encode()).hexdigest()
        assert source_digest == prior["provenance"]["parser_sources_sha256"]
        assert typed["failure_count"] == typed["typed_mismatch_count"] == 0 and not typed["missing"]
        save("public-corpus.json", prior)
        save("typed-evidence.json", typed)
        steps.append({"name": "reuse_receipt_verified_fresh_audit", "exit_code": 0,
            "audit_report_sha256": sha(args.reuse_audit / "public-corpus.json"),
            "typed_report_sha256": sha(args.reuse_audit / "typed-evidence.json"),
            "original_runner_commit": prior["provenance"]["parser_commit"],
            "same_executable_and_parser_sources": True})
        save("steps.json", steps)
    else:
        audit = args.output / "corpus-work"
        run("public-corpus", [sys.executable, "-W", "error", str(args.source / "tools/verify_build_corpus.py"),
            "--exe", str(args.exe), "--corpus", str(aliases), "--work-dir", str(audit),
            "--output", str(args.output / "public-corpus.json"), "--jobs", "1"])
        run("typed-raw", [sys.executable, "-W", "error", str(args.source / "tools/validate_type_evidence.py"),
            str(audit), str(args.source / "tools/fixtures/public_fixture_type_evidence.json"),
            "--compare-typed", "--output", str(args.output / "typed-evidence.json")])
    assert set(json.loads((audit / "inputs.json").read_text())) == set(inputs)
    parity, checkpoint, gameplay, candidates, identity = [], [], [], [], []
    for digest, replay in inputs.items():
        export = audit / digest / "export"
        manifest = json.loads((export / "manifest.json").read_text())
        previous = expected[digest]["output"]
        delta = {"parents": 0, "children": 0, "empty_trailers": 0, "unchanged_rows": 0}
        if args.focus_route and manifest["replay_build"] == "++Ares-Core+release-12.05":
            before = args.historical_exports / digest / "export"
            compare_tables(before, previous["tables"])
            delta = decoder_delta.compare_fields(before / "fields.parquet", export / "fields.parquet")
            tables = {path.name: {"bytes": path.stat().st_size, "sha256": sha(path)}
                      for path in sorted(export.glob("*.parquet"))}
            assert {key: value for key, value in tables.items() if key != "fields.parquet"} == {
                key: value for key, value in previous["tables"].items() if key != "fields.parquet"}
            exact_tables = 12
        else:
            tables = compare_tables(export, previous["tables"])
            exact_tables = len(tables)
        normalized = decoder_delta.normalize_manifest(manifest, delta) if args.focus_route else manifest
        stable = canonical({key: value for key, value in normalized.items() if key not in ("source_file", "elapsed_ms")})
        assert stable == previous["manifest_sha256"], "nonvolatile manifest changed"
        counters = summary_counters.read((audit / digest / "export.log").read_text(), summary_counters.WHERE)
        normalized_counters = decoder_delta.normalize_counters(counters, delta) if args.focus_route else counters
        assert normalized_counters == previous["counters"], "summary counters changed"
        parity.append({"build": expected[digest]["build"], "input_sha256": digest,
            "baseline": "historical accepted Phase B" if digest in historical else "fresh frozen Phase B CLI",
            "exact_parquet_tables": exact_tables, "unexplained_manifest_differences": 0,
            "unexplained_summary_counter_differences": 0, "intentional_focus_delta": delta,
            "tables": tables, "normalized_manifest_sha256": stable})
        callback_export = args.output / (digest + "-callback")
        log = run(digest + "-checkpoint", [str(args.checkpoint_probe), str(replay), str(callback_export)])
        report = compare_checkpoints(log, export, callback_export, manifest["quality"])
        checkpoint.append({"build": expected[digest]["build"], "input_sha256": digest, **report})
        log = run(digest + "-gameplay", [str(args.gameplay_probe), str(replay)])
        report = compare_gameplay(log, export, validate_event_payload)
        assert report["skipped_checkpoints"] == manifest["quality"]["checkpoints"]["checkpoint_chunks"]
        gameplay.append({"build": expected[digest]["build"], "input_sha256": digest, **report})
        candidates.append({"build": expected[digest]["build"], "input_sha256": digest,
                           "tables": candidate_inventory(export)})
        assert sha(replay) == digest
        save("export-equivalence.json", parity)
        save("checkpoint-callbacks.json", checkpoint)
        save("gameplay.json", gameplay)
        save("decoder-candidates.json", candidates)
    print("all ten checkpoint callback/table/counter comparisons passed", flush=True)
    for digest, replay in inputs.items():
        export = audit / digest / "export"
        report = compare_identity(args.identity_probe, replay, export, args.output / (digest + "-identity.stderr"))
        identity.append({"build": expected[digest]["build"], "input_sha256": digest, **report})
        save("identity.json", identity)
        print(digest[:12] + ": identity exact joins passed", flush=True)
    assert binaries == {name: sha(path) for name, path in (("cli", args.exe),
        ("checkpoint_probe", args.checkpoint_probe), ("gameplay_probe", args.gameplay_probe), ("identity_probe", args.identity_probe))}
    receipt = {"complete": True, "source_commit": source_commit, "source_tree": source_tree,
        "executable_build_commit": build["source_commit"], "executable_build_tree": build["source_tree"],
        "build_receipt_sha256": sha(args.build_receipt), "runtime_sources_match_frozen_build": True,
        "binaries_sha256": binaries, "public_replays": 10, "private_replays": 0,
        "checkpoint_chunks": sum(row["snapshot_chunks"] for row in checkpoint),
        "parquet_tables_byte_equal": sum(row["exact_parquet_tables"] for row in parity),
        "callback_tables_byte_equal": sum(row["callback_tables_byte_equal"] for row in checkpoint),
        "native_observations": sum(row["native_compared"] for row in gameplay),
        "phase_observations": sum(row["phase_compared"] for row in gameplay),
        "identity_movement_rows": sum(row["movement_rows_compared"] for row in identity),
        "spawned_character_raw_values": sum(row["spawned_character_raw_values_compared"] for row in identity),
        "mismatches": 0, "performance_claim": None, "steps": steps}
    save("receipt.json", receipt)
    print(json.dumps({key: value for key, value in receipt.items() if key not in ("steps", "binaries_sha256")}), flush=True)


if __name__ == "__main__":
    main()
