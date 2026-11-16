"""Independent main-stream interval join over exported raw evidence.

The Rust probe supplies actual public API results. This comparator constructs
packet intervals from actor rows and packed GUID declarations independently,
then checks every movement, resolution, and exact borrowed proof tuple. It
never uses a final GUID path, wall-clock proximity, or player account data.
"""
from bisect import bisect_left
from collections import Counter, defaultdict
from functools import lru_cache
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys

import pyarrow.compute as pc
import pyarrow.parquet as pq

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "tools"))
from validate_type_evidence import Bits

GROUPS = {
    "/Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C",
    "/Game/GameModes/_Development/Swiftplay_EndOfRoundCredits/Swiftplay_EoRCredits_PlayerState.Swiftplay_EoRCredits_PlayerState_C",
}
FLOATS = ("pos_x", "pos_y", "pos_z", "yaw", "pitch", "vel_x", "vel_y", "vel_z")


def declarations(export):
    result = []
    with pq.ParquetFile(export / "fields.parquet", pre_buffer=False) as file:
        for batch in file.iter_batches(batch_size=4096, use_threads=False):
            for row in batch.filter(pc.equal(batch["field_name"], "SpawnedCharacter")).to_pylist():
                assert row["group_path"] in GROUPS and row["compatible_checksum"] == 1413303948
                assert row["actor_net_guid"] != 0 and row["object_net_guid"] is None
                assert all(row[key] is None for key in ("value_bool", "value_f64", "value_str"))
                assert type(row["value_i64"]) is int and 0 <= row["value_i64"] < 2**32
                assert row["raw_bits"] is not None and row["bit_count"] > 0 and row["bit_count"] % 8 == 0
                raw = Bits(row["raw_bits"], row["bit_count"])
                assert raw.int_packed() == row["value_i64"] and raw.remaining() == 0
                result.append(row)
    assert all(a["packet_id"] <= b["packet_id"] for a, b in zip(result, result[1:]))
    return result


def declaration_line(row):
    return ["D", *(str(row[key]) for key in ("packet_id", "time_ms", "channel_index",
        "actor_net_guid", "value_i64", "bit_count", "handle", "compatible_checksum", "group_path")),
        row["raw_bits"].hex()]


class Oracle:
    """Offline intervals retain future rows but every query cuts strictly by packet."""

    def __init__(self, actors, fields):
        self.fields = fields
        self.claims = defaultdict(list)
        for row in fields:
            self.claims[row["actor_net_guid"]].append(row)
        self.claim_packets = {key: [row["packet_id"] for row in rows] for key, rows in self.claims.items()}
        self.epochs = defaultdict(list)
        occupied = {}
        ended = {}

        def close(guid, row):
            if self.epochs[guid] and self.epochs[guid][-1]["end"] is None:
                self.epochs[guid][-1]["end"] = row
            ended[guid] = row["packet_id"]
            for channel in [key for key, value in occupied.items() if value == guid]:
                del occupied[channel]

        assert all(a["packet_id"] <= b["packet_id"] for a, b in zip(actors, actors[1:]))
        for row in actors:
            guid, channel = row["actor_net_guid"], row["channel_index"]
            assert guid != 0 and row["event"] in ("open", "close", "dormant")
            if row["event"] == "close":
                close(guid, row)
            elif row["event"] == "dormant":
                if occupied.get(channel) == guid:
                    del occupied[channel]
            else:
                other = occupied.get(channel)
                if other is not None and other != guid:
                    close(other, row)
                active = guid in occupied.values()
                if not self.epochs[guid] or self.epochs[guid][-1]["end"] is not None:
                    self.epochs[guid].append({"open": row, "end": None,
                        "previous_end": ended.get(guid), "conflict": None})
                elif active and self.epochs[guid][-1]["conflict"] is None:
                    self.epochs[guid][-1]["conflict"] = row
                occupied[channel] = guid
        self.open_packets = {key: [epoch["open"]["packet_id"] for epoch in epochs]
                             for key, epochs in self.epochs.items()}

    def epoch(self, guid, before):
        index = bisect_left(self.open_packets.get(guid, []), before) - 1
        return self.epochs[guid][index] if index >= 0 else None

    @staticmethod
    def active(epoch, before):
        return epoch is not None and all(epoch[key] is None or epoch[key]["packet_id"] >= before
                                         for key in ("end", "conflict"))

    @lru_cache(maxsize=2048)
    def movement(self, character, initial, completion):
        assert initial <= completion
        if character == 0:
            return ("null",)
        candidates = []
        for state in sorted(self.claims):
            index = bisect_left(self.claim_packets[state], initial) - 1
            if index < 0:
                continue
            row = self.claims[state][index]
            if row["value_i64"] != character:
                continue
            epoch = self.epoch(state, initial)
            if epoch is not None and epoch["end"] is not None and epoch["end"]["packet_id"] < initial:
                continue
            candidates.append(row)
        if len(candidates) > 1:
            return ("ambiguous", *(f"{r['actor_net_guid']}:{r['packet_id']}:{r['time_ms']}:{r['raw_bits'].hex()}"
                                    for r in candidates))
        if not candidates:
            return ("unresolved", "NoReference")
        row = candidates[0]
        state = self.epoch(row["actor_net_guid"], initial)
        body = self.epoch(character, initial)
        if not self.active(state, initial) or not self.active(body, initial):
            return ("unresolved", "Lifecycle")
        if row["packet_id"] < state["open"]["packet_id"] or any(
                epoch["previous_end"] is not None and row["packet_id"] <= epoch["previous_end"]
                for epoch in (state, body)):
            return ("unresolved", "PreviousLifetime")
        for declaration in self.fields:
            if initial <= declaration["packet_id"] <= completion and (
                    declaration["actor_net_guid"] == row["actor_net_guid"] or declaration["value_i64"] == character):
                return ("unresolved", "ConcurrentChange")
        for guid in (row["actor_net_guid"], character):
            for epoch in self.epochs[guid]:
                if any(epoch[key] is not None and initial <= epoch[key]["packet_id"] <= completion
                       for key in ("open", "end", "conflict")):
                    return ("unresolved", "ConcurrentChange")
        return ("resolved", *(str(row[key]) for key in ("actor_net_guid", "packet_id", "time_ms",
            "channel_index", "handle", "bit_count")), row["raw_bits"].hex(),
            *(str(epoch["open"][key]) for epoch in (state, body) for key in ("packet_id", "time_ms", "channel_index")))


