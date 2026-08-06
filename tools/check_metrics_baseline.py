"""Assert that each BUILD's preserved replay still produces sane MATCH METRICS.

Every other check in this project reads counters that describe the FRAMING --
blocks, fields, RPCs, malformed packets, skipped bits -- or compares bytes
against a frozen export. None of them can see a semantic break, because a
decoder that stops producing values emits no rows and moves no framing counter.

Section 26 is the worked example. Build 13.02 shifted `RoundResults` from
handle 93 to 81, the decoder matched on the old numbers, and the match score
stopped being written. `validate_corpus.py`, `check_corpus_baseline.py`,
`check_export_baseline.py` and the byte oracle were ALL green, because every
number they read was identical. The break was only visible one layer up, where
the rows become a scoreboard.

So this guard runs the layer that could see it:

    vrfkit export -> tools/to_valplay_bundle.py -> valplay compute_metrics.py

VERIFIABLE CLAIM: the 13.02 fixture would have FAILED this guard before commit
bcc7d70. Measured in that session -- `RoundResults[N].*` leaves were 0 on
`1.vrf`, so `objective.round_count` was 0 while `rounds.round_count` was 21.
That is invariant R2 below.

Two kinds of check, and the first matters more
---------------------------------------------

**Invariants** hold for any replay of any build and need no baseline. They
survive legitimate changes that move counts, which pinned numbers do not, and
they are the ones that encode the section-26 failure directly.

**Pinned values** are drift detection for everything else. They live in ONE
file covering every build rather than one file per build, so that a build
disappearing from the set is itself a failure. Per-file baselines cannot see
that -- which is exactly how `check_corpus_baseline.py` was silently guarding
nothing when the game rotated four pinned replays out of `Saved\\Demos`.

`kills == deaths` is deliberately NOT an invariant, and section 34 measured the
real reason. RESURRECTION breaks it structurally: a resurrected player who dies
again in the same round gets two `bDied` reports, so `deaths` counts both,
while `kills` counts DidKill per (round, subject) interaction, which collapses
them into one. Across five Swiftplay replays the gap was 0 where there were no
resurrections and exactly 1 in each of the two that had one -- so an equality
assertion would fail on correct data every time an agent resurrects. It is
pinned instead.

Cost
----

This is the slowest check in the repo: it exports, re-nests and recomputes two
full 50-65 MB matches. Run it after a non-trivial change, not in a fast sweep.
Its value is the layer it exercises, not its speed.

Usage:
    python tools/check_metrics_baseline.py
    python tools/check_metrics_baseline.py --update
    python tools/check_metrics_baseline.py --only 13.02 --jobs 1
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEFAULT_EXE = REPO / "target" / "release" / "vrfkit.exe"
BUNDLE_TOOL = REPO / "tools" / "to_valplay_bundle.py"
DEFAULT_BASELINE = REPO / "tools" / "baselines" / "metrics_builds.json"

#: valplay is NEVER modified; it is only ever invoked by absolute path.
#: Set VRFKIT_VALPLAY_DIR to the valplay checkout root.
COMPUTE_METRICS = Path(
    os.environ.get("VRFKIT_VALPLAY_DIR", "")
) / "pipeline" / "metrics" / "compute_metrics.py"

#: One replay per build. 13.01 has no preserved fixture of its own -- it is the
#: reference replay the whole project is developed against, and it lives in the
#: read-only valplay corpus.
#:
#: The other four point at %LOCALAPPDATA%\vrfkit\baseline-corpora and MUST keep
#: doing so. They used to point at %LOCALAPPDATA%\VALORANT\Saved\Demos, which
#: the GAME owns and rotates; on 2026-08-02 all four pinned replays were gone.
#: A baseline over a directory another program writes to guards nothing.
REPLAYS = {
    "12.10": r"%LOCALAPPDATA%\vrfkit\baseline-corpora\build_1210"
             r"\9f8b32c5-c243-41ec-bbbb-832582edf652.12_10.vrf",
    "12.11": r"%LOCALAPPDATA%\vrfkit\baseline-corpora\build_1211"
             r"\5c673443-5bdc-4576-b416-aab3f62471a5.12_11.vrf",
    "13.00": r"%LOCALAPPDATA%\vrfkit\baseline-corpora\build_1300"
             r"\12974d2b-848f-490d-80ba-5f03a033c2d5.13_00.vrf",
    "13.01": "02d4d478-1dfb-4412-9a77-29ca29105a9d.vrf",
    "13.02": r"%LOCALAPPDATA%\vrfkit\baseline-corpora\build_1302\1.vrf",
}


# ---------------------------------------------------------------------------
# metric extraction
# ---------------------------------------------------------------------------

def _resolve_replay(raw: str) -> Path:
    """Expand env vars in a REPLAYS entry; anchor a bare filename in
    VRFKIT_CORPUS_DIR so a portable baseline still finds the replay."""
    p = Path(os.path.expandvars(raw))
    if not p.is_absolute():
        corpus_dir = os.environ.get("VRFKIT_CORPUS_DIR", "")
        if corpus_dir:
            p = Path(corpus_dir) / p
    return p


def _sum(d: dict, field: str) -> int:
    return sum(p.get(field) or 0 for p in d.values())


def extract(m: dict) -> dict:
    """The pinned figures, all of them semantic rather than framing."""
    combat = m["combat"]["per_player"]
    tac = m["tactical"]["per_player"]
    return {
        "rounds_rpc": m["rounds"]["round_count"],
        "rounds_objective": m["objective"]["round_count"],
        "client_round_starts": m["rounds"]["client_round_start_events"],
        "team_score": m["objective"]["team_score"],
        "plants": m["objective_detail"]["plant_count"],
        "defuses": m["objective_detail"]["defuse_count"],
        "players": len(m["players"]),
        "combat_players": len(combat),
        "kills": _sum(combat, "kills"),
        "deaths": _sum(combat, "deaths"),
        "assists": _sum(combat, "assists"),
        "headshots": _sum(combat, "headshots"),
        "damage_dealt": round(sum(p["damage_dealt"] for p in combat.values()), 2),
        "first_bloods": _sum(tac, "first_bloods"),
        "trade_kills": _sum(tac, "trade_kills"),
        "kast_rounds": _sum(m["kast"]["per_player"], "kast_rounds"),
        "ultimate_casts": m["ultimate"]["total_casts"],
        "distinct_weapons": m["weapons"]["distinct_weapons"],
        "shots": sum(m["weapons"]["shots_by_weapon"].values()),
        "shot_rays": m["shot_rays"]["ray_count"],
        "ability_spawns": m["ability_usage"]["ability_spawn_count"],
        "movement_samples": m["movement_summary"]["movement_samples"],
        "economy_rounds": m["economy_detail"]["rounds"],
    }


def invariants(v: dict) -> list[str]:
    """Checks that need no baseline. Each returns a message when it FAILS.

    R1-R3 are the section-26 break stated three ways. Before bcc7d70 the 13.02
    fixture violated R2 and R3 (objective 0 vs rpc 21) and R1.
    """
    bad = []
    if v["rounds_objective"] <= 0:
        bad.append(
            f"R1 objective.round_count is {v['rounds_objective']}: the "
            f"BombGameState round results produced nothing. This is the exact "
            f"shape of the 13.02 RoundResults handle shift (section 26)."
        )
    if v["rounds_rpc"] != v["rounds_objective"]:
        bad.append(
            f"R2 round count disagrees between its two independent sources: "
            f"ClientRoundStart RPCs say {v['rounds_rpc']}, BombGameState "
            f"RoundResults say {v['rounds_objective']}."
        )
    score_total = sum(v["team_score"].values())
    if score_total != v["rounds_objective"]:
        bad.append(
            f"R3 team_score sums to {score_total} but there are "
            f"{v['rounds_objective']} rounds: every round has exactly one "
            f"winner, so these cannot differ."
        )
    if v["players"] <= 0:
        bad.append("R4 no players were identified at all.")
    if v["kills"] > 0 and v["damage_dealt"] <= 0:
        bad.append(
            f"R5 {v['kills']} kills but zero damage: the combat report "
            f"stopped decoding while the kill timeline kept working."
        )
    return bad


# ---------------------------------------------------------------------------
# pipeline
# ---------------------------------------------------------------------------

def run_one(build: str, replay: Path, exe: Path) -> tuple[str, dict | None, str]:
    """export -> bundle -> compute_metrics, into a scratch dir that is removed."""
    if not replay.is_file():
        return build, None, f"replay not found: {replay}"
    out = Path(tempfile.mkdtemp(prefix=f"vrfkit-metrics-{build.replace('.', '_')}-"))
    try:
        steps = (
            ("export", [str(exe), "export", str(replay), "--out", str(out)]),
            ("bundle", [sys.executable, str(BUNDLE_TOOL), str(out),
                        "-o", str(out / "bundle")]),
            ("metrics", [sys.executable, str(COMPUTE_METRICS), str(out / "bundle"),
                         "-o", str(out / "metrics.json")]),
        )
        for name, cmd in steps:
            r = subprocess.run(cmd, capture_output=True, text=True,
                               encoding="utf-8", errors="replace", timeout=1800)
            if r.returncode != 0:
                tail = ((r.stderr or "") + (r.stdout or "")).strip().splitlines()
                return build, None, f"{name} failed rc={r.returncode}: " + (
                    " | ".join(tail[-3:])[:300] if tail else "no output")
        mj = out / "metrics.json"
        if not mj.exists():
            return build, None, "metrics.json was not written"
        return build, extract(json.loads(mj.read_text(encoding="utf-8"))), ""
    except subprocess.TimeoutExpired:
        return build, None, "timeout"
    finally:
        shutil.rmtree(out, ignore_errors=True)


def merged_metrics(stored: dict, fresh: dict, only) -> dict:
    """The metrics to write on `--update`, given what was already pinned.

    A scoped run (`--only 13.02`) looked at one build and knows nothing about
    the others, so it must not delete them. It used to: the payload was built
    from this run's results alone, so re-pinning after inspecting a single
    build left a one-build baseline and the docstring's guarantee -- "a build
    disappearing from the set is itself a failure" -- was silently retired.

    An unscoped run is the opposite case: it looked at every build in REPLAYS,
    so it is the only thing allowed to retire one. Merging there would keep a
    build pinned after it left REPLAYS and fail every later run with
    "MISSING from this run".
    """
    return dict(fresh) if only is None else {**stored, **fresh}


def compare(build: str, got: dict, want: dict) -> list[str]:
    drift = []
    for key in sorted(set(got) | set(want)):
        a, b = got.get(key, "<absent>"), want.get(key, "<absent>")
        if a != b:
            drift.append(f"{build} {key}: got {a}, baseline {b}")
    return drift


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--baseline", type=Path, default=DEFAULT_BASELINE)
    ap.add_argument("--exe", type=Path, default=DEFAULT_EXE)
    ap.add_argument("--jobs", type=int, default=3)
    ap.add_argument("--only", action="append", default=None,
                    help="limit to these builds (repeatable)")
    ap.add_argument("--update", action="store_true",
                    help="rewrite the baseline from this run")
    args = ap.parse_args()

    if not args.exe.is_file():
        print(f"executable not found: {args.exe}\n"
              f"build it with: cargo build --release -p vrfkit", file=sys.stderr)
        return 2
    if not COMPUTE_METRICS.is_file():
        print(f"valplay compute_metrics not found: {COMPUTE_METRICS}\n"
              f"set VRFKIT_VALPLAY_DIR to the valplay checkout root",
              file=sys.stderr)
        return 2

    builds = {k: v for k, v in REPLAYS.items()
              if args.only is None or k in args.only}
    if not builds:
        print(f"no builds selected; known: {sorted(REPLAYS)}", file=sys.stderr)
        return 2

    print(f"running export -> bundle -> compute_metrics on {len(builds)} build(s), "
          f"{args.jobs}-wide")
    started = time.time()
    results, failures = {}, []
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for build, values, err in pool.map(
            lambda kv: run_one(kv[0], _resolve_replay(kv[1]), args.exe),
            sorted(builds.items()),
        ):
            if values is None:
                failures.append(f"{build}: {err}")
                print(f"  {build}  PIPELINE FAILED  {err}")
            else:
                results[build] = values
                print(f"  {build}  rounds={values['rounds_objective']:>3} "
                      f"score={values['team_score']} "
                      f"players={values['players']:>2} "
                      f"kills={values['kills']:>4}")
    elapsed = time.time() - started
    print(f"elapsed {elapsed:.1f}s")

    # Invariants first: they are the point, and they apply even with --update.
    broken = []
    for build, values in sorted(results.items()):
        for msg in invariants(values):
            broken.append(f"{build}: {msg}")

    if broken:
        print(f"\nFAILED: {len(broken)} invariant violation(s)", file=sys.stderr)
        for msg in broken:
            print(f"    {msg}", file=sys.stderr)
        if args.update:
            print("  baseline NOT updated -- refusing to pin a broken run.",
                  file=sys.stderr)
        return 1
    if failures:
        print(f"\nFAILED: {len(failures)} build(s) did not complete the pipeline",
              file=sys.stderr)
        for msg in failures:
            print(f"    {msg}", file=sys.stderr)
        return 1

    if args.update:
        stored = json.loads(args.baseline.read_text(encoding="utf-8")) \
            if args.baseline.is_file() else {}
        metrics = merged_metrics(stored.get("metrics", {}), results, args.only)
        payload = {
            "note": "Semantic metrics per build. See the module docstring: "
                    "framing counters cannot see a decoder that stops "
                    "producing values.",
            "replays": REPLAYS,
            "metrics": metrics,
        }
        args.baseline.write_text(
            json.dumps(payload, indent=1, sort_keys=True) + "\n", encoding="utf-8")
        kept = sorted(set(metrics) - set(results))
        print(f"\nwrote {args.baseline}: re-pinned {len(results)} build(s)"
              + (f", kept {len(kept)} not looked at ({', '.join(kept)})"
                 if kept else ""))
        return 0

    if not args.baseline.is_file():
        print(f"\nbaseline not found: {args.baseline}\n"
              f"create it with --update", file=sys.stderr)
        return 2
    baseline = json.loads(args.baseline.read_text(encoding="utf-8"))
    want = baseline.get("metrics", {})

    drift = []
    for build in sorted(results):
        if build not in want:
            drift.append(f"{build}: present in this run, absent from the baseline")
        else:
            drift.extend(compare(build, results[build], want[build]))
    for build in sorted(want):
        if build not in results and (args.only is None or build in args.only):
            drift.append(f"{build}: in the baseline, MISSING from this run")

    if drift:
        print(f"\nFAILED: {len(drift)} metric(s) drifted from "
              f"{args.baseline.name}", file=sys.stderr)
        for msg in drift:
            print(f"    {msg}", file=sys.stderr)
        print("  If the change is intended, re-pin with --update.", file=sys.stderr)
        return 1

    n_inv = len(results) * 5
    print(f"\nOK: {len(results)} build(s) pass {n_inv} invariant checks and match "
          f"{sum(len(v) for v in results.values())} pinned metric values")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
