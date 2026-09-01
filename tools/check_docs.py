"""Assert that the prose docs still describe THIS repo.

Documentation in this project goes stale in a specific, repeatable way: a
number that was measured once gets quoted forever. The workspace test count
alone has been wrong six times (238, 246, 249, 252, 257, 287 all outlived their
truth), the overlay table size was quoted as 1,185 after it became 1,187, and
README listed four 13.02 replays for weeks after the game deleted them.

None of that is caught by any other check, because a stale sentence compiles
and passes every test. So this reads the repo and the docs and compares:

  1. every tools/*.py script is mentioned in USAGE -- an unmentioned tool is
     an undiscoverable one, and 19 of them existed with no reference page
  2. every script USAGE names actually exists
  3. every crate has a row in the layer table
  4. every relative link resolves
  5. the overlay table sizes quoted are the live ones
  6. no quoted table size in any of `ALL_DOCS` is stale, even beside a live
     one -- presence alone was not enough, see `stale_table_size_claims`
  7. no Rust doc comment or Cargo.toml quotes a stale one
  8. the test counts quoted are the live ones, and no stale count sits beside
     a live one -- presence alone was not enough, see `stale_test_counts`
  9. no quoted `check_ascii` file count or correction count is stale, in any
     of `ALL_DOCS` -- see `stale_measured_counts`, and a count that could not
     be MEASURED is reported rather than skipped
 10. every relative link resolves in `docs/DATA.md`, `CONTRIBUTING.md` and
     `CLAUDE.md` too
 11. generated-file inventories include every live target and generator
 12. README and USAGE export rows/bytes match the committed baseline JSON
 13. the five overlay buckets in the committed baseline still partition
     `overlay_rows_offered` exactly, and every counter the docs quote is still
     present in it -- see `overlay_partition_problems`
 14. no quoted overlay counter or `Typed` ratio in any of `ALL_DOCS` is stale,
     against that same baseline -- see `stale_overlay_counters`
 15. README still carries the overlay summary block at all, so (14) cannot be
     satisfied by deleting it -- see `check_overlay_counters_present`

(6) is (5) upgraded the way (8) was: (5) asks only whether the live number
appears somewhere in README and USAGE, so a stale size could sit one line from
the correct one and be excused by it -- exactly how `387 tests` and
`355 passing` coexisted for twelve commits.

(13)-(15) are (12) extended to the same file's `counters`. (12) compared the
parquet row/byte table against `tools/baselines/export_02d4d478.json` and stopped
there, so README's overlay summary block -- `Decoded OK`, `Raw/Skip`, `Not in
table`, `No field name`, `Typed` -- was an older snapshot of the same replay,
partitioning the same 988,983 rows differently, contradicting the baseline this
repo commits, with nothing reading it. All of it is derivable from that committed
JSON, so none of these needs a `.vrf`.

(9) and (10) exist because this file used to read exactly two documents. Every
number in `docs/DATA.md` -- the most number-dense file in the repo -- and in
`CONTRIBUTING.md` was unguarded, and two counts rotted *inside* the two files
it did read: the ASCII sweep said 114 files against a live 115, and USAGE.md
managed to say 85, 86 and 49 corrections at once. Reading a file is not the
same as checking a number in it.

What (9) deliberately does not do is guard `docs/DATA.md`'s measurements --
"377,487 elements", "1,021 windows". Those come from analysis runs, not from
anything this can execute, so a check would either be a second copy of the
number or a day of work. The rule is narrower: a number is guarded here when
something in the repo can be *run* to produce it.

It runs the test suites to get (8), so it is not free -- roughly the cost of
`cargo test` plus the tools suite. Run it when touching docs, or before
calling a session finished.

**CI runs `--fast`, so (8) does not run there** and cannot: the Python job is
Ubuntu-only by design (the Rust job needs Windows for the Oodle FFI), and (8)
shells out to `cargo test`. Check (8) is a local gate, not an enforced one --
which is precisely how `355 passing` survived twelve commits next to a correct
`387 tests`. Run the full guard by hand before finishing a session.

Usage:
    python tools/check_docs.py
    python tools/check_docs.py --fast     # skip (8), no test runs
"""
from __future__ import annotations

