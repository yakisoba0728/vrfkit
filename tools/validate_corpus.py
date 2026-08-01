"""Run the grammar oracle across an entire replay corpus and summarise.

A single replay proving out says the decoder works on that replay. Robustness is a
different claim: no file may crash, and every file must land at essentially the
same oracle pass rate. A file that drops to a low rate would mean either a build
we mis-detected or a stream shape we have never seen.

Usage:
    python tools/validate_corpus.py <vrfkit.exe> <dir-with-vrf-files> [limit]
"""

from __future__ import annotations

import collections
import re
import subprocess
import sys
import time
from pathlib import Path

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


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        raise SystemExit(__doc__)
    exe, root = Path(argv[1]), Path(argv[2])
    limit = int(argv[3]) if len(argv) > 3 else None
    files = sorted(root.rglob("*.vrf"))[:limit]
    if not files:
        raise SystemExit(f"no .vrf under {root}")

    print(f"validating {len(files)} replays with {exe}\n")
    ok = 0
    failures: list[tuple[str, str]] = []
    branches: collections.Counter[str] = collections.Counter()
    rates: list[tuple[float, str]] = []
    totals = collections.Counter()
    missing: collections.Counter[str] = collections.Counter()
    started = time.time()

    for i, f in enumerate(files, 1):
        try:
            # The oracle prints to stdout; stderr carries progress noise.
            #
            # Decode as UTF-8 explicitly: the CLI writes a few non-ASCII glyphs
            # and Python would otherwise pick the Windows console codepage and
            # raise UnicodeDecodeError mid-stream.
            r = subprocess.run(
                [str(exe), "validate", str(f)],
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
                timeout=300,
            )
        except subprocess.TimeoutExpired:
            failures.append((f.name, "timeout"))
            continue
        out = (r.stdout or "") + (r.stderr or "")
        got = {k: p.search(out) for k, p in PATTERNS.items()}
        if r.returncode != 0 or not got["rate"]:
            tail = " | ".join(line for line in out.splitlines()[-3:] if line.strip())
            failures.append((f.name, f"exit {r.returncode}: {tail[:160]}"))
            continue
        ok += 1
        branches[got["branch"].group(1)] += 1
        rate = float(got["rate"].group(1))
        rates.append((rate, f.name))
        for key in ("blocks", "malformed", "skipped", "fields", "rpcs"):
            if got[key]:
                totals[key] += int(got[key].group(1))
            else:
                # A counter the oracle stopped printing must not read as zero.
                # That is precisely how the malformed figure stayed a vacuous
                # 0 for the whole corpus while its pattern was wrong.
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
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
