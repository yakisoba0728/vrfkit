"""Assert that the type overlay and the struct-blob decoders decode cleanly
across a whole corpus.

`vrfkit validate` does not print the overlay counters -- only `export` does --
so validate_corpus.py cannot see a decode error. The overlay is strict both
ways (Err(BitIo) past the end, Err(NotFullyConsumed) on leftover bits), so
`Decode errors: 0` over a corpus is a statement about every (group, field)
type in the table; only a corpus catches a type whose two readings consume the
same bits on the reference replay.

Each replay exports into a temporary directory deleted once its counters are
read, so peak disk is jobs x one replay's output. Discovery is corpus_scan.py's
(top level unless `--recursive`).

Usage:
    python tools/check_decode_errors_corpus.py <vrfkit.exe> <corpus dir>
    python tools/check_decode_errors_corpus.py <vrfkit.exe> <corpus dir> --jobs 8
    python tools/check_decode_errors_corpus.py <vrfkit.exe> <corpus dir> --recursive
    python tools/check_decode_errors_corpus.py <vrfkit.exe> <corpus dir> --checkpoints

Every G line of summary_counters.py must print on every replay; `--checkpoints`
adds the checkpoint block's. Exit 0 only when every `FAILURES` counter is zero
on every replay, every `MUST_MOVE` work counter moved across the corpus, and
the overlay buckets reconcile. Every failure gate is backed in `MUST_MOVE` by
the work counter whose movement makes its zero evidence, or listed in
`UNBACKED` with the reason none exists and printed as unbacked on every run.
Checkpoint `conflicts` is printed, never gated: a conflict is the overlay
refusing to type a renamed handle, the protection working.
"""
from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import corpus_scan
import summary_counters as sc

#: Counters that must be zero on every replay: verify_build_corpus.py's
#: SINK_ZERO read off the summary (the test pins the correspondence).
#: CNC `attempted` and native component envelope/header work are printed,
#: not zero gates. Any malformed or unrepresented component is an error.
FAILURES = (
    "overlay_decode_errors", "struct_blobs_failed", "movement_rpc_errors", "array_errors",
    "array_truncations", "array_unconsumed_root_bits", "array_unconsumed_nested_bits",
    "array_implicit_terminations", "array_leaf_decode_errors", "truncated_rpcs",
    "cnc_bruteforce_payloads_unwalked", "movement_component_errors",
)
CHECKPOINT_FAILURES = tuple("cp_" + key for key in FAILURES)

#: `(work counter, the FAILURES it backs)`: corpus totals that cannot stay at
#: zero, or every gate they back reads 0 vacuously. The array walker, leaf
#: decoders and movement decoder are additive like the struct blobs: stopped,
#: they move nothing else. Minimum per replay over the 1,018-export audit:
#: 4,654 decoded rows, 16 struct blobs, 32 array elements, 182 array fields and
#: 3,001 movement rows; RPC parameter walks, not in that audit: 441 (the 12.11
#: public fixture, lowest of 25 replays of 11.06-13.06). So a one-replay corpus
#: passes. A corpus total cannot catch a walker stopped on one build. The
#: per-route child counts print but gate nothing: routes are admitted per build
#: (four on the 12.10-13.00 fixtures, three on 11.06), so a one-build corpus
#: can total 0 on a route; check_export_baseline.py pins each on the reference
#: replay.
MUST_MOVE = (
    ("overlay_decoded_ok", ("overlay_decode_errors",)),
    ("struct_blobs_decoded", ("struct_blobs_failed",)),
    ("array_elements_decoded", ("array_errors", "array_truncations", "array_unconsumed_root_bits",
                                "array_unconsumed_nested_bits", "array_implicit_terminations")),
    ("array_fields_emitted", ("array_leaf_decode_errors",)),
    ("movement_rows", ("movement_rpc_errors", "movement_component_errors")),
    ("rpc_param_walks", ("truncated_rpcs",)),
)
#: The checkpoint pass: the same minus movement (at least 2,054 decoded
#: fields, 16 blobs, 30 array elements and 255 array fields per replay).
CHECKPOINT_MUST_MOVE = tuple(("cp_" + work, tuple("cp_" + g for g in gates))
                             for work, gates in MUST_MOVE[:4])

#: `(failure counter, why no work counter backs it)`.
UNBACKED = (
    ("cnc_bruteforce_payloads_unwalked",
     "`CNC brute force: N attempted` is legitimately 0 on the 12.10 and 12.11 public "
     "fixtures, so it cannot be a must-move counter"),
)
#: No checkpoint RPC reached `on_rpc` on the 1,018 audit replays (every one a
#: post-RepLayout ClassNetCache tail), so these gates have nothing to back them.
CHECKPOINT_UNBACKED = (
    ("cp_movement_rpc_errors", "no checkpoint RPC reaches the movement decoder"),
    ("cp_truncated_rpcs", "no checkpoint RPC reaches the parameter walk"),
    ("cp_movement_component_errors", "no checkpoint RPC reaches the movement decoder"),
    ("cp_cnc_bruteforce_payloads_unwalked",
     "no checkpoint RPC reaches the fc=34 brute-force walk (attempted is 0)"),
)
PASSES = {False: (FAILURES, MUST_MOVE, UNBACKED),
          True: (CHECKPOINT_FAILURES, CHECKPOINT_MUST_MOVE, CHECKPOINT_UNBACKED)}


