"""Assert that the prose docs still describe THIS repo.

A number measured once gets quoted forever, and a stale sentence compiles and
passes every test. So this reads the repo and the docs and compares:

  1. every tools/*.py script is mentioned in USAGE, and every one it names exists
  2. every crate has a row in USAGE's layer table
  3. README and USAGE quote the live overlay table sizes, and no quoted size
     in `ALL_DOCS`, Rust doc comments or Cargo.toml is stale
  4. README and USAGE quote the live test counts, and no stale count sits
     beside a live one; `--fast` keeps only `contradicting_test_counts`
  5. no `MEASURED_RE` count in `ALL_DOCS` is stale, and a count that could not
     be measured is reported rather than skipped
  6. every relative link in `ALL_DOCS` and docs/*.md resolves
  7. CONTRIBUTING and the PR template name every generated target
  8. USAGE's export rows/bytes match the committed baseline JSON, whose five
     overlay buckets still partition `overlay_rows_offered`; no quoted overlay
     counter or `Typed` ratio is stale, and README still carries the block
  9. README's build table covers exactly the registered payload transforms,
     with the audit's clean/checked counts and one verification method; and
     BUILD_VERIFICATION.md's arithmetic-evidence paragraph counts the
     registered builds, each vector group's builds and its cases per build as
     crates/vrf-transform/tests/data holds them
 10. every `#anchor` a doc links, and every `docs/<name>.md[#<anchor>]` a
     tracked .rs, .py or .json file names, exists by GitHub's slug rules;
     Setext headings are not read, so a link to one is reported

A number is guarded when something in the repo can be *run* to produce it;
DATA.md's measurements come from analysis runs, so they are not. (8) reads
committed JSON, so no `.vrf` is needed. (4) runs `cargo test` and the tools
suite; CI runs the full guard in the Windows MSRV job and `--fast` on the
Python matrix.

Usage:
    python tools/check_docs.py
    python tools/check_docs.py --fast     # skip (4)'s suite runs
"""
from __future__ import annotations

import argparse
import functools
import json
import os
import re
import subprocess
import sys
import unicodedata
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from urllib.parse import unquote

REPO = Path(__file__).resolve().parent.parent
README = REPO / "README.md"
USAGE = REPO / "docs" / "USAGE.md"
EXPORT_BASELINE = REPO / "tools" / "baselines" / "export_02d4d478.json"

GENERATED_INVENTORY = {
    "crates/vrf-decode/src/checksum_table.rs": "tools/extract_checksum_types.py",
    "crates/vrf-decode/src/scoped_types.rs": "tools/generate_scoped_types.py",
    "crates/vrf-transform/tests/data/native_vectors.rs": "tools/capture_native_transforms.py",
}
GENERATED_INVENTORY_DOCS = (
    "CONTRIBUTING.md",
    ".github/PULL_REQUEST_TEMPLATE.md",
)

#: Named in the docs but not shipped here.
EXTERNAL_SCRIPTS = {"compute_metrics.py", "python_interop.py"}

BUILD_AUDIT = REPO / "tools/fixtures/build_verification.json"
BUILD_METHOD = "Validation + checkpoints + typed/raw"
TRANSFORM_VECTORS = REPO / "crates/vrf-transform/tests/data"

