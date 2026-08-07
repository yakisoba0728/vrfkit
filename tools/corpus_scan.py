"""Shared `.vrf` discovery for `validate_corpus.py` and
`check_decode_errors_corpus.py`.

The defect this closes: pointed at the same directory, `validate_corpus.py`
walked it with `root.rglob("*.vrf")` and `check_decode_errors_corpus.py`
walked it with `args.corpus.glob("*.vrf")` -- 153 files against 126. The
27-file gap lived in a `Demos/old` subdirectory, and those 27 replays had
their framing checked (by the recursive tool) but never their overlay or
struct-blob decoders (by the non-recursive one). Nothing printed that the two
counts disagreed; an auditor had to notice it by hand and run the missing 27
separately.

Neither glob was wrong on its own -- the defect was that a reader comparing
the two tools' output could not tell they had scanned different sets. This
module is the fix: both tools now ask this one function what "the corpus" is,
so they cannot silently drift apart on the answer again.

**The default is non-recursive**, deliberately, even though that narrows
`validate_corpus.py`'s prior behaviour (it used to walk `Demos/old` and, in
the incident above, that is exactly how the 27 files were noticed missing
from the other tool -- this trades that framing coverage away). A
subdirectory is not guaranteed to hold more of the same corpus: `Demos/old`
is where the live VALORANT client archives replays it is about to rotate
out, which may span a build boundary, and a preserved corpus could just as
easily carry an `archive/` or `duplicates/` folder nobody meant to include in
a sweep. CONTRIBUTING.md already tells contributors not to point a *baseline*
at the live Demos folder for this reason; defaulting a *sweep* to recurse into
it silently would reintroduce the same risk one level down. `--recursive`
turns the choice into something visible on the command line, on both tools at
once, rather than something that depends on which glob call a given script
happens to use.

Because the trade must never be silent either, `discover()` always reports
`excluded` -- the number of `.vrf` files that exist under `root`, in
subdirectories, that a non-recursive scan will not touch. It is computed
whether or not the caller asked for it, and callers print it unconditionally,
zero included: a line that only appears when `excluded > 0` cannot tell "nothing
was left out" from "this code stopped checking", which is the exact failure
mode this whole toolset exists to avoid (see `CLAUDE.md`).

`limit`, where a caller applies one, is the caller's job, applied strictly
after `discover()` returns -- so `excluded` always means "invisible to this
run because of the recursion setting", never "left out because of a caller's
`--limit`, and blamed on the wrong knob".
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class CorpusScan:
    """What one discovery call found, and enough about how it looked to make
    the scope legible without re-reading the code that produced it."""

    files: list[Path]
    scanned_root: Path
    recursive: bool
    #: `.vrf` files under `scanned_root`, in subdirectories, NOT included in
    #: `files` because `recursive` was False. Always 0 when `recursive` is True.
    excluded: int


def discover(root: Path, recursive: bool) -> CorpusScan:
    """Find the `.vrf` files that make up "the corpus" rooted at `root`.

    Always computes the recursive count so `excluded` is a real number, not a
    guess -- the cost is one extra `rglob` on the non-recursive path, which is
    negligible next to the `vrfkit` subprocess each file goes on to cost.
    """
    top = sorted(root.glob("*.vrf"))
    if recursive:
        everything = sorted(root.rglob("*.vrf"))
        return CorpusScan(files=everything, scanned_root=root, recursive=True,
                          excluded=0)
    everything = sorted(root.rglob("*.vrf"))
    excluded = len(everything) - len(top)
    return CorpusScan(files=top, scanned_root=root, recursive=False,
                      excluded=excluded)


def scope_line(scan: CorpusScan) -> str:
    """One line that states the corpus scope from the printed output alone.

    Printed unconditionally by callers, `excluded=0` included -- see the
    module docstring for why a conditional line here would reintroduce the
    defect this module exists to close.
    """
    mode = "recursive" if scan.recursive else "top-level only"
    line = (f"corpus scope: {len(scan.files)} .vrf file(s) under "
            f"{scan.scanned_root} ({mode}); {scan.excluded} more in "
            f"subdirectories excluded")
    if not scan.recursive:
        line += " (pass --recursive to include)"
    return line
