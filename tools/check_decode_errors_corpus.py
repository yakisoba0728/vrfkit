"""Assert that the type overlay and the struct-blob decoders decode cleanly
across a whole corpus.

`vrfkit validate` does not print the overlay counters at all -- only `export`
does -- so validate_corpus.py cannot see a decode error, and never could. That
matters because a wrong overlay type is exactly the failure this project is
least able to notice by other means: the row still emits, the block still
walks, the block/field/RPC totals do not move, and every counter
validate_corpus.py reads stays identical.

The overlay decoder is strict in both directions -- a decoder that runs off the
end of the payload returns Err(BitIo) and one that leaves bits behind returns
Err(NotFullyConsumed) -- so `Decode errors: 0` over a corpus is a real
statement about every (group, field) type in the table, not just the ones one
replay happens to exercise.

This is the only check that can catch a per-class type whose two candidate
readings happen to be indistinguishable on the reference replay. A rotator
quantization, for instance, is not on the wire: it is a descriptor choice, and
when a class never replicates a rotation both readings consume the same bits.
The choice only becomes observable on a replay where some payload does set a
rotator flag, which may be any replay in the corpus but is not necessarily the
one being developed against.

Exports run into a temporary directory that is deleted as soon as its counters
have been read, so the peak disk cost is (jobs x one replay's Parquet output)
rather than the whole corpus.

Corpus discovery is shared with `validate_corpus.py` through `corpus_scan.py`
-- read that module's docstring for why the default does not recurse into
subdirectories (a `validate_corpus.py`/`check_decode_errors_corpus.py` run
pointed at the same directory used to disagree by exactly this: 153 files
against 126, a 27-file gap in a `Demos/old` subdirectory that this tool's
narrower glob silently skipped, with nothing printed to say so) and why the
excluded count always prints. Pass `--recursive` to walk subdirectories too.

Usage:
    python tools/check_decode_errors_corpus.py <vrfkit.exe> <corpus dir>
    python tools/check_decode_errors_corpus.py <vrfkit.exe> <corpus dir> --jobs 8
    python tools/check_decode_errors_corpus.py <vrfkit.exe> <corpus dir> --recursive
    python tools/check_decode_errors_corpus.py <vrfkit.exe> <corpus dir> --checkpoints

`--checkpoints` passes the same flag on to `vrfkit export`, so it additionally
decodes every Checkpoint chunk each replay carries, and this tool then checks
the checkpoint counters the same way it checks the main pass: every one of the
eleven checkpoint counters ("Overlay: ... / Checkpoint blobs: ... /
Checkpoint fails: ...") must be present, and the five failure counters among
them must be zero. It is opt-in, not the default: decoding checkpoints is
real extra work, and both this tool's own docstring and every corpus sweep in
this repo need a `--checkpoints`-free invocation to keep meaning the same
thing it always has (see docs/USAGE.md). Before this flag existed, checkpoint
decoding was verified on exactly one pinned replay
(`tools/baselines/checkpoint_02d4d478.json`) and never across a corpus, even
though docs/archive/PROJECT_STATUS.md measured 4,024 checkpoints across the
215-replay corpus.

The struct-blob decoders (RoundResults, TeamEconomy, RoundInfos) are checked
here for the same reason and are, if anything, a worse case: they are additive,
so a total failure moves NOTHING else on the summary. Build 13.02 shifted
RoundResults from handle 93 to 81 and the export stayed clean on every counter
above while the match score silently stopped being written. "Struct blobs:
N decoded / 0 failed" is the statement that did not exist then.

Exit code is 0 only when every replay reported "Decode errors: 0" and
"Struct blobs: ... / 0 failed", AND every replay reported both counters at
all, AND the corpus as a whole decoded something. A counter that stops being
printed must not read as zero; that is how the corpus malformed figure stayed
a vacuous 0 for the project's whole history (see
docs/archive/PROJECT_STATUS.md 5-O).

The last of those three is the same argument one step further, and it was
missing: `Decoded OK` and `Struct blobs: N decoded` were summed, printed, and
never read again. An exporter whose decoders never ran prints

    Rows offered: 0 / Decoded OK: 0 / Decode errors: 0 / Struct blobs: 0
    decoded / 0 failed

for every replay -- every counter a truthful zero, no error anywhere -- and
that used to print "OK: every replay reported Decode errors: 0" and exit 0. A
counter that CANNOT MOVE must not read as success either; see `dead_counters`.

The process exit status is read for the same reason. `vrfkit export` prints
this summary before it finalises the Parquet files, so an exporter that dies
writing them has already printed `Decode errors: 0`. A nonzero exit makes the
replay unreadable rather than clean; see `read_counters`.
"""
from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import corpus_scan