LINK_RE = re.compile(r"\[`?([^\]]+?)`?\]\(([^)]+)\)")
SCRIPT_RE = re.compile(r"`?([a-z_][a-z0-9_]*\.py)`?")


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def check_build_verification(readme: str, usage: str, registry: str, report: dict) -> list[str]:
    """README's build table must cover exactly the registered transforms with the
    audit's clean/checked counts and one acceptance rule; USAGE's layer table
    must count the same builds."""
    problems = []
    match = re.search(r"^transforms! \{$(.*?)^\}$", registry, re.S | re.M)
    if not match:
        return ["cannot read supported transform registry"]
    versions = {f"{a}.{b}" for a, b in re.findall(r"\bV(\d\d)(\d\d)\b", match[1])}
    if not versions:
        return ["supported transform registry is empty"]
    measured = {branch.removeprefix("++Ares-Core+release-"): row
                for branch, row in report.get("builds", {}).items()}
    if set(measured) != versions:
        problems.append("build audit does not cover exactly the supported registry")
    if report.get("executable_changed") is not False:
        problems.append("build audit executable changed or its integrity result is absent")
    if report.get("build_errors"):
        problems.append("build audit has unresolved build-level errors")
    for version, row in measured.items():
        counts = [row.get(key) for key in ("replays", "passed", "failed")]
        if (any(type(value) is not int or value < 0 for value in counts)
                or counts[0] == 0 or counts[1] + counts[2] != counts[0]):
            problems.append(f"build audit {version}: invalid replay accounting")
            continue
        hashes = row.get("input_sha256", [])
        if len(hashes) != counts[0] or len(set(hashes)) != counts[0]:
            problems.append(f"build audit {version}: input hashes do not match replay count")
        work = row.get("counts", {})
        checkpoint_counts = [work.get(key) for key in
                             ("checkpoint_content_blocks", "checkpoint_overlay_decoded_ok")]
        if (row.get("checkpoint_evidence") != "observed"
                or any(type(value) is not int or value <= 0 for value in checkpoint_counts)):
            problems.append(f"build audit {version}: no positive checkpoint decoding evidence")
    for quoted in re.findall(r"Payload transform \((\d+) builds\)", readme + usage):
        if int(quoted) != len(versions):
            problems.append(f"transform layer lists {quoted} builds, registry has {len(versions)}")
    rows = {}
    for line in readme.splitlines():
        cells = [cell.strip().replace("**", "") for cell in line.strip().strip("|").split("|")]
        if not cells or not re.fullmatch(r"\d{2}\.\d{2}", cells[0]):
            continue
        version = cells[0]
        if version in rows:
            problems.append(f"README: duplicate build row {version}")
        rows[version] = cells
    if set(rows) != versions:
        problems.append("README: support table differs from supported registry")
    for version in sorted(versions & set(rows) & set(measured)):
        cells, actual = rows[version], measured[version]
        expected = f"{actual['passed']}/{actual['replays']}"
        if len(cells) != 4 or cells[2] != expected:
            problems.append(f"README: {version} clean/checked must be {expected}")
        if cells[-1] != BUILD_METHOD:
            problems.append(f"README: {version} uses a different verification method")
        if cells[1] != f"`release-{version}`":
            problems.append(f"README: {version} branch label differs")
    return problems


NUMBER_WORDS = {word: i for i, word in enumerate(
    "zero one two three four five six seven eight nine ten eleven twelve thirteen fourteen "
    "fifteen sixteen seventeen eighteen nineteen".split())}
NUMBER_WORDS.update({word: 10 * i for i, word in enumerate(
    "twenty thirty forty fifty sixty seventy eighty ninety".split(), 2)})
VERSION_RE = re.compile(r"\d\d\.\d\d")
#: One vector tuple per line, `("++Ares-Core+release-11.06", ...` or `(287, "...`.
BRANCH_TUPLE_RE = re.compile(r'^\s*\("\+\+Ares-Core\+release-(\d\d\.\d\d)",', re.M)
BIT_TUPLE_RE = re.compile(r'^\s*\(\d+,\s*"', re.M)
RECOVERED_RE = re.compile(r"recovered_(\d\d)(\d\d)_vectors\.rs")


def number(text: str) -> int | None:
    """`79`, `eleven` or `twenty-seven` as an int; None for anything else."""
    text = text.lower()
    if text.isdigit():
        return int(text)
    tens, _, unit = text.partition("-")
    if unit:
        return (NUMBER_WORDS[tens] + NUMBER_WORDS[unit]
                if tens in NUMBER_WORDS and NUMBER_WORDS.get(unit, 10) < 10 else None)
    return NUMBER_WORDS.get(text)


def vector_cases(vectors: dict[str, str]) -> dict[str, dict[str, int]]:
    """`{source: {build: cases}}` read off the transform test data files:
    `native` and `golden` by branch, `recovered` by file name."""
    out = {"native": {}, "golden": {}, "recovered": {}}
    for name, text in vectors.items():
        if name in ("native_vectors.rs", "golden_vectors.rs"):
            source = out[name.removesuffix("_vectors.rs")]
            for version in BRANCH_TUPLE_RE.findall(text):
                source[version] = source.get(version, 0) + 1
        elif match := RECOVERED_RE.fullmatch(name):
            out["recovered"][f"{match[1]}.{match[2]}"] = len(BIT_TUPLE_RE.findall(text))
    return out


