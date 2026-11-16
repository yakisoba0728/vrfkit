#!/usr/bin/env python3
"""Pin the export path's own numbers and fail when they drift.

The export summary's counters and the six main Parquet files (the checkpoint
tables too, under `--checkpoints`), checked three ways:

  1. CROSS-CHECK: the counters that are Parquet row counts
     (`cross_check_identities`) must equal them, baseline or not; a counter
     the summary did not print fails too. fields.parquet's is the sink's
     `Sink tally` fields (NetStats' `Fields:` counts framed properties only).
  2. MANIFEST: the counts the manifest publishes beside the summary
     (`MANIFEST_CHECKS`) must agree with it, and under `--checkpoints` every
     checkpoint GUID path must match the main stream's own declaration
     (`checkpoint_guid_crosscheck`).
  3. BASELINE: every counter and each file's rows, bytes and SHA-256 against
     a pinned JSON -- the data moving while the summary faithfully reports
     it. Bytes moving with every counter equal means the values moved, or
     the parquet crate did (Cargo.lock pins it).

The baseline names its replay by bare filename, resolved against
VRFKIT_CORPUS_DIR; a missing replay is SKIPPED unless --require-input or
VRFKIT_REQUIRE_CORPUS is set.

Usage:
    python tools/check_export_baseline.py --baseline tools/baselines/export_02d4d478.json
    python tools/check_export_baseline.py --baseline <path> --update
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

import summary_counters as sc
from atomic_io import sha256_file
from wire_bits import read_parquet

REPO = Path(__file__).resolve().parent.parent
DEFAULT_EXE = REPO / "target" / "release" / "vrfkit.exe"

#: The summary counters the baselines pin, main pass and `--checkpoints` pass.
COUNTERS = sc.keys("P", checkpoint=False)
CHECKPOINT_COUNTERS = sc.keys("P", checkpoint=True)
# Archived private measurements retain their exact key set and values. These
# sets are for archive schema validation only, never current comparisons.
LEGACY_COUNTERS = tuple(key for key in COUNTERS if key not in (*sc.MOVEMENT_COUNTERS, *sc.FOCUS_PROJECTILES_COUNTERS)) \
    + sc.LEGACY_PINNED_MOVEMENT_COUNTERS
LEGACY_CHECKPOINT_COUNTERS = tuple(key for key in CHECKPOINT_COUNTERS
                                  if key.removeprefix("cp_") not in (*sc.MOVEMENT_COUNTERS, *sc.FOCUS_PROJECTILES_COUNTERS)) \
    + tuple("cp_" + key for key in sc.LEGACY_PINNED_MOVEMENT_COUNTERS)
#: The frame-walk tallies, after the manifest's `frame_` / `checkpoint_frame_`.
FRAME_SKIP_KEYS = ("external_data_blobs", "external_data_bytes", "game_specific_bytes",
                   "non_finite_times")
#: Native component and opaque-delta tallies, reconciled with the manifest's
#: `sink` blocks (checkpoint summary keys use `cp_`).
SINK_TALLY_KEYS = (*sc.MOVEMENT_COUNTERS, "active_blinds_empty_trailers",
                   *sc.FOCUS_PROJECTILES_COUNTERS)


PARQUET_FILES = ("fields", "movement", "actors", "net_guids", "events", "partials")
CHECKPOINT_PARQUET_FILES = (
    "checkpoint_fields", "checkpoint_actors", "checkpoint_net_guids", "checkpoint_blocks",
    "checkpoint_guid_entries", "checkpoint_export_groups", "checkpoint_export_fields",
)


def cross_check_identities(counters: dict, parquet: dict) -> list:
    """`(label, printed value, Parquet rows)` for each counter that is a row count."""
    identities = [
        ("NetGUID rows", counters.get("net_guid_rows"), parquet["net_guids"]["rows"]),
        ("Movement rows", counters.get("movement_rows"), parquet["movement"]["rows"]),
        ("Event rows", counters.get("event_rows"), parquet["events"]["rows"]),
        ("Sink tally fields", counters.get("fields_emitted"), parquet["fields"]["rows"]),
        (
            "Partial raw rows (main + checkpoint)",
            None if counters.get("partial_rows") is None or counters.get("cp_partial_rows", 0) is None
            else counters["partial_rows"] + counters.get("cp_partial_rows", 0),
            parquet["partials"]["rows"],
        ),
        (
            "Actor opens + Actor closes",
            None
            if counters.get("actor_opens") is None or counters.get("actor_closes") is None
            else counters["actor_opens"] + counters["actor_closes"],
            parquet["actors"]["rows"],
        ),
    ]
    if "cp_fields_emitted" in counters or "checkpoint_fields" in parquet:
        identities.append(("Checkpoint sink fields", counters.get("cp_fields_emitted"),
                           parquet.get("checkpoint_fields", {}).get("rows")))
    if "cp_actor_rows_written" in counters or "checkpoint_actors" in parquet:
        identities.append(("Checkpoint actors", counters.get("cp_actor_rows_written"),
                           parquet.get("checkpoint_actors", {}).get("rows")))
    if "cp_net_guid_rows_written" in counters or "checkpoint_net_guids" in parquet:
        identities.append(("Checkpoint GUID rows", counters.get("cp_net_guid_rows_written"),
                           parquet.get("checkpoint_net_guids", {}).get("rows")))
    if "cp_block_rows_written" in counters or "checkpoint_blocks" in parquet:
        identities.append(("Checkpoint blocks", counters.get("cp_block_rows_written"),
                           parquet.get("checkpoint_blocks", {}).get("rows")))
    for label, written, parsed, table in (
        ("Checkpoint GUID entries", "cp_guid_entry_rows_written", "cp_guid_entries",
         "checkpoint_guid_entries"),
        ("Checkpoint export groups", "cp_export_group_rows_written", "cp_group_records",
         "checkpoint_export_groups"),
        ("Checkpoint export fields", "cp_export_field_rows_written", "cp_exported_fields",
         "checkpoint_export_fields"),
    ):
        if written in counters or parsed in counters or table in parquet:
            actual = parquet.get(table, {}).get("rows")
            identities.append((label, counters.get(written), actual))
            # Compare the schema reader's independent count too: a writer that
            # drops a record must fail even if its own printed count agrees.
            identities.append((label + " parsed", counters.get(parsed), actual))
    return identities


def cross_checks(counters: dict, parquet: dict) -> list[str]:
    """Each identity that fails: a disagreement, or a counter not printed."""
    out = []
    for label, printed, actual in cross_check_identities(counters, parquet):
        if printed is None:
            out.append(f"{label}: the export summary did not print it")
        elif printed != actual:
            out.append(f"{label}: summary says {printed}, Parquet holds {actual}")
    if "cp_guid_entries" in counters:
        required = ("cp_guid_entries", "cp_literal_paths", "cp_indexed_paths",
                    "cp_resolved_path_indices")
        missing = [key for key in required if counters.get(key) is None]
        out.extend(f"{key}: the export summary did not print it" for key in missing)
        if not missing:
            if counters["cp_literal_paths"] + counters["cp_indexed_paths"] != counters["cp_guid_entries"]:
                out.append("Checkpoint GUID paths: literals + indices do not equal GUID entries")
            if counters["cp_resolved_path_indices"] != counters["cp_indexed_paths"]:
                out.append("Checkpoint GUID paths: resolved indices do not equal indexed paths")
    return out


def unpinnable(current: dict) -> list[str]:
    """Counters the summary did not print: a pinned None would match a
    summary that stopped printing them."""
    return [f"{key}: the export summary did not print it"
            for key in sorted(current["counters"])
            if current["counters"][key] is None]


def checkpoint_manifest_errors(out_dir: Path, counters: dict | None = None) -> list[str]:
    """Checkpoint actor rows must be written, never silently dropped."""
    try:
        manifest = json.loads((out_dir / "manifest.json").read_text(encoding="utf-8"))
        checkpoints = manifest["quality"]["checkpoints"]
        dropped = checkpoints["checkpoint_actor_rows_dropped"]
        mode = checkpoints["checkpoint_path_resolution_mode"]
        literals = checkpoints["checkpoint_literal_paths"]
        indices = checkpoints["checkpoint_indexed_paths"]
        resolved = checkpoints["checkpoint_resolved_path_indices"]
        guid_entries = checkpoints["checkpoint_guid_entries"]
    except (OSError, KeyError, TypeError, json.JSONDecodeError) as exc:
        return [f"checkpoint manifest omits required checkpoint quality data: {exc}"]
    if dropped != 0:
        return [f"checkpoint manifest says checkpoint_actor_rows_dropped={dropped}, expected 0"]
    errors = []
    values = {"cp_literal_paths": literals, "cp_indexed_paths": indices,
              "cp_resolved_path_indices": resolved, "cp_guid_entries": guid_entries}
    if any(type(value) is not int or value < 0 for value in values.values()):
        return ["checkpoint manifest GUID path counts must be nonnegative integers"]
    if mode != "preceding_literal_zero_based":
        errors.append(f"checkpoint manifest path resolution mode is {mode!r}")
    if literals + indices != guid_entries:
        errors.append("checkpoint manifest GUID paths: literals + indices do not equal GUID entries")
    if resolved != indices:
        errors.append("checkpoint manifest GUID paths: resolved indices do not equal indexed paths")
    if counters is not None:
        for key, value in values.items():
            if counters.get(key) != value:
                errors.append(f"checkpoint manifest {key}={value} disagrees with summary {counters.get(key)}")
    return errors


#: The columns `checkpoint_guid_crosscheck` reads from each table.
GUID_ENTRY_COLUMNS = ("checkpoint_index", "ordinal", "net_guid", "outer_net_guid",
                      "path_is_string", "literal_path", "name_index")
MAIN_GUID_COLUMNS = ("net_guid", "path", "outer_net_guid")

#: Every count `checkpoint_guid_crosscheck` returns, in print order, zeros
#: included. Per kind, joined = path_equal + path_differs = outer_equal +
#: outer_value_differs + outer_presence_differs; indexed entries are joined,
#: unjoined or unresolved.
GUID_CROSSCHECK_KEYS = (
    "indexed_joined", "indexed_path_equal", "indexed_path_differs",
    "indexed_unresolved", "indexed_outer_equal", "indexed_outer_value_differs",
    "indexed_outer_presence_differs", "indexed_unjoined",
    "literal_joined", "literal_path_equal", "literal_path_differs",
    "literal_outer_equal", "literal_outer_value_differs",
    "literal_outer_presence_differs", "literal_unjoined",
    "main_duplicate_guids", "malformed_entries", "ordinal_errors",
)

#: Counts that fail the check when nonzero, as `indexed_joined == 0` does.
#: Unjoined entries do not: a checkpoint may declare a GUID the main stream
#: never exported.
GUID_CROSSCHECK_FAILURES = {
    "indexed_path_differs": "indexed entries resolve to a path the main stream does not declare for that GUID",
    "indexed_unresolved": "indexed entries name a position past the literals that precede them",
    "indexed_outer_value_differs": "indexed entries carry a different outer GUID than the main stream",
    "indexed_outer_presence_differs": "indexed entries disagree with the main stream on whether an outer GUID exists",
    "literal_path_differs": "literal entries carry a different path than the main stream",
    "literal_outer_value_differs": "literal entries carry a different outer GUID than the main stream",
    "literal_outer_presence_differs": "literal entries disagree with the main stream on whether an outer GUID exists",
    "main_duplicate_guids": "net_guids.parquet rows repeat a net_guid, so the join is ambiguous",
}


def format_guid_crosscheck(counts: dict) -> str:
    """One line with every cross-check count, zeros included."""
    return "Checkpoint GUID cross-check: " + ", ".join(
        f"{key.replace('_', ' ')} {counts[key]}" for key in GUID_CROSSCHECK_KEYS)


def _outer_verdict(checkpoint_outer: int, main_outer: int | None) -> str:
    """Outers under an explicit rule, never null folded into 0: the
    checkpoint's wire 0 ("no outer") must meet main null, and since net_guids
    never writes 0, a main 0 is a presence difference."""
    checkpoint_present = checkpoint_outer != 0
    main_present = main_outer is not None
    if checkpoint_present != main_present:
        return "outer_presence_differs"
    if checkpoint_present and checkpoint_outer != main_outer:
        return "outer_value_differs"
    return "outer_equal"


def checkpoint_guid_crosscheck(out_dir: Path) -> tuple[dict, list[str]]:
    """`(counts, errors)`: every GUID_CROSSCHECK_KEYS count, and one message
    per reason the check fails (none is a pass).

    Each raw entry's path is rebuilt by the reader's rule
    (docs/CHECKPOINT_PATH_RESOLUTION.md): an index is a zero-based position
    among the literals earlier in the same `checkpoint_index` (never
    `checkpoint_id`, which repeats), rows ordered by (checkpoint_index,
    ordinal). Each entry joins net_guids.parquet -- the main stream's own
    reader, never the index rule -- by `net_guid`, and path and outer are
    compared. Fails on any difference, an index past the preceding literals,
    duplicate main GUIDs, malformed rows or ordinal gaps, and when no
    indexed entry joined at all.
    """
    counts = dict.fromkeys(GUID_CROSSCHECK_KEYS, 0)
    try:
        entries = read_parquet(out_dir / "checkpoint_guid_entries.parquet",
                                columns=list(GUID_ENTRY_COLUMNS)).to_pydict()
        main = read_parquet(out_dir / "net_guids.parquet",
                             columns=list(MAIN_GUID_COLUMNS)).to_pydict()
    except (OSError, ValueError, pa.ArrowException) as exc:
        return counts, [f"checkpoint GUID cross-check cannot read its tables: {exc}"]

    main_rows: dict[int, tuple] = {}
    for guid, path, outer in zip(main["net_guid"], main["path"], main["outer_net_guid"]):
        if guid in main_rows:
            counts["main_duplicate_guids"] += 1
        main_rows[guid] = (path, outer)

    rows = list(zip(*(entries[name] for name in GUID_ENTRY_COLUMNS)))
    well_formed = []
    for row in rows:
        checkpoint, ordinal, guid, outer, is_literal, literal, index = row
        if (None in (checkpoint, ordinal, guid, outer, is_literal)
                or (is_literal and (literal is None or index is not None))
                or (not is_literal and (index is None or literal is not None))):
            counts["malformed_entries"] += 1
        else:
            well_formed.append(row)
    well_formed.sort(key=lambda row: (row[0], row[1]))
    previous = None
    for checkpoint, ordinal, *_ in well_formed:
        expected = previous[1] + 1 if previous and previous[0] == checkpoint else 0
        if ordinal != expected:
            counts["ordinal_errors"] += 1
        previous = (checkpoint, ordinal)
    if counts["malformed_entries"] or counts["ordinal_errors"]:
        return counts, [
            f"checkpoint_guid_entries.parquet is not a complete raw record "
            f"({counts['malformed_entries']} malformed entries, "
            f"{counts['ordinal_errors']} ordinal errors); paths were not compared"]

    # The first example of each difference, for the error message.
    first: dict[str, str] = {}

    def note(key: str, row: tuple, detail: str) -> None:
        if key not in first:
            first[key] = (f"checkpoint_index {row[0]} ordinal {row[1]} "
                          f"net_guid {row[2]}: {detail}")

    literals: list[str] = []
    current = None
    for row in well_formed:
        checkpoint, _, guid, outer, is_literal, literal, index = row
        if checkpoint != current:
            current, literals = checkpoint, []
        if is_literal:
            literals.append(literal)
            kind, path = "literal", literal
        elif index < len(literals):
            kind, path = "indexed", literals[index]
        else:
            counts["indexed_unresolved"] += 1
            note("indexed_unresolved", row, f"index {index}, {len(literals)} preceding literals")
            continue
        if guid not in main_rows:
            counts[f"{kind}_unjoined"] += 1
            continue
        counts[f"{kind}_joined"] += 1
        main_path, main_outer = main_rows[guid]
        if path == main_path:
            counts[f"{kind}_path_equal"] += 1
        else:
            counts[f"{kind}_path_differs"] += 1
            note(f"{kind}_path_differs", row, f"checkpoint {path!r}, main {main_path!r}")
        verdict = _outer_verdict(outer, main_outer)
        counts[f"{kind}_{verdict}"] += 1
        if verdict != "outer_equal":
            note(f"{kind}_{verdict}", row, f"checkpoint outer {outer}, main outer {main_outer}")

    errors = []
    if counts["indexed_joined"] == 0:
        errors.append("no indexed checkpoint GUID entry joined a main-stream GUID, "
                      "so the path-index rule was not compared at all")
    for key, reason in GUID_CROSSCHECK_FAILURES.items():
        if counts[key]:
            example = f" (first: {first[key]})" if key in first else ""
            errors.append(f"checkpoint GUID cross-check: {counts[key]} {reason}{example}")
    return counts, errors


def _sink(*keys):
    """A MANIFEST_CHECKS pick for `quality.sink[key]` and its checkpoint twin."""
    def pick(quality, checkpoints):
        values = {key: quality["sink"][key] for key in keys}
        if checkpoints:
            values.update({"cp_" + key: quality["checkpoints"]["sink"][key] for key in keys})
        return values
    return pick


def _frames(quality, checkpoints):
    values = {f"frame_{key}": quality[f"frame_{key}"] for key in FRAME_SKIP_KEYS}
    if checkpoints:
        values.update({f"cp_frame_{key}": quality["checkpoints"][f"checkpoint_frame_{key}"]
                       for key in FRAME_SKIP_KEYS})
    return values


#: `(what, pick)`: counts the manifest publishes that must equal the summary's,
#: zeros included. None is a zero gate -- each is a measured shape, legitimately
#: nonzero. Summary and manifest read one variable for the frame skips and
#: `Trailing bytes`, so crates/vrfkit/tests/frame_skips.rs and
#: checkpoint_unread.rs guard the wiring this cannot see.
MANIFEST_CHECKS = (
    ("tracked rewards opaque-empty", _sink("tracked_rewards_opaque_empty_variants")),
    ("targeting world-location", _sink("targeting_world_locations_decoded")),
    ("sink component/opaque tally", _sink(*SINK_TALLY_KEYS)),
    ("frame-skip", _frames),
    ("checkpoint trailing-bytes", lambda quality, checkpoints: {
        "cp_trailing_bytes": quality["checkpoints"]["checkpoint_trailing_bytes"]}
        if checkpoints else {}),
)


def manifest_errors(out_dir: Path, counters: dict, checkpoints: bool) -> list[str]:
    """Each MANIFEST_CHECKS count must be a nonnegative integer equal to the
    summary's; `what` names it in every message."""
    try:
        quality = json.loads((out_dir / "manifest.json").read_text(encoding="utf-8"))["quality"]
    except (OSError, ValueError, KeyError, TypeError) as exc:
        return [f"manifest omits its quality data: {exc}"]
    errors = []
    scopes = [("main", quality)]
    if checkpoints:
        scopes.append(("checkpoint", quality.get("checkpoints")))
    for name, scope in scopes:
        if not isinstance(scope, dict) or not isinstance(scope.get("sink"), dict):
            errors.append(f"manifest omits {name} sink quality data")
            continue
        errors.extend(f"manifest {name}: {error}" for error in
                      sc.movement_contract_errors(scope["sink"]))
    for what, pick in MANIFEST_CHECKS:
        try:
            values = pick(quality, checkpoints)
        except (KeyError, TypeError) as exc:
            errors.append(f"manifest omits {what} quality data: {exc}")
            continue
        if any(type(value) is not int or value < 0 for value in values.values()):
            errors.append(f"{what} counts must be nonnegative integers")
            continue
        errors += [f"manifest {key}={value} disagrees with summary {counters.get(key)}"
                   for key, value in values.items() if counters.get(key) != value]
    return errors


