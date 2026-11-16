"""What the guard tools share: the export summary's counter lines as one table,
and the helpers that run vrfkit and resolve a baseline's input.

Each row is a line `crates/vrfkit/src/driver/summary.rs` prints, whitespace
collapsed and `report::` formatters expanded, with one key per placeholder.
Every regex is anchored on the whole line: free-text lines (`Struct blob err:`,
`Movement err:`) can quote a label, and `RPCs:` is a suffix of `Truncated
RPCs:`. A line that is not printed reads as None, never 0.

Standard library only: check_decode_errors_corpus.py runs without pyarrow.
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import NamedTuple

from atomic_io import atomic_write_text

ROUTE_CHILDREN = ("Route children: {} player info / {} rewards / {} selected / {} kills / "
                  "{} active effects / {} ignore actors / {} blinds / {} projectile path")
ROUTES = tuple(f"route_children_{name}" for name in (
    "player_information", "tracked_rewards", "selected_v2", "kill_data",
    "server_active_effects", "requested_ignore_actors", "active_blinds", "projectile_path"))

#: Who reads a line. P: check_export_baseline.py pins every key (the committed
#: `*_02d4d478.json` hold exactly these). G: check_decode_errors_corpus.py
#: gates on it. M: fresh movement-boundary measurements, validated and printed
#: without inventing values in historical baselines. `cp_` keys are the
#: `=== Checkpoints ===` block, printed only
#: under `--checkpoints`, so a default run never reads them.
LINES = (
    ("Chunks: {}", "chunks", "P"),
    ("Unknown chunks: {}", "unknown_chunks", "P"),
    ("Frame skips: {} external blobs / {} external bytes / {} game-specific bytes",
     "frame_external_data_blobs frame_external_data_bytes frame_game_specific_bytes", "P"),
    ("Frame times: {} non-finite", "frame_non_finite_times", "P"),
    ("Packets: {}", "packets", "P"),
    ("Export groups: {}", "export_groups", "P"),
    ("Content blocks: {}", "content_blocks", "P"),
    ("RepLayout blocks: {}", "rep_layout_blocks", "P"),
    ("ClassNetCache: {}", "class_net_cache_blocks", "P"),
    ("Fields: {}", "fields", "P"),
    ("RPCs: {}", "rpcs", "P"),
    ("Actor opens: {}", "actor_opens", "P"),
    ("Actor closes: {}", "actor_closes", "P"),
    ("Partial raw rows: {} ({} bits)", "partial_rows partial_bits", "P"),
    # `fields` is the fields.parquet row count (check_export_baseline.py
    # cross-checks it); the rest are the sink's twins of RPCs, opens, closes
    # and content blocks.
    ("Sink tally: {} fields / {} RPCs / {} opens / {} closes / {} content blocks",
     "fields_emitted rpcs_emitted sink_actor_opens sink_actor_closes sink_content_blocks", "P"),
    ("Bunches: {}", "bunches", "P"),
    ("Malformed pkts: {}", "malformed_packets", "P"),
    ("Skipped bits: {}", "skipped_bits", "P"),
    ("Movement rows: {}", "movement_rows", "PG"),
    ("NetGUID rows: {}", "net_guid_rows", "P"),
    ("Event rows: {}", "event_rows", "P"),
    # `decoded` keeps a stopped decoder (0 decoded, 0 failed) from reading clean.
    ("Struct blobs: {} decoded / {} failed", "struct_blobs_decoded struct_blobs_failed", "PG"),
    ("Movement errors: {}", "movement_rpc_errors", "G"),
    # A measured tally in sized and open windows, not a loss verdict.
    ("Movement tails: {} sized ({} bits) / {} open ({} bits)",
     "movement_sized_section_tails movement_sized_section_tail_bits "
     "movement_open_section_tails movement_open_section_tail_bits", "PG"),
    ("Movement section trailers: {} streams / {} bits",
     "movement_sized_section_trailers movement_sized_section_trailer_bits", "GM"),
    # Bits after each byte-wrapped movement envelope, which nothing reads:
    # verify_build_corpus.py fails a replay whose bits are not 24 per stream.
    ("Envelope trailers: {} streams / {} bits",
     "movement_envelope_trailers movement_envelope_trailer_bits", "PG"),
    ("Array decode: {} elements / {} fields / {} errors / {} truncations",
     "array_elements_decoded array_fields_emitted array_errors array_truncations", "G"),
    ("Array residual: {} root bits / {} nested bits / {} implicit ends",
     "array_unconsumed_root_bits array_unconsumed_nested_bits array_implicit_terminations", "G"),
    ("Array leaf errs: {}", "array_leaf_decode_errors", "G"),
    ("Target locations: {} array children", "targeting_world_locations_decoded", "P"),
    # One count per measured array route: a route that stops typing shows here
    # while the others keep the array totals up.
    (ROUTE_CHILDREN, " ".join(ROUTES), "PG"),
    # A measured opaque shape, legitimately nonzero; not a decode-error count.
    ("Reward opaque: {} empty variants", "tracked_rewards_opaque_empty_variants", "PG"),
    # Empty deltas whose trailing zero byte was spared: a measured tolerance.
    ("ActiveBlinds trailers: {} empty deltas", "active_blinds_empty_trailers", "PG"),
    ("Truncated RPCs: {}", "truncated_rpcs", "G"),
    ("RPC param walks: {}", "rpc_param_walks", "PG"),
    ("Event layout err: {}", "event_layout_mismatches", "P"),
    ("Event payloads: {} decoded / {} unknown groups",
     "event_payloads_decoded event_payload_unknown_groups", "P"),
    ("CNC RPC rows: {}", "cnc_rpcs_emitted", "P"),
    ("CNC brute force: {} attempted / {} unwalked",
     "cnc_bruteforce_payloads_attempted cnc_bruteforce_payloads_unwalked", "PG"),
    ("RepLayout tails: {} decoded / {} preserved",
     "rep_layout_cnc_tails_decoded rep_layout_cnc_tails_preserved", "P"),
    ("Checkpoints: {}", "cp_chunks", "P"),
    ("Checkpoint partial raw: {} rows / {} bits", "cp_partial_rows cp_partial_bits", "P"),
    ("Trailing bytes: {}", "cp_trailing_bytes", "P"),
    ("GUID entries: {}", "cp_guid_entries", "P"),
    ("GUID paths: {} literals / {} indices / {} resolved",
     "cp_literal_paths cp_indexed_paths cp_resolved_path_indices", "P"),
    ("Group records: {}", "cp_group_records", "P"),
    ("Exported fields: {}", "cp_exported_fields", "P"),
    ("Frames: {}", "cp_frames", "P"),
    ("Frame packets: {}", "cp_frame_packets", "P"),
    ("Checkpoint frame skips: {} external blobs / {} external bytes / {} game-specific bytes",
     "cp_frame_external_data_blobs cp_frame_external_data_bytes cp_frame_game_specific_bytes",
     "P"),
    ("Checkpoint frame times: {} non-finite", "cp_frame_non_finite_times", "P"),
    ("Checkpoint rows: {}", "cp_field_rows", "P"),
    ("Checkpoint actors:{} rows", "cp_actor_rows_written", "P"),
    ("Checkpoint GUID rows: {}", "cp_net_guid_rows_written", "P"),
    ("Checkpoint blocks:{} rows", "cp_block_rows_written", "P"),
    ("Checkpoint GUID entries: {} rows", "cp_guid_entry_rows_written", "P"),
    ("Checkpoint export groups: {} rows", "cp_export_group_rows_written", "P"),
    ("Checkpoint export fields: {} rows", "cp_export_field_rows_written", "P"),
    ("Overlay: {} decoded / {} errors / {} raw-skip / {} not-in-table / {} unnamed / "
     "{} conflicts / {} effect blobs",
     "cp_overlay_decoded_ok cp_overlay_decode_errors cp_overlay_raw_skip "
     "cp_overlay_not_in_table cp_overlay_no_field_name cp_overlay_handle_conflicts_refused "
     "cp_effect_blobs_decoded", "G"),
    ("Checkpoint blobs: {} decoded / {} failed", "cp_struct_blobs_decoded cp_struct_blobs_failed",
     "PG"),
    ("Checkpoint sink: {} fields / {} RPCs / {} opens / {} closes / {} content blocks",
     "cp_fields_emitted cp_rpcs_emitted cp_sink_actor_opens cp_sink_actor_closes "
     "cp_sink_content_blocks", "P"),
    ("Checkpoint fails: {} array / {} truncated RPC / {} movement",
     "cp_array_errors cp_truncated_rpcs cp_movement_rpc_errors", "G"),
    ("Checkpoint array: {} elements / {} fields / {} truncations / {} root bits / "
     "{} nested bits / {} implicit ends",
     "cp_array_elements_decoded cp_array_fields_emitted cp_array_truncations "
     "cp_array_unconsumed_root_bits cp_array_unconsumed_nested_bits "
     "cp_array_implicit_terminations", "G"),
    ("Checkpoint leaf: {} typed decode errors", "cp_array_leaf_decode_errors", "G"),
    ("Checkpoint targets: {} array children", "cp_targeting_world_locations_decoded", "P"),
    ("Checkpoint r" + ROUTE_CHILDREN[1:],
     " ".join("cp_" + key for key in ROUTES), "PG"),
    ("Checkpoint RPC walks: {}", "cp_rpc_param_walks", "PG"),
    ("Checkpoint reward opaque: {} empty variants",
     "cp_tracked_rewards_opaque_empty_variants", "PG"),
    ("Checkpoint ActiveBlinds trailers: {} empty deltas", "cp_active_blinds_empty_trailers",
     "PG"),
    ("Checkpoint movement tails: {} sized ({} bits) / {} open ({} bits)",
     "cp_movement_sized_section_tails cp_movement_sized_section_tail_bits "
     "cp_movement_open_section_tails cp_movement_open_section_tail_bits", "PG"),
    ("Checkpoint movement section trailers: {} streams / {} bits",
     "cp_movement_sized_section_trailers cp_movement_sized_section_trailer_bits", "GM"),
    ("Checkpoint envelope trailers: {} streams / {} bits",
     "cp_movement_envelope_trailers cp_movement_envelope_trailer_bits", "PG"),
    ("Checkpoint CNC: {} RPC rows", "cp_cnc_rpcs_emitted", "P"),
    ("Checkpoint CNC brute force: {} attempted / {} unwalked",
     "cp_cnc_bruteforce_payloads_attempted cp_cnc_bruteforce_payloads_unwalked", "PG"),
    ("Checkpoint tails: {} decoded / {} preserved",
     "cp_rep_layout_cnc_tails_decoded cp_rep_layout_cnc_tails_preserved", "P"),
    # `=== Type overlay ===`: Rows offered = the five buckets above it.
    ("Decoded OK: {}", "overlay_decoded_ok", "PG"),
    ("Decode errors: {}", "overlay_decode_errors", "PG"),
    ("Raw/Skip: {}", "overlay_raw_skip", "PG"),
    ("Not in table: {}", "overlay_not_in_table", "PG"),
    ("No field name: {}", "overlay_no_field_name", "PG"),
    ("Rows offered: {total}", "overlay_rows_offered", "PG"),
    # Outside the ratio: the buckets are decided before the effect pass runs.
    ("Effect blobs: {effect_blobs_decoded}", "effect_blobs_decoded", "P"),
)

PLACEHOLDER = re.compile(r"\{\w*\}")


class Line(NamedTuple):
    fmt: str
    keys: tuple[str, ...]
    readers: str
    pattern: re.Pattern[str]


def _pattern(fmt: str) -> re.Pattern[str]:
    body = r"(\d+)".join(re.escape(part).replace(r"\ ", r"\s+")
                         for part in PLACEHOLDER.split(fmt))
    return re.compile(rf"(?m)^\s*{body}\s*$")


SPEC = tuple(Line(fmt, tuple(keys.split()), readers, _pattern(fmt))
             for fmt, keys, readers in LINES)
#: key -> (its line, its placeholder index).
WHERE = {key: (line, i) for line in SPEC for i, key in enumerate(line.keys)}
assert len(WHERE) == sum(len(line.keys) for line in SPEC), "a key is on two lines"


def lines(reader: str, checkpoint: bool) -> tuple[Line, ...]:
    """The lines `reader` reads in one block."""
    return tuple(line for line in SPEC
                 if reader in line.readers and line.keys[0].startswith("cp_") == checkpoint)


def keys(reader: str, checkpoint: bool) -> tuple[str, ...]:
    return tuple(key for line in lines(reader, checkpoint) for key in line.keys)


def read(text: str, wanted) -> dict[str, int | None]:
    """`{key: int}` for each of `wanted`, None where its line was not printed."""
    found: dict[Line, re.Match[str] | None] = {}
    out = {}
    for key in wanted:
        line, index = WHERE[key]
        if line not in found:
            found[line] = line.pattern.search(text)
        match = found[line]
        out[key] = None if match is None else int(match.group(index + 1))
    return out


def label(key: str) -> str:
    """How a message names `key`: its line's label and the words after its
    number ("Array decode errors"), prefixed "Checkpoint" in that block."""
    line, index = WHERE[key]
    parts = PLACEHOLDER.split(line.fmt)
    unit = re.split(r"[/()]", parts[index + 1])[0].strip()
    name = parts[0].strip().rstrip(":") + (f" {unit}" if unit else "")
    return f"Checkpoint {name}" if key.startswith("cp_") and not name.startswith("Checkpoint") \
        else name


def render(line: Line, values: dict[str, int]) -> str:
    """`line` as summary.rs prints it, with `values` and thousands separators."""
    parts = PLACEHOLDER.split(line.fmt)
    return parts[0] + "".join(f"{values[key]:,}{part}" for key, part in zip(line.keys, parts[1:]))


def tail(text: str, lines: int = 3, width: int = 200) -> str:
    """The last non-blank lines of a process's output, for an error message."""
    return " | ".join(l for l in text.splitlines()[-lines:] if l.strip())[:width]