def check_transform_evidence(build_doc: str, registry: str,
                             vectors: dict[str, str]) -> list[str]:
    """BUILD_VERIFICATION.md's arithmetic-evidence paragraph against the
    registry and the vector files: the total, each group's build count and
    range, and its per-build case count. Every claim must parse; a group
    nobody claims, or a build no group covers, fails."""
    match = re.search(r"^transforms! \{$(.*?)^\}$", registry, re.S | re.M)
    order = sorted({f"{a}.{b}" for a, b in re.findall(r"\bV(\d\d)(\d\d)\b", match[1])}
                   if match else ())
    if not order:
        return ["BUILD_VERIFICATION.md: cannot read the transform registry"]
    section = re.search(r"^## Arithmetic evidence$(.*?)(?=^## |\Z)", build_doc, re.S | re.M)
    if not section:
        return ["BUILD_VERIFICATION.md: no '## Arithmetic evidence' section"]
    text = " ".join(section[1].split())
    problems = []

    total = re.findall(r"cover all (\S+) registered builds", text)
    if len(total) != 1 or number(total[0]) != len(order):
        problems.append(f"BUILD_VERIFICATION.md: transform tests must claim all "
                        f"{len(order)} registered builds, found {total}")

    def expand(listed: str) -> list[str]:
        builds = []
        for first, last in re.findall(r"(\d\d\.\d\d)(?:--(\d\d\.\d\d))?", listed):
            last = last or first
            if first not in order or last not in order:
                problems.append(f"BUILD_VERIFICATION.md: {first}--{last} is not registered")
                continue
            builds += order[order.index(first):order.index(last) + 1]
        return builds

    claims = [("native" if "native" in kind else "golden", count, listed, cases)
              for count, listed, cases, kind in re.findall(
                  r"[Tt]he (\S+) builds ((?:\d\d\.\d\d|--|, | and )+?) have (\S+) "
                  r"([\w -]*?) cases each", text)]
    claims += [("recovered", "one", version, cases) for version, cases in re.findall(
        r"(\d\d\.\d\d) has (\S+) fixed boundary vectors", text)]
    data = vector_cases(vectors)
    covered: list[str] = []
    for source, count, listed, cases in claims:
        builds = expand(listed)
        covered += builds
        where = f"BUILD_VERIFICATION.md: {source} group {listed}"
        if number(count) != len(builds):
            problems.append(f"{where} says {count} builds, lists {len(builds)}")
        # A listed build its source lacks has no count, so it fails here; with
        # every registered build listed exactly once, no vector goes unclaimed.
        wrong = sorted(b for b in builds if data[source].get(b) != number(cases))
        if wrong:
            problems.append(f"{where} says {cases} cases each; {wrong} have "
                            f"{[data[source].get(b) for b in wrong]}")
    if sorted(covered) != order:
        problems.append(f"BUILD_VERIFICATION.md: no group lists "
                        f"{sorted(set(order) - set(covered))}; listed twice: "
                        f"{sorted({b for b in covered if covered.count(b) > 1})}")
    return problems


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
    """The overlay table's declared lengths, as quoted in prose."""
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


#: How prose states the two table sizes, narrow enough that a match is always
#: that claim; every match must be live, so a stale figure cannot hide beside a
#: correct one.
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


#: The phrase Rust doc comments and Cargo.toml use for the table's size, kept
#: this narrow (not any "N entries") so a match is always a size claim.
ENTRY_PHRASE_RE = re.compile(r"([\d,]+)-entry (?:generated )?table")


def stale_entry_phrases(text: str, live: set[str]) -> list[tuple[int, str]]:
    """`(line number, quoted size)` for every table-size claim not in `live`,
    which holds both spellings (1188 and 1,188 are the same claim)."""
    return [(i, quoted)
            for i, line in enumerate(text.splitlines(), 1)
            for quoted in ENTRY_PHRASE_RE.findall(line)
            if quoted not in live]


def check_source_table_size() -> list[str]:
    """The table size as quoted in Rust prose and Cargo.toml (`vrf-decode`'s
    crate docs, its feature table and its manifest)."""
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