def export_errors(out_dir: Path, counters: dict, checkpoints: bool) -> tuple[list[str], dict]:
    """`(errors, GUID cross-check counts)` of every check an export must pass
    besides its baseline and the row identities; the counts are empty without
    `checkpoints`."""
    errors = manifest_errors(out_dir, counters, checkpoints)
    if not checkpoints:
        return errors, {}
    guid_counts, guid_errors = checkpoint_guid_crosscheck(out_dir)
    return errors + checkpoint_manifest_errors(out_dir, counters) + guid_errors, guid_counts


def parquet_shape(out_dir: Path, names, sha: bool = False) -> dict:
    """Rows and bytes (and SHA-256) of each `{name}.parquet`; a missing file
    raises the OSError that names it."""
    shape = {}
    for name in names:
        path = out_dir / f"{name}.parquet"
        with pq.ParquetFile(path, pre_buffer=False) as source:
            shape[name] = {"rows": source.metadata.num_rows,
                           "bytes": path.stat().st_size}
        if sha:
            shape[name]["sha256"] = sha256_file(path)
    return shape


def measure(exe: Path, replay: Path, out_dir: Path, checkpoints: bool = False) -> dict:
    """Export one replay; its summary counters and Parquet shape. The exporter
    replaces `out_dir` transactionally, so a failed export leaves the previous
    complete one."""
    code, text = sc.vrfkit(exe, "export", replay, out_dir, checkpoints, timeout=1800)
    if code != 0:
        raise SystemExit(f"export failed (exit {code}): {sc.tail(text, 5, 400)}")
    pinned = COUNTERS + (CHECKPOINT_COUNTERS if checkpoints else ())
    counters = sc.read(text, pinned)
    try:
        parquet = parquet_shape(
            out_dir, PARQUET_FILES + (CHECKPOINT_PARQUET_FILES if checkpoints else ()), sha=True)
    except OSError as exc:
        raise SystemExit(f"export wrote no readable Parquet file: {exc}")
    errors, guid_counts = export_errors(out_dir, counters, checkpoints)
    if checkpoints:
        # Printed, never pinned: pinned, every existing checkpoint baseline
        # would report them as drift from None.
        print(format_guid_crosscheck(guid_counts))
    if errors:
        raise SystemExit("; ".join(errors))
    return {"counters": {key: counters[key] for key in pinned}, "parquet": parquet}