def run(cmd, timeout: float | None = None) -> tuple[int, str]:
    """`(exit code, stdout + stderr)`, decoded as UTF-8 with replacement (the
    CLI prints glyphs a console codepage cannot decode). A timeout or a
    process that cannot start raises, for the caller to report."""
    r = subprocess.run([str(part) for part in cmd], capture_output=True, text=True,
                       encoding="utf-8", errors="replace", timeout=timeout)
    return r.returncode, (r.stdout or "") + (r.stderr or "")


def vrfkit(exe: Path, verb: str, replay: Path, out: Path | None = None,
           checkpoints: bool = False, timeout: float | None = None) -> tuple[int, str]:
    """`vrfkit <verb> <replay> [--out <out>] [--checkpoints]` through `run`."""
    return run([exe, verb, replay, *(("--out", out) if out else ()),
                *(("--checkpoints",) if checkpoints else ())], timeout)


def no_exe(exe: Path) -> bool:
    """Whether the vrfkit binary is missing (a usage error, exit 2), said so."""
    if exe.is_file():
        return False
    print(f"build the release binary first: {exe}", file=sys.stderr)
    return True


def load_baseline(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}


def baseline_input(given: Path | None, stored: str) -> Path | None:
    """The input a baseline names: `given`, else `stored` with environment
    variables expanded, a relative path joined to VRFKIT_CORPUS_DIR. None when
    neither names one -- decided on the text, since Path("") is Path("."),
    which exists."""
    named = given or os.path.expandvars(stored)
    if not named:
        return None
    path, corpus_dir = Path(named), os.environ.get("VRFKIT_CORPUS_DIR", "")
    return Path(corpus_dir) / path if corpus_dir and not path.is_absolute() else path


