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

What the ratio verdicts DO NOT cover
------------------------------------

A game build renaming a component. This tool's failure text used to claim that
as the likely cause of a `broken` verdict, and a rename cannot produce one: the
replay stops declaring the old leaf altogether, so `bare_rows` is 0, the ratio
is 0, and the pair reads `ok` for as long as any sibling keeps the native group
busy -- nine leaves map to `EquippableStateMachineComponent` -- or `absent`
when none does. `broken` means something else: the rows are still arriving
under the leaf and are not reaching the native group.

The renamed component does not leave the export, though. It arrives bare under
its NEW name, which no pair claims. `unmapped_bare_groups` lists exactly those,
worst first, and that list is where a rename is visible. It is reported rather
than failed on: a replay legitimately carries bare Blueprint components that
have no native remap at all, so their presence is not by itself a fault. Read
the list against the previous build's.

Two ways this run can tell you nothing, and they are not the same:
`SKIP` means there was no export to read. `FAILED: nothing checked` means the
export was read and not one pair in the table appears in it -- which is a fault,
because a real match export exercises many of them.

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


def unmapped_bare_groups(rows_by_group, pairs, min_rows: int = 1) -> list:
    """`(group, rows)` for bare groups no pair claims, worst first.

    The rename signal. A build that renames a component makes its old leaf
    vanish from the replay -- which no ratio can see, because a vanished leaf
    holds zero rows -- and introduces a new bare group under the new name that
    the remap table does not know about.

    Native `/Script/...` paths are excluded: they are the targets, not
    candidates for renaming out from under the table. Groups at zero are
    excluded too, which is what keeps the RepLayout-only remaps out of the
    list: `bare_counts` has already dropped their ClassNetCache rows, so they
    arrive here at 0.
    """
    leaves = {leaf for leaf, _ in pairs}
    return sorted(
        ((group, rows) for group, rows in rows_by_group.items()
         if not group.startswith("/") and group not in leaves and rows >= min_rows),
        key=lambda kv: (-kv[1], kv[0]),
    )


def exit_code(verdicts_) -> int:
    return 1 if any(v.state == "broken" for v in verdicts_) else 0


def nothing_checked(verdicts_) -> bool:
    """True when no pair appeared in this export, so the run verified nothing.

    Distinct from the `SKIP` path, which is "there was no export to read".
    """
    return all(v.state == "absent" for v in verdicts_)


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

    rows = row_counts(args.export)
    results = verdicts(pairs, rows)
    tally = collections.Counter(v.state for v in results)

    for v in results:
        if v.state == "broken" or args.verbose:
            print(f"  {v.state:<7} {v.leaf} -> {v.native.split('.')[-1]}: {v.detail}")

    print(f"{len(pairs)} remaps: {tally['ok']} ok, {tally['absent']} absent, "
          f"{tally['broken']} broken")

    # Where a RENAME shows up. No verdict above can see one -- see the module
    # docstring -- so the list is printed on every run, pass or fail.
    suspects = unmapped_bare_groups(rows, pairs)
    print(f"\n{len(suspects)} bare group(s) no remap claims"
          + (" (a renamed component appears here, not above):" if suspects else ""))
    for group, n in suspects[:10]:
        print(f"  {n:>8,}  {group}")
    if len(suspects) > 10:
        print(f"  ... and {len(suspects) - 10} more")

    if tally["broken"]:
        print("\nFAILED: a remap stopped matching -- its rows are still "
              "arriving under the bare leaf instead of reaching the native "
              "group. Check that the pair still spells the leaf the way this "
              "replay declares it; if the component was RENAMED the leaf would "
              "be gone from the export entirely and would show up in the "
              "unclaimed list above, not here. Re-derive the pair from the "
              "cooked asset (docs/DATA.md) rather than guessing a new name.",
              file=sys.stderr)
        return 1
    if nothing_checked(results):
        print(f"\nFAILED: nothing checked -- not one of the {len(pairs)} remap "
              f"pairs appears in this export. A real match exercises many of "
              f"them, so this is an empty or wrong export rather than a clean "
              f"result.", file=sys.stderr)
        return 1
    print(f"\nOK: {tally['ok']} remap(s) are doing work; {tally['absent']} do "
          f"not appear in this replay, which says nothing about them. A "
          f"renamed component is not covered by this verdict -- read the "
          f"unclaimed bare groups above against the previous build.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