def diff(baseline: dict, current: dict) -> list[str]:
    """Every way the pinned numbers and the current ones disagree."""
    if sc.obsolete_movement_contract(baseline["counters"]):
        return [sc.OBSOLETE_MOVEMENT_CONTRACT]
    out = []
    for key in sorted(set(baseline["counters"]) | set(current["counters"])):
        want = baseline["counters"].get(key)
        got = current["counters"].get(key)
        if want != got:
            out.append(f"counter {key}: {got} (baseline {want})")
    for name in sorted(set(baseline["parquet"]) | set(current["parquet"])):
        want = baseline["parquet"].get(name, {})
        got = current["parquet"].get(name, {})
        for field in ("rows", "bytes", "sha256"):
            if want.get(field) != got.get(field):
                out.append(
                    f"{name}.parquet {field}: {got.get(field)} "
                    f"(baseline {want.get(field)})"
                )
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--baseline", type=Path, required=True)
    ap.add_argument("--exe", type=Path, default=DEFAULT_EXE)
    ap.add_argument("--replay", type=Path, default=None,
                    help="overrides the replay path stored in the baseline")
    ap.add_argument("--out", type=Path, default=None,
                    help="transactionally replaced export directory "
                         "(default: out/export_check)")
    ap.add_argument("--update", action="store_true",
                    help="rewrite the baseline from the current numbers")
    ap.add_argument("--checkpoints", action="store_true",
                    help="also run the optional Checkpoint pass: pin its summary "
                         "counters and every checkpoint_*.parquet table, require "
                         "its manifest block (no dropped actor rows, GUID path "
                         "counts that add up and match the summary), and "
                         "cross-check its GUID paths against net_guids.parquet")
    ap.add_argument("--require-input", action="store_true",
                    help="fail instead of skipping when the replay is absent")
    args = ap.parse_args()

    if sc.no_exe(args.exe):
        return 2
    stored = sc.load_baseline(args.baseline)
    obsolete = sc.obsolete_movement_contract(stored.get("counters", {}))
    if not args.update and obsolete:
        print(sc.OBSOLETE_MOVEMENT_CONTRACT, file=sys.stderr)
        return 1
    refusal = args.update and sc.machine_path(args.replay, stored.get("replay"), "replay",
                                              args.baseline)
    if refusal:
        print(refusal, file=sys.stderr)
        return 2
    replay = sc.baseline_input(args.replay, stored.get("replay", ""))
    if replay is None or not replay.exists():
        return sc.missing_input("no replay named (pass --replay or store one in the baseline)"
                                if replay is None else f"replay not present ({replay})",
                                args.require_input or obsolete)

    current = measure(args.exe, replay, args.out or REPO / "out" / "export_check",
                      checkpoints=args.checkpoints)
    # Before any baseline is written: pinning a summary that contradicts its
    # own Parquet output would pin the lie.
    lies = cross_checks(current["counters"], current["parquet"])
    if lies:
        print(f"CROSS-CHECK FAILED: {len(lies)} counter(s) disagree with the "
              f"Parquet files they name")
        for line in lies:
            print(f"  {line}")
        return 1

    pinned = {"replay": stored.get("replay") or str(args.replay), **current}
    verdict = sc.pin_or_diff(args.baseline, stored, pinned, args.update,
                             unpinnable(current), diff)
    if verdict is not None:
        return verdict
    c = current["counters"]
    n_identities = len(cross_check_identities(c, current["parquet"]))
    print(f"OK: {replay.name} matches the baseline "
          f"(NetGUID rows {c['net_guid_rows']}, "
          f"blocks {c['content_blocks']}, fields {c['fields']}, "
          f"rpcs {c['rpcs']}, movement {c['movement_rows']}, "
          f"events {c['event_rows']}, "
          f"decode errors {c['overlay_decode_errors']}); "
          f"{n_identities} printed counters cross-check against their Parquet files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