import argparse
import json
import re
import importlib.util
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
README = REPO / "README.md"
USAGE = REPO / "docs" / "USAGE.md"

GENERATED_INVENTORY = {
    "crates/vrf-decode/src/table.rs": "tools/extract_descriptors.py",
    "crates/vrf-decode/src/checksum_table.rs": "tools/extract_checksum_types.py",
    "crates/vrf-transform/src/sbox.rs": "tools/extract_sboxes.py",
    "crates/vrf-transform/tests/data/golden_vectors.rs": "tools/extract_golden.py",
    "tools/equippable_table.py": "tools/extract_equippables.py",
}
GENERATED_INVENTORY_DOCS = (
    "README.md",
    "CONTRIBUTING.md",
    ".github/PULL_REQUEST_TEMPLATE.md",
)

#: Named in the docs but not shipped here.
EXTERNAL_SCRIPTS = {"compute_metrics.py", "python_interop.py"}

LINK_RE = re.compile(r"\[`?([^\]]+?)`?\]\(([^)]+)\)")
SCRIPT_RE = re.compile(r"`?([a-z_][a-z0-9_]*\.py)`?")


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def check_tools(usage: str) -> list[str]:
    """Every shipped tool is documented, and every documented tool ships."""
    problems = []
    shipped = {p.name for p in (REPO / "tools").glob("*.py")}
    named = {n for n in SCRIPT_RE.findall(usage) if not n.startswith("test_")}

    for name in sorted(shipped - named):
        problems.append(f"tools/{name} exists but docs/USAGE.md never mentions it")
    for name in sorted(named - shipped - EXTERNAL_SCRIPTS):
        problems.append(f"docs/USAGE.md names tools/{name}, which does not exist")
    return problems


def check_crates(usage: str) -> list[str]:
    crates = {p.parent.name for p in (REPO / "crates").glob("*/Cargo.toml")}
    return [f"crate {c} has no row in the docs/USAGE.md layer table"
            for c in sorted(crates) if f"`{c}`" not in usage]


def check_links(path: Path, text: str) -> list[str]:
    problems = []
    for _label, target in LINK_RE.findall(text):
        if target.startswith(("http://", "https://", "#", "mailto:")):
            continue
        target = target.split("#", 1)[0]
        if target and not (path.parent / target).exists():
            problems.append(f"{path.name}: link -> {target} does not resolve")
    return problems


def table_lengths() -> tuple[str, str] | None:
    table = read(REPO / "crates" / "vrf-decode" / "src" / "table.rs")
    entries = re.search(r"OVERLAY_TABLE: \[OverlayEntry; (\d+)\]", table)
    handles = re.search(r"OVERLAY_HANDLE_TABLE: \[OverlayHandleEntry; (\d+)\]", table)
    return (entries.group(1), handles.group(1)) if entries and handles else None


def check_table_sizes(docs: dict[str, str]) -> list[str]:
    """The generated overlay table's declared lengths, as quoted in prose."""
    lengths = table_lengths()
    if lengths is None:
        return ["table.rs: could not read the declared slice lengths"]

    problems = []
    for n, what, where in ((lengths[0], "overlay table", ("README.md", "USAGE.md")),
                           (lengths[1], "handle table", ("USAGE.md",))):
        pretty = f"{int(n):,}"
        for name in where:
            if n not in docs[name] and pretty not in docs[name]:
                problems.append(f"{name}: {what} is {pretty}, not quoted")
    return problems


#: How prose states the two generated table sizes, narrow enough that a match
#: is always that claim. `check_table_sizes` asks whether the live number
#: appears SOMEWHERE in the file -- the membership test README defeated by
#: carrying `387 tests` and `355 passing` at once. These ask the stronger
#: question `stale_test_counts` already asks of the suite sizes: every number
#: that claims to BE a table size must be the live one, so a stale figure
#: cannot sit one line away from the correct one.
TABLE_CLAIM_RE = (
    ("overlay table", 0, re.compile(r"(\d[\d,]*)\s+entries\b")),
    ("handle table", 1, re.compile(r"(\d[\d,]*)\s+handles\b")),
)


