"""Bounded candidate presence inventory, not a semantic decoder or type claim.

Only group/field/checksum names and aggregate widths are recorded. Player
identifiers, raw payloads and input filenames are excluded from the report.
"""

from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import sys

import pyarrow.compute as pc
import pyarrow.parquet as pq


CANDIDATES = (
    "FocusProjectiles", "ReplicatedMovement", "InputEventData", "StopEffectType",
    "OwnerActor", "AvatarActor", "ActiveGameplayEffects", "ActivatableAbilities",
    "AbilitiesAndBuffs", "Healing", "HealingDone", "HealingTaken",
)


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def inventory(export):
    findings = []
    for table_name in ("fields", "checkpoint_fields"):
        path = export / (table_name + ".parquet")
        grouped = defaultdict(lambda: {"rows": 0, "raw_present": 0, "typed_present": 0,
                                        "bit_widths": Counter()})
        columns = ["group_path", "field_name", "compatible_checksum", "bit_count", "raw_bits",
                   "value_i64", "value_f64", "value_bool", "value_str"]
        with pq.ParquetFile(path, pre_buffer=False) as file:
            for batch in file.iter_batches(batch_size=4096, columns=columns, use_threads=False):
                mask = pc.is_in(batch.column("field_name"), value_set=__import__("pyarrow").array(CANDIDATES))
                for row in batch.filter(mask).to_pylist():
                    key = (row["group_path"], row["field_name"], row["compatible_checksum"])
                    item = grouped[key]
                    item["rows"] += 1
                    item["raw_present"] += row["raw_bits"] is not None
                    item["typed_present"] += any(row[name] is not None for name in columns[-4:])
                    item["bit_widths"][row["bit_count"]] += 1
        findings.append({
            "table": table_name, "table_sha256": sha(path),
            "candidates_absent": sorted(set(CANDIDATES) - {key[1] for key in grouped}),
            "observed": [dict(group=key[0], field=key[1], checksum=key[2], **{
                **value, "bit_widths": sorted(value["bit_widths"].items()),
            }) for key, value in sorted(grouped.items(), key=lambda pair: repr(pair[0]))],
        })
    return findings


if __name__ == "__main__":
    root = Path(sys.argv[1])
    result = {"scope": "Observed candidate field names only; no inferred meaning or width acceptance",
              "reports": []}
    for export in sorted(root.glob("*/export")):
        manifest = json.loads((export / "manifest.json").read_text())
        result["reports"].append({"input_sha256": export.parent.name,
            "build": manifest["replay_build"], "tables": inventory(export)})
    print(json.dumps(result, indent=2))
