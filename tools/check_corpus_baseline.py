#!/usr/bin/env python3
"""Pin a corpus's oracle numbers and fail when they drift.

validate_corpus.py prints what a corpus currently does. That answers "is it
working today" but not "did my change move it", which is the question a
regression guard has to answer. The 13.01 numbers are pinned by hand in
PROJECT_STATUS; the 13.02 build had nothing pinned at all, so a transform
change could have broken it silently -- that gap is item 7-E.

This stores per-file and total figures in a JSON baseline and compares
against it, exiting non-zero on any difference.

The 13.02 replays live outside the repo (%LOCALAPPDATA%\\VALORANT\\Saved\\Demos)
and are machine-specific, so a missing corpus is reported and SKIPPED rather
than failed. A guard that fails on someone else's machine gets disabled, and
a disabled guard protects nothing.

Usage:
    python tools/check_corpus_baseline.py --baseline tools/baselines/build_1302.json
    python tools/check_corpus_baseline.py --baseline <path> --update
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from validate_corpus import PATTERNS  # noqa: E402  (shared so regexes cannot drift)

REPO = Path(__file__).resolve().parent.parent
DEFAULT_EXE = REPO / "target" / "release" / "vrfkit.exe"


def measure(exe: Path, root: Path) -> dict:
    """Run the oracle over every .vrf under root and collect the numbers."""
    files = sorted(root.rglob("*.vrf"))
    per_file = {}
    totals = {"blocks": 0, "fields": 0, "rpcs": 0, "malformed": 0, "skipped": 0}
    branches = {}

    for f in files:
        r = subprocess.run(
            [str(exe), "validate", str(f)],
            capture_output=True, text=True, encoding="utf-8",
            errors="replace", timeout=300,
        )
        out = (r.stdout or "") + (r.stderr or "")
        got = {k: p.search(out) for k, p in PATTERNS.items()}
        if r.returncode != 0 or not got["rate"]:
            per_file[f.name] = {"error": f"exit {r.returncode}"}
            continue
        branch = got["branch"].group(1)
        branches[branch] = branches.get(branch, 0) + 1
        entry = {"branch": branch, "rate": got["rate"].group(1)}
        for key in totals:
            match = got.get(key)
            if match is None:
                # Recorded rather than defaulted to 0: a counter the oracle
                # stopped printing is a change worth failing on, and folding
                # it into a total would hide it.
                entry[key] = None
                continue
            value = int(match.group(1))
            entry[key] = value
            totals[key] += value
        per_file[f.name] = entry

    return {"branches": branches, "totals": totals, "per_file": per_file}


def diff(baseline: dict, current: dict) -> list[str]:
    """Every way the two disagree, as human-readable lines."""
    out = []
    for key, want in baseline["totals"].items():
        got = current["totals"].get(key)
        if got != want:
            out.append(f"total {key}: {got} (baseline {want})")
    if baseline["branches"] != current["branches"]:
        out.append(f"branches: {current['branches']} (baseline {baseline['branches']})")

    b_files, c_files = set(baseline["per_file"]), set(current["per_file"])
    for name in sorted(b_files - c_files):
        out.append(f"missing replay: {name}")
    for name in sorted(c_files - b_files):
        out.append(f"replay not in baseline: {name}")
    for name in sorted(b_files & c_files):
        want, got = baseline["per_file"][name], current["per_file"][name]
        for key in sorted(set(want) | set(got)):
            if want.get(key) != got.get(key):
                out.append(f"{name} {key}: {got.get(key)} (baseline {want.get(key)})")
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--baseline", type=Path, required=True)
    ap.add_argument("--exe", type=Path, default=DEFAULT_EXE)
    ap.add_argument("--corpus", type=Path, default=None,
                    help="overrides the corpus path stored in the baseline")
    ap.add_argument("--update", action="store_true",
                    help="rewrite the baseline from the current numbers")
    args = ap.parse_args()

    if not args.exe.exists():
        print(f"build the release binary first: {args.exe}", file=sys.stderr)
        return 2

    stored = json.loads(args.baseline.read_text(encoding="utf-8")) \
        if args.baseline.exists() else {}
    corpus = args.corpus or Path(os.path.expandvars(stored.get("corpus", "")))
    # A relative path in the baseline resolves against VRFKIT_CORPUS_DIR so the
    # repo ships no absolute path; absolute paths and --corpus are used as-is.
    if corpus.name and not corpus.is_absolute():
        corpus_dir = os.environ.get("VRFKIT_CORPUS_DIR", "")
        if corpus_dir:
            corpus = Path(corpus_dir) / corpus
    if not corpus or not corpus.exists():
        print(f"SKIP: corpus not present ({corpus})")
        print("      these replays are machine-local; nothing to guard here.")
        return 0

    current = measure(args.exe, corpus)
    if not current["per_file"]:
        print(f"SKIP: no .vrf under {corpus}")
        return 0

    if args.update:
        payload = {"corpus": stored.get("corpus") or str(corpus), **current}
        args.baseline.parent.mkdir(parents=True, exist_ok=True)
        args.baseline.write_text(json.dumps(payload, indent=1) + "\n",
                                 encoding="utf-8")
        n = len(current["per_file"])
        print(f"wrote {args.baseline} ({n} replays, "
              f"branches {current['branches']})")
        return 0

    if not stored:
        print(f"no baseline at {args.baseline} -- run with --update",
              file=sys.stderr)
        return 2

    problems = diff(stored, current)
    n = len(current["per_file"])
    if problems:
        print(f"DRIFT: {len(problems)} difference(s) across {n} replays")
        for line in problems:
            print(f"  {line}")
        return 1

    print(f"OK: {n} replays match the baseline "
          f"(branches {current['branches']}, "
          f"malformed {current['totals']['malformed']})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