def stale_table_size_claims(docs: dict[str, str], lengths) -> list[str]:
    """Every quoted table size, in any doc, that is not the live one."""
    if lengths is None:
        return ["table.rs: could not read the declared slice lengths"]
    return [
        f"{name}:{i}: says {quoted} {what.split()[0]} entries/handles, but the "
        f"{what} holds {int(lengths[index]):,}"
        for name, text in docs.items()
        for i, line in enumerate(text.splitlines(), 1)
        for what, index, pattern in TABLE_CLAIM_RE
        for quoted in pattern.findall(line)
        if quoted.replace(",", "") != lengths[index]
    ]


#: The phrase Rust doc comments and Cargo.toml use for the table's size. Kept
#: to this exact wording rather than any "N entries" -- narrow enough that a
#: match is always a size claim, so the check has no judgement to make.
ENTRY_PHRASE_RE = re.compile(r"([\d,]+)-entry (?:generated )?table")


def stale_entry_phrases(text: str, live: set[str]) -> list[tuple[int, str]]:
    """`(line number, quoted size)` for every table-size claim not in `live`.

    Split out from the file walk so it can be tested on a string. `live` holds
    both spellings of the same number -- 1188 and 1,188 are the same claim.
    """
    return [(i, quoted)
            for i, line in enumerate(text.splitlines(), 1)
            for quoted in ENTRY_PHRASE_RE.findall(line)
            if quoted not in live]


def check_source_table_size() -> list[str]:
    """Rust prose quotes the table size too, and nothing was reading it.

    `check_table_sizes` covers README and USAGE. The same number is also written
    into `vrf-decode`'s crate docs, its feature table and its Cargo.toml, and all
    three still said 1,185 after the table reached 1,188 -- the exact rot this
    file exists to catch, one directory over from where it was looking.
    """
    lengths = table_lengths()
    if lengths is None:
        return []
    n = int(lengths[0])
    live = {lengths[0], f"{n:,}"}

    sources = sorted((REPO / "crates").rglob("*.rs"))
    sources += sorted((REPO / "crates").rglob("Cargo.toml"))
    return [f"{path.relative_to(REPO).as_posix()}:{i}: says {quoted}-entry "
            f"table; it is {n:,}"
            for path in sources
            for i, quoted in stale_entry_phrases(read(path), live)]


#: The phrase the docs use to state a suite size. Narrow enough that a match is
#: always a claim about one of the two suites, so the check has no judgement to
#: make -- the same bargain `ENTRY_PHRASE_RE` strikes one check up.
TEST_COUNT_RE = re.compile(r"(\d[\d,]*)\s+(?:tests|passing)\b")


def stale_test_counts(text: str, live: set[str]) -> list[tuple[int, str]]:
    """`(line number, quoted count)` for every suite-size claim not in `live`.

    Asking whether the live number appears *somewhere* is not enough: README
    carried `387 tests` and `355 passing` at once and satisfied that check with
    the first while the second rotted. `live` holds both suite counts in both
    spellings, and every claim must be one of them.
    """
    return [(i, quoted)
            for i, line in enumerate(text.splitlines(), 1)
            for quoted in TEST_COUNT_RE.findall(line)
            if quoted not in live]


def contradicting_test_counts(docs: dict[str, str]) -> list[str]:
    """Suite-size claims that cannot all be true at once.

    `stale_test_counts` needs the real numbers, so it only runs in the full
    mode -- which CI cannot use, because that mode shells out to `cargo test`
    and the Python job is Ubuntu-only for the Oodle split. This is the part of
    the same check that survives `--fast`, and therefore the part CI can run.

    It cannot know which number is right. It does not have to: the repo has
    exactly two suites, so a third distinct value is a contradiction on its
    face. That is the shape the real bug had -- 387 and 355 in one file, both
    about `cargo test` -- and it went twelve commits unnoticed.

    Blind to a count that is wrong in the same way everywhere; only the full
    mode catches that.
    """
    seen: list[tuple[str, int, str]] = [
        (name, i, quoted)
        for name, text in docs.items()
        for i, line in enumerate(text.splitlines(), 1)
        for quoted in TEST_COUNT_RE.findall(line)
    ]
    distinct = {quoted.replace(",", "") for _, _, quoted in seen}
    if len(distinct) <= 2:
        return []
    sites = ", ".join(f"{name}:{i} says {quoted}" for name, i, quoted in seen)
    return [
        f"{len(distinct)} different test counts claimed but there are two "
        f"suites, so at least one is stale -- {sites}"
    ]


