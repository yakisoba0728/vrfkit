"""Analyze overlay coverage gaps: classify 'Not in table' fields into
'C# descriptor missing' vs 'extractor failure'.

Reads:
  - out/nested/manifest.json  (replay groups + fields)
  - crates/vrf-decode/src/table.rs  (current overlay entries)
  - C# descriptor directory (to list which groups have descriptors)

Outputs:
  - Groups in replay with no overlay entry, split by whether a C# descriptor
    exists for that group.
  - Top uncovered groups by field count.

Usage:
    python tools/analyze_coverage.py [--csharp-dir <path>]
"""
from __future__ import annotations

import json
import os
import re
import sys
from pathlib import Path
from collections import Counter

VRFKIT_ROOT = Path(__file__).parent.parent
MANIFEST_PATH = VRFKIT_ROOT / "out" / "nested" / "manifest.json"
TABLE_RS_PATH = VRFKIT_ROOT / "crates" / "vrf-decode" / "src" / "table.rs"

# Default C# descriptor location. Set VRFKIT_CSHARP_DIR to the
# ValorantReplayParser checkout root; defaults to empty so a machine without
# it degrades to "no C# descriptors" rather than crashing.
DEFAULT_CSHARP_DIR = Path(os.environ.get("VRFKIT_CSHARP_DIR", "")) / "src" / "Replay.Valorant"

PATH_RE = re.compile(r'override\s+string\s+Path\s*=>\s*"(?P<path>[^"]+)"')


def extract_csharp_paths(csharp_dir: Path) -> set[str]:
    """Extract all Path declarations from C# descriptor files."""
    paths = set()
    for cs_file in sorted(csharp_dir.rglob("*.cs")):
        source = cs_file.read_text(encoding="utf-8-sig")
        for m in PATH_RE.finditer(source):
            paths.add(m.group("path"))
    return paths


def extract_overlay_groups(table_rs: Path) -> set[str]:
    """Extract distinct group_path values from the overlay table .rs file."""
    groups = set()
    for line in table_rs.read_text(encoding="utf-8").splitlines():
        m = re.search(r'group_path:\s*"([^"]+)"', line)
        if m:
            groups.add(m.group(1))
    return groups


def main(argv: list[str]) -> int:
    csharp_dir = DEFAULT_CSHARP_DIR
    for i, arg in enumerate(argv[1:], 1):
        if arg == "--csharp-dir" and i + 1 < len(argv):
            csharp_dir = Path(argv[i + 1])

    if not MANIFEST_PATH.exists():
        print(f"ERROR: {MANIFEST_PATH} not found. Run export first.", file=sys.stderr)
        return 1

    manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    groups_in_replay = manifest["net_field_export_groups"]

    overlay_groups = extract_overlay_groups(TABLE_RS_PATH)
    if csharp_dir.is_dir():
        csharp_paths = extract_csharp_paths(csharp_dir)
    else:
        csharp_paths = set()
        print(f"NOTE: C# descriptor dir not found ({csharp_dir}); "
              f"set VRFKIT_CSHARP_DIR to classify extractor-missed groups. "
              f"All uncovered groups will read as 'no descriptor'.",
              file=sys.stderr)

    print(f"Replay groups: {len(groups_in_replay)}")
    print(f"Overlay groups: {len(overlay_groups)}")
    print(f"C# descriptor paths: {len(csharp_paths)}")
    print()

    # Classify each replay group
    covered = 0
    no_descriptor = 0
    extractor_missed = 0

    uncovered_no_desc: list[tuple[str, int]] = []
    uncovered_ext_miss: list[tuple[str, int]] = []

    for g in groups_in_replay:
        path = g["path"]
        field_count = len(g["fields"])
        if path in overlay_groups:
            covered += 1
        elif path in csharp_paths:
            extractor_missed += 1
            uncovered_ext_miss.append((path, field_count))
        else:
            no_descriptor += 1
            uncovered_no_desc.append((path, field_count))

    print(f"=== Replay group classification ===")
    print(f"  Covered by overlay:    {covered}")
    print(f"  C# descriptor exists but extractor missed: {extractor_missed}")
    print(f"  No C# descriptor (raw-only):               {no_descriptor}")
    print()

    # Count total fields in each category
    total_fields_in_replay = sum(len(g["fields"]) for g in groups_in_replay)
    covered_fields = sum(
        len(g["fields"]) for g in groups_in_replay if g["path"] in overlay_groups
    )
    ext_miss_fields = sum(fc for _, fc in uncovered_ext_miss)
    no_desc_fields = sum(fc for _, fc in uncovered_no_desc)

    print(f"=== Field-level breakdown ===")
    print(f"  Total declared fields in replay: {total_fields_in_replay}")
    print(f"  In overlay-covered groups:       {covered_fields}")
    print(f"  In extractor-missed groups:      {ext_miss_fields}")
    print(f"  In no-descriptor groups:         {no_desc_fields}")
    print()

    if uncovered_ext_miss:
        print(f"=== Extractor-missed groups (C# descriptor exists, overlay missing) ===")
        uncovered_ext_miss.sort(key=lambda x: -x[1])
        for path, fc in uncovered_ext_miss[:20]:
            print(f"  {fc:>4} fields  {path}")
        print()

    if uncovered_no_desc:
        print(f"=== Top no-descriptor groups (by field count) ===")
        uncovered_no_desc.sort(key=lambda x: -x[1])
        for path, fc in uncovered_no_desc[:20]:
            print(f"  {fc:>4} fields  {path}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
