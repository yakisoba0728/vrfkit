#!/usr/bin/env python3
"""Check that the Blueprint-component remaps still match a build's replays.

`KNOWN_SUBOBJECT_CLASS_PATHS` in `crates/vrfkit/src/sink/paths.rs` maps bare
Blueprint component names to the native groups a replay declares. Those pairs
were read out of a shipped game, and a later build can rename a component
without anything here noticing: the replay never named it either, so no test
can fail on its own.

The export baseline does pin `overlay_no_field_name` and would catch it -- on
the one replay that has a baseline. This works on any export, which is the point:
run it against a replay from a new build before trusting the output.

For each pair it compares how many rows are still bare under the leaf against
how many reached the native group.

    bare well under the target -> ok       (the remap is doing work)
    bare a large share of it   -> broken   (it stopped matching)
    neither present            -> absent   (not in this replay; says nothing)

Asking only "does the target have rows" is not enough, and that is the whole
reason for the ratio: nine leaves map to `EquippableStateMachineComponent`, so
one of them being renamed leaves the other eight keeping the target busy while
that component's blocks are all bare again.

A leaf lingering beside a working target is normal -- some blocks resolve by
another route. On the reference replay `ZoomStateMachine` drops from 8,112 rows
to 70, not to zero: 0.12% of the target. Simulating a rename puts it at 15.58%.
The threshold sits between those, far from both.

Usage:
    python tools/check_component_remaps.py --export out/probe
"""

from __future__ import annotations

import argparse
import collections
import re
import sys
from pathlib import Path
from typing import NamedTuple

REPO = Path(__file__).resolve().parents[1]
PATHS_RS = REPO / "crates" / "vrfkit" / "src" / "sink" / "paths.rs"

#: One `(leaf, native, GroupKind)` tuple of the remap table, rustfmt'd.
PAIR_RE = re.compile(
    r'\(\s*"([^"]+)",\s*"(/Script/[^"]+)",\s*GroupKind::\w+\s*,?\s*\)', re.S
)

#: Share of the native group's rows the bare leaf may still hold. Healthy is
#: ~0.1%; a renamed component measured 15.6%. Nothing observed lands between.
BARE_SHARE_LIMIT = 0.05

#: Field names that mark a ClassNetCache block rather than a RepLayout property.
#: Two of the remaps are RepLayout-only by design, so their RPC stream stays bare
#: and must not be read as the remap failing.
CNC_MARKERS = ("_cnc_h", "__vrfkit_unresolved_class_net_cache_payload__")


class Verdict(NamedTuple):
    leaf: str
    native: str
    state: str
    detail: str


def remap_pairs() -> list[tuple[str, str]]:
    """The `(leaf, native path)` pairs, read from the Rust table itself."""
    src = PATHS_RS.read_text(encoding="utf-8")
    start = src.index("const KNOWN_SUBOBJECT_CLASS_PATHS")
    end = src.index("\n];", start)
    return PAIR_RE.findall(src[start:end])


def verdicts(pairs, rows_by_group) -> list[Verdict]:
    """Classify each pair against a `{group_path: row count}` map."""
    out = []
    for leaf, native in pairs:
        native_rows = rows_by_group.get(native, 0)
        bare_rows = rows_by_group.get(leaf, 0)
        if not native_rows and not bare_rows:
            out.append(Verdict(leaf, native, "absent", "not in this replay"))
            continue
        share = bare_rows / native_rows if native_rows else float("inf")
        if share > BARE_SHARE_LIMIT:
            detail = (f"{bare_rows} rows still bare against {native_rows} on the "
                      f"native group")
            out.append(Verdict(leaf, native, "broken", detail))
        else:
            out.append(Verdict(
                leaf, native, "ok",
                f"{native_rows} rows, {bare_rows} still bare ({share:.1%})"))
    return out


def bare_counts(fields_by_group) -> dict:
    """Bare rows per group, counting RepLayout blocks only.

    `fields_by_group` maps a group path to a `Counter` of its field names.
    ClassNetCache rows are dropped because the RepLayout-only remaps leave those
    unresolved deliberately -- see `CNC_MARKERS`.
    """
    return {
        group: sum(n for name, n in names.items()
                   if not any(m in (name or "") for m in CNC_MARKERS))
        for group, names in fields_by_group.items()
    }


def exit_code(verdicts_) -> int:
    return 1 if any(v.state == "broken" for v in verdicts_) else 0


def row_counts(export_dir: Path) -> dict:
    """`{group_path: rows}`, with bare groups counting RepLayout blocks only."""
    import pyarrow.parquet as pq

    table = pq.read_table(export_dir / "fields.parquet",
                          columns=["group_path", "field_name"])
    groups = table.column("group_path").to_pylist()
    names = table.column("field_name").to_pylist()
    totals = collections.Counter(groups)
    by_group = collections.defaultdict(collections.Counter)
    for group, name in zip(groups, names):
        if not group.startswith("/"):
            by_group[group][name] += 1
    out = dict(totals)
    out.update(bare_counts(by_group))
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--export", type=Path, required=True,
                    help="directory written by `vrfkit export`")
    ap.add_argument("--verbose", action="store_true",
                    help="list every pair, not just the problems")
    args = ap.parse_args()

    fields = args.export / "fields.parquet"
    if not fields.is_file():
        print(f"SKIP: no fields.parquet in {args.export}", file=sys.stderr)
        return 0

    pairs = remap_pairs()
    if not pairs:
        print(f"FAILED: parsed no pairs out of {PATHS_RS.name}", file=sys.stderr)
        return 1

    results = verdicts(pairs, row_counts(args.export))
    tally = collections.Counter(v.state for v in results)

    for v in results:
        if v.state == "broken" or args.verbose:
            print(f"  {v.state:<7} {v.leaf} -> {v.native.split('.')[-1]}: {v.detail}")

    print(f"{len(pairs)} remaps: {tally['ok']} ok, {tally['absent']} absent, "
          f"{tally['broken']} broken")
    if tally["broken"]:
        print("\nFAILED: a remap stopped matching. The likely cause is a game "
              "build renaming the component -- re-derive the pair from the "
              "cooked asset (docs/DATA.md) rather than guessing a new name.",
              file=sys.stderr)
        return 1
    print("\nOK: every remap that appears in this replay is doing work")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
