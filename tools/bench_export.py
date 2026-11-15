#!/usr/bin/env python3
"""Time `vrfkit export` against a recorded baseline.

A smoke detector, not a profiler: wall clock is noisy, so the default
tolerance is 25% and it answers "did something get twice as slow". A run well
under the baseline fails too: the baseline no longer describes the code, and
a stale baseline is how the next regression hides. `--checkpoints` times
`export --checkpoints` against its own slot, `export_checkpoints`.

Usage:
    python tools/bench_export.py --exe ./target/release/vrfkit.exe \\
        --replay "$VRFKIT_CORPUS_DIR/02d4d478-....vrf"
    python tools/bench_export.py --exe ... --replay ... [--checkpoints] --update
"""

from __future__ import annotations

import argparse
import json
import math
import shutil
import statistics
import sys
import tempfile
import time
from pathlib import Path

import summary_counters as sc
from atomic_io import atomic_write_text

REPO = Path(__file__).resolve().parents[1]
DEFAULT_BASELINE = REPO / "tools" / "baselines" / "bench.json"
#: The timing slots bench.json holds; check_baseline_schemas.py validates them.
TIMINGS = ("export", "export_checkpoints")
#: Either side of the baseline, noise rather than news: a browser waking up
#: moves wall clock more than most of what this guards.
DEFAULT_TOLERANCE = 0.25


def positive_finite(value: float, name: str) -> None:
    """Reject missing/invalid measurements instead of treating them as noise."""
    if (isinstance(value, bool) or not isinstance(value, (int, float))
            or not math.isfinite(value) or value <= 0):
        raise ValueError(f"{name} must be positive and finite, got {value!r}")


def valid_tolerance(value: float) -> None:
    if (isinstance(value, bool) or not isinstance(value, (int, float))
            or not math.isfinite(value) or not 0 <= value < 1):
        raise ValueError(f"tolerance must be finite and in [0, 1), got {value!r}")


def compare(measured: float, baseline: float, tolerance: float) -> tuple[str, float]:
    """`(verdict, ratio)`: `slower` past the tolerance, `faster` under it, `ok`
    between."""
    positive_finite(measured, "measured time")
    positive_finite(baseline, "baseline")
    valid_tolerance(tolerance)
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
        try:
            start = time.perf_counter()
            code, text = sc.vrfkit(exe, "export", replay, out, checkpoints)
            elapsed = time.perf_counter() - start
            if code != 0:
                raise SystemExit(f"export failed ({code}):\n{text[-2000:]}")
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
    ap.add_argument("--tolerance", type=float, default=DEFAULT_TOLERANCE,
                    help="relative tolerance, finite and in [0, 1)")
    ap.add_argument("--checkpoints", action="store_true",
                    help="time export --checkpoints, against export_checkpoints")
    ap.add_argument("--update", action="store_true",
                    help="record this run's timing in the baseline")
    args = ap.parse_args()
    try:
        valid_tolerance(args.tolerance)
    except ValueError as exc:
        ap.error(str(exc))
    if args.repeats < 1:
        ap.error("--repeats must be at least 1: no sample is not a zero")

    if sc.no_exe(args.exe):
        return 2
    if not args.replay.exists():
        return sc.missing_input(f"no replay at {args.replay}", False)

    key = TIMINGS[args.checkpoints]
    stored = sc.load_baseline(args.baseline)
    same_replay = stored.get("replay") == args.replay.name
    if args.update and args.checkpoints and not (same_replay and "export" in stored):
        # check_baseline_schemas rejects a bench.json without `export`.
        print(f"record the export slot for {args.replay.name} first (--update without "
              f"--checkpoints)", file=sys.stderr)
        return 2
    try:
        # Reject an unusable baseline before spending time on an export.
        if not args.update and same_replay and key in stored:
            positive_finite(stored[key], "baseline")
        samples = time_export(args.exe, args.replay, args.repeats, args.checkpoints)
        if len(samples) != args.repeats:
            raise ValueError("benchmark returned the wrong number of samples")
        for sample in samples:
            positive_finite(sample, "measured time")
    except ValueError as exc:
        print(f"FAILED: {exc}", file=sys.stderr)
        return 2
    seconds = statistics.median(samples)
    print(f"{key}: median {seconds:.3f}s over {args.repeats} runs "
          f"(min {min(samples):.3f}, max {max(samples):.3f})")

    if args.update:
        # The other slot is kept only beside the replay it timed; nothing else is.
        data = {k: v for k, v in stored.items() if k in TIMINGS} if same_replay else {}
        data.update({key: seconds, "replay": args.replay.name})
        atomic_write_text(args.baseline, json.dumps(dict(sorted(data.items())), indent=2) + "\n")
        print(f"wrote {args.baseline}")
        return 0
    if not same_replay or key not in stored:
        return sc.missing_input(
            f"{args.baseline.name} has no {key} timing for {args.replay.name} (it times "
            f"{stored.get('replay')!r}); record one with --update", False)

    verdict, ratio = compare(seconds, stored[key], args.tolerance)
    print(f"  baseline {stored[key]:.3f}s, ratio {ratio:.2f}x -> {verdict}")
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