#: Every doc this guard reads. README and USAGE must *quote* the live numbers;
#: the rest only have to not contradict them. `docs/DATA.md` was outside this
#: set entirely -- the most number-dense file in the repo, with no link check,
#: no size check and no count check -- and `CONTRIBUTING.md` names the suites a
#: contributor is told to run. `CLAUDE.md` carries a relative link nothing was
#: checking; it quotes no counts by design ("counts are omitted on purpose"),
#: so joining this tier -- not the quote-it tier -- imposes no new obligation.
ALL_DOCS = ("README.md", "docs/USAGE.md", "docs/DATA.md", "CONTRIBUTING.md",
            "CLAUDE.md")

#: Numbers that are quoted in prose *and* produced by something runnable, with
#: the phrasing narrow enough that a match is always that claim. All three of
#: these rotted while sitting in files this guard already read.
#: `(count pattern, context, context window)`. The context is what keeps "files"
#: from meaning the corpus: README says "all 215 files" about replays two lines
#: apart from nothing to do with ASCII. A line must name the check to be read as
#: claiming its count.
#:
#: The window is how many lines ABOVE the count the context may appear on, and 0
#: -- same line only -- is the default every entry had when this was a pair. It
#: exists because prose wraps: USAGE.md writes "The `ADDITIONS` pass ... There
#: are" and then "currently 73 of them" on the next line, so a same-line context
#: matched nothing and the count went unchecked while the guard still printed
#: "OK: the docs still describe this repo". Widening the *pattern* instead would
#: have been the other way to fix that, and the wrong one: "currently N of them"
#: with no context is a generic English phrase that some future paragraph will
#: use about something else.
MEASURED_RE = {
    "ascii": (re.compile(r"(\d+) files?\b"), re.compile(r"ascii", re.I), 0),
    "corrections": (re.compile(r"(\d+) corrections"), None, 0),
    # `ADDITIONS` is the descriptor-silent subset of the corrections list, and
    # it rotted exactly the way `corrections` did -- USAGE said 70 while the
    # list held 73. `expectation_count` was guarded; this was not.
    #
    # The context is what makes the phrasing safe to read as this claim.
    # "currently N of them" is not a sentence anything else in these docs
    # writes, but it is generic on its own, so a line only counts when it also
    # names ADDITIONS. That pairing is the same bargain the `ascii` row strikes
    # to keep "all 215 files" from being read as the ASCII sweep's count.
    "additions": (re.compile(r"currently (\d+) of them"),
                  re.compile(r"ADDITIONS"), 2),
}


def check_generated_inventory(docs: dict[str, str]) -> list[str]:
    """Every live generated target is named wherever contributors check it."""
    problems = []
    for name, text in docs.items():
        for target, generator in GENERATED_INVENTORY.items():
            target_claim = Path(target).name if name.endswith("PULL_REQUEST_TEMPLATE.md") else target
            if f"`{target_claim}`" not in text:
                problems.append(f"{name}: generated inventory is missing `{target_claim}`")
            if (
                not name.endswith("PULL_REQUEST_TEMPLATE.md")
                and f"`{generator}`" not in text
            ):
                problems.append(f"{name}: generated inventory is missing `{generator}`")
    return problems