#: How the docs state a suite size, narrow enough that a match is always a
#: claim about one of the two suites; the suite may sit between the number and
#: the noun ("807 Rust tests").
TEST_COUNT_RE = re.compile(r"(\d[\d,]*)\s+(?:(Rust|Python)\s+)?(?:tests|passing)\b")


def test_count_claims(text: str, platform: str | None = None) -> list[tuple[int, str, str]]:
    """Suite counts, optionally scoped by a following platform qualifier.
    Unqualified claims apply on every platform."""
    claims = []
    for i, line in enumerate(text.splitlines(), 1):
        for match in TEST_COUNT_RE.finditer(line):
            scope = re.match(r"\s+\((Windows|non-Windows)\)", line[match.end():])
            if platform is None or scope is None or scope.group(1) == platform:
                claims.append((i, match.group(1), match.group(2) or ""))
    return claims


def stale_test_counts(text: str, live: set[str],
                      by_suite: dict[str, set[str]] | None = None, *,
                      platform: str | None = None) -> list[tuple[int, str]]:
    """`(line number, quoted count)` for every suite-size claim not in `live`
    (both suites' counts in both spellings): presence is not agreement. A claim
    that names its suite must be that suite's, when `by_suite` gives it."""
    return [(i, quoted)
            for i, quoted, suite in test_count_claims(text, platform)
            if quoted not in ((by_suite or {}).get(suite) or live)]


def unquoted_test_counts(docs: dict[str, str], counts: dict[str, int], *,
                         platform: str | None = None) -> list[str]:
    """Each doc must quote each live suite size in a `TEST_COUNT_RE` claim, in
    either spelling; a longer number that contains it quotes nothing."""
    return [f"{name}: {label} test count is {count}, not quoted"
            for label, count in counts.items()
            for name, text in docs.items()
            if not any(q.replace(",", "") == str(count)
                       for _i, q, _suite in test_count_claims(text, platform))]


def contradicting_test_counts(docs: dict[str, str]) -> list[str]:
    """Suite-size claims that cannot all be true at once.

    `stale_test_counts` needs the real numbers, so it runs only in full mode;
    this survives `--fast`. It cannot know which number is right and need not:
    there are exactly two suites, so a third distinct value is a
    contradiction. Blind to a count wrong the same way everywhere.
    """
    for platform in ("Windows", "non-Windows"):
        seen = [
            (name, i, quoted)
            for name, text in docs.items()
            for i, quoted, _suite in test_count_claims(text, platform)
        ]
        distinct = {quoted.replace(",", "") for _, _, quoted in seen}
        if len(distinct) > 2:
            sites = ", ".join(f"{name}:{i} says {quoted}" for name, i, quoted in seen)
            return [
                f"{len(distinct)} different test counts claimed for {platform} "
                f"but there are two suites, so at least one is stale -- {sites}"
            ]
    return []


#: Every doc this guard reads. README and USAGE must *quote* the live numbers;
#: the rest only must not contradict them (CLAUDE.md quotes no counts by
#: design, so this tier adds no obligation there).
ALL_DOCS = ("README.md", "docs/USAGE.md", "docs/DATA.md", "CONTRIBUTING.md",
            "CLAUDE.md")

#: Numbers quoted in prose *and* produced by something runnable, as `(count
#: pattern, context, context window)`, narrow enough that a match is always
#: that claim. With a context, the count counts only when the context sits on
#: its line or within `window` lines above (prose wraps: "`ADDITIONS` ..." then
#: "currently 142 of them").
MEASURED_RE = {
    "corrections": (re.compile(r"(\d+) corrections"), None, 0),
    "additions": (re.compile(r"currently (\d+) of them"),
                  re.compile(r"ADDITIONS"), 2),
    "golden": (re.compile(r"(\d+) mechanically extracted golden vectors"), None, 0),
    # "N builds" is generic, so only on a line naming the semantic guard.
    "metrics_builds": (re.compile(r"(\d+) builds\b"),
                       re.compile(r"check_metrics_baseline"), 0),
}

GOLDEN_LEN_RE = re.compile(r"pub const VECTORS: \[\(&str, usize, &str\); (\d+)\]")


def link_checked_docs() -> list[Path]:
    """Every doc whose relative links are checked: `ALL_DOCS` plus docs/*.md."""
    paths = {REPO / name for name in ALL_DOCS}
    paths.update((REPO / "docs").glob("*.md"))
    return sorted(paths)