DECODE_ERRORS = re.compile(r"Decode errors:\s+(\d+)")
DECODED_OK = re.compile(r"Decoded OK:\s+(\d+)")
NOT_IN_TABLE = re.compile(r"Not in table:\s+(\d+)")
RAW_SKIP = re.compile(r"Raw/Skip:\s+(\d+)")
NO_FIELD_NAME = re.compile(r"No field name:\s+(\d+)")
ROWS_OFFERED = re.compile(r"Rows offered:\s+(\d+)")
STRUCT_DECODED = re.compile(r"Struct blobs:\s+(\d+) decoded")
STRUCT_FAILED = re.compile(r"Struct blobs:\s+\d+ decoded / (\d+) failed")


#: `(key, regex)` for every counter read off the export summary. `no_field_name`
#: is here -- and REQUIRED below -- because summary.rs defines
#: `Rows offered = decoded_ok + decoded_err + raw_or_skip + not_in_table +
#: no_field_name`; leaving it out (as this tool used to) means the four
#: categories it prints sum to about 0.3% less than the `rows offered` line it
#: also prints, and a reader has to go read Rust source to know why. See
#: `reconcile`.
COUNTERS = (
    ("decode_errors", DECODE_ERRORS),
    ("decoded_ok", DECODED_OK),
    ("raw_skip", RAW_SKIP),
    ("not_in_table", NOT_IN_TABLE),
    ("no_field_name", NO_FIELD_NAME),
    ("rows_offered", ROWS_OFFERED),
    ("struct_blobs_decoded", STRUCT_DECODED),
    ("struct_blobs_failed", STRUCT_FAILED),
)

#: Counters a replay MUST report for its run to mean anything. `decoded_ok` and
#: `struct_blobs_decoded` are here as well as the two error counters because a
#: zero in an error counter is only evidence when the matching work counter
#: proves the work happened. `no_field_name` is required for the same reason
#: every other line here is: `summary.rs` prints it unconditionally on a
#: healthy export, so its absence means this run's summary cannot be trusted,
#: not that the category was legitimately empty -- and `reconcile` depends on
#: it being a real number, never a defaulted one.
REQUIRED = (
    ("decode_errors", "Decode errors"),
    ("decoded_ok", "Decoded OK"),
    ("raw_skip", "Raw/Skip"),
    ("not_in_table", "Not in table"),
    ("no_field_name", "No field name"),
    ("rows_offered", "Rows offered"),
    ("struct_blobs_decoded", "Struct blobs ... decoded"),
    ("struct_blobs_failed", "Struct blobs ... failed"),
)

#: Corpus totals that cannot legitimately stay at zero, and the label to name
#: in the failure. Over a whole corpus of real matches both of these are large;
#: a zero means the decoder never ran, not that it ran and found nothing.
MUST_MOVE = (
    ("decoded_ok", "Decoded OK"),
    ("struct_blobs_decoded", "Struct blobs ... decoded"),
)

# --- Checkpoint counters, parsed only when --checkpoints was passed --------
#
# summary.rs's print_checkpoints packs several counters onto each line (see
# crates/vrfkit/src/driver/summary.rs), unlike the main pass which gets one
# regex per counter -- so each of these three patterns carries more than one
# capture group, and CHECKPOINT_COUNTERS names which group is which counter.
CHECKPOINT_OVERLAY = re.compile(
    r"Overlay:\s+(\d+) decoded / (\d+) errors / (\d+) raw-skip / "
    r"(\d+) not-in-table / (\d+) unnamed / (\d+) effect blobs")
CHECKPOINT_BLOBS = re.compile(r"Checkpoint blobs:\s+(\d+) decoded / (\d+) failed")
CHECKPOINT_FAILS = re.compile(
    r"Checkpoint fails:\s+(\d+) array / (\d+) truncated RPC / (\d+) movement")

