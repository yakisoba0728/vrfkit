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

Only the default (checkpoint-free) timing is RECORDABLE.
`tools/check_baseline_schemas.py` accepts exactly `{export, replay}` in
`bench.json` and rejects anything else, so `--checkpoints --update` is refused
rather than writing an `export_checkpoints` key that makes the committed
baseline fail this repo's own pre-PR sweep. `--checkpoints` without `--update`
still times and prints the number; see `BASELINE_KEYS`.

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

#: The exact key set `tools/check_baseline_schemas.py` accepts in `bench.json`
#: (`validate_bench_baseline`, via `_keys`). It is an EQUALITY check, not a
#: subset one, so any other key makes the committed baseline invalid and the
#: pre-PR sweep in CONTRIBUTING.md go red on `check_baseline_schemas.py`.
#:
#: This tool used to write `export_checkpoints` here under `--checkpoints
#: --update` -- a generator emitting a file its own repo's validator rejects.
#: The SKIP message on the comparison path then advised recording one with
#: `--update`, which is advice that cannot be followed: doing it produces a
#: baseline that fails the sweep. `--checkpoints --update` is now refused
#: loudly and names what would have to change, rather than writing the key and
#: leaving the rejection to be discovered later by a confusing error about a
#: file this tool wrote.
BASELINE_KEYS = frozenset({"export", "replay"})

#: The timing key each mode measures. Only `export` has a slot in
#: `BASELINE_KEYS`; `export_checkpoints` is measurable and printable but not
#: recordable, and `--update` says so instead of writing it.
TIMING_KEYS = {False: "export", True: "export_checkpoints"}

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
    ap.add_argument("--checkpoints", action="store_true",
                    help="also decode checkpoints; times and prints the number "
                         "but cannot be recorded -- bench.json has no slot for "
                         "it (see BASELINE_KEYS)")
    ap.add_argument("--update", action="store_true",
                    help="rewrite the baseline with this run's numbers")
    args = ap.parse_args()

    for path, what in ((args.exe, "binary"), (args.replay, "replay")):
        if not path.exists():
            print(f"SKIP: no {what} at {path}", file=sys.stderr)
            return 0

    key = TIMING_KEYS[bool(args.checkpoints)]

    # Refused BEFORE the timing runs: recording is the only thing --update
    # does, so spending minutes on repeated exports to then reject the write
    # would waste the run and read as a benchmark failure.
    if args.update and key not in BASELINE_KEYS:
        print(f"--checkpoints --update cannot be recorded: bench.json accepts "
              f"exactly {sorted(BASELINE_KEYS)}, and check_baseline_schemas.py "
              f"rejects any other key, so writing {key!r} would produce a "
              f"baseline that fails the pre-PR sweep. Re-run without "
              f"--checkpoints to record the export baseline, or time "
              f"checkpoints without --update to just read the number. Adding "
              f"a checkpoint baseline means teaching "
              f"check_baseline_schemas.py's validate_bench_baseline the key "
              f"first.", file=sys.stderr)
        return 2

    samples = time_export(args.exe, args.replay, args.repeats, args.checkpoints)
    seconds = median(samples)
    print(f"{key}: median {seconds:.3f}s over {args.repeats} runs "
          f"(min {min(samples):.3f}, max {max(samples):.3f})")

    if args.update:
        # Written from scratch, NOT merged into what is already there. The
        # merge kept every key the old file had while replacing `replay`, so
        # recording against a different replay left the previous replay's
        # timing sitting under the new replay's name -- a plausible wrong
        # number attributed to a measurement that never happened. `replay` and
        # the timing beside it come from the same run or neither does.
        data = {"export": round(seconds, 3), "replay": args.replay.name}
        assert set(data) == set(BASELINE_KEYS), data
        atomic_write_text(args.baseline, json.dumps(data, indent=2) + "\n")
        print(f"wrote {args.baseline}")
        return 0

    if not args.baseline.exists():
        print(f"SKIP: no baseline at {args.baseline} -- record one with --update")
        return 0

    data = json.loads(args.baseline.read_text(encoding="utf-8"))

    # The baseline's timing means nothing unless it was recorded against THIS
    # replay -- a shorter or longer replay times faster or slower for reasons
    # that have nothing to do with the code, and `--update` above already
    # treats `replay` and its timing as one fact that must come from the same
    # run. A baseline with no `replay` at all (an older file, or one written
    # before this check existed) is not "assume it matches" -- that is exactly
    # the unmeasured comparison this check exists to refuse.
    baseline_replay = data.get("replay")
    if baseline_replay != args.replay.name:
        recorded = repr(baseline_replay) if baseline_replay is not None else \
            "(no replay recorded)"
        print(f"SKIP: baseline was recorded against {recorded}, not "
              f"{args.replay.name!r} -- comparing them would time two "
              f"different replays against each other. Record a baseline for "
              f"this replay with --update, or pass the replay the baseline "
              f"names.")
        return 0

    if key not in data:
        # No "record one with --update" here for the checkpoint key: that is
        # advice this tool cannot honour (see BASELINE_KEYS), and pointing a
        # reader at a command that produces an invalid baseline is worse than
        # saying plainly that there is nothing to compare against.
        recordable = key in BASELINE_KEYS
        print(f"SKIP: baseline has no {key} entry"
              + (" -- record one with --update" if recordable else
                 f" and bench.json has no slot for one, so this timing has "
                 f"nothing to compare against (only "
                 f"{sorted(BASELINE_KEYS)} are recorded)"))
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
