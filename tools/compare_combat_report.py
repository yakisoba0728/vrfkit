"""Compare CombatReport values as multisets per path shape.

A leaf-by-leaf path join under-counts: C# writes array positions into its own
`Index` field while we encode the wire index in the path brackets. Comparing
the multiset of values for each *shape* (indices collapsed) sidesteps that and
still catches a wrong decoder, whose error changes the values themselves.
Restricted to the leaves valplay derives metrics from.

Three of the ten leaves are spelled differently on the wire (DamageRecieved,
HitsRecieved, bDidKill), so our side is relabelled through the bundle
adapter's declaration-aware names (shared so the two cannot drift);
`INTERESTING` keeps the C# spelling.

The reference is the C# reference parser's `export` of the 13.01 reference
replay, kept machine-local because it carries per-player values; docs/USAGE.md
section 6 has the commands that produce it. compare_rpc_params.py shares
`parse_args`, `missing_input` and `verdict`.

Usage:
    python tools/compare_combat_report.py [--reference EVENTS] [--ours PARQUET]
"""

import argparse
import collections
import json
import os
import sys
from pathlib import Path

import pyarrow as pa
import pyarrow.compute as pc
from wire_bits import read_parquet

sys.path.insert(0, str(Path(__file__).resolve().parent))
from to_valplay_bundle import (  # noqa: E402
    _combat_report_declared_names, _combat_report_leaf_name,
)

#: The C# reference parser's export of replay 02d4d478.
REFERENCE_DIR = r"%LOCALAPPDATA%\vrfkit\csharp-reference\02d4d478-1dfb-4412-9a77-29ca29105a9d"
#: Its CombatReportComponent lines (the whole events.ndjson works too). The C#
#: build must decode Rounds; one that keeps it a raw payload has no values.
DEFAULT_REFERENCE = REFERENCE_DIR + r"\combat_report.ndjson"
#: vrfkit's export of the same replay.
DEFAULT_OURS = "out/nested/fields.parquet"

# The leaves that drive K/D/A, ADR, HS%, multikills and wallbangs.
INTERESTING = {
    "Rounds[].Reports[].Interactions[].DamageDealt",
    "Rounds[].Reports[].Interactions[].HitsDealt",
    "Rounds[].Reports[].Interactions[].DamageReceived",
    "Rounds[].Reports[].Interactions[].HitsReceived",
    "Rounds[].Reports[].Interactions[].DidKill",
    "Rounds[].Reports[].Interactions[].AssistType",
    "Rounds[].Reports[].Interactions[].DealtInteractions[].Regions[].Hits",
    "Rounds[].Reports[].Interactions[].DealtInteractions[].Regions[].Damage",
    "Rounds[].Reports[].Interactions[].ReceivedInteractions[].Regions[].Hits",
    "Rounds[].Reports[].Interactions[].ReceivedInteractions[].Regions[].Damage",
}


def flatten(prefix, node, out):
    if isinstance(node, list):
        for i, item in enumerate(node):
            flatten(f"{prefix}[{i}]", item, out)
    elif isinstance(node, dict):
        for k, v in node.items():
            flatten(f"{prefix}.{k}" if prefix else k, v, out)
    else:
        out[prefix] = node


def shape(path):
    out, i = [], 0
    while i < len(path):
        if path[i] == "[":
            out.append("[]")
            i = path.index("]", i) + 1
        else:
            out.append(path[i])
            i += 1
    return "".join(out)


#: Decimal places floats are rounded to before the multisets are compared:
#: `MATCH` means identical to this precision and no further.
FLOAT_PLACES = 3


def norm(v):
    """Normalise so 1/True and 35.0/35 compare equal across JSON and Parquet."""
    if isinstance(v, bool):
        return int(v)
    if isinstance(v, float):
        return round(v, FLOAT_PLACES)
    if isinstance(v, int):
        return v
    return str(v)


def load_cs(path):
    cs = collections.defaultdict(collections.Counter)
    with path.open("rb") as f:
        for line in f:
            if b"CombatReportComponent" not in line:
                continue
            o = json.loads(line)
            rounds = (o.get("payload") or {}).get("Rounds")
            if not rounds:
                continue
            leaves = {}
            flatten("Rounds", rounds, leaves)
            for p, v in leaves.items():
                s = shape(p)
                if s in INTERESTING and v is not None:
                    cs[s][norm(v)] += 1
    return cs


