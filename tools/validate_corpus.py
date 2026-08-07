"""Run the grammar oracle across an entire replay corpus and summarise.

A single replay proving out says the decoder works on that replay. Robustness is a
different claim: no file may crash, and every file must land at essentially the
same oracle pass rate. A file that drops to a low rate would mean either a build
we mis-detected or a stream shape we have never seen.

Replays are independent, so they run one subprocess each, several at a time.
Set VRFKIT_JOBS to override the worker count (default: cores - 2, capped at
16). This changes no number -- each subprocess owns its own output and shares
nothing. Parallelising *inside* a replay is a different question, measured
and closed in docs/archive/PROJECT_STATUS.md 7-F.

Corpus discovery is shared with `check_decode_errors_corpus.py` through
`corpus_scan.py` -- read that module's docstring for why the default does not
recurse into subdirectories and why the excluded count always prints. Pass
`--recursive` to walk subdirectories too.

Usage:
    python tools/validate_corpus.py <vrfkit.exe> <dir-with-vrf-files> [limit]
    python tools/validate_corpus.py <vrfkit.exe> <dir-with-vrf-files> --recursive
"""

from __future__ import annotations

import argparse
import collections
import os
import re
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import corpus_scan

PATTERNS = {
    "branch": re.compile(r"Branch:\s+(\S+)"),
    "blocks": re.compile(r"Total content blocks:\s+(\d+)"),
    # The oracle prints "Malformed framing:  0". This pattern used to be
    # "Malformed:\s+(\d+)", which never matched -- and the accumulator below
    # skips a pattern that does not match, so every corpus run has reported
    # malformed 0 without ever reading the number. Anchored on the real label.
    "malformed": re.compile(r"Malformed framing:\s+(\d+)"),
    "skipped": re.compile(r"Skipped bits:\s+(\d+)"),
    "rate": re.compile(r"ORACLE PASS RATE:\s+([\d.]+)%"),
    "fields": re.compile(r"Fields emitted:\s+(\d+)"),
    "rpcs": re.compile(r"RPCs emitted:\s+(\d+)"),
}


def problems(failures, missing) -> list[str]:
    """Everything that makes this sweep a failure rather than a measurement.

    A replay the oracle could not validate has always been fatal. A counter it
    stopped PRINTING was not, and the accumulator above already argues that it
    should be: "A counter the oracle stopped printing must not read as zero.
    That is precisely how the malformed figure stayed a vacuous 0". The counter
    was recorded as absent, printed as a WARNING, and the run exited 0 -- so
    the corpus totals below could be summed over a subset nobody was told
    about.

    The pass rates deliberately stay informational. The docstring's robustness
    claim is about them, but the threshold cannot be defended from here without
    the corpus in hand, and `check_corpus_baseline.py` already pins each
    replay's rate against a baseline -- which catches a rate that MOVED, the
    thing a fixed threshold would only approximate.
    """
    out = [f"{name}: {why}" for name, why in failures]
    out += [f"the oracle did not print '{key}' on {count} replay(s), so the "
            f"corpus total for it is summed over the rest"
            for key, count in sorted(missing.items())]
    return out


def _run_one(exe: Path, path: Path) -> tuple[str | None, str]:
    """Validate one replay. Returns (error, combined output).

    The oracle prints to stdout; stderr carries progress noise, and both are
    searched. UTF-8 is forced because the CLI writes a few non-ASCII glyphs
    and Python would otherwise pick the Windows console codepage and raise
    UnicodeDecodeError mid-stream.
    """
    try:
        r = subprocess.run(
            [str(exe), "validate", str(path)],
            capture_output=True, text=True,
            encoding="utf-8", errors="replace", timeout=300,
        )
    except subprocess.TimeoutExpired:
        return "timeout", ""
    out = (r.stdout or "") + (r.stderr or "")
    if r.returncode != 0:
        tail = " | ".join(l for l in out.splitlines()[-3:] if l.strip())
        return f"exit {r.returncode}: {tail[:160]}", out
    return None, out