FENCE_OPEN_RE = re.compile(r"^ {0,3}(`{3,}|~{3,})")
FENCE_CLOSE_RE = re.compile(r"^ {0,3}(`{3,}|~{3,})[ \t]*$")
ATX_HEADING_RE = re.compile(r"^ {0,3}#{1,6}(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$")
CODE_SPAN_RE = re.compile(r"(`+)(.+?)\1")
INLINE_LINK_RE = re.compile(r"!?\[([^\]]*)\]\([^)]*\)")
#: A source or fixture naming a doc, with or without a heading. A slug never
#: holds a dot, so a sentence-ending `.` is not read as part of it.
CODE_ANCHOR_RE = re.compile(r"\b(docs/(?:[\w.-]+/)*[\w.-]+\.md)(?:#([\w-]+))?")


def unfenced_lines(text: str):
    """`(line number, line)` for every line outside a fenced code block, where
    a `#` line is a comment and a `[x](y)` is sample text, not a link."""
    fence = None
    for i, line in enumerate(text.splitlines(), 1):
        if fence:
            close = FENCE_CLOSE_RE.match(line)
            if close and close.group(1)[0] == fence[0] and len(close.group(1)) >= len(fence):
                fence = None
            continue
        opened = FENCE_OPEN_RE.match(line)
        if opened:
            fence = opened.group(1)
            continue
        yield i, line


def github_slug(heading: str) -> str:
    """The anchor GitHub renders for a heading: the rendered text (code spans
    keep their content, links their text, images drop out) lowercased, every
    character that is not a letter, mark, digit, `_`, `-` or space removed,
    and each space turned into `-`. Underscore emphasis is not rendered."""
    def outside_code(segment: str) -> str:
        return INLINE_LINK_RE.sub(
            lambda m: "" if m.group(0).startswith("!") else m.group(1), segment)

    parts, pos = [], 0
    for span in CODE_SPAN_RE.finditer(heading):
        parts.append(outside_code(heading[pos:span.start()]))
        code = span.group(2)
        if code.startswith(" ") and code.endswith(" ") and code.strip():
            code = code[1:-1]
        parts.append(code)
        pos = span.end()
    parts.append(outside_code(heading[pos:]))
    return "".join(
        ch for ch in "".join(parts).lower()
        if ch in " -" or unicodedata.category(ch) in ("Nd", "Pc")
        or unicodedata.category(ch)[0] in "LM").replace(" ", "-")


def heading_anchors(text: str) -> set[str]:
    """Every anchor a document's ATX headings give, with GitHub's `-1`, `-2`
    suffixes on repeats."""
    seen: dict[str, int] = {}
    anchors = set()
    for _, line in unfenced_lines(text):
        heading = ATX_HEADING_RE.match(line)
        if not heading:
            continue
        slug = github_slug((heading.group(1) or "").strip())
        count = seen.get(slug, 0)
        anchors.add(f"{slug}-{count}" if count else slug)
        seen[slug] = count + 1
    return anchors


@functools.lru_cache(maxsize=None)
def anchors_of(path: Path) -> frozenset[str] | None:
    """The anchors of a markdown file on disk; `None` when it does not exist."""
    return frozenset(heading_anchors(read(path))) if path.is_file() else None


def broken_markdown_anchors(path: Path, text: str, lookup=anchors_of,
                            checked: list | None = None) -> list[str]:
    """Links in one doc whose `#anchor` names no heading of their target;
    each link checked is appended to `checked`. A missing target file is
    `check_links`'s report, and an anchor on a non-markdown target (a `#L10`
    line link) is GitHub's, not a heading."""
    problems = []
    for i, line in unfenced_lines(text):
        for _label, target in LINK_RE.findall(line):
            target = target.strip().split()[0] if target.strip() else ""
            if target.startswith(("http://", "https://", "mailto:")) or "#" not in target:
                continue
            file_part, fragment = target.split("#", 1)
            dest = (path.parent / file_part).resolve() if file_part else path
            if not fragment or dest.suffix.lower() != ".md":
                continue
            anchors = lookup(dest)
            if anchors is None:
                continue
            if checked is not None:
                checked.append(target)
            if unquote(fragment) not in anchors:
                problems.append(f"{path.name}:{i}: link -> {target}: "
                                f"{dest.name} has no heading #{unquote(fragment)}")
    return problems


