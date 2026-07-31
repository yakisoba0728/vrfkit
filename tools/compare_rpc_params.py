"""Compare RPC parameter values between our Rust Parquet and the C# NDJSON export.

Validates multiset equality for key RPC functions:
  - MulticastNotifyKilledEnemy: KillerCharacter, KilledCharacter, MultikillLevel
  - MulticastNotifyDamage_Point: DamageDealt, DamageTaken, RegionalDamage, bDamageKilledTarget
  - MulticastEndRound: NewRoundNumber

The C# export (events.ndjson) has been slim-processed so only a subset of RPCs
survive. We compare only the surviving records (by multiset of values, not by
position/time).

Usage:
    python tools/compare_rpc_params.py [parquet_path]

Defaults to out/nested/fields.parquet for the Rust side and the fixed C# export
at valplay/pipeline/exports/02d4d478-.../events.ndjson.
"""

import collections
import json
import sys
from pathlib import Path

import pyarrow.parquet as pq

# Paths
CS_PATH = Path(
    r"C:\Users\yakihyuk0728\Documents\GitHub\valplay\pipeline\exports"
    r"\02d4d478-1dfb-4412-9a77-29ca29105a9d\events.ndjson"
)

PARQUET_PATH = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("out/nested/fields.parquet")

# RPC functions and the parameters to compare.
# For each function: list of (param_name, value_type) where value_type is how
# the C# JSON stores it ('int', 'float', 'bool', 'str').
# Special: 'enum_byte' means the C# stores as string but Rust stores as i64.
RPCS_TO_CHECK = {
    "MulticastNotifyKilledEnemy": [
        ("KillerCharacter", "int"),
        ("KilledCharacter", "int"),
        ("MultikillLevel", "int"),
    ],
    "MulticastNotifyDamage_Point": [
        ("DamageDealt", "float"),
        ("DamageTaken", "float"),
        ("RegionalDamage", "enum_byte"),
        ("bDamageKilledTarget", "bool"),
    ],
    "MulticastEndRound": [
        ("NewRoundNumber", "int"),
    ],
}

# Mapping from C# EAresRegionalDamage enum strings to byte values.
# Derived from the C# enum declaration and confirmed against wire data.
REGIONAL_DAMAGE_MAP = {
    "regional_damage__normal": 0,
    "regional_damage__headshot": 1,
    "regional_damage__legshot": 2,
    "regional_damage__utility": 3,
    "regional_damage__armor": 4,
    "regional_damage__invalid": 5,
}

# C# field name aliases: the C# export may use different names than the replay
# schema exports (e.g. "DamageKilledTarget" in JSON payload vs
# "bDamageKilledTarget" in the export group field name table).
CS_FIELD_ALIASES = {
    "MulticastNotifyDamage_Point": {
        "DamageKilledTarget": "bDamageKilledTarget",
    },
}


def norm(v, vtype):
    """Normalise a value for multiset comparison."""
    if v is None:
        return None
    if vtype == "int":
        return int(v)
    if vtype == "float":
        return round(float(v), 2)
    if vtype == "bool":
        return 1 if v else 0
    if vtype == "enum_byte":
        # If the value is a string (C# side), map to int via known enum table.
        if isinstance(v, str):
            return REGIONAL_DAMAGE_MAP.get(v, v)
        return int(v)
    return str(v)


