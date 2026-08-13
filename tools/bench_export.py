#!/usr/bin/env python3
"""Time `vrfkit export` against a recorded baseline.

Every performance claim in this repo -- the SmallVec inlining, the hash hoist,
the dictionary-encoded string columns, the vectorised movement path -- was
measured once and then lived only in a commit message. Nothing in the tree could
re-measure any of it, so nobody could tell whether a later change gave it back.

This is deliberately not a microbenchmark and does not pull in criterion. The
workspace has two direct dependencies and vendors its own hasher rather than
take a third; a benchmarking framework with a few dozen transitive crates would
cost more than the question is worth. What it does instead is what the rest of
`tools/` already does: run the real thing, record the number in
`tools/baselines/`, and compare against it.

**Wall clock is noisy.** The default tolerance is deliberately loose, and this
is a smoke detector, not a profiler: it answers "did something get twice as
slow" and nothing finer. A run well *under* the baseline is reported too --
that means the baseline no longer describes the code, which needs the same
attention as a regression.

Usage:
    python tools/bench_export.py --exe ./target/release/vrfkit.exe \\
        --replay "$VRFKIT_CORPUS_DIR/02d4d478-....vrf"
    python tools/bench_export.py --exe ... --replay ... --update
"""

from __future__ import annotations

import argparse
import json
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

if __package__:
    from .atomic_io import atomic_write_text
else:  # direct script execution
    from atomic_io import atomic_write_text

REPO = Path(__file__).resolve().parents[1]
DEFAULT_BASELINE = REPO / "tools" / "baselines" / "bench.json"

#: Fraction either side of the baseline that counts as noise rather than news.
#: Wall clock on a developer machine moves more than people expect -- a browser
#: waking up is worth more than most of the optimisations this guards.
DEFAULT_TOLERANCE = 0.25


def median(values: list[float]) -> float:
    """Median of `values`. Empty input is an error, not a zero.

    A zero would read as an infinitely fast run and pass every comparison.
    """
    if not values:
        raise ValueError("no samples to take a median of")
    return statistics.median(values)


def compare(measured: float, baseline: float, tolerance: float) -> tuple[str, float]:
    """`(verdict, ratio)` for one timing against its baseline.

    `slower` past the tolerance, `faster` under it, `ok` between. Faster is
    reported rather than swallowed: it means the recorded number no longer
    describes the code, and a stale baseline is how a later regression hides.
    """
    if baseline <= 0:
        raise ValueError(f"baseline must be positive, got {baseline}")
    ratio = measured / baseline
    if ratio > 1 + tolerance:
        return "slower", ratio
    if ratio < 1 - tolerance:
        return "faster", ratio
    return "ok", ratio


def time_export(exe: Path, replay: Path, repeats: int,
                checkpoints: bool) -> list[float]:
    """Wall-clock seconds for each of `repeats` full export runs."""
    samples = []
    for _ in range(repeats):
        out = Path(tempfile.mkdtemp(prefix="vrfkit-bench-"))
        cmd = [str(exe), "export", str(replay), "--out", str(out)]
        if checkpoints:
            cmd.append("--checkpoints")
        try:
            start = time.perf_counter()
            result = subprocess.run(cmd, capture_output=True, text=True)
            elapsed = time.perf_counter() - start
            if result.returncode != 0:
                raise SystemExit(
                    f"export failed ({result.returncode}):\n{result.stderr[-2000:]}")
            samples.append(elapsed)
        finally:
            shutil.rmtree(out, ignore_errors=True)
    return samples


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--exe", type=Path, required=True,
                    help="release vrfkit binary (a debug build measures nothing)")
    ap.add_argument("--replay", type=Path, required=True,
                    help="replay to export; see VRFKIT_CORPUS_DIR in CONTRIBUTING")
    ap.add_argument("--baseline", type=Path, default=DEFAULT_BASELINE)
    ap.add_argument("--repeats", type=int, default=5)
    ap.add_argument("--tolerance", type=float, default=DEFAULT_TOLERANCE)
    ap.add_argument("--checkpoints", action="store_true")
    ap.add_argument("--update", action="store_true",
                    help="rewrite the baseline with this run's numbers")
    args = ap.parse_args()

    for path, what in ((args.exe, "binary"), (args.replay, "replay")):
        if not path.exists():
            print(f"SKIP: no {what} at {path}", file=sys.stderr)
            return 0

    samples = time_export(args.exe, args.replay, args.repeats, args.checkpoints)
    seconds = median(samples)
    key = "export_checkpoints" if args.checkpoints else "export"
    print(f"{key}: median {seconds:.3f}s over {args.repeats} runs "
          f"(min {min(samples):.3f}, max {max(samples):.3f})")

    if args.update:
        data = {}
        if args.baseline.exists():
            data = json.loads(args.baseline.read_text(encoding="utf-8"))
        data[key] = round(seconds, 3)
        data["replay"] = args.replay.name
        atomic_write_text(args.baseline, json.dumps(data, indent=2) + "\n")
        print(f"wrote {args.baseline}")
        return 0

    if not args.baseline.exists():
        print(f"SKIP: no baseline at {args.baseline} -- record one with --update")
        return 0

    data = json.loads(args.baseline.read_text(encoding="utf-8"))
    if key not in data:
        print(f"SKIP: baseline has no {key} entry -- record one with --update")
        return 0

    verdict, ratio = compare(seconds, data[key], args.tolerance)
    print(f"  baseline {data[key]:.3f}s, ratio {ratio:.2f}x -> {verdict}")
    if verdict == "ok":
        print(f"\nOK: within {args.tolerance:.0%} of the baseline")
        return 0
    if verdict == "faster":
        print(f"\nFASTER than the baseline by more than {args.tolerance:.0%}. "
              f"Good news, but the baseline no longer describes the code -- "
              f"re-record it with --update so the next regression is visible.",
              file=sys.stderr)
        return 1
    print(f"\nSLOWER than the baseline by more than {args.tolerance:.0%}. "
          f"Re-run before believing it -- wall clock is noisy -- and if it "
          f"holds, find the change before recording a new baseline.",
          file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