def broken_code_anchors(name: str, text: str, lookup=anchors_of,
                        checked: list | None = None) -> list[str]:
    """`docs/<name>.md[#<anchor>]` references in one file that name a doc or
    a heading that does not exist; each one is appended to `checked`. Paths
    are repository-relative."""
    problems = []
    for i, line in enumerate(text.splitlines(), 1):
        for doc, fragment in CODE_ANCHOR_RE.findall(line):
            cite = f"{doc}#{fragment}" if fragment else doc
            if checked is not None:
                checked.append(cite)
            anchors = lookup((REPO / doc).resolve())
            if anchors is None:
                problems.append(f"{name}:{i}: cites {cite}, but {doc} does not exist")
            elif fragment and fragment not in anchors:
                problems.append(f"{name}:{i}: cites {cite}, but {doc} has no such heading")
    return problems


def anchor_problems(checked: dict[str, list] | None = None) -> list[str]:
    """Every broken anchor in the link-checked docs, and every missing doc or
    heading the tracked Rust, Python and JSON files cite. `checked["docs"]`
    and `checked["code"]` receive every reference read, so a run that read
    none can say so. A source list that cannot be read is reported, not
    treated as an empty one."""
    checked = {} if checked is None else checked
    docs, code = checked.setdefault("docs", []), checked.setdefault("code", [])
    problems = [p for path in link_checked_docs()
                for p in broken_markdown_anchors(path, read(path), checked=docs)]
    r = subprocess.run(["git", "-C", str(REPO), "ls-files", "--", "*.rs", "*.py", "*.json"],
                       capture_output=True, text=True, encoding="utf-8",
                       errors="replace", timeout=120)
    if r.returncode != 0:
        return problems + [
            f"could not list the sources: git ls-files exited "
            f"{r.returncode} ({(r.stderr or '').strip()[:120]}); every code "
            f"reference went unchecked"]
    for name in r.stdout.splitlines():
        if name.strip():
            text = (REPO / name).read_text(encoding="utf-8", errors="replace")
            problems += broken_code_anchors(name, text, checked=code)
    # A scan that read nothing would otherwise pass.
    problems += [f"anchor check read no {what}" for what, found in
                 (("doc links", docs), ("code references to docs", code)) if not found]
    return problems


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


def baseline_table_figures(export: dict | None = None) -> dict[str, tuple[int, int]]:
    """Rows and bytes promised by the committed reference export baselines:
    the main tables from the export baseline (the docs quote the default
    main-only run), every `checkpoint_*` table from the checkpoint baseline."""
    export = export or json.loads(read(EXPORT_BASELINE))
    checkpoint = json.loads(read(EXPORT_BASELINE.with_name("checkpoint_02d4d478.json")))
    figures = {
        f"{name}.parquet": (int(values["rows"]), int(values["bytes"]))
        for name, values in export["parquet"].items()
    }
    figures.update(
        (f"{name}.parquet", (int(values["rows"]), int(values["bytes"])))
        for name, values in checkpoint["parquet"].items()
        if name.startswith("checkpoint_")
    )
    return figures


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


#: The overlay summary block README reprints, keyed by the label as printed and
#: mapped to the baseline counter it quotes (no `.vrf` needed). `Decode errors`
#: is absent on purpose: "Decode errors: 0" names a failure mode in CLAUDE.md,
#: DATA and USAGE, not this replay's value.
OVERLAY_COUNTER_KEYS = {
    "Decoded OK": "overlay_decoded_ok",
    "Raw/Skip": "overlay_raw_skip",
    "Not in table": "overlay_not_in_table",
    "No field name": "overlay_no_field_name",
    "Effect blobs": "effect_blobs_decoded",
}

#: The five buckets `print_overlay` (crates/vrfkit/src/driver/summary.rs) sums
#: to print `Rows offered`. Their sum is `overlay_rows_offered` exactly, so a
#: break means a bucket was added or one stopped counting.
#: `overlay_decode_errors` belongs here although `OVERLAY_COUNTER_KEYS` excludes
#: it: that exclusion is about doc prose, not arithmetic, and without it the
#: check would pass only while the pinned decode-error count is 0.
OVERLAY_PARTITION = ("overlay_decoded_ok", "overlay_decode_errors",
                     "overlay_raw_skip", "overlay_not_in_table",
                     "overlay_no_field_name")

