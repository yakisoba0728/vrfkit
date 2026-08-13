"""Find which replays still skip bits, and how many.

After the velocity fix the corpus total dropped from 153,096 to 3,671 skipped
bits, but the reference replay now reports zero. So the remainder is concentrated
in a subset of files rather than spread evenly, which is what makes it worth
locating: a handful of files with a shared cause is tractable, an even smear is
not.

Usage:
    python tools/find_skips.py <vrfkit.exe> <dir-with-vrf-files> [limit]
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

SKIPPED = re.compile(r"Skipped bits:\s+(\d+)")
MALFORMED = re.compile(r"Malformed framing:\s+(\d+)")
RATE = re.compile(r"ORACLE PASS RATE:\s+([\d.]+)%")


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        raise SystemExit(__doc__)
    exe, root = Path(argv[1]), Path(argv[2])
    limit = int(argv[3]) if len(argv) > 3 else None
    files = sorted(root.rglob("*.vrf"))[:limit]

    offenders: list[tuple[int, int, float, str]] = []
    total_skipped = 0
    clean = 0
    for i, f in enumerate(files, 1):
        r = subprocess.run(
            [str(exe), "validate", str(f)],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=300,
        )
        out = (r.stdout or "") + (r.stderr or "")
        m = SKIPPED.search(out)
        if not m:
            offenders.append((-1, -1, 0.0, f.name))
            continue
        skipped = int(m.group(1))
        total_skipped += skipped
        if skipped == 0:
            clean += 1
        else:
            mal = int(MALFORMED.search(out).group(1)) if MALFORMED.search(out) else -1
            rate = float(RATE.search(out).group(1)) if RATE.search(out) else 0.0
            offenders.append((skipped, mal, rate, f.name))
        if i % 25 == 0 or i == len(files):
            print(f"  [{i}/{len(files)}] clean={clean} with_skips={len(offenders)}")

    print(f"\nfiles checked      : {len(files)}")
    print(f"zero skipped bits  : {clean}")
    print(f"nonzero            : {len(offenders)}")
    print(f"total skipped bits : {total_skipped:,}")
    if offenders:
        print("\nfiles that still skip bits (descending):")
        for skipped, mal, rate, name in sorted(offenders, reverse=True):
            print(f"  {skipped:>7} bits  malformed={mal:<3} rate={rate:.6f}%  {name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
