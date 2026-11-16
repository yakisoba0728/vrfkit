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
    for line in log.splitlines():
        words = line.split("\t")
        if words[0] in ("C", "M"):
            assert len(words) == 4
            key = tuple(words[:3])
            assert key not in observations, "duplicate counter"
            observations[key] = int(words[3])
        else:
            assert words[0] == "R" and len(words) == 3
            assert words[1] not in rows
            rows[words[1]] = int(words[2])
    observed = {}
    for scope, expected in (("M", quality), ("C", quality["checkpoints"])):
        for category in ("net", "sink"):
            actual = {key[2]: value for key, value in observations.items()
                      if key[:2] == (scope, category)}
            assert actual == expected[category], (scope, category, "counter coverage/value")
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
    return {"callback_tables_byte_equal": 7, "snapshot_chunks": scalars["checkpoint_chunks"],
            "scalar_counters_equal": len(scalars), "counter_maps_equal": observed,
            "callback_rows": rows, "mismatches": 0}


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
            for row in batch.filter(pc.equal(batch.column("field_name"), "RoundPhase")).to_pylist():
                assert row["group_path"] == "/Game/GameModes/Bomb/BombGameState.BombGameState_C"
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
    for name in ("source", "exe", "checkpoint-probe", "gameplay-probe", "corpus", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
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
    inputs = {}
    for path in args.corpus.rglob("*"):
        if path.is_file() and path.suffix.casefold() in (".vrf", ".replay"):
            digest = sha(path)
            assert digest not in inputs, "duplicate input"
            inputs[digest] = path.resolve()
    assert len(inputs) == 6 and set(inputs) == set(expected), "six-input scope mismatch"
    source_commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=args.source, text=True).strip()
    source_tree = subprocess.check_output(["git", "rev-parse", "HEAD^{tree}"], cwd=args.source, text=True).strip()
    binaries = {name: sha(path) for name, path in (("cli", args.exe),
        ("checkpoint_probe", args.checkpoint_probe), ("gameplay_probe", args.gameplay_probe))}
    aliases = args.output / "corpus-aliases"
    aliases.mkdir()
    for digest, path in inputs.items():
        (aliases / (digest + ".vrf")).symlink_to(path)
    audit = args.output / "corpus-work"
    run("public-corpus", [sys.executable, "-W", "error", str(args.source / "tools/verify_build_corpus.py"),
        "--exe", str(args.exe), "--corpus", str(aliases), "--work-dir", str(audit),
        "--output", str(args.output / "public-corpus.json"), "--jobs", "1"])
    run("typed-raw", [sys.executable, "-W", "error", str(args.source / "tools/validate_type_evidence.py"),
        str(audit), str(args.source / "tools/fixtures/public_fixture_type_evidence.json"),
        "--compare-typed", "--output", str(args.output / "typed-evidence.json")])
    assert set(json.loads((audit / "inputs.json").read_text())) == set(inputs)
    parity, checkpoint, gameplay, candidates = [], [], [], []
    for digest, replay in inputs.items():
        export = audit / digest / "export"
        manifest = json.loads((export / "manifest.json").read_text())
        previous = expected[digest]["output"]
        tables = compare_tables(export, previous["tables"])
        stable = canonical({key: value for key, value in manifest.items() if key not in ("source_file", "elapsed_ms")})
        assert stable == previous["manifest_sha256"], "nonvolatile manifest changed"
        counters = summary_counters.read((audit / digest / "export.log").read_text(), summary_counters.WHERE)
        assert counters == previous["counters"], "summary counters changed"
        parity.append({"build": expected[digest]["build"], "input_sha256": digest,
            "exact_parquet_tables": len(tables), "all_nonvolatile_manifest_values_equal": True,
            "all_summary_counters_equal": True, "tables": tables, "manifest_sha256": stable})
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
    assert binaries == {name: sha(path) for name, path in (("cli", args.exe),
        ("checkpoint_probe", args.checkpoint_probe), ("gameplay_probe", args.gameplay_probe))}
    receipt = {"complete": True, "source_commit": source_commit, "source_tree": source_tree,
        "binaries_sha256": binaries, "public_replays": 6, "private_replays": 0,
        "checkpoint_chunks": sum(row["snapshot_chunks"] for row in checkpoint),
        "parquet_tables_byte_equal": sum(row["exact_parquet_tables"] for row in parity),
        "callback_tables_byte_equal": sum(row["callback_tables_byte_equal"] for row in checkpoint),
        "native_observations": sum(row["native_compared"] for row in gameplay),
        "phase_observations": sum(row["phase_compared"] for row in gameplay),
        "mismatches": 0, "performance_claim": None, "steps": steps}
    save("receipt.json", receipt)
    print(json.dumps({key: value for key, value in receipt.items() if key not in ("steps", "binaries_sha256")}), flush=True)


if __name__ == "__main__":
    main()