def baseline_table_figures() -> dict[str, tuple[int, int]]:
    """Rows and bytes promised by the committed reference export baselines."""
    export = json.loads(read(REPO / "tools" / "baselines" / "export_02d4d478.json"))
    checkpoint = json.loads(
        read(REPO / "tools" / "baselines" / "checkpoint_02d4d478.json")
    )
    figures = {
        f"{name}.parquet": (int(values["rows"]), int(values["bytes"]))
        for name, values in export["parquet"].items()
    }
    cp = checkpoint["parquet"]["checkpoint_fields"]
    figures["checkpoint_fields.parquet"] = (int(cp["rows"]), int(cp["bytes"]))
    return figures


def format_baseline_table(figures: dict[str, tuple[int, int]]) -> str:
    """Canonical Markdown rows, also useful to migration/error tooling."""
    return "\n".join(
        f"| `{name}` | {rows:,} | {size:,} |"
        for name, (rows, size) in figures.items()
    )


def check_baseline_figures(
    docs: dict[str, str], figures: dict[str, tuple[int, int]]
) -> list[str]:
    """Every measured export row in active docs must match the live baseline."""
    problems = []
    for doc_name, text in docs.items():
        for table_name, expected in figures.items():
            pattern = re.compile(
                rf"^\|\s*`?{re.escape(table_name)}`?\s*\|"
                rf"\s*([\d,]+)\s*\|\s*([\d,]+)\s*\|",
                re.MULTILINE,
            )
            matches = pattern.findall(text)
            if not matches:
                problems.append(
                    f"{doc_name}: measured export table is missing {table_name}"
                )
                continue
            for quoted_rows, quoted_bytes in matches:
                actual = (
                    int(quoted_rows.replace(",", "")),
                    int(quoted_bytes.replace(",", "")),
                )
                if actual != expected:
                    problems.append(
                        f"{doc_name}: {table_name} says {actual[0]:,} rows / "
                        f"{actual[1]:,} bytes, baseline says {expected[0]:,} / "
                        f"{expected[1]:,}"
                    )
    return problems


#: The overlay summary block README reprints, mapped to the baseline counter
#: each line quotes. `vrfkit export` prints these; the committed baseline
#: `tools/baselines/export_02d4d478.json` records them for the reference replay,
#: so every one of them is checkable with no `.vrf` on disk -- which matters,
#: because CI has none and neither does any machine without the private corpus.
#:
#: `check_baseline_figures` already compares the parquet row/byte table against
#: this same file. It did not read these, and they drifted: the block was an
#: older snapshot that partitioned the same 988,983 rows differently, taken
#: before overlay entries moved rows out of `Not in table`. It contradicted the
#: baseline this repo commits for the same replay and nothing said so.
#:
#: Keyed by the label as printed, so a match is always that counter.
#:
#: `Decode errors` is deliberately NOT here, though the same block prints it and
#: the baseline records it. The string "Decode errors: 0" is used across this
#: repo as the NAME of a failure mode rather than as a measurement of this
#: replay -- CLAUDE.md twice ("means the decoder did not throw"), docs/DATA.md
#: once, and docs/USAGE.md as an annotated illustration of what to watch. All
#: four are correct English about the general case and none is a claim about
#: `export_02d4d478.json`. Guarding it would fire on every one of them the first
#: time the baseline records a nonzero value, which is a guard that gets deleted
#: rather than fixed -- and the counter it would protect is the one the repo's
#: own doctrine says proves the least ("Decode errors: 0 means the decoder did
#: not throw. It does not mean the values are right."). The five it does guard
#: are the ones that only ever appear as this replay's measured figures.
OVERLAY_COUNTER_KEYS = {
    "Decoded OK": "overlay_decoded_ok",
    "Raw/Skip": "overlay_raw_skip",
    "Not in table": "overlay_not_in_table",
    "No field name": "overlay_no_field_name",
    "Effect blobs": "effect_blobs_decoded",
}