#: `Typed` is not stored: it is `Decoded OK / Rows offered` as a percentage,
#: derived rather than pinned so it cannot drift from the two counters.
TYPED_RE = re.compile(r"Typed:\s*([\d.]+)%")


def overlay_partition_problems(counters: dict[str, int]) -> list[str]:
    """The buckets must still add up, and the keys must still be there: a
    missing key would leave every line quoting it unchecked while
    `stale_overlay_counters` skipped it and this printed OK."""
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

    Scoped to the printed `Label: N` form, so prose that merely names a bucket
    (README's "most of `Not in table` is RPC parameters") is not a quote.
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
    """README must still carry the block, not merely not contradict it:
    deleting it would satisfy `stale_overlay_counters` with nothing quoted."""
    return [f"README.md: overlay summary block is missing `{label}:`"
            for label, key in OVERLAY_COUNTER_KEYS.items()
            if key in counters
            and not re.search(rf"{re.escape(label)}:\s*[\d,]+", readme)]


def measured_counts(problems: list[str] | None = None) -> dict[str, int]:
    """The live values, read from the things that produce them. A key that
    could not be measured is left out and, with `problems`, reported: a
    missing key would make `stale_measured_counts` check nothing."""
    import apply_type_corrections as atc

    counts = {"corrections": atc.expectation_count(read(atc.TABLE_RS)),
              # Imported, not line-counted: each entry spans a commented block.
              "additions": len(atc.ADDITIONS)}
    golden = GOLDEN_LEN_RE.search(read(
        REPO / "crates" / "vrf-transform" / "tests" / "data" / "golden_vectors.rs"))
    if golden:
        counts["golden"] = int(golden.group(1))
    metrics = json.loads(read(EXPORT_BASELINE.with_name("metrics_builds.json")))
    if isinstance(metrics.get("replays"), dict) and metrics["replays"]:
        counts["metrics_builds"] = len(metrics["replays"])
    if problems is not None:
        problems += [f"could not measure the {what} count; every quoted one went unchecked"
                     for what in MEASURED_RE if what not in counts]
    return counts


def stale_measured_counts(docs: dict[str, str], live: dict[str, int]) -> list[str]:
    """Every quoted measured count that is not the live one.

    Every match must be right, so a file saying 85, 86 and 49 corrections
    reports two problems. A count is a claim only when its context is on the
    same line or within `window` lines above it (see `MEASURED_RE`).
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


def _run(cmd: list[str], timeout: int) -> tuple[int, str]:
    r = subprocess.run(cmd, cwd=REPO, capture_output=True, text=True,
                       encoding="utf-8", errors="replace", timeout=timeout)
    return r.returncode, (r.stdout or "") + (r.stderr or "")


def measure_tests(modules: list[str] | None = None) -> tuple[int, int, list[str]]:
    """`(rust passed, tools ran, problems)`: `cargo test --workspace` beside the
    tools suite, one process per test module (run serially the suite takes
    ~20 s on one core). Every run must exit 0 and print its count, and a skipped
    Python test is a problem, so a passing summary cannot hide a failure."""
    if modules is None:
        modules = sorted(p.name for p in (REPO / "tools" / "tests").glob("test_*.py"))
    with ThreadPoolExecutor(max_workers=min(8, os.cpu_count() or 1) + 1) as pool:
        # CI's toolchain and lockfile: a newer default toolchain could measure
        # code that 1.86 rejects.
        rust_run = pool.submit(_run, ["cargo", "+1.86.0", "test", "--workspace", "--locked",
                                      "--quiet"], 3600)
        # `-b` keeps a passing test's output out of the capture.
        tool_runs = [(module, pool.submit(_run, [sys.executable, "-W", "error", "-m", "unittest",
                                                 "discover", "-b", "-s", "tools/tests", "-p",
                                                 module], 1800))
                     for module in modules]
    problems = [] if tool_runs else ["found no tools test module"]
    rc, out = rust_run.result()
    passed = re.findall(r"^test result: ok\. (\d+) passed;", out, re.M)
    rust = sum(int(n) for n in passed)
    if rc != 0:
        problems.append("cargo test did not pass; doc counts not checked against it\n" + out[-4000:])
    elif rust == 0:
        # No "N passed" line after exit 0 means the output format changed, not
        # that nothing ran; a defaulted 0 would fail every quoted count wrongly.
        problems.append("cargo test exited 0 but reported no passing test; the rust "
                        "test count was not measured")
    tools_n = 0
    for module, run in tool_runs:
        rc, out = run.result()
        ran = re.search(r"^Ran (\d+) tests? in ", out, re.M)
        if rc != 0:
            problems.append(f"tools test module {module} did not pass\n" + out[-4000:])
        elif ran is None or ran.group(1) == "0":
            problems.append(f"tools test module {module} exited 0 but ran no test; "
                            f"the tools test count was not measured")
        else:
            tools_n += int(ran.group(1))
        if re.search(r"^OK \(.*skipped=[1-9]", out, re.M):
            problems.append(f"tools test module {module} skipped tests; not every "
                            f"reported test passed")
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
    #: Every doc, for the checks that only ask a number not to be wrong; `docs`
    #: (README+USAGE) for the ones that require a number to be *present*.
    every = {name: read(REPO / name) for name in ALL_DOCS}
    generated_docs = {
        name: read(REPO / name) for name in GENERATED_INVENTORY_DOCS
    }

    measurement_problems: list[str] = []
    live_counts = measured_counts(measurement_problems)
    export_baseline = json.loads(read(EXPORT_BASELINE))
    overlay_counters = {k: int(v) for k, v in export_baseline["counters"].items()}
    anchors_checked: dict[str, list] = {}

    # One entry per check: the summary prints len(checks), never a literal.
    checks = [
        check_tools(usage),
        check_crates(usage),
        check_table_sizes(docs),
        stale_table_size_claims(every, table_lengths()),
        check_source_table_size(),
        contradicting_test_counts(every),
        measurement_problems,
        stale_measured_counts(every, live_counts),
        check_generated_inventory(generated_docs),
        check_baseline_figures({"USAGE.md": usage}, baseline_table_figures(export_baseline)),
        overlay_partition_problems(overlay_counters),
        stale_overlay_counters(every, overlay_counters),
        check_overlay_counters_present(readme, overlay_counters),
        [p for path in link_checked_docs() for p in check_links(path, read(path))],
        check_build_verification(
            readme, usage, read(REPO / "crates/vrf-transform/src/lib.rs"),
            json.loads(read(BUILD_AUDIT))),
        check_transform_evidence(
            read(REPO / "docs/BUILD_VERIFICATION.md"),
            read(REPO / "crates/vrf-transform/src/lib.rs"),
            {path.name: read(path) for path in TRANSFORM_VECTORS.glob("*.rs")}),
        anchor_problems(anchors_checked),
    ]

    if not args.fast:
        rust, tools_n, run_problems = measure_tests()
        platform = "Windows" if sys.platform == "win32" else "non-Windows"
        run_problems += unquoted_test_counts(
            docs, {"rust": rust, "tools": tools_n}, platform=platform)
        by_suite = {suite: {str(c), f"{c:,}"}
                    for suite, c in (("Rust", rust), ("Python", tools_n))}
        live = by_suite["Rust"] | by_suite["Python"]
        for name, text in every.items():
            run_problems += [
                f"{name}:{i}: says {quoted}; the suites are {rust} (Rust) "
                f"and {tools_n} (Python)"
                for i, quoted in stale_test_counts(text, live, by_suite, platform=platform)]
        print(f"tests: rust {rust} ({platform}), tools {tools_n}")
        checks.append(run_problems)
    problems = [p for found in checks for p in found]

    n_tools = len(list((REPO / "tools").glob("*.py")))
    n_crates = len({p.parent.name for p in (REPO / "crates").glob("*/Cargo.toml")})
    print(f"docs: {len(ALL_DOCS)} files ({len(link_checked_docs())} link-checked)   "
          f"{n_tools} tools, {n_crates} crates, "
          f"{len(anchors_checked.get('docs', []))} doc links and "
          f"{len(anchors_checked.get('code', []))} code references to docs, "
          f"{len(checks)} checks")

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