#: `(key, regex, group)` for every checkpoint counter. Only consulted when the
#: caller asks `read_counters` for `require_checkpoints=True` -- a summary from
#: a run without `--checkpoints` never has this block at all, and treating its
#: absence as failure there would break the existing, checkpoint-free
#: invocation this tool has always supported.
CHECKPOINT_COUNTERS = (
    ("checkpoint_decoded", CHECKPOINT_OVERLAY, 1),
    ("checkpoint_errors", CHECKPOINT_OVERLAY, 2),
    ("checkpoint_raw_skip", CHECKPOINT_OVERLAY, 3),
    ("checkpoint_not_in_table", CHECKPOINT_OVERLAY, 4),
    ("checkpoint_unnamed", CHECKPOINT_OVERLAY, 5),
    ("checkpoint_effect_blobs", CHECKPOINT_OVERLAY, 6),
    ("checkpoint_blobs_decoded", CHECKPOINT_BLOBS, 1),
    ("checkpoint_blobs_failed", CHECKPOINT_BLOBS, 2),
    ("checkpoint_fail_array", CHECKPOINT_FAILS, 1),
    ("checkpoint_fail_truncated_rpc", CHECKPOINT_FAILS, 2),
    ("checkpoint_fail_movement", CHECKPOINT_FAILS, 3),
)

#: Every checkpoint counter is REQUIRED, on the same reasoning as `REQUIRED`
#: above: `with_checkpoints.then_some(&cp_stats)` in driver/mod.rs means the
#: whole `=== Checkpoints ===` block prints, zeros included, on every export
#: run with `--checkpoints` -- even for a replay with no checkpoint chunks at
#: all. Its absence therefore means this run's checkpoint pass cannot be
#: trusted, not that there was nothing to report. Labels name the printed line
#: they come from, exactly as `REQUIRED` above does.
CHECKPOINT_REQUIRED = (
    ("checkpoint_decoded", "Overlay ... decoded (checkpoint)"),
    ("checkpoint_errors", "Overlay ... errors (checkpoint)"),
    ("checkpoint_raw_skip", "Overlay ... raw-skip (checkpoint)"),
    ("checkpoint_not_in_table", "Overlay ... not-in-table (checkpoint)"),
    ("checkpoint_unnamed", "Overlay ... unnamed (checkpoint)"),
    ("checkpoint_effect_blobs", "Overlay ... effect blobs (checkpoint)"),
    ("checkpoint_blobs_decoded", "Checkpoint blobs ... decoded"),
    ("checkpoint_blobs_failed", "Checkpoint blobs ... failed"),
    ("checkpoint_fail_array", "Checkpoint fails ... array"),
    ("checkpoint_fail_truncated_rpc", "Checkpoint fails ... truncated RPC"),
    ("checkpoint_fail_movement", "Checkpoint fails ... movement"),
)

#: Checkpoint corpus totals that cannot legitimately stay at zero once
#: --checkpoints is on, mirroring MUST_MOVE for the main pass. This is the
#: 13.02 RoundResults incident one level down: `Checkpoint blobs: N decoded`
#: is additive, so a checkpoint decoder that stopped running would move
#: nothing else on the summary, and "0 failed" beside a corpus-wide zero
#: `decoded` says nothing.
CHECKPOINT_MUST_MOVE = (
    ("checkpoint_decoded", "Overlay ... decoded (checkpoint)"),
    ("checkpoint_blobs_decoded", "Checkpoint blobs ... decoded"),
)


def read_counters(
    text: str, returncode: int, require_checkpoints: bool = False,
) -> tuple[dict[str, int] | None, str]:
    """`(counters, error)` for one export's output. `counters` is None on failure.

    A nonzero exit is a failure even when the summary parsed cleanly: the
    exporter prints these counters before it finalises the Parquet files, so a
    run that dies writing them has already printed `Decode errors: 0`.

    `require_checkpoints` defaults to False so a summary from a run without
    `--checkpoints` -- which has no `=== Checkpoints ===` block at all -- keeps
    reading exactly as it always has. Pass it only when this replay's export
    was itself run with `--checkpoints`.
    """
    tail = " | ".join(l for l in text.splitlines()[-3:] if l.strip())
    if returncode != 0:
        return None, f"exit {returncode}: {tail[:200]}"
    counters: dict[str, int] = {}
    for key, pattern in COUNTERS:
        m = pattern.search(text)
        if m:
            counters[key] = int(m.group(1))
    for required, label in REQUIRED:
        if required not in counters:
            return None, f"no {label} counter: {tail[:200]}"
    if require_checkpoints:
        for key, pattern, group in CHECKPOINT_COUNTERS:
            m = pattern.search(text)
            if m:
                counters[key] = int(m.group(group))
        for required, label in CHECKPOINT_REQUIRED:
            if required not in counters:
                return None, f"no {label} counter: {tail[:200]}"
    return counters, ""