def machine_path(given: Path | None, stored: str, what: str, baseline: Path) -> str | None:
    """Why `--update` must not pin `given` into a new baseline: a path would
    put one machine's directory into a committed file."""
    if stored or given is None or not given.anchor:
        return None
    return (f"FAILED: --update would write the path {given} into {baseline.name}; a "
            f"baseline names its {what} relative to VRFKIT_CORPUS_DIR. Set "
            f"VRFKIT_CORPUS_DIR to {given.parent} and pass --{what} {given.name}.")


def missing_input(message: str, require: bool) -> int:
    """SKIP (0) an input that lives on another machine, unless --require-input
    or VRFKIT_REQUIRE_CORPUS makes it fatal (2): a guard that fails on someone
    else's machine gets disabled."""
    if require or os.environ.get("VRFKIT_REQUIRE_CORPUS"):
        print(f"REQUIRED INPUT MISSING: {message}", file=sys.stderr)
        return 2
    print(f"SKIP: {message}")
    return 0


def pin_or_diff(baseline: Path, stored: dict, current: dict, update: bool,
                refusals: list[str], diff) -> int | None:
    """`--update`: write `current`, or refuse (1) a run with `refusals` -- a
    figure never measured, pinned, is matched by the same failure next time.
    Otherwise 2 without a baseline, 1 on drift, None when `current` matches."""
    if update:
        if refusals:
            print(f"FAILED: refusing to pin a run with {len(refusals)} figure(s) never "
                  f"measured", file=sys.stderr)
            for line in refusals[:15]:
                print(f"  {line}", file=sys.stderr)
            return 1
        atomic_write_text(baseline, json.dumps(current, indent=1) + "\n")
        print(f"wrote {baseline}")
        return 0
    if not stored:
        print(f"no baseline at {baseline} -- run with --update", file=sys.stderr)
        return 2
    problems = diff(stored, current)
    if problems:
        print(f"DRIFT: {len(problems)} difference(s) from {baseline.name}")
        for line in problems:
            print(f"  {line}")
        return 1
    return None