def parse_args(argv: list[str]) -> argparse.Namespace:
    """Parsed as `argv[1:]` (not `sys.argv` directly) so this is testable
    without monkeypatching -- pass a fake argv and read the Namespace back."""
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("exe", type=Path)
    ap.add_argument("corpus", type=Path)
    ap.add_argument("limit", type=int, nargs="?", default=None,
                    help="only validate the first N discovered replays")
    ap.add_argument("--recursive", action="store_true",
                    help="also walk subdirectories of <corpus> -- see "
                         "corpus_scan.py for why this is opt-in")
    return ap.parse_args(argv[1:])


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    exe, root = args.exe, args.corpus
    # Leave two cores for the OS; each worker is a whole vrfkit process.
    jobs = max(1, min(int(os.environ.get("VRFKIT_JOBS", "0")) or (os.cpu_count() or 2) - 2, 16))

    scan = corpus_scan.discover(root, args.recursive)
    # Unconditional, `excluded=0` included -- see corpus_scan.py. A line that
    # only appeared when something was left out could not be told apart from
    # a scan that silently stopped discovering files at all.
    print(corpus_scan.scope_line(scan))
    files = scan.files
    if args.limit is not None:
        files = files[: args.limit]
        print(f"limited to the first {len(files)} of {len(scan.files)} discovered")
    if not files:
        raise SystemExit(f"no .vrf under {root}")

    print(f"\nvalidating {len(files)} replays with {exe} ({jobs} workers)\n")
    ok = 0
    failures: list[tuple[str, str]] = []
    branches: collections.Counter[str] = collections.Counter()
    rates: list[tuple[float, str]] = []
    totals = collections.Counter()
    missing: collections.Counter[str] = collections.Counter()
    started = time.time()

    # One subprocess per replay, run jobs-wide. Each already owns its own
    # output and shares nothing, so this is near-linear with no effect on any
    # number -- unlike parallelising inside a replay, which 7-F measured and
    # closed because content blocks are order-dependent.
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        results = pool.map(lambda f: (f, _run_one(exe, f)), files)

        for i, (f, outcome) in enumerate(results, 1):
            err, out = outcome
            if err is not None:
                failures.append((f.name, err))
                continue
            got = {k: p.search(out) for k, p in PATTERNS.items()}
            if not got["rate"]:
                tail = " | ".join(l for l in out.splitlines()[-3:] if l.strip())
                failures.append((f.name, f"no pass rate: {tail[:160]}"))
                continue
            ok += 1
            branches[got["branch"].group(1)] += 1
            rates.append((float(got["rate"].group(1)), f.name))
            for key in ("blocks", "malformed", "skipped", "fields", "rpcs"):
                if got[key]:
                    totals[key] += int(got[key].group(1))
                else:
                    # A counter the oracle stopped printing must not read as
                    # zero. That is precisely how the malformed figure stayed a
                    # vacuous 0 for the whole corpus while its pattern wrong.
                    missing[key] += 1
            if i % 25 == 0 or i == len(files):
                print(f"  [{i}/{len(files)}] ok={ok} failed={len(failures)}")

    elapsed = time.time() - started
    print(f"\nelapsed {elapsed:.1f}s ({elapsed / max(len(files), 1):.2f}s per replay)")
    print(f"succeeded: {ok}/{len(files)}")
    print(f"failed   : {len(failures)}")
    for name, why in failures[:15]:
        print(f"    {name}: {why}")

    if missing:
        print("\nWARNING: counters the oracle did not print (NOT counted as 0):")
        for key, count in missing.most_common():
            print(f"  {key}: absent on {count} replay(s)")

    print("\nbranches seen:")
    for b, c in branches.most_common():
        print(f"  {c:>4}  {b}")

    if rates:
        rates.sort()
        print("\noracle pass rate:")
        print(f"  min    {rates[0][0]:.6f}%  ({rates[0][1]})")
        print(f"  median {rates[len(rates) // 2][0]:.6f}%")
        print(f"  max    {rates[-1][0]:.6f}%")
        below = [(r, n) for r, n in rates if r < 99.99]
        print(f"  below 99.99%: {len(below)}")
        for r, n in below[:10]:
            print(f"    {r:.6f}%  {n}")

    print("\ncorpus totals:")
    for key in ("blocks", "fields", "rpcs", "malformed", "skipped"):
        print(f"  {key:<10} {totals[key]:>14,}")

    found = problems(failures, missing)
    if found:
        print(f"\nFAILED: {len(found)} problem(s) across {len(files)} replays",
              file=sys.stderr)
        for line in found[:20]:
            print(f"    {line}", file=sys.stderr)
        return 1
    print(f"\nOK: {ok}/{len(files)} replays validated, every counter printed on "
          f"every one. Pass rates are reported above, not gated -- "
          f"check_corpus_baseline.py pins them per replay.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