def load_cs_rpc_values():
    """Load RPC parameter values from C# NDJSON export."""
    result = {}  # (function_name, param_name) -> Counter of values
    for func_name, params in RPCS_TO_CHECK.items():
        for pname, _ in params:
            result[(func_name, pname)] = collections.Counter()

    aliases = CS_FIELD_ALIASES

    with CS_PATH.open("r", encoding="utf-8") as f:
        for line in f:
            if b"rpc_received" if isinstance(line, bytes) else "rpc_received" not in line:
                continue
            rec = json.loads(line)
            if rec.get("type") != "rpc_received":
                continue
            func = rec.get("function_name", "")
            if func not in RPCS_TO_CHECK:
                continue
            payload = rec.get("payload", {})
            if not payload:
                continue
            func_aliases = aliases.get(func, {})
            for pname, vtype in RPCS_TO_CHECK[func]:
                # Try the canonical name first, then check aliases
                val = payload.get(pname)
                if val is None:
                    # Try reverse alias lookup
                    for cs_name, our_name in func_aliases.items():
                        if our_name == pname:
                            val = payload.get(cs_name)
                            break
                if val is not None:
                    result[(func, pname)][norm(val, vtype)] += 1

    return result


def load_rust_rpc_values():
    """Load RPC parameter values from Rust Parquet export."""
    result = {}  # (function_name, param_name) -> Counter of values
    for func_name, params in RPCS_TO_CHECK.items():
        for pname, _ in params:
            result[(func_name, pname)] = collections.Counter()

    t = pq.read_table(str(PARQUET_PATH))
    fn_col = t.column("field_name").to_pylist()
    vi_col = t.column("value_i64").to_pylist()
    vf_col = t.column("value_f64").to_pylist()
    vb_col = t.column("value_bool").to_pylist()
    vs_col = t.column("value_str").to_pylist()

    for i, field_name in enumerate(fn_col):
        if not field_name or "." not in field_name:
            continue
        dot = field_name.index(".")
        func = field_name[:dot]
        param = field_name[dot + 1:]
        if func not in RPCS_TO_CHECK:
            continue

        # Find the matching param entry
        for pname, vtype in RPCS_TO_CHECK[func]:
            if param != pname:
                continue
            # Get the value from the appropriate column
            if vtype in ("int", "enum_byte"):
                val = vi_col[i]
            elif vtype == "float":
                val = vf_col[i]
            elif vtype == "bool":
                val = vb_col[i]
            else:
                val = vs_col[i]
            if val is not None:
                result[(func, pname)][norm(val, vtype)] += 1
            break

    return result


def main():
    print(f"C# source: {CS_PATH}")
    print(f"Rust source: {PARQUET_PATH}")
    print()

    cs = load_cs_rpc_values()
    rust = load_rust_rpc_values()

    all_match = True
    print(f"{'Function':<35} {'Param':<25} {'C#':>5} {'Rust':>5}  Verdict")
    print("-" * 100)

    for func_name, params in RPCS_TO_CHECK.items():
        for pname, vtype in params:
            key = (func_name, pname)
            cs_vals = cs[key]
            rust_vals = rust[key]

            cs_total = sum(cs_vals.values())
            rust_total = sum(rust_vals.values())

            if cs_vals == rust_vals:
                verdict = "MATCH"
            elif not cs_vals and not rust_vals:
                verdict = "both empty"
            else:
                extra_rust = sum((rust_vals - cs_vals).values())
                extra_cs = sum((cs_vals - rust_vals).values())
                verdict = f"DIFFER (+{extra_rust} rust / +{extra_cs} C#)"
                all_match = False

            print(f"{func_name:<35} {pname:<25} {cs_total:>5} {rust_total:>5}  {verdict}")

    print()
    if all_match:
        print("ALL RPC PARAMETER VALUES MATCH")
    else:
        print("SOME VALUES DIFFER -- see above")

    # Print sample values for verification
    print()
    print("=== Sample values (first 5 per function) ===")
    for func_name, params in RPCS_TO_CHECK.items():
        pname0, vtype0 = params[0]
        key = (func_name, pname0)
        cs_sample = list(cs[key].most_common(5))
        rust_sample = list(rust[key].most_common(5))
        print(f"\n{func_name}.{pname0}:")
        print(f"  C#:   {cs_sample}")
        print(f"  Rust: {rust_sample}")

    return 0 if all_match else 1


if __name__ == "__main__":
    raise SystemExit(main())
