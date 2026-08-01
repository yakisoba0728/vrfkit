#!/usr/bin/env python3
"""Reproduce metrics.json for every replay that has a C# reference bundle.

Every figure in PROJECT_STATUS section 6 rests on a single replay
(02d4d478). This runs the whole pipeline -- vrfkit export, the valplay
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
import subprocess
import sys
import time
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
VALPLAY = Path(r"C:\Users\yakihyuk0728\Documents\GitHub\valplay")
EXPORTS = VALPLAY / "pipeline" / "exports"
VRF_DIR = VALPLAY / "data" / "raw" / "vrf"
COMPUTE = VALPLAY / "pipeline" / "metrics" / "compute_metrics.py"
VRFKIT = REPO / "target" / "release" / "vrfkit.exe"
ADAPTER = REPO / "tools" / "to_valplay_bundle.py"

# Present in metrics.json but not a metric: provenance that necessarily differs
# because the two bundles live at different paths.
NON_METRIC_KEYS = {"source"}


def discover():
    """Replay ids that have both a reference metrics.json and a source .vrf."""
    have_vrf = {p.stem for p in VRF_DIR.glob("*.vrf")}
    out = []
    for d in sorted(EXPORTS.iterdir()):
        if d.is_dir() and (d / "metrics.json").exists() and d.name in have_vrf:
            out.append(d.name)
    return out


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8",
                          errors="replace", **kw)


def process(replay_id: str) -> dict:
    """Export, adapt and compute metrics for one replay. Returns a result dict."""
    t0 = time.time()
    export_dir = REPO / "out" / "xval" / replay_id
    bundle_root = REPO / "out" / "xval_bundle" / replay_id

    r = run([str(VRFKIT), "export", str(VRF_DIR / f"{replay_id}.vrf"),
             "--out", str(export_dir)])
    if r.returncode != 0:
        return {"id": replay_id, "stage": "export", "error": r.stderr[-400:]}

    r = run([sys.executable, str(ADAPTER), str(export_dir), "-o", str(bundle_root)])
    if r.returncode != 0:
        return {"id": replay_id, "stage": "adapter", "error": r.stderr[-400:]}

    r = run([sys.executable, str(COMPUTE), str(bundle_root)])
    if r.returncode != 0:
        return {"id": replay_id, "stage": "metrics", "error": r.stderr[-400:]}

    ours_path = bundle_root / "metrics.json"
    if not ours_path.exists():
        return {"id": replay_id, "stage": "metrics", "error": "no metrics.json written"}

    ours = json.loads(ours_path.read_text(encoding="utf-8"))
    ref = json.loads((EXPORTS / replay_id / "metrics.json").read_text(encoding="utf-8"))

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
    print(f"sections exact on ALL   : {len(always)} / {len(all_sections)}")
    print(f"  {', '.join(always) if always else '(none)'}")

    summary = REPO / "out" / "xval_summary.json"
    summary.write_text(json.dumps(
        {"replays": order, "always_exact": always}, indent=1), encoding="utf-8")
    print(f"\nwrote {summary}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