def dead_counters(totals: dict[str, int]) -> list[str]:
    """Corpus totals that never moved, as human-readable failures.

    `Decode errors: 0` is only a statement about the overlay if something was
    decoded, and `Struct blobs: 0 failed` is only a statement about the struct
    decoders if some blob was decoded. Both counters were summed and printed
    and then never read, so a corpus on which nothing ran at all reported a
    clean sweep.
    """
    return [f"{label} totalled 0 across the corpus: nothing decoded, so the "
            f"zero in its error counter says nothing"
            for key, label in MUST_MOVE if not totals.get(key)]


def dead_checkpoint_counters(totals: dict[str, int]) -> list[str]:
    """`dead_counters`, for the checkpoint pass. Only meaningful -- and only
    ever called -- when `--checkpoints` was requested; see `CHECKPOINT_MUST_MOVE`.
    """
    return [f"{label} totalled 0 across the corpus (with --checkpoints): "
            f"nothing decoded, so the zero in its failure counter says nothing"
            for key, label in CHECKPOINT_MUST_MOVE if not totals.get(key)]


def reconcile(totals: dict[str, int]) -> str | None:
    """None if the five overlay categories sum to `rows_offered`; else why not.

    summary.rs defines `Rows offered = decoded_ok + decoded_err + raw_or_skip
    + not_in_table + no_field_name`. This tool printed the first four for a
    long time and never `no_field_name`, so its own printed categories summed
    to about 0.3% less than its own `rows offered` line -- correct numbers,
    illegible arithmetic, and a reader had to go read the Rust source to know
    the fifth category existed at all.

    `no_field_name` is indexed directly, never `totals.get("no_field_name",
    0)`: a `.get` with a default would make an ABSENT counter reconcile
    silently, which is the exact doctrine this function exists to enforce
    against -- see `REQUIRED`, which is what keeps this KeyError from ever
    firing on a real run.

    `no_field_name` deliberately does NOT join `MUST_MOVE`/`dead_counters`
    above: unlike `decoded_ok` and `struct_blobs_decoded`, which are large on
    any real corpus and a corpus-wide zero for either means the decoder never
    ran, a corpus where every handle happens to resolve to a name is a
    legitimate (if unlikely) outcome for `no_field_name`, not evidence the
    check never ran. Gating on it would be a false failure on a clean corpus.
    """
    reconciled = (totals["decoded_ok"] + totals["decode_errors"]
                 + totals["raw_skip"] + totals["not_in_table"]
                 + totals["no_field_name"])
    offered = totals["rows_offered"]
    if reconciled == offered:
        return None
    return (f"decoded OK + decode errors + raw/skip + not in table + no "
            f"field name = {reconciled:,}, but rows offered = {offered:,} "
            f"({reconciled - offered:+,}) -- summary.rs's five categories no "
            f"longer sum to its own total")


def export_command(exe: Path, replay: Path, out: Path, with_checkpoints: bool) -> list[str]:
    """The `vrfkit export` argv, split out so the `--checkpoints` wiring is
    testable without a subprocess."""
    cmd = [str(exe), "export", str(replay), "--out", str(out)]
    if with_checkpoints:
        cmd.append("--checkpoints")
    return cmd