#: The five buckets that partition every row offered to the overlay -- see
#: `print_overlay` in crates/vrfkit/src/driver/summary.rs, which sums exactly
#: these five (`decoded_ok + decoded_err + raw_or_skip + not_in_table +
#: no_field_name`) to print `Rows offered`. `overlay_decode_errors` belongs
#: here even though `OVERLAY_COUNTER_KEYS` above deliberately excludes it: that
#: exclusion is about not flagging CLAUDE.md's generic "Decode errors: 0" text
#: as stale, which has nothing to do with whether the five buckets actually
#: sum to the total. Their sum is `overlay_rows_offered` exactly -- not
#: approximately -- so the relationship is checkable arithmetic rather than
#: six independent equalities. A future baseline that breaks it means either a
#: bucket was added or one of these stopped counting, and both are worth a red
#: build. Dropping `overlay_decode_errors` from this tuple made the check pass
#: only because the pinned baseline's decode-error count happens to be 0.
OVERLAY_PARTITION = ("overlay_decoded_ok", "overlay_decode_errors",
                     "overlay_raw_skip", "overlay_not_in_table",
                     "overlay_no_field_name")

#: `Typed` is not stored; it is `Decoded OK / Rows offered` as a percentage, and
#: it is quoted in both README and USAGE. Derived rather than pinned, so it
#: cannot drift away from the two counters it is a ratio of.
TYPED_RE = re.compile(r"Typed:\s*([\d.]+)%")


def baseline_overlay_counters() -> dict[str, int]:
    """The overlay counters the committed reference export recorded."""
    export = json.loads(read(REPO / "tools" / "baselines" / "export_02d4d478.json"))
    return {k: int(v) for k, v in export["counters"].items()}


def overlay_partition_problems(counters: dict[str, int]) -> list[str]:
    """The buckets must still add up, and the keys must still be there.

    A missing key would otherwise make every quoted line unverifiable while
    `stale_overlay_counters` skipped it and this file printed OK -- the same
    hole `measured_counts` had for the ascii count.
    """
    problems = []
    needed = set(OVERLAY_PARTITION) | {"overlay_rows_offered"} | set(
        OVERLAY_COUNTER_KEYS.values())
    for key in sorted(needed - counters.keys()):
        problems.append(
            f"export_02d4d478.json: counters.{key} is missing, so every doc "
            f"line quoting it went unchecked")
    if needed - counters.keys():
        return problems

    total = sum(counters[k] for k in OVERLAY_PARTITION)
    offered = counters["overlay_rows_offered"]
    if total != offered:
        problems.append(
            f"export_02d4d478.json: the five overlay buckets sum to {total:,} "
            f"but counters.overlay_rows_offered is {offered:,}; they are "
            f"documented as a partition of every row offered "
            f"({' + '.join(k.removeprefix('overlay_') for k in OVERLAY_PARTITION)})")
    return problems


def stale_overlay_counters(docs: dict[str, str],
                           counters: dict[str, int]) -> list[str]:
    """Every quoted overlay counter, in any doc, that is not the live one.

    The stronger question `stale_measured_counts` asks, for the same reason: a
    stale figure must not be excused by a correct one nearby. Scoped to the
    printed `Label: N` form, so prose that merely names a bucket -- README's
    "most of `Not in table` is RPC parameters" -- is not read as quoting it.
    """
    problems = []
    for name, text in docs.items():
        for i, line in enumerate(text.splitlines(), 1):
            for label, key in OVERLAY_COUNTER_KEYS.items():
                if key not in counters:
                    continue
                for quoted in re.findall(
                        rf"{re.escape(label)}:\s*([\d,]+)", line):
                    if int(quoted.replace(",", "")) != counters[key]:
                        problems.append(
                            f"{name}:{i}: says {label} {quoted}, but "
                            f"counters.{key} is {counters[key]:,}")
            if "overlay_decoded_ok" not in counters:
                continue
            for quoted in TYPED_RE.findall(line):
                # Compared at the precision the doc chose, so 75.1 and 75.10
                # are the same claim and 75.2 is not.
                places = len(quoted.partition(".")[2])
                live = round(
                    100 * counters["overlay_decoded_ok"]
                    / counters["overlay_rows_offered"], places)
                if float(quoted) != live:
                    problems.append(
                        f"{name}:{i}: says Typed {quoted}%, but "
                        f"overlay_decoded_ok / overlay_rows_offered is "
                        f"{live}% ({counters['overlay_decoded_ok']:,} / "
                        f"{counters['overlay_rows_offered']:,})")
    return problems


