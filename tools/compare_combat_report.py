"""Compare CombatReport values as multisets per path shape.

A leaf-by-leaf path join under-counts: C# writes array positions into its own
`Index` field while we encode the wire index in the path brackets, so two records
carrying identical data can spell their paths differently. Comparing the multiset
of values for each *shape* (indices collapsed) sidesteps that and still catches a
wrong decoder -- a bit-level error would change the values themselves, not just
their addresses.

Restricted to the fields valplay actually derives metrics from.

The two sides no longer spell these leaves the same way. `fields.parquet` now
labels each array leaf with the name the REPLAY declares for its handle, which
for six of the ten shapes below is not what the C# reference calls it: the wire
says `DamageRecieved` and `HitsRecieved` (Riot's typos), `bDidKill`,
`bIsWallPen`, and `ParticipantSubject`. So our side is relabelled through the
same handle -> reference-name table the bundle adapter uses before the shapes
are compared. Sharing that table is deliberate -- a second copy could drift, and
this comparison is what would then quietly stop testing anything.

`INTERESTING` stays in the C# spelling because the C# side of this comparison is
read straight from the reference's own `events.ndjson`.
"""

import collections
import json
import os
import sys
from pathlib import Path

import pyarrow.parquet as pq

sys.path.insert(0, str(Path(__file__).resolve().parent))
from to_valplay_bundle import _combat_report_leaf_name  # noqa: E402

CS = Path(
    os.environ.get("VRFKIT_VALPLAY_DIR", "")
) / "pipeline" / "exports" / "02d4d478-1dfb-4412-9a77-29ca29105a9d" / "events.ndjson"

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


#: Decimal places floats are rounded to before the multisets are compared.
#: `IDENTICAL multiset` therefore means identical to this precision and no
#: further -- a real tolerance, which the verdict line now states rather than
#: leaving the reader to find it here.
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


def load_ours(parquet="out/nested/fields.parquet"):
    t = pq.read_table(parquet)
    cols = {
        n: t.column(n).to_pylist()
        for n in ("group_path", "field_name", "handle",
                  "value_i64", "value_f64", "value_bool", "value_str")
    }
    ours = collections.defaultdict(collections.Counter)
    for i, g in enumerate(cols["group_path"]):
        if "CombatReportComponent" not in g:
            continue
        n = cols["field_name"][i]
        if not n or not n.startswith("Rounds"):
            continue
        s = shape(_combat_report_leaf_name(g, n, cols["handle"][i]))
        if s not in INTERESTING:
            continue
        for c in ("value_i64", "value_f64", "value_bool", "value_str"):
            v = cols[c][i]
            if v is not None:
                ours[s][norm(v)] += 1
                break
    return ours


def compare(cs, ours, interesting):
    """`(printable rows, everything matched)`.

    Split out from the printing so the verdict can be asserted on. It could
    not be before: the whole comparison ran at import, which is why the script
    had no way to report failure to anything but a reader.
    """
    rows, all_match = [], True
    for s in sorted(interesting):
        a, b = cs.get(s, collections.Counter()), ours.get(s, collections.Counter())
        # Emptiness is tested FIRST. Two empty counters satisfy `a == b`, so
        # this arm sat below the equality test and could never be reached --
        # every shape of a replay carrying none of them read `IDENTICAL
        # multiset`. `all_match` is deliberately left alone: per shape that is
        # still not a disagreement. What it is not is a comparison, and that is
        # what `compared_shapes` answers.
        if not a and not b:
            verdict = "absent both sides"
        elif a == b:
            verdict = "IDENTICAL multiset"
        else:
            extra_ours = sum((b - a).values())
            extra_cs = sum((a - b).values())
            verdict = f"DIFFER (+{extra_ours} ours / +{extra_cs} C#)"
            all_match = False
        label = s.replace("Rounds[].Reports[].Interactions[]", "..Interactions[]")
        rows.append(f"{label:<66} {sum(a.values()):>7,} {sum(b.values()):>7,}  "
                    f"{verdict}")
    return rows, all_match


def compared_shapes(cs, ours, interesting) -> int:
    """How many of the interesting shapes actually had something to compare.

    `compare` reports every shape absent from both sides as a match, which per
    shape is true and useless. Without this the whole run could compare nothing
    -- a wrong parquet path, the wrong reference bundle, a CombatReport decoder
    that stopped emitting -- and still print `ALL INTERESTING SHAPES MATCH`.
    """
    return sum(1 for s in interesting
               if cs.get(s, collections.Counter()) or ours.get(s, collections.Counter()))


def main(cs=None, ours=None, interesting=None):
    """Exit 0 only if every interesting shape matches, and some shape existed.

    A mismatch here means the CombatReport decoder disagrees with the C#
    reference on values, not just on how they are addressed -- the one thing
    this comparison exists to catch. Returning 0 regardless made it a report.
    A run that compared nothing exits 2: it is neither agreement nor
    disagreement, and reporting it as agreement is what this guards against.
    """
    if cs is None:
        if not CS.is_file():
            print(f"set VRFKIT_VALPLAY_DIR to the valplay checkout root; "
                  f"events.ndjson not found at {CS}", file=sys.stderr)
            return 2
        cs = load_cs(CS)
    if ours is None:
        ours = load_ours()

    shapes = interesting or INTERESTING
    rows, all_match = compare(cs, ours, shapes)
    checked = compared_shapes(cs, ours, shapes)
    print(f"{'shape':<66} {'C#':>7} {'ours':>7}  verdict")
    print("-" * 96)
    for row in rows:
        print(row)
    print()
    if not checked:
        print(f"NOTHING COMPARED: none of the {len(shapes)} interesting shapes "
              f"carries a value on either side. This is not agreement -- check "
              f"the parquet path and the reference bundle.")
        return 2
    print(f"ALL {checked} INTERESTING SHAPES PRESENT MATCH "
          f"(values to {FLOAT_PLACES} decimal places)" if all_match
          else "SOME SHAPES DIFFER -- see above")
    return 0 if all_match else 1


if __name__ == "__main__":
    raise SystemExit(main())