def _export_one(
    exe: Path, replay: Path, with_checkpoints: bool = False,
) -> tuple[str, dict[str, int] | None, str]:
    """Export one replay to a scratch dir and return its overlay counters."""
    out = Path(tempfile.mkdtemp(prefix="vrfkit-decode-"))
    try:
        r = subprocess.run(
            export_command(exe, replay, out, with_checkpoints),
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=600,
        )
        counters, err = read_counters((r.stdout or "") + (r.stderr or ""),
                                      r.returncode, require_checkpoints=with_checkpoints)
        return replay.name, counters, err
    except subprocess.TimeoutExpired:
        return replay.name, None, "timeout"
    finally:
        shutil.rmtree(out, ignore_errors=True)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    """`argv=None` defers to `sys.argv[1:]` (argparse's own default); tests pass
    an explicit list instead of monkeypatching `sys.argv`."""
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
    return ap.parse_args(argv)


def main() -> int:
    args = parse_args()

    if not args.exe.is_file():
        print(f"executable not found: {args.exe}", file=sys.stderr)
        return 2

    scan = corpus_scan.discover(args.corpus, args.recursive)
    # Unconditional, `excluded=0` included -- see corpus_scan.py's docstring.
    print(corpus_scan.scope_line(scan))
    files = scan.files
    if args.limit:
        files = files[: args.limit]
        print(f"limited to the first {len(files)} of {len(scan.files)} discovered")
    if not files:
        print(f"no .vrf files under {args.corpus}", file=sys.stderr)
        return 2

    if args.checkpoints:
        print("also decoding Checkpoint chunks (--checkpoints); this costs "
              "real extra time and disk per replay")

    print(f"exporting {len(files)} replays {args.jobs}-wide to read the overlay counters")
    started = time.time()
    unreadable: list[tuple[str, str]] = []
    offenders: list[tuple[str, int]] = []
    blob_offenders: list[tuple[str, int]] = []
    checkpoint_offenders: list[tuple[str, int]] = []
    totals = {"decode_errors": 0, "decoded_ok": 0, "raw_skip": 0,
              "not_in_table": 0, "no_field_name": 0, "rows_offered": 0,
              "struct_blobs_decoded": 0, "struct_blobs_failed": 0}
    if args.checkpoints:
        totals.update({key: 0 for key, _pattern, _group in CHECKPOINT_COUNTERS})
    done = 0

    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for name, counters, err in pool.map(
            lambda f: _export_one(args.exe, f, with_checkpoints=args.checkpoints),
            files,
        ):
            done += 1
            if counters is None:
                unreadable.append((name, err))
            else:
                for k, v in counters.items():
                    totals[k] += v
                if counters["decode_errors"]:
                    offenders.append((name, counters["decode_errors"]))
                if counters["struct_blobs_failed"]:
                    blob_offenders.append((name, counters["struct_blobs_failed"]))
                if args.checkpoints:
                    cp_failures = (counters["checkpoint_errors"]
                                   + counters["checkpoint_blobs_failed"]
                                   + counters["checkpoint_fail_array"]
                                   + counters["checkpoint_fail_truncated_rpc"]
                                   + counters["checkpoint_fail_movement"])
                    if cp_failures:
                        checkpoint_offenders.append((name, cp_failures))
            if done % 25 == 0 or done == len(files):
                print(f"  [{done}/{len(files)}] unreadable={len(unreadable)} "
                      f"with_errors={len(offenders)} "
                      f"blob_failures={len(blob_offenders)}"
                      + (f" checkpoint_failures={len(checkpoint_offenders)}"
                         if args.checkpoints else ""))

    elapsed = time.time() - started
    print(f"\nelapsed {elapsed:.1f}s ({elapsed / len(files):.2f}s per replay)")
    print(f"replays read      : {len(files) - len(unreadable)}/{len(files)}")
    print(f"decode errors     : {totals['decode_errors']:,}")
    print(f"decoded OK        : {totals['decoded_ok']:,}")
    print(f"raw/skip          : {totals['raw_skip']:,}")
    print(f"not in table      : {totals['not_in_table']:,}")
    print(f"no field name     : {totals['no_field_name']:,}")
    print(f"rows offered      : {totals['rows_offered']:,}")
    print(f"struct blobs      : {totals['struct_blobs_decoded']:,} decoded / "
          f"{totals['struct_blobs_failed']:,} failed")
    if args.checkpoints:
        # Unconditional, zeros included, on the same reasoning as every other
        # line here: a conditional line could not tell "the checkpoint pass
        # ran clean" from "the checkpoint pass never reached these counters".
        print(f"checkpoint overlay: {totals['checkpoint_decoded']:,} decoded / "
              f"{totals['checkpoint_errors']:,} errors / "
              f"{totals['checkpoint_raw_skip']:,} raw-skip / "
              f"{totals['checkpoint_not_in_table']:,} not-in-table / "
              f"{totals['checkpoint_unnamed']:,} unnamed / "
              f"{totals['checkpoint_effect_blobs']:,} effect blobs")
        print(f"checkpoint blobs  : {totals['checkpoint_blobs_decoded']:,} "
              f"decoded / {totals['checkpoint_blobs_failed']:,} failed")
        print(f"checkpoint fails  : {totals['checkpoint_fail_array']:,} array / "
              f"{totals['checkpoint_fail_truncated_rpc']:,} truncated RPC / "
              f"{totals['checkpoint_fail_movement']:,} movement")

    if unreadable:
        print(f"\nFAILED: {len(unreadable)} replay(s) did not report the counter",
              file=sys.stderr)
        for name, err in unreadable[:15]:
            print(f"    {name}: {err}", file=sys.stderr)
        return 1
    if offenders:
        offenders.sort(key=lambda kv: -kv[1])
        print(f"\nFAILED: {len(offenders)} replay(s) reported decode errors",
              file=sys.stderr)
        for name, count in offenders[:20]:
            print(f"    {name}: {count}", file=sys.stderr)
        return 1
    if blob_offenders:
        blob_offenders.sort(key=lambda kv: -kv[1])
        print(f"\nFAILED: {len(blob_offenders)} replay(s) reported struct-blob "
              f"decode failures. Re-run one by hand and read the "
              f"'Struct blob err:' line -- it names the member and handle.",
              file=sys.stderr)
        for name, count in blob_offenders[:20]:
            print(f"    {name}: {count}", file=sys.stderr)
        return 1
    if checkpoint_offenders:
        checkpoint_offenders.sort(key=lambda kv: -kv[1])
        print(f"\nFAILED: {len(checkpoint_offenders)} replay(s) reported "
              f"checkpoint decode failures (overlay errors, checkpoint blob "
              f"failures, array/RPC/movement failures)", file=sys.stderr)
        for name, count in checkpoint_offenders[:20]:
            print(f"    {name}: {count}", file=sys.stderr)
        return 1
    dead = dead_counters(totals)
    if dead:
        print(f"\nFAILED: {len(dead)} counter(s) never moved, so the clean "
              f"error counters beside them are vacuous", file=sys.stderr)
        for line in dead:
            print(f"    {line}", file=sys.stderr)
        return 1
    if args.checkpoints:
        dead_cp = dead_checkpoint_counters(totals)
        if dead_cp:
            print(f"\nFAILED: {len(dead_cp)} checkpoint counter(s) never "
                  f"moved, so the clean checkpoint failure counters beside "
                  f"them are vacuous", file=sys.stderr)
            for line in dead_cp:
                print(f"    {line}", file=sys.stderr)
            return 1

    # Fails loudly rather than printing a plausible wrong subtotal: a
    # mismatch here means summary.rs's five overlay categories no longer sum
    # to its own `rows offered` line -- most likely a sixth category was added
    # in Rust that this tool does not know to parse yet, which is exactly the
    # kind of drift a passing sweep must not paper over. See `reconcile`.
    mismatch = reconcile(totals)
    if mismatch:
        print(f"\nFAILED: the overlay categories do not reconcile: {mismatch}",
              file=sys.stderr)
        return 1
    print(f"reconciles        : decoded OK + decode errors + raw/skip + not "
          f"in table + no field name = rows offered "
          f"({totals['rows_offered']:,})")

    ok_msg = (f"\nOK: {len(files)} replays reported Decode errors: 0 and 0 "
              f"struct-blob failures, over {totals['decoded_ok']:,} decoded "
              f"rows and {totals['struct_blobs_decoded']:,} decoded struct "
              f"blobs")
    if args.checkpoints:
        ok_msg += (f"; checkpoints clean over "
                   f"{totals['checkpoint_decoded']:,} decoded checkpoint "
                   f"fields and {totals['checkpoint_blobs_decoded']:,} "
                   f"decoded checkpoint blobs")
    print(ok_msg)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
