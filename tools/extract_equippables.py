#!/usr/bin/env python3
"""Generate tools/equippable_table.py from the C# parser's resolver.

Weapon display names ("Vandal", "Sheriff") exist nowhere in the replay wire
format. The game ships them as client-side assets; the C# reference parser
carries a hand-maintained table in

    ValorantReplayParser/src/Replay.Valorant/Combat/ValorantEquippableResolver.cs

as a list of Define(classPath, name, category) entries. Reproducing
shot.equippable.name therefore requires a table, and this generator extracts it
from that authoritative source rather than having anyone retype 24 paths.

This does NOT weaken the parser's "no hardcoded names" invariant: the Rust
crates stay free of name tables and emit class_path only. The mapping lives on
the presentation side, in the Python adapter, where turning
"AssaultRifle_AK" into "Vandal" is a labelling concern rather than a parsing
rule. See docs/archive/PROJECT_STATUS.md section 8 and
docs/archive/NEXT_STEPS_FINDINGS.md.

Usage:
    python tools/extract_equippables.py [--csharp-root <path>] [--check]

--check exits non-zero if the generated file is stale, for CI use.
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path

if __package__:
    from .atomic_io import atomic_write_text
else:  # direct script execution
    from atomic_io import atomic_write_text

DEFAULT_CSHARP_ROOT = Path(os.environ.get("VRFKIT_CSHARP_DIR", ""))
RESOLVER_RELPATH = Path("src/Replay.Valorant/Combat/ValorantEquippableResolver.cs")
OUTPUT_PATH = Path(__file__).parent / "equippable_table.py"

# Define("<class path>", "<display name>", ValorantEquippableCategory.<Category>)
DEFINE_RE = re.compile(
    r'Define\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*,\s*'
    r"ValorantEquippableCategory\.(\w+)\s*\)"
)


def pascal_to_snake(name: str) -> str:
    """SniperRifle -> sniper_rifle, Smg -> smg.

    Matches the category strings the C# JSON writer emits, verified against
    02d4d478's reference bundle: machine_gun, sniper_rifle, sidearm, smg,
    rifle, shotgun, ability.
    """
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def parse_definitions(source: str) -> list[tuple[str, str, str]]:
    """Extract (class_path, display_name, category) triples in source order."""
    out = []
    for class_path, display_name, category in DEFINE_RE.findall(source):
        out.append((class_path, display_name, pascal_to_snake(category)))
    return out


def render(definitions: list[tuple[str, str, str]], source_rel: str) -> str:
    """Render the generated Python module."""
    lines = [
        '"""Equippable class path -> display name and category.',
        "",
        "GENERATED FILE -- DO NOT EDIT BY HAND.",
        f"Regenerate with: python tools/extract_equippables.py",
        f"Source: {source_rel}",
        "",
        "Keys cover the three path shapes that appear in replay data, mirroring",
        "the C# CreateDefinitions(): the full 'Package.Class_C' path, the package",
        "path alone, and the 'Default__Class_C' archetype form.",
        '"""',
        "",
        "# fmt: off",
        "",
        "EQUIPPABLE_DEFINITIONS = [",
    ]
    for class_path, name, category in definitions:
        lines.append(f"    ({class_path!r}, {name!r}, {category!r}),")
    lines += [
        "]",
        "",
        "",
        "def _build_lookup():",
        '    """class path (all three shapes) -> (name, category, canonical path)."""',
        "    out = {}",
        "    for class_path, name, category in EQUIPPABLE_DEFINITIONS:",
        "        value = (name, category, class_path)",
        "        out[class_path] = value",
        "        if '.' in class_path:",
        "            package, _, class_name = class_path.rpartition('.')",
        "            out[package] = value",
        "            out['Default__' + class_name] = value",
        "    return out",
        "",
        "",
        "EQUIPPABLE_BY_PATH = _build_lookup()",
        "",
        "# fmt: on",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--csharp-root", type=Path, default=DEFAULT_CSHARP_ROOT)
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit non-zero if the generated file is stale",
    )
    args = parser.parse_args()

    resolver = args.csharp_root / RESOLVER_RELPATH
    if not resolver.exists():
        print(f"resolver not found: {resolver}\n"
              f"set VRFKIT_CSHARP_DIR to the ValorantReplayParser checkout root, "
              f"or pass --csharp-root", file=sys.stderr)
        return 2

    definitions = parse_definitions(resolver.read_text(encoding="utf-8"))
    if not definitions:
        print(f"no Define(...) entries matched in {resolver}", file=sys.stderr)
        return 2

    rendered = render(definitions, RESOLVER_RELPATH.as_posix())

    if args.check:
        if not OUTPUT_PATH.exists():
            print(f"{OUTPUT_PATH} missing", file=sys.stderr)
            return 1
        if OUTPUT_PATH.read_text(encoding="utf-8") != rendered:
            print(f"{OUTPUT_PATH} is stale -- rerun the generator", file=sys.stderr)
            return 1
        print(f"{OUTPUT_PATH.name} up to date ({len(definitions)} definitions)")
        return 0

    atomic_write_text(OUTPUT_PATH, rendered)
    print(f"wrote {OUTPUT_PATH} ({len(definitions)} definitions)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