def read_counters(
    text: str, returncode: int, require_checkpoints: bool = False,
) -> tuple[dict[str, int] | None, str]:
    """`(counters, error)` for one export's output; `counters` is None on failure.

    A nonzero exit fails even when the summary parsed: the exporter prints it
    before it finalises the Parquet files.
    """
    tail = sc.tail(text)
    if returncode != 0:
        return None, f"exit {returncode}: {tail}"
    wanted = sc.keys("G", False) + (sc.keys("G", True) if require_checkpoints else ())
    counters = sc.read(text, wanted)
    missing = [key for key, value in counters.items() if value is None]
    if missing:
        return None, f"no {sc.WHERE[missing[0]][0].fmt!r} line: {tail}"
    for prefix in (("", "cp_") if require_checkpoints else ("",)):
        problems = sc.movement_contract_errors(counters, prefix)
        if problems:
            return None, "; ".join(problems)
    return counters, ""


def replay_failures(counters: dict[str, int], checkpoints: bool) -> list[tuple[str, int]]:
    """`(key, count)` for every failure counter that is nonzero on one replay.
    Indexed, never `.get(key, 0)`: a missing counter must raise."""
    gated = FAILURES + (CHECKPOINT_FAILURES if checkpoints else ())
    return [(key, counters[key]) for key in gated if counters[key]]


def dead_counters(totals: dict[str, int], must_move=MUST_MOVE, suffix: str = "") -> list[str]:
    """Corpus totals that never moved: `Decode errors: 0` says something only
    if something was decoded."""
    return [f"{sc.label(key)} totalled 0 across the corpus{suffix}: nothing decoded, so the "
            f"zero in {', '.join(map(sc.label, gates))} says nothing"
            for key, gates in must_move if not totals.get(key)]


def unbacked_line(unbacked) -> str:
    return "; ".join(f"{sc.label(key)} ({why})" for key, why in unbacked)


def backing_summary(must_move, unbacked, totals: dict[str, int]) -> str:
    """How many zero failure counters are evidence, and of what; built from
    the tables so the OK line cannot count an unbacked gate as backed."""
    backed = sum(len(gates) for _key, gates in must_move)
    work = ", ".join(f"{sc.label(key)} {totals[key]:,}" for key, _gates in must_move)
    none = ", ".join(sc.label(key) for key, _why in unbacked)
    return (f"{backed} backed by work that moved ({work}), {len(unbacked)} "
            f"with no work counter ({none})")


def reconcile(totals: dict[str, int]) -> str | None:
    """None if the five overlay buckets sum to `Rows offered`, as summary.rs
    defines it; else why not. Indexed: a default would reconcile an absent
    counter silently."""
    reconciled = (totals["overlay_decoded_ok"] + totals["overlay_decode_errors"]
                  + totals["overlay_raw_skip"] + totals["overlay_not_in_table"]
                  + totals["overlay_no_field_name"])
    offered = totals["overlay_rows_offered"]
    if reconciled == offered:
        return None
    return (f"decoded OK + decode errors + raw/skip + not in table + no "
            f"field name = {reconciled:,}, but rows offered = {offered:,} "
            f"({reconciled - offered:+,}) -- summary.rs's five categories no "
            f"longer sum to its own total")


def _export_one(
    exe: Path, replay: Path, with_checkpoints: bool = False,
) -> tuple[dict[str, int] | None, str]:
    """Export one replay to a scratch dir and return its counters."""
    out = Path(tempfile.mkdtemp(prefix="vrfkit-decode-"))
    try:
        code, text = sc.vrfkit(exe, "export", replay, out, with_checkpoints, timeout=600)
        return read_counters(text, code, require_checkpoints=with_checkpoints)
    except subprocess.TimeoutExpired:
        return None, "timeout"
    finally:
        shutil.rmtree(out, ignore_errors=True)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    ap = argparse.ArgumentParser()
    ap.add_argument("exe", type=Path)
    ap.add_argument("corpus", type=Path)
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument("--limit", type=int, default=0, help="only the first N replays")
    ap.add_argument("--recursive", action="store_true",
                    help="also walk subdirectories of <corpus> -- see "
                         "corpus_scan.py for why this is opt-in")
    ap.add_argument("--checkpoints", action="store_true",
                    help="also pass --checkpoints to vrfkit export and check "
                         "the checkpoint counters (opt-in: real extra time "
                         "and disk per replay)")
    ap.add_argument("--redact-identifiers", action="store_true",
                    help="replace corpus paths and replay filenames in output "
                         "with private, run-local labels")
    return ap.parse_args(argv)


