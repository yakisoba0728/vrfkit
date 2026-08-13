#!/usr/bin/env python3
"""Reproduce metrics.json for every replay that has a C# reference bundle.

Every figure in docs/archive/PROJECT_STATUS.md section 6 rests on a single
replay (02d4d478). This runs the whole pipeline -- vrfkit export, the valplay
adapter, compute_metrics.py -- against each replay that has BOTH a source
.vrf and a reference metrics.json, then diffs section by section.

The question it answers is not "does our parser work" (validate_corpus.py
already answers that at the bit level) but "does the section-level agreement
measured on 02d4d478 generalise". A section that is EXACT on one replay and
differs on ten is not EXACT; it is lucky.

Nothing under valplay/ or ValorantReplayParser/ is written to. Our outputs
go to out/xval/<id>/ and out/xval_bundle/<id>/.

Usage:
    python tools/validate_metrics_corpus.py [--limit N] [--only <id>]
                                            [--jobs N]
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

try:
    from .atomic_io import atomic_write_text, remove_tree, require_descendant
except ImportError:  # direct script execution
    from atomic_io import atomic_write_text, remove_tree, require_descendant

REPO = Path(__file__).resolve().parent.parent
VALPLAY = Path(os.environ.get("VRFKIT_VALPLAY_DIR", ""))
EXPORTS = VALPLAY / "pipeline" / "exports"
VRF_DIR = VALPLAY / "data" / "raw" / "vrf"
COMPUTE = VALPLAY / "pipeline" / "metrics" / "compute_metrics.py"
VRFKIT = REPO / "target" / "release" / "vrfkit.exe"
ADAPTER = REPO / "tools" / "to_valplay_bundle.py"

# Present in metrics.json but not a metric: provenance that necessarily differs
# because the two bundles live at different paths.
NON_METRIC_KEYS = {"source"}
REPLAY_ID_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]*\Z")
EXPORT_TIMEOUT_SECONDS = 1800
ADAPTER_TIMEOUT_SECONDS = 600
METRICS_TIMEOUT_SECONDS = 600


def discover():
    """Replay ids that have both a reference metrics.json and a source .vrf."""
    if not EXPORTS.is_dir():
        print(f"set VRFKIT_VALPLAY_DIR to the valplay checkout root; "
              f"exports dir not found at {EXPORTS}", file=sys.stderr)
        return []
    if not VRF_DIR.is_dir():
        print(f"set VRFKIT_VALPLAY_DIR to the valplay checkout root; "
              f"corpus dir not found at {VRF_DIR}", file=sys.stderr)
        return []
    have_vrf = {p.stem for p in VRF_DIR.glob("*.vrf")}
    out = []
    for d in sorted(EXPORTS.iterdir()):
        if d.is_dir() and (d / "metrics.json").exists() and d.name in have_vrf:
            out.append(d.name)
    return out


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8",
                          errors="replace", **kw)


def run_stage(cmd: list[str], *, timeout: float):
    """Run one pipeline stage and turn process failures into readable errors."""
    try:
        return run(cmd, timeout=timeout), None
    except subprocess.TimeoutExpired:
        return None, f"timeout after {timeout:g} seconds"
    except OSError as exc:
        return None, f"could not start process: {exc}"


def fresh_dir(path: Path, root: Path | None = None) -> Path:
    """Delete `path` and recreate it empty.

    These output directories persist between runs, and `compute_metrics.py` is
    invoked without `-o`, so the comparison reads `metrics.json` from inside
    the bundle directory. A previous run's file sitting there is read by the
    next one whenever compute_metrics exits 0 without writing -- and a stale
    metrics.json compared against its own reference looks EXACT. The same
    applies one step earlier: a bundle built over an export that has stopped
    writing a table silently mixes two runs.

    `check_export_baseline.py` already states the rule for its own output.
    """
    root = root or path.parent
    require_descendant(path, root)
    remove_tree(path, root)
    path.mkdir(parents=True, exist_ok=True)
    return path


def failures(results: list[dict]) -> list[str]:
    """Replays that did not complete the pipeline, as readable lines.

    The exit status was `return 0` once any single replay finished. Nineteen
    dead replays out of twenty exited 0, and so did a run in which every
    section differed.
    """
    return [f"{r['id']}: failed at {r['stage']} -- {str(r.get('error', ''))[:160]}"
            for r in results if r["stage"] != "ok"]


def process(replay_id: str) -> dict:
    """Export, adapt and compute metrics for one replay. Returns a result dict."""
    t0 = time.time()
    if not isinstance(replay_id, str) or not REPLAY_ID_RE.fullmatch(replay_id):
        return {"id": str(replay_id), "stage": "input", "error": "invalid replay id"}

    source = VRF_DIR / f"{replay_id}.vrf"
    reference = EXPORTS / replay_id / "metrics.json"
    if not source.is_file():
        return {"id": replay_id, "stage": "input", "error": f"missing replay: {source}"}
    if not reference.is_file():
        return {"id": replay_id, "stage": "input", "error": f"missing reference: {reference}"}
    try:
        ref = json.loads(reference.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        return {"id": replay_id, "stage": "input", "error": f"invalid reference: {exc}"}
    if not isinstance(ref, dict):
        return {"id": replay_id, "stage": "input", "error": "reference is not an object"}

    export_root = REPO / "out" / "xval"
    bundle_parent = REPO / "out" / "xval_bundle"
    export_dir = export_root / replay_id
    bundle_root = bundle_parent / replay_id
    try:
        require_descendant(export_dir, export_root)
        require_descendant(bundle_root, bundle_parent)
        fresh_dir(export_dir, export_root)
        fresh_dir(bundle_root, bundle_parent)
    except (OSError, ValueError) as exc:
        return {"id": replay_id, "stage": "input", "error": str(exc)}

    r, error = run_stage(
        [str(VRFKIT), "export", str(source), "--out", str(export_dir)],
        timeout=EXPORT_TIMEOUT_SECONDS,
    )
    if r is None:
        return {"id": replay_id, "stage": "export", "error": error}
    if r.returncode != 0:
        return {"id": replay_id, "stage": "export", "error": r.stderr[-400:]}

    r, error = run_stage(
        [sys.executable, str(ADAPTER), str(export_dir), "-o", str(bundle_root)],
        timeout=ADAPTER_TIMEOUT_SECONDS,
    )
    if r is None:
        return {"id": replay_id, "stage": "adapter", "error": error}
    if r.returncode != 0:
        return {"id": replay_id, "stage": "adapter", "error": r.stderr[-400:]}

    r, error = run_stage(
        [sys.executable, str(COMPUTE), str(bundle_root)],
        timeout=METRICS_TIMEOUT_SECONDS,
    )
    if r is None:
        return {"id": replay_id, "stage": "metrics", "error": error}
    if r.returncode != 0:
        return {"id": replay_id, "stage": "metrics", "error": r.stderr[-400:]}

    ours_path = bundle_root / "metrics.json"
    if not ours_path.exists():
        return {"id": replay_id, "stage": "metrics", "error": "no metrics.json written"}

    ours = json.loads(ours_path.read_text(encoding="utf-8"))
    sections = sorted((set(ours) | set(ref)) - NON_METRIC_KEYS)
    status = {s: ("EXACT" if ours.get(s) == ref.get(s) else "differs") for s in sections}
    return {
        "id": replay_id,
        "stage": "ok",
        "elapsed_s": round(time.time() - t0, 1),
        "sections": status,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--limit", type=int, default=None)
    ap.add_argument("--only", action="append", default=None)
    ap.add_argument("--jobs", type=int, default=3,
                    help="parallel replays; each uses ~2 GB, so keep it small")
    args = ap.parse_args()

    if not VRFKIT.exists():
        print(f"build the release binary first: {VRFKIT}", file=sys.stderr)
        return 2

    ids = args.only or discover()
    if args.limit:
        ids = ids[: args.limit]
    if not ids:
        print("no replays with both a reference bundle and a source .vrf",
              file=sys.stderr)
        return 2

    print(f"cross-validating {len(ids)} replays with {args.jobs} workers")
    results = []
    with ProcessPoolExecutor(max_workers=args.jobs) as pool:
        futures = {pool.submit(process, r): r for r in ids}
        for fut in as_completed(futures):
            res = fut.result()
            results.append(res)
            if res["stage"] != "ok":
                print(f"  {res['id']}  FAILED at {res['stage']}: {res['error'][:160]}")
            else:
                n_exact = sum(1 for v in res["sections"].values() if v == "EXACT")
                print(f"  {res['id']}  {n_exact}/{len(res['sections'])} exact"
                      f"  ({res['elapsed_s']}s)")

    ok = [r for r in results if r["stage"] == "ok"]
    if not ok:
        print("\nno replay completed", file=sys.stderr)
        return 1

    all_sections = sorted({s for r in ok for s in r["sections"]})
    order = sorted(ok, key=lambda r: r["id"])

    print()
    print(f"{'section':18s} " + " ".join(r["id"][:8] for r in order) + "   all-exact")
    print("-" * (19 + 9 * len(order) + 12))
    always = []
    for s in all_sections:
        marks = []
        for r in order:
            v = r["sections"].get(s)
            marks.append("   ok    " if v == "EXACT" else ("   --    " if v else "   ?     "))
        every = all(r["sections"].get(s) == "EXACT" for r in order)
        if every:
            always.append(s)
        print(f"{s:18s} " + "".join(marks) + ("   YES" if every else "   no"))

    print()
    print(f"replays compared        : {len(ok)} of {len(ids)}")
    print(f"sections exact on ALL {len(ok):>2}: {len(always)} / {len(all_sections)}")
    print(f"  {', '.join(always) if always else '(none)'}")
    if len(ok) < 2:
        # "EXACT on all" over one replay is the claim this tool exists to
        # test, not evidence for it.
        print("  NOTE: 'ALL' is one replay here; that is the single-replay "
              "claim this run was supposed to generalise.")

    summary = REPO / "out" / "xval_summary.json"
    atomic_write_text(summary, json.dumps(
        {"replays": order, "always_exact": always}, indent=1))
    print(f"\nwrote {summary}")

    dead = failures(results)
    if dead:
        print(f"\nFAILED: {len(dead)} of {len(ids)} replay(s) did not complete "
              f"the pipeline", file=sys.stderr)
        for line in dead[:15]:
            print(f"    {line}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
