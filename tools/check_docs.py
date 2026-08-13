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

(6) is (5) upgraded the way (8) was: (5) asks only whether the live number
appears somewhere in README and USAGE, so a stale size could sit one line from
the correct one and be excused by it -- exactly how `387 tests` and
`355 passing` coexisted for twelve commits.

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
    ("overlay table", 0, re.compile(r"([\d,]+)\s+entries\b")),
    ("handle table", 1, re.compile(r"([\d,]+)\s+handles\b")),
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
#: the phrasing narrow enough that a match is always that claim. Both of these
#: rotted while sitting in files this guard already read.
#: `(count pattern, line context)`. The context is what keeps "files" from
#: meaning the corpus: README says "all 215 files" about replays two lines
#: apart from nothing to do with ASCII. A line must name the check to be read
#: as claiming its count.
MEASURED_RE = {
    "ascii": (re.compile(r"(\d+) files?\b"), re.compile(r"ascii", re.I)),
    "corrections": (re.compile(r"(\d+) corrections"), None),
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

    spec = importlib.util.spec_from_file_location(
        "_atc", REPO / "tools" / "apply_type_corrections.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    counts["corrections"] = module.expectation_count(read(module.TABLE_RS))
    return counts


def stale_measured_counts(docs: dict[str, str], live: dict[str, int]) -> list[str]:
    """Every quoted measured count that is not the live one.

    Not "does the right number appear somewhere" -- that is the check that let
    README hold 387 and 355 at once. Every match must be right, so a file
    saying 85, 86 and 49 corrections reports two problems, not zero.
    """
    return [f"{name}:{i}: says {quoted} {what}, but it is {live[what]}"
            for name, text in docs.items()
            for i, line in enumerate(text.splitlines(), 1)
            for what, (pattern, context) in MEASURED_RE.items() if what in live
            if context is None or context.search(line)
            for quoted in pattern.findall(line)
            if int(quoted) != live[what]]


def measure_tests() -> tuple[int, int, list[str]]:
    problems = []
    r = subprocess.run(["cargo", "test", "--quiet"], cwd=REPO, capture_output=True,
                       text=True, encoding="utf-8", errors="replace", timeout=3600)
    out = (r.stdout or "") + (r.stderr or "")
    rust = sum(int(m) for m in re.findall(r"(\d+) passed", out))
    if r.returncode != 0:
        problems.append("cargo test did not pass; doc counts not checked against it")

    r2 = subprocess.run([sys.executable, "-m", "unittest", "discover",
                         "-s", "tools/tests", "-p", "test_*.py"],
                        cwd=REPO, capture_output=True, text=True,
                        encoding="utf-8", errors="replace", timeout=1800)
    out2 = (r2.stdout or "") + (r2.stderr or "")
    m = re.search(r"Ran (\d+) tests", out2)
    tools_n = int(m.group(1)) if m else 0
    if r2.returncode != 0:
        problems.append("tools test suite did not pass")
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
        + [p for name in ALL_DOCS
           for p in check_links(REPO / name, every[name])
           if name not in ("README.md", "docs/USAGE.md")]
    )

    checked = 12
    if not args.fast:
        rust, tools_n, run_problems = measure_tests()
        problems += run_problems
        for count, label in ((rust, "rust"), (tools_n, "tools")):
            for name, text in docs.items():
                if label == "tools" and name == "README.md":
                    continue  # README does not quote the tools suite
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
