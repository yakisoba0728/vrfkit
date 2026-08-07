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
ROWS_OFFERED = re.compile(r"Rows offered:\s+(\d+)")
STRUCT_DECODED = re.compile(r"Struct blobs:\s+(\d+) decoded")
STRUCT_FAILED = re.compile(r"Struct blobs:\s+\d+ decoded / (\d+) failed")


#: `(key, regex)` for every counter read off the export summary.
COUNTERS = (
    ("decode_errors", DECODE_ERRORS),
    ("decoded_ok", DECODED_OK),
    ("raw_skip", RAW_SKIP),
    ("not_in_table", NOT_IN_TABLE),
    ("rows_offered", ROWS_OFFERED),
    ("struct_blobs_decoded", STRUCT_DECODED),
    ("struct_blobs_failed", STRUCT_FAILED),
)

#: Counters a replay MUST report for its run to mean anything. `decoded_ok` and
#: `struct_blobs_decoded` are here as well as the two error counters because a
#: zero in an error counter is only evidence when the matching work counter
#: proves the work happened.
REQUIRED = (
    ("decode_errors", "Decode errors"),
    ("decoded_ok", "Decoded OK"),
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


def read_counters(text: str, returncode: int) -> tuple[dict[str, int] | None, str]:
    """`(counters, error)` for one export's output. `counters` is None on failure.

    A nonzero exit is a failure even when the summary parsed cleanly: the
    exporter prints these counters before it finalises the Parquet files, so a
    run that dies writing them has already printed `Decode errors: 0`.
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


def _export_one(exe: Path, replay: Path) -> tuple[str, dict[str, int] | None, str]:
    """Export one replay to a scratch dir and return its overlay counters."""
    out = Path(tempfile.mkdtemp(prefix="vrfkit-decode-"))
    try:
        r = subprocess.run(
            [str(exe), "export", str(replay), "--out", str(out)],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=600,
        )
        counters, err = read_counters((r.stdout or "") + (r.stderr or ""),
                                      r.returncode)
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

    print(f"exporting {len(files)} replays {args.jobs}-wide to read the overlay counters")
    started = time.time()
    unreadable: list[tuple[str, str]] = []
    offenders: list[tuple[str, int]] = []
    blob_offenders: list[tuple[str, int]] = []
    totals = {"decode_errors": 0, "decoded_ok": 0, "raw_skip": 0,
              "not_in_table": 0, "rows_offered": 0,
              "struct_blobs_decoded": 0, "struct_blobs_failed": 0}
    done = 0

    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for name, counters, err in pool.map(
            lambda f: _export_one(args.exe, f), files
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
            if done % 25 == 0 or done == len(files):
                print(f"  [{done}/{len(files)}] unreadable={len(unreadable)} "
                      f"with_errors={len(offenders)} "
                      f"blob_failures={len(blob_offenders)}")

    elapsed = time.time() - started
    print(f"\nelapsed {elapsed:.1f}s ({elapsed / len(files):.2f}s per replay)")
    print(f"replays read      : {len(files) - len(unreadable)}/{len(files)}")
    print(f"decode errors     : {totals['decode_errors']:,}")
    print(f"decoded OK        : {totals['decoded_ok']:,}")
    print(f"raw/skip          : {totals['raw_skip']:,}")
    print(f"not in table      : {totals['not_in_table']:,}")
    print(f"rows offered      : {totals['rows_offered']:,}")
    print(f"struct blobs      : {totals['struct_blobs_decoded']:,} decoded / "
          f"{totals['struct_blobs_failed']:,} failed")

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
    dead = dead_counters(totals)
    if dead:
        print(f"\nFAILED: {len(dead)} counter(s) never moved, so the clean "
              f"error counters beside them are vacuous", file=sys.stderr)
        for line in dead:
            print(f"    {line}", file=sys.stderr)
        return 1

    print(f"\nOK: {len(files)} replays reported Decode errors: 0 and 0 "
          f"struct-blob failures, over {totals['decoded_ok']:,} decoded rows "
          f"and {totals['struct_blobs_decoded']:,} decoded struct blobs")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
