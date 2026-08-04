"""Assert that README.md and docs/USAGE.md still describe THIS repo.

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
  6. the test counts quoted are the live ones

It runs the test suites to get (6), so it is not free -- roughly the cost of
`cargo test` plus the tools suite. Run it when touching docs, or before
calling a session finished.

Usage:
    python tools/check_docs.py
    python tools/check_docs.py --fast     # skip (6), no test runs
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
README = REPO / "README.md"
USAGE = REPO / "docs" / "USAGE.md"

#: Named in the docs but not shipped here.
EXTERNAL_SCRIPTS = {"compute_metrics.py"}

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


def check_table_sizes(docs: dict[str, str]) -> list[str]:
    """The generated overlay table's declared lengths, as quoted in prose."""
    table = read(REPO / "crates" / "vrf-decode" / "src" / "table.rs")
    entries = re.search(r"OVERLAY_TABLE: \[OverlayEntry; (\d+)\]", table)
    handles = re.search(r"OVERLAY_HANDLE_TABLE: \[OverlayHandleEntry; (\d+)\]", table)
    if not entries or not handles:
        return ["table.rs: could not read the declared slice lengths"]

    problems = []
    for n, what, where in ((entries.group(1), "overlay table", ("README.md", "USAGE.md")),
                           (handles.group(1), "handle table", ("USAGE.md",))):
        pretty = f"{int(n):,}"
        for name in where:
            if n not in docs[name] and pretty not in docs[name]:
                problems.append(f"{name}: {what} is {pretty}, not quoted")
    return problems


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

    problems = (
        check_tools(usage)
        + check_crates(usage)
        + check_links(README, readme)
        + check_links(USAGE, usage)
        + check_table_sizes(docs)
    )

    checked = 5
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
        print(f"tests: rust {rust}, tools {tools_n}")
        checked += 1

    n_tools = len(list((REPO / "tools").glob("*.py")))
    n_crates = len({p.parent.name for p in (REPO / "crates").glob("*/Cargo.toml")})
    print(f"docs: README.md + docs/USAGE.md   "
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