def load_ours(parquet=DEFAULT_OURS):
    manifest_path = Path(parquet).with_name("manifest.json")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8")) if manifest_path.is_file() else {}
    declared_names = _combat_report_declared_names(manifest)
    columns = ["group_path", "field_name", "handle",
               "value_i64", "value_f64", "value_bool", "value_str"]
    t = read_parquet(parquet, columns=columns)
    # Only the CombatReport Rounds rows become Python objects (of ~1.3M).
    keep = pc.and_(
        pc.match_substring(t.column("group_path").cast(pa.string()), "CombatReportComponent"),
        pc.starts_with(t.column("field_name").cast(pa.string()), "Rounds"))
    t = t.filter(pc.fill_null(keep, False))
    cols = {n: t.column(n).to_pylist() for n in columns}
    ours = collections.defaultdict(collections.Counter)
    for i, (g, n) in enumerate(zip(cols["group_path"], cols["field_name"])):
        s = shape(_combat_report_leaf_name(g, n, cols["handle"][i], declared_names))
        if s not in INTERESTING:
            continue
        for c in ("value_i64", "value_f64", "value_bool", "value_str"):
            v = cols[c][i]
            if v is not None:
                ours[s][norm(v)] += 1
                break
    return ours


def verdict(cs, ours):
    """`(text, matched, compared)` for one key's C# and vrfkit Counters.

    Emptiness goes first, because two empty Counters are equal: an empty pair
    is no disagreement, but it is not counted as compared.
    """
    if not cs and not ours:
        return "absent both sides", True, False
    if cs == ours:
        return "MATCH", True, True
    return (f"DIFFER (+{sum((ours - cs).values())} vrfkit / "
            f"+{sum((cs - ours).values())} C#)", False, True)


def compare(cs, ours, interesting):
    """`(printable rows, everything matched, how many shapes were compared)`."""
    rows, all_match, checked = [], True, 0
    for s in sorted(interesting):
        a, b = cs.get(s, collections.Counter()), ours.get(s, collections.Counter())
        text, matched, compared = verdict(a, b)
        all_match &= matched
        checked += compared
        label = s.replace("Rounds[].Reports[].Interactions[]", "..Interactions[]")
        rows.append(f"{label:<66} {sum(a.values()):>7,} {sum(b.values()):>7,}  {text}")
    return rows, all_match, checked


def parse_args(argv, doc, reference, lines):
    """`(reference path, parquet)` from --reference and --ours."""
    parser = argparse.ArgumentParser(description=doc.splitlines()[0])
    parser.add_argument("--reference", default=reference,
                        help=f"C# export events.ndjson, or its {lines} "
                             "(default: %(default)s)")
    parser.add_argument("--ours", default=DEFAULT_OURS,
                        help="vrfkit fields.parquet of the same replay "
                             "(default: %(default)s)")
    args = parser.parse_args(argv)
    return Path(os.path.expandvars(args.reference)), args.ours


def missing_input(reference, parquet) -> bool:
    """Report the first input that is not a file (None skips one)."""
    if reference is not None and not reference.is_file():
        print(f"C# reference not found at {reference}; produce it with the "
              f"commands in docs/USAGE.md section 6, or pass --reference",
              file=sys.stderr)
        return True
    if parquet is not None and not Path(parquet).is_file():
        print(f"vrfkit fields.parquet not found at {parquet}; export the "
              f"same replay, or pass --ours", file=sys.stderr)
        return True
    return False


def main(cs=None, ours=None, interesting=None, argv=None):
    """Exit 0 only if every interesting shape matches and every one was there;
    1 when the CombatReport decoder disagrees with the C# reference on values;
    2 when an input is missing or a shape carried nothing on either side
    (every shape is present in the reference replay).
    """
    if cs is None or ours is None:
        reference, parquet = parse_args(argv, __doc__, DEFAULT_REFERENCE,
                                        "CombatReport lines")
        if missing_input(reference if cs is None else None,
                         parquet if ours is None else None):
            return 2
        cs = load_cs(reference) if cs is None else cs
        ours = load_ours(parquet) if ours is None else ours

    shapes = interesting or INTERESTING
    rows, all_match, checked = compare(cs, ours, shapes)
    print(f"{'shape':<66} {'C#':>7} {'ours':>7}  verdict")
    print("-" * 96)
    print("\n".join(rows))
    print()
    if not all_match:
        print("SOME SHAPES DIFFER -- see above")
        return 1
    if checked < len(shapes):
        print(f"INCOMPLETE: {len(shapes) - checked} of the {len(shapes)} "
              f"interesting shapes carry no value on either side, so they were "
              f"not compared. This is not agreement -- check the parquet path "
              f"and the reference.")
        return 2
    print(f"ALL {checked} INTERESTING SHAPES MATCH "
          f"(values to {FLOAT_PLACES} decimal places)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(argv=sys.argv[1:]))
