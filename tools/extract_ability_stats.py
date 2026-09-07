#!/usr/bin/env python3
"""Extract and validate the ability-stat data dictionary from vrfkit exports.

``AbilityCastsThisRound[].Effects[]`` carries both ``Statistic`` (an integer)
and ``LocalizedStat`` (an FText key). They describe the same array element, so
the wire supplies its own integer-to-name dictionary. This tool pairs only rows
that share the complete serialized instance context and validates the result
against mappings observed in the release-13.01--13.05 corpus.

Counts are serialized snapshot observations. They are not ability casts: the
same cast can be repeated in later array snapshots or at checkpoints.

Usage:
    python tools/extract_ability_stats.py \
        --export path/to/export --out ability_stats.json

Repeat ``--export`` to aggregate multiple exports. The JSON is written even
when validation fails so unknown IDs and conflicts remain inspectable.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path

import pyarrow.parquet as pq

if __package__:
    from .atomic_io import atomic_write_text
else:
    from atomic_io import atomic_write_text


GROUP = (
    "/Game/Characters/_Core/Comp_AbilityStatisticsReplicator."
    "Comp_AbilityStatisticsReplicator_C"
)
TABLES = ("fields", "checkpoint_fields")
PAIR_COLUMNS = (
    "time_ms",
    "packet_id",
    "channel_index",
    "actor_net_guid",
    "object_net_guid",
    "field_name",
    "value_i64",
    "value_str",
)
PAIR_KEY_NAMES = (
    "table",
    "packet_id",
    "time_ms",
    "channel_index",
    "actor_net_guid",
    "object_net_guid",
    "cast_element_index",
    "effect_element_index",
)
MEMBER_RE = re.compile(
    r"^AbilityCastsThisRound\[(\d+)\]\.Effects\[(\d+)\]\."
    r"(Statistic|LocalizedStat)(?:_\d+_[0-9A-F]{32})?$",
    re.IGNORECASE,
)
BUILD_RE = re.compile(r"release-(\d+\.\d+)", re.IGNORECASE)


# Measured over all 714 release-13.01--13.05 exports on 2026-09-08. The scan
# paired 155,150 main/checkpoint observations using PAIR_KEY_NAMES, with zero
# missing partners, duplicate members, ID conflicts, or reverse-name conflicts.
# Builds 13.01--13.04 exposed the same 31 IDs; 13.05 added TimeSprinting.
_BASE_STAT_NAMES = {
    0: "EnemiesBlinded",
    1: "DamageDealt",
    2: "Kills",
    3: "Assists",
    4: "EnemiesDazed",
    5: "EnemiesDisplaced",
    6: "EnemiesRevealed",
    7: "EnemiesBlocked",
    8: "EnemiesNearsighted",
    9: "HealingDone",
    10: "AlliesStimmed",
    12: "EnemiesSlowed",
    13: "DamageReceived",
    14: "BoostKills",
    15: "EnemiesSpotted",
    16: "EnemiesVulnerabled",
    17: "AlliesBlinded",
    18: "EnemiesDetained",
    19: "EnemiesSuppressed",
    25: "AbilityStat_ShotsFired",
    35: "EnemiesMarked",
    36: "EnemiesSeized",
    50: "AlliesConcussed",
    51: "AlliesSeized",
    53: "AlliesMarked",
    56: "AlliesSlowed",
    57: "EnemiesJammed",
    62: "UtilDestroyed",
    65: "DebuffResisted",
    67: "AlliesHealed",
    68: "EnemiesHealed",
}
KNOWN_STAT_NAMES = {
    "13.01": dict(_BASE_STAT_NAMES),
    "13.02": dict(_BASE_STAT_NAMES),
    "13.04": dict(_BASE_STAT_NAMES),
    "13.05": {**_BASE_STAT_NAMES, 27: "TimeSprinting"},
}


@dataclass(frozen=True, order=True)
class PairKey:
    table: str
    packet_id: int
    time_ms: int
    channel_index: int
    actor_net_guid: int
    object_net_guid: int | None
    cast_element_index: int
    effect_element_index: int

    def as_dict(self) -> dict:
        return dict(zip(PAIR_KEY_NAMES, self.__dict__.values(), strict=True))


@dataclass
class PairingResult:
    pairs: Counter[tuple[int, str, str]] = field(default_factory=Counter)
    issues: Counter[str] = field(default_factory=Counter)
    samples: dict[str, list[dict]] = field(
        default_factory=lambda: defaultdict(list)
    )
    relevant_rows: int = 0

    def issue(self, kind: str, key: PairKey, detail: dict) -> None:
        self.issues[kind] += 1
        if len(self.samples[kind]) < 10:
            self.samples[kind].append({**key.as_dict(), **detail})


def replay_build(export_dir: Path) -> str:
    manifest_path = export_dir / "manifest.json"
    if not manifest_path.is_file():
        raise ValueError(f"no manifest.json in {export_dir}")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    raw = str(manifest.get("replay_build") or "")
    match = BUILD_RE.search(raw)
    if not match:
        raise ValueError(
            f"cannot extract release build from manifest replay_build={raw!r}"
        )
    return match.group(1)


def pair_rows(rows: list[dict], table_name: str) -> PairingResult:
    """Pair Statistic/LocalizedStat rows without crossing instance contexts."""
    slots: dict[PairKey, dict[str, list]] = defaultdict(
        lambda: {"Statistic": [], "LocalizedStat": []}
    )
    result = PairingResult()
    for row in rows:
        match = MEMBER_RE.match(row.get("field_name") or "")
        if not match:
            continue
        result.relevant_rows += 1
        key = PairKey(
            table=table_name,
            packet_id=row["packet_id"],
            time_ms=row["time_ms"],
            channel_index=row["channel_index"],
            actor_net_guid=row["actor_net_guid"],
            object_net_guid=row["object_net_guid"],
            cast_element_index=int(match.group(1)),
            effect_element_index=int(match.group(2)),
        )
        member = (
            "Statistic"
            if match.group(3).casefold() == "statistic"
            else "LocalizedStat"
        )
        value = row["value_i64"] if member == "Statistic" else row["value_str"]
        slots[key][member].append(value)

    for key, members in slots.items():
        stats = members["Statistic"]
        names = members["LocalizedStat"]
        if len(stats) != 1:
            kind = "missing_statistic" if not stats else "duplicate_statistic"
            result.issue(kind, key, {"values": stats})
        if len(names) != 1:
            kind = "missing_localized_stat" if not names else "duplicate_localized_stat"
            result.issue(kind, key, {"values": names})
        if len(stats) != 1 or len(names) != 1:
            continue
        if stats[0] is None:
            result.issue("null_statistic", key, {})
            continue
        if not names[0]:
            result.issue("null_localized_stat", key, {})
            continue
        result.pairs[(int(stats[0]), str(names[0]), table_name)] += 1
    return result


def scan_table(path: Path, table_name: str) -> PairingResult:
    if not path.is_file():
        raise ValueError(f"no {path.name} in {path.parent}")
    schema = pq.read_schema(path)
    missing = sorted(set(PAIR_COLUMNS) - set(schema.names))
    if missing:
        raise ValueError(f"{path}: missing columns {missing}")
    table = pq.read_table(
        path,
        columns=list(PAIR_COLUMNS),
        filters=[("group_path", "=", GROUP)],
    )
    return pair_rows(table.to_pylist(), table_name)


def validation_status(build: str, statistic_id: int, name: str) -> str:
    known = KNOWN_STAT_NAMES.get(build)
    if known is None:
        return "unknown_build"
    expected = known.get(statistic_id)
    if expected is None:
        return "unknown_statistic_id"
    if expected != name:
        return "statistic_name_conflict"
    reverse = {value: key for key, value in known.items()}
    if reverse.get(name) != statistic_id:
        return "localized_name_conflict"
    return "known"


def known_dictionary() -> dict[str, list[dict]]:
    return {
        build: [
            {"statistic_id": statistic_id, "localized_stat": name}
            for statistic_id, name in sorted(mapping.items())
        ]
        for build, mapping in sorted(KNOWN_STAT_NAMES.items())
    }


def deduplicate_exports(exports: list[Path]) -> list[Path]:
    """Return each canonical export directory once, preserving argument order."""
    unique = []
    seen = set()
    for export_dir in exports:
        canonical = export_dir.resolve()
        if canonical in seen:
            continue
        seen.add(canonical)
        unique.append(canonical)
    return unique


def validate_output_path(out: Path, exports: list[Path]) -> None:
    """Refuse to replace a manifest or Parquet input through a path alias."""
    output = out.resolve()
    sources = []
    for export_dir in exports:
        sources.append(export_dir / "manifest.json")
        sources.extend(export_dir.glob("*.parquet"))
    for source in sources:
        if output == source.resolve() or (
            out.exists() and source.exists() and os.path.samefile(out, source)
        ):
            raise ValueError(
                f"--out aliases source export file {source}; refusing to overwrite input"
            )


def extract(
    exports: list[Path], *, requested_exports: int | None = None
) -> tuple[dict, list[str]]:
    observations: Counter[tuple[str, int, str, str]] = Counter()
    builds_seen: Counter[str] = Counter()
    issues: Counter[tuple[str, str]] = Counter()
    samples: dict[tuple[str, str], list[dict]] = defaultdict(list)
    export_rows = []

    for export_dir in sorted(exports, key=lambda path: str(path)):
        build = replay_build(export_dir)
        builds_seen[build] += 1
        relevant_rows = paired_observations = 0
        export_issues = Counter()
        table_presence = {
            table_name: (export_dir / f"{table_name}.parquet").is_file()
            for table_name in TABLES
        }
        for table_name in TABLES:
            if table_name == "checkpoint_fields" and not table_presence[table_name]:
                continue
            result = scan_table(export_dir / f"{table_name}.parquet", table_name)
            relevant_rows += result.relevant_rows
            paired_observations += sum(result.pairs.values())
            for (statistic_id, name, source_table), count in result.pairs.items():
                observations[(build, statistic_id, name, source_table)] += count
            for kind, count in result.issues.items():
                issues[(build, kind)] += count
                export_issues[kind] += count
                key = (build, kind)
                room = 10 - len(samples[key])
                samples[key].extend(result.samples[kind][:room])
        export_rows.append(
            {
                "path": str(export_dir),
                "build": build,
                "table_presence": table_presence,
                "relevant_member_rows": relevant_rows,
                "paired_observations": paired_observations,
                "structural_issues": dict(sorted(export_issues.items())),
            }
        )

    observed_by_build: dict[str, dict] = {}
    failures = []
    for build in sorted(builds_seen):
        grouped: dict[tuple[int, str], Counter[str]] = defaultdict(Counter)
        for (row_build, statistic_id, name, table_name), count in observations.items():
            if row_build == build:
                grouped[(statistic_id, name)][table_name] += count
        names_by_id: dict[int, set[str]] = defaultdict(set)
        ids_by_name: dict[str, set[int]] = defaultdict(set)
        for statistic_id, name in grouped:
            names_by_id[statistic_id].add(name)
            ids_by_name[name].add(statistic_id)
        id_conflicts = [
            {"statistic_id": statistic_id, "localized_stats": sorted(names)}
            for statistic_id, names in sorted(names_by_id.items())
            if len(names) > 1
        ]
        name_conflicts = [
            {"localized_stat": name, "statistic_ids": sorted(statistic_ids)}
            for name, statistic_ids in sorted(ids_by_name.items())
            if len(statistic_ids) > 1
        ]
        for conflict in id_conflicts:
            failures.append(
                f"{build}: observed statistic ID collision: {conflict}"
            )
        for conflict in name_conflicts:
            failures.append(
                f"{build}: observed localized-stat collision: {conflict}"
            )
        mappings = []
        for (statistic_id, name), counts in sorted(grouped.items()):
            status = validation_status(build, statistic_id, name)
            if status != "known":
                failures.append(
                    f"{build}: {statistic_id}={name!r} is {status}"
                )
            mappings.append(
                {
                    "statistic_id": statistic_id,
                    "localized_stat": name,
                    "fields_observations": counts["fields"],
                    "checkpoint_observations": counts["checkpoint_fields"],
                    "total_observations": sum(counts.values()),
                    "validation": status,
                }
            )
        build_issues = {
            kind: count
            for (row_build, kind), count in sorted(issues.items())
            if row_build == build
        }
        for kind, count in build_issues.items():
            failures.append(f"{build}: {count} {kind} pairing issue(s)")
        observed_by_build[build] = {
            "exports": builds_seen[build],
            "mappings": mappings,
            "mapping_conflicts": {
                "statistic_ids": id_conflicts,
                "localized_stats": name_conflicts,
            },
            "structural_issues": build_issues,
            "issue_samples": {
                kind: samples[(build, kind)] for kind in build_issues
            },
        }

    paired_total = sum(observations.values())
    if paired_total == 0:
        failures.append(
            "no Statistic/LocalizedStat observations were paired; refusing an "
            "empty data dictionary"
        )
    document = {
        "schema_version": 1,
        "count_semantics": (
            "serialized array-element snapshot observations, not ability casts"
        ),
        "pair_key": list(PAIR_KEY_NAMES),
        "known_dictionary": known_dictionary(),
        "exports": export_rows,
        "observed_by_build": observed_by_build,
        "totals": {
            "exports": len(exports),
            "exports_requested": (
                len(exports) if requested_exports is None else requested_exports
            ),
            "duplicate_exports_ignored": (
                0 if requested_exports is None else requested_exports - len(exports)
            ),
            "exports_with_fields": sum(
                row["table_presence"]["fields"] for row in export_rows
            ),
            "exports_with_checkpoint_fields": sum(
                row["table_presence"]["checkpoint_fields"] for row in export_rows
            ),
            "paired_observations": paired_total,
            "structural_issues": sum(issues.values()),
            "validation_failures": len(failures),
        },
    }
    return document, failures


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--export",
        action="append",
        required=True,
        type=Path,
        dest="exports",
        help="vrfkit export directory; repeat to aggregate exports",
    )
    parser.add_argument("--out", required=True, type=Path, help="output JSON path")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        exports = deduplicate_exports(args.exports)
        validate_output_path(args.out, exports)
        document, failures = extract(
            exports, requested_exports=len(args.exports)
        )
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"FAILED: {exc}", file=sys.stderr)
        return 1

    args.out.parent.mkdir(parents=True, exist_ok=True)
    atomic_write_text(
        args.out,
        json.dumps(document, indent=2, ensure_ascii=True) + "\n",
    )
    totals = document["totals"]
    print(
        f"wrote {args.out} ({totals['exports']} export(s), "
        f"{totals['paired_observations']} snapshot observation(s), "
        f"{totals['structural_issues']} structural issue(s), "
        f"{totals['validation_failures']} validation failure(s))"
    )
    for build, result in document["observed_by_build"].items():
        print(
            f"  {build}: {result['exports']} export(s), "
            f"{len(result['mappings'])} observed mapping(s)"
        )
    if failures:
        print(f"FAILED: {len(failures)} issue(s)", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