def main() -> int:
    args = parse_args()

    if not args.exe.is_file():
        print(f"executable not found: {args.exe}", file=sys.stderr)
        return 2

    scan = corpus_scan.discover(args.corpus, args.recursive)
    print(corpus_scan.scope_line(scan, args.redact_identifiers))
    files = scan.files
    if args.limit:
        files = files[: args.limit]
        print(f"limited to the first {len(files)} of {len(scan.files)} discovered")
    if not files:
        where = "<private corpus>" if args.redact_identifiers else str(args.corpus)
        print(f"no .vrf files under {where}", file=sys.stderr)
        return 2

    if args.checkpoints:
        print("also decoding Checkpoint chunks (--checkpoints); this costs "
              "real extra time and disk per replay")

    print(f"exporting {len(files)} replays {args.jobs}-wide to read the overlay counters")
    started = time.time()
    unreadable: list[tuple[str, str]] = []
    failing: list[tuple[str, list[tuple[str, int]]]] = []
    passes = (False, True) if args.checkpoints else (False,)
    totals = dict.fromkeys((k for cp in passes for k in sc.keys("G", cp)), 0)
    done = 0

    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for counters, err in pool.map(
            lambda f: _export_one(args.exe, f, with_checkpoints=args.checkpoints),
            files,
        ):
            done += 1
            name = corpus_scan.replay_label(
                files[done - 1], done, args.redact_identifiers)
            err = corpus_scan.diagnostic(err, args.redact_identifiers)
            if counters is None:
                unreadable.append((name, err))
            else:
                for k, v in counters.items():
                    totals[k] += v
                failures = replay_failures(counters, args.checkpoints)
                if failures:
                    failing.append((name, failures))
            if done % 25 == 0 or done == len(files):
                print(f"  [{done}/{len(files)}] unreadable={len(unreadable)} "
                      f"failing={len(failing)}")

    elapsed = time.time() - started
    print(f"\nelapsed {elapsed:.1f}s ({elapsed / len(files):.2f}s per replay)")
    print(f"replays read: {len(files) - len(unreadable)}/{len(files)}")
    # Every total, zeros included: a line printed only when nonzero could not
    # tell "clean" from "never read".
    for cp in passes:
        for line in sc.lines("G", cp):
            print(f"  {sc.render(line, totals)}")
        print(f"  {'checkpoint ' * cp}unbacked gates: {unbacked_line(PASSES[cp][2])}")

    if unreadable:
        print(f"\nFAILED: {len(unreadable)} replay(s) did not report the counter",
              file=sys.stderr)
        for name, err in unreadable[:15]:
            print(f"    {name}: {err}", file=sys.stderr)
        return 1
    if failing:
        failing.sort(key=lambda item: -sum(count for _key, count in item[1]))
        print(f"\nFAILED: {len(failing)} replay(s) reported a nonzero failure "
              f"counter", file=sys.stderr)
        for name, failures in failing[:20]:
            print(f"    {name}: " + ", ".join(
                f"{sc.label(key)}={count:,}" for key, count in failures), file=sys.stderr)
        print("  Re-run one by hand and read its summary: the 'Struct blob err:', "
              "'Movement err:' and 'Checkpoint blob error:' lines name the "
              "first failure of their kind.", file=sys.stderr)
        return 1
    for cp in passes:
        dead = dead_counters(totals, PASSES[cp][1], " (with --checkpoints)" * cp)
        if dead:
            print(f"\nFAILED: {len(dead)} {'checkpoint ' * cp}counter(s) never moved, so the "
                  f"clean failure counters beside them are vacuous", file=sys.stderr)
            for line in dead:
                print(f"    {line}", file=sys.stderr)
            return 1

    mismatch = reconcile(totals)
    if mismatch:
        print(f"\nFAILED: the overlay categories do not reconcile: {mismatch}",
              file=sys.stderr)
        return 1
    print(f"reconciles: decoded OK + decode errors + raw/skip + not in table + no "
          f"field name = rows offered ({totals['overlay_rows_offered']:,})")

    ok_msg = (f"\nOK: {len(files)} replays reported 0 on all {len(FAILURES)} "
              f"failure counters: {backing_summary(MUST_MOVE, UNBACKED, totals)}")
    if args.checkpoints:
        ok_msg += (f"; checkpoints 0 on all {len(CHECKPOINT_FAILURES)} failure "
                   f"counters: "
                   f"{backing_summary(CHECKPOINT_MUST_MOVE, CHECKPOINT_UNBACKED, totals)}")
    print(ok_msg)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