def check_overlay_counters_present(readme: str,
                                   counters: dict[str, int]) -> list[str]:
    """README must still carry the block, not merely not contradict it.

    Without this, deleting the summary block would satisfy
    `stale_overlay_counters` perfectly -- nothing quoted, nothing wrong -- and
    the guard would go on reporting that it checked something.
    """
    return [f"README.md: overlay summary block is missing `{label}:`"
            for label, key in OVERLAY_COUNTER_KEYS.items()
            if key in counters
            and not re.search(rf"{re.escape(label)}:\s*[\d,]+", readme)]


def measured_counts(problems: list[str] | None = None) -> dict[str, int]:
    """The live values, read from the things that produce them.

    A measurement that could not be taken is left out of the returned dict --
    and `stale_measured_counts` skips any key it does not find, so an unmeasured
    count silently checked nothing while the guard still printed "OK: the docs
    still describe this repo". Pass `problems` to hear about that instead.
    """
    counts = {}
    r = subprocess.run(["git", "-C", str(REPO), "ls-files", "--", "*.rs"],
                       capture_output=True, text=True, encoding="utf-8",
                       errors="replace", timeout=120)
    if r.returncode == 0:
        counts["ascii"] = len([ln for ln in r.stdout.splitlines() if ln.strip()])
    elif problems is not None:
        problems.append(
            f"could not measure the ascii file count: git ls-files exited "
            f"{r.returncode} ({(r.stderr or '').strip()[:120]}); every quoted "
            f"count went unchecked")

    # `tools/` on the path first: apply_type_corrections.py imports `atomic_io`
    # from beside itself, and under `spec_from_file_location` that import is
    # resolved against sys.path, not against the file's own directory. Without
    # this the exec_module below raises ModuleNotFoundError.
    if str(REPO / "tools") not in sys.path:
        sys.path.insert(0, str(REPO / "tools"))
    spec = importlib.util.spec_from_file_location(
        "_atc", REPO / "tools" / "apply_type_corrections.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    counts["corrections"] = module.expectation_count(read(module.TABLE_RS))
    # Measured by importing the module, not by counting source lines -- the
    # list spans a commented block per entry, so any line-counting heuristic
    # would be a second thing to keep in step with it.
    counts["additions"] = len(module.ADDITIONS)
    return counts


def stale_measured_counts(docs: dict[str, str], live: dict[str, int]) -> list[str]:
    """Every quoted measured count that is not the live one.

    Not "does the right number appear somewhere" -- that is the check that let
    README hold 387 and 355 at once. Every match must be right, so a file
    saying 85, 86 and 49 corrections reports two problems, not zero.

    A count is only read as a claim when its context appears on the same line or
    within `window` lines above it -- see `MEASURED_RE` for why a window exists
    at all.
    """
    problems = []
    for name, text in docs.items():
        lines = text.splitlines()
        for i, line in enumerate(lines, 1):
            for what, (pattern, context, window) in MEASURED_RE.items():
                if what not in live:
                    continue
                if context is not None:
                    scope = lines[max(0, i - 1 - window):i]
                    if not any(context.search(ln) for ln in scope):
                        continue
                for quoted in pattern.findall(line):
                    if int(quoted) != live[what]:
                        problems.append(
                            f"{name}:{i}: says {quoted} {what}, but it is "
                            f"{live[what]}")
    return problems


def measure_tests() -> tuple[int, int, list[str]]:
    problems = []
    r = subprocess.run(["cargo", "test", "--quiet"], cwd=REPO, capture_output=True,
                       text=True, encoding="utf-8", errors="replace", timeout=3600)
    out = (r.stdout or "") + (r.stderr or "")
    passed_matches = re.findall(r"(\d+) passed", out)
    rust = sum(int(m) for m in passed_matches)
    if r.returncode != 0:
        problems.append("cargo test did not pass; doc counts not checked against it")
    elif not passed_matches:
        # `measured_counts` above already carries this rule for the ASCII
        # count: a parse failure defaulting to 0 is indistinguishable from a
        # genuinely empty suite, and everything downstream (`live`, the
        # stale-count report) then treats every doc-quoted number as wrong for
        # the wrong reason. `cargo test` exiting 0 with no "N passed" line
        # means its output format changed, not that nothing ran.
        problems.append(
            "cargo test exited 0 but printed no 'N passed' line; the rust "
            "test count (0) was not measured")

    r2 = subprocess.run([sys.executable, "-m", "unittest", "discover",
                         "-s", "tools/tests", "-p", "test_*.py"],
                        cwd=REPO, capture_output=True, text=True,
                        encoding="utf-8", errors="replace", timeout=1800)
    out2 = (r2.stdout or "") + (r2.stderr or "")
    m = re.search(r"Ran (\d+) tests", out2)
    tools_n = int(m.group(1)) if m else 0
    if r2.returncode != 0:
        problems.append("tools test suite did not pass")
    elif m is None:
        problems.append(
            "the tools test suite exited 0 but printed no 'Ran N tests' "
            "line; the tools test count (0) was not measured")
    return rust, tools_n, problems


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--fast", action="store_true",
                    help="skip the test-count check (does not run the suites)")
    args = ap.parse_args()

    if not USAGE.is_file():
        print(f"missing: {USAGE}", file=sys.stderr)
        return 2

    readme, usage = read(README), read(USAGE)
    docs = {"README.md": readme, "USAGE.md": usage}
    #: Every doc, for the checks that only ask a number not to be wrong.
    #: `docs` stays README+USAGE for the ones that require a number to be
    #: *present*: DATA.md has no reason to quote the suite sizes.
    every = {name: read(REPO / name) for name in ALL_DOCS}
    generated_docs = {
        name: read(REPO / name) for name in GENERATED_INVENTORY_DOCS
    }

    measurement_problems: list[str] = []
    live_counts = measured_counts(measurement_problems)
    overlay_counters = baseline_overlay_counters()

    problems = (
        check_tools(usage)
        + check_crates(usage)
        + check_links(README, readme)
        + check_links(USAGE, usage)
        + check_table_sizes(docs)
        + stale_table_size_claims(every, table_lengths())
        + check_source_table_size()
        + contradicting_test_counts(every)
        + measurement_problems
        + stale_measured_counts(every, live_counts)
        + check_generated_inventory(generated_docs)
        + check_baseline_figures(docs, baseline_table_figures())
        + overlay_partition_problems(overlay_counters)
        + stale_overlay_counters(every, overlay_counters)
        + check_overlay_counters_present(readme, overlay_counters)
        + [p for name in ALL_DOCS
           for p in check_links(REPO / name, every[name])
           if name not in ("README.md", "docs/USAGE.md")]
    )

    checked = 15
    if not args.fast:
        rust, tools_n, run_problems = measure_tests()
        problems += run_problems
        for count, label in ((rust, "rust"), (tools_n, "tools")):
            for name, text in docs.items():
                if str(count) not in text:
                    problems.append(
                        f"{name}: {label} test count is {count}, not quoted")
        live = {s for c in (rust, tools_n) for s in (str(c), f"{c:,}")}
        for name, text in every.items():
            problems += [
                f"{name}:{i}: says {quoted}; the suites are {rust} and {tools_n}"
                for i, quoted in stale_test_counts(text, live)]
        print(f"tests: rust {rust}, tools {tools_n}")
        checked += 1

    n_tools = len(list((REPO / "tools").glob("*.py")))
    n_crates = len({p.parent.name for p in (REPO / "crates").glob("*/Cargo.toml")})
    print(f"docs: {len(ALL_DOCS)} files   "
          f"{n_tools} tools, {n_crates} crates, {checked} checks")

    if problems:
        print(f"\nFAILED: {len(problems)} stale or missing doc claim(s)",
              file=sys.stderr)
        for p in problems:
            print(f"    {p}", file=sys.stderr)
        return 1

    print("\nOK: the docs still describe this repo")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
