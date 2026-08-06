#!/usr/bin/env python3
"""Pin the export path's own numbers and fail when they drift.

`check_corpus_baseline.py` guards the *validate* path. Nothing guarded the
*export* path, so every figure the export summary prints -- content blocks,
fields, RPCs, movement rows, NetGUID rows, decode errors, the four Parquet
files themselves -- was pinned only in a comment in the task brief and in
docs/archive/PROJECT_STATUS.md prose. `NetGUID rows: 16167` is the clearest
case: no harness read it, because `validate` never writes Parquet and
`validate_corpus.py`'s PATTERNS has no entry for a counter the oracle does
not print.

That is the same shape as the malformed counter, which went unread for the
project's whole history because its regex never matched. A number nobody
machine-checks is not guarded, however often it appears in a document.

Two independent checks run here, and they fail on different things:

  1. CROSS-CHECK.   Some printed counters are identities against the Parquet
     files: `NetGUID rows` is net_guids.parquet's row count, `Movement rows`
     is movement.parquet's, `Event rows` is events.parquet's, and
     `Actor opens + Actor closes` is actors.parquet's. If the summary and the
     file disagree, the summary is lying, and this fails with no baseline
     needed. The set lives in `cross_check_identities`; do not restate its
     size here, where it cannot be checked.
     (fields.parquet has no such identity: it also carries RPC parameters and
     flattened dynamic-array leaves, so its row count is pinned, not derived.)

  2. BASELINE.      Every counter and every Parquet row count and byte size is
     compared against a pinned JSON. This is what catches the other failure
     mode -- the data moving and the summary faithfully reporting the new,
     wrong number. A cross-check alone cannot see that.
     A byte-size difference with every counter equal means the row VALUES
     moved -- or that the parquet crate version did. Cargo.lock pins it, so
     check that before assuming a data bug, and do not disable the guard.

Both were confirmed to fail on a deliberately broken build before this was
committed; see the commit message.

The .vrf lives outside the repo (under valplay), so a missing replay is
reported and SKIPPED rather than failed -- the same reasoning as
check_corpus_baseline.py: a guard that fails on someone else's machine gets
disabled, and a disabled guard protects nothing.

Usage:
    python tools/check_export_baseline.py --baseline tools/baselines/export_02d4d478.json
    python tools/check_export_baseline.py --baseline <path> --update
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

import pyarrow.parquet as pq

REPO = Path(__file__).resolve().parent.parent
DEFAULT_EXE = REPO / "target" / "release" / "vrfkit.exe"

# Every counter the export summary prints, except `Elapsed` and the manifest
# path. Anchored on the exact labels driver.rs emits; a label that stops being
# printed is reported as missing rather than defaulted to 0, because a counter
# that silently reads as absent is how this class of bug survives.
COUNTERS = {
    "chunks": r"Chunks:\s+(\d+)",
    "packets": r"Packets:\s+(\d+)",
    "export_groups": r"Export groups:\s+(\d+)",
    "content_blocks": r"Content blocks:\s+(\d+)",
    "rep_layout_blocks": r"RepLayout blocks:\s+(\d+)",
    "class_net_cache_blocks": r"ClassNetCache:\s+(\d+)",
    "fields": r"Fields:\s+(\d+)",
    "rpcs": r"RPCs:\s+(\d+)",
    "actor_opens": r"Actor opens:\s+(\d+)",
    "actor_closes": r"Actor closes:\s+(\d+)",
    "bunches": r"Bunches:\s+(\d+)",
    "malformed_packets": r"Malformed pkts:\s+(\d+)",
    "skipped_bits": r"Skipped bits:\s+(\d+)",
    "movement_rows": r"Movement rows:\s+(\d+)",
    "net_guid_rows": r"NetGUID rows:\s+(\d+)",
    "event_rows": r"Event rows:\s+(\d+)",
    "overlay_decoded_ok": r"Decoded OK:\s+(\d+)",
    "overlay_decode_errors": r"Decode errors:\s+(\d+)",
    "overlay_raw_skip": r"Raw/Skip:\s+(\d+)",
    "overlay_not_in_table": r"Not in table:\s+(\d+)",
    "overlay_no_field_name": r"No field name:\s+(\d+)",
    "overlay_rows_offered": r"Rows offered:\s+(\d+)",
    # Not part of the overlay ratio: the overlay buckets are decided before the
    # effect pass runs, so a successful effect decode moves none of the five
    # counters above. Without this line the only evidence that the decoder ran
    # is fields.parquet's byte count, and a byte count cannot say whether the
    # decoder produced values or merely different padding.
    "effect_blobs_decoded": r"Effect blobs:\s+(\d+)",
    # The counter that would have caught build 13.02 on day one. Its decoders
    # are additive, so when RoundResults moved from handle 93 to 81 nothing
    # else on this summary twitched: same blocks, same fields, same rows, same
    # "Decode errors: 0" -- and no match score in the Parquet. `failed` is the
    # alarm; `decoded` is what keeps a decoder that silently stops running
    # (0 decoded, 0 failed) from reading the same as a clean one.
    "struct_blobs_decoded": r"Struct blobs:\s+(\d+) decoded",
    "struct_blobs_failed": r"Struct blobs:\s+\d+ decoded / (\d+) failed",
}
PATTERNS = {k: re.compile(v) for k, v in COUNTERS.items()}

# Only printed under `--checkpoints`, so they live apart from COUNTERS -- a
# default run must not record them as None and then diff that against a
# baseline taken with the flag.
CHECKPOINT_COUNTERS = {
    "cp_chunks": r"Checkpoints:\s+(\d+)",
    "cp_guid_entries": r"GUID entries:\s+(\d+)",
    "cp_group_records": r"Group records:\s+(\d+)",
    "cp_exported_fields": r"Exported fields:\s+(\d+)",
    "cp_frames": r"Frames:\s+(\d+)",
    "cp_frame_packets": r"Frame packets:\s+(\d+)",
    "cp_field_rows": r"Checkpoint rows:\s+(\d+)",
    # Deliberately a different label from the main block's "Struct blobs", so
    # these regexes cannot match each other's line.
    "cp_struct_blobs_decoded": r"Checkpoint blobs:\s+(\d+) decoded",
    "cp_struct_blobs_failed": r"Checkpoint blobs:\s+\d+ decoded / (\d+) failed",
}

PARQUET_FILES = ("fields", "movement", "actors", "net_guids", "events")


def cross_check_identities(counters: dict, parquet: dict) -> list:
    """The printed counters that ARE Parquet row counts.

    Each entry is (label, printed value, actual rows). Split out from
    `cross_checks` so the pass message can count them instead of stating a
    literal: the message said "3" for as long as there were three, and adding
    the fourth made it report a number it had not checked -- the same class of
    claim this whole script exists to catch.
    """
    return [
        ("NetGUID rows", counters.get("net_guid_rows"), parquet["net_guids"]["rows"]),
        ("Movement rows", counters.get("movement_rows"), parquet["movement"]["rows"]),
        ("Event rows", counters.get("event_rows"), parquet["events"]["rows"]),
        (
            "Actor opens + Actor closes",
            None
            if counters.get("actor_opens") is None or counters.get("actor_closes") is None
            else counters["actor_opens"] + counters["actor_closes"],
            parquet["actors"]["rows"],
        ),
    ]


def cross_checks(counters: dict, parquet: dict) -> list[str]:
    """Disagreement between a printed counter and its Parquet file = a lie.

    A counter the summary did not print is itself a failure: the identity
    cannot be checked, which is exactly the state that let the malformed
    counter read as a vacuous 0.
    """
    out = []
    for label, printed, actual in cross_check_identities(counters, parquet):
        if printed is None:
            out.append(f"{label}: the export summary did not print it")
        elif printed != actual:
            out.append(f"{label}: summary says {printed}, Parquet holds {actual}")
    return out


def unpinnable(current: dict) -> list[str]:
    """Counters this run did not measure, which therefore must not be pinned.

    `measure` records a counter the summary did not print as None rather than
    0, for the reason stated at `COUNTERS`. Writing that None into the baseline
    undoes the whole point: from then on a summary that has STOPPED printing
    the counter compares equal to it, and the drift check reports OK.

    The cross-check already refuses this for the four counters that are Parquet
    row identities. This covers the rest, which had nothing.
    """
    return [f"{key}: the export summary did not print it"
            for key in sorted(current["counters"])
            if current["counters"][key] is None]


def measure(exe: Path, replay: Path, out_dir: Path, checkpoints: bool = False) -> dict:
    """Export one replay and collect the summary counters and Parquet shape.

    The output directory is deleted first. Exporting over a previous run would
    leave a file the exporter has stopped writing sitting there with last
    run's contents, and both checks below would then read it and pass -- the
    exact way "a stale file makes a matching hash meaningless" applies here.

    `checkpoints` runs the optional Checkpoint pass and pins its counters and
    its table too. That path is off by default in the exporter, and an
    unguarded optional path is the shape of every silent change this script
    exists to prevent -- so it gets a baseline of its own rather than none.
    """
    shutil.rmtree(out_dir, ignore_errors=True)
    out_dir.mkdir(parents=True, exist_ok=True)

    cmd = [str(exe), "export", str(replay), "--out", str(out_dir)]
    if checkpoints:
        cmd.append("--checkpoints")
    r = subprocess.run(
        cmd, capture_output=True, text=True, encoding="utf-8",
        errors="replace", timeout=1800,
    )
    text = (r.stdout or "") + (r.stderr or "")
    if r.returncode != 0:
        tail = " | ".join(l for l in text.splitlines()[-5:] if l.strip())
        raise SystemExit(f"export failed (exit {r.returncode}): {tail[:400]}")

    patterns = dict(PATTERNS)
    files = list(PARQUET_FILES)
    if checkpoints:
        patterns.update({k: re.compile(v) for k, v in CHECKPOINT_COUNTERS.items()})
        files.append("checkpoint_fields")

    counters = {}
    for key, pat in patterns.items():
        match = pat.search(text)
        counters[key] = None if match is None else int(match.group(1))

    parquet = {}
    for name in files:
        path = out_dir / f"{name}.parquet"
        if not path.exists():
            raise SystemExit(f"export wrote no {name}.parquet in {out_dir}")
        parquet[name] = {
            "rows": pq.ParquetFile(path).metadata.num_rows,
            "bytes": path.stat().st_size,
        }

    return {"counters": counters, "parquet": parquet}


def diff(baseline: dict, current: dict) -> list[str]:
    """Every way the pinned numbers and the current ones disagree."""
    out = []
    for key in sorted(set(baseline["counters"]) | set(current["counters"])):
        want = baseline["counters"].get(key)
        got = current["counters"].get(key)
        if want != got:
            out.append(f"counter {key}: {got} (baseline {want})")
    for name in sorted(set(baseline["parquet"]) | set(current["parquet"])):
        want = baseline["parquet"].get(name, {})
        got = current["parquet"].get(name, {})
        for field in ("rows", "bytes"):
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
                    help="export directory, DELETED first (default: out/export_check)")
    ap.add_argument("--update", action="store_true",
                    help="rewrite the baseline from the current numbers")
    ap.add_argument("--checkpoints", action="store_true",
                    help="run the optional Checkpoint pass and pin its counters "
                         "and checkpoint_fields.parquet too")
    args = ap.parse_args()

    if not args.exe.exists():
        print(f"build the release binary first: {args.exe}", file=sys.stderr)
        return 2

    stored = json.loads(args.baseline.read_text(encoding="utf-8")) \
        if args.baseline.exists() else {}
    replay = args.replay or Path(os.path.expandvars(stored.get("replay", "")))
    # A bare filename in the baseline resolves against VRFKIT_CORPUS_DIR so the
    # repo ships no absolute path; an absolute path (old baselines, --replay) is
    # used as-is. Unset env + filename -> relative -> not found -> SKIP below.
    if replay.name and not replay.is_absolute():
        corpus_dir = os.environ.get("VRFKIT_CORPUS_DIR", "")
        if corpus_dir:
            replay = Path(corpus_dir) / replay
    if not replay.name or not replay.exists():
        print(f"SKIP: replay not present ({replay})")
        print("      the corpus lives outside this repo; nothing to guard here.")
        return 0

    out_dir = args.out or (REPO / "out" / "export_check")
    current = measure(args.exe, replay, out_dir, checkpoints=args.checkpoints)

    # The cross-check runs whether or not a baseline exists, and before the
    # baseline is written: pinning a summary that already contradicts its own
    # Parquet output would pin the lie.
    lies = cross_checks(current["counters"], current["parquet"])
    if lies:
        print(f"CROSS-CHECK FAILED: {len(lies)} counter(s) disagree with the "
              f"Parquet files they name")
        for line in lies:
            print(f"  {line}")
        return 1

    if args.update:
        refusals = unpinnable(current)
        if refusals:
            print(f"FAILED: refusing to pin a run with {len(refusals)} "
                  f"unmeasured counter(s)")
            for line in refusals:
                print(f"  {line}")
            print("  A None in the baseline is matched by the counter going "
                  "missing again, which is the failure this file exists to "
                  "catch.")
            return 1
        payload = {"replay": stored.get("replay") or str(replay), **current}
        args.baseline.parent.mkdir(parents=True, exist_ok=True)
        args.baseline.write_text(json.dumps(payload, indent=1) + "\n",
                                 encoding="utf-8")
        print(f"wrote {args.baseline} (NetGUID rows "
              f"{current['counters']['net_guid_rows']})")
        return 0

    if not stored:
        print(f"no baseline at {args.baseline} -- run with --update",
              file=sys.stderr)
        return 2

    problems = diff(stored, current)
    if problems:
        print(f"DRIFT: {len(problems)} difference(s) from {args.baseline.name}")
        for line in problems:
            print(f"  {line}")
        return 1

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