def movement_rows(export):
    with pq.ParquetFile(export / "movement.parquet", pre_buffer=False) as file:
        for batch in file.iter_batches(batch_size=4096, use_threads=False):
            yield from batch.to_pylist()


def compare(probe, replay, export, stderr_path):
    fields = declarations(export)
    with pq.ParquetFile(export / "actors.parquet", pre_buffer=False) as file:
        actors = file.read().to_pylist()
    oracle = Oracle(actors, fields)
    movement = iter(movement_rows(export))
    observed = Counter()
    count = declaration_count = groups = fragmented = 0
    group = previous_source = None
    previous_move = None
    digest = hashlib.sha256()
    trailer = None
    with stderr_path.open("wb") as errors:
        with subprocess.Popen([str(probe), str(replay)], stdout=subprocess.PIPE, stderr=errors) as process:
            try:
                for line in process.stdout:
                    assert len(line) < 8192 and trailer is None
                    digest.update(line)
                    words = line.decode("ascii").rstrip("\n").split("\t")
                    if words[0] == "D":
                        assert declaration_count < len(fields) and words == declaration_line(fields[declaration_count])
                        declaration_count += 1
                    elif words[0] == "S":
                        assert len(words) == 4
                        trailer = tuple(map(int, words[1:]))
                    else:
                        assert words[0] == "M" and len(words) >= 31
                        row = next(movement)
                        assert int(words[1]) == count
                        values = [row[key] for key in ("time_ms", "packet_id", "character_net_guid", "timestamp", "movement_state", "move_type")]
                        values += [struct.unpack("<I", struct.pack("<f", row[key]))[0] for key in FLOATS]
                        assert list(map(int, words[2:16])) == values, "movement values/order changed"
                        initial, completion = map(int, words[16:18])
                        assert completion == row["packet_id"] and initial <= completion
                        source = tuple(words[16:29])
                        current_group = (completion, words[23], words[25], words[26], words[28])
                        move_index = int(words[29])
                        if current_group != group:
                            assert move_index == 0
                            groups += 1
                            group = current_group
                        else:
                            assert source == previous_source and move_index == previous_move + 1
                        previous_source, previous_move = source, move_index
                        expected = oracle.movement(row["character_net_guid"], initial, completion)
                        assert tuple(words[30:]) == expected, "identity/proof tuple differs"
                        observed[":".join(expected[:2]) if expected[0] == "unresolved" else expected[0]] += 1
                        fragmented += initial != completion
                        count += 1
                process.stdout.close()
                assert process.wait() == 0, "probe process failed; inspect local stderr"
            except BaseException:
                process.kill()
                process.stdout.close()
                process.wait()
                raise
    assert next(movement, None) is None, "probe omitted movement rows"
    assert declaration_count == len(fields)
    manifest = json.loads((export / "manifest.json").read_text())
    expected_checkpoints = manifest["quality"]["checkpoints"]["checkpoint_chunks"]
    assert trailer == (count, declaration_count, expected_checkpoints)
    return {"movement_rows_compared": count, "exact_proof_tuples_compared": count,
            "movement_components": groups, "fragmented_movement_rows": fragmented,
            "spawned_character_raw_values_compared": len(fields),
            "spawned_character_widths": sorted(Counter(row["bit_count"] for row in fields).items()),
            "resolution_counts": dict(observed), "probe_output_sha256": digest.hexdigest(),
            "skipped_checkpoint_chunks": expected_checkpoints, "mismatches": 0,
            "scope": "Main packet intervals only; no account, event timestamp, or final GUID path join"}
