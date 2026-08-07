# Working in this repository

Orientation for an AI agent. Build commands, the required pre-PR sweep and the
environment variables live in [CONTRIBUTING.md](CONTRIBUTING.md) — read that
first and do not duplicate it here.

## What this is

vrfkit parses VALORANT `.vrf` replay files — Unreal Engine's network replay
format — into Parquet. It is reverse-engineered against a format that changes
every game build, so almost every claim in this repo is a *measurement*, not a
specification, and the measurements are dated.

Ten Rust crates under `crates/`, Python tooling under `tools/`. The Rust side
decodes; the Python side generates the type overlay, checks the output, and
derives analytical views.

## Where the truth is

| Question | Read |
|---|---|
| What can I get out of a replay, and is it typed? | `docs/DATA.md` |
| How do I run it, and what does each tool do? | `docs/USAGE.md` |
| How do I build, test, and what must pass? | `CONTRIBUTING.md` |
| Why is the code shaped this way? | the doc comment next to it |
| What was tried and rejected? | `docs/archive/` |

`README.md` is the front page and repeats the highlights. When it disagrees
with `docs/`, `docs/` is newer.

**Read the doc comments.** This codebase carries long explanatory comments
exactly where the code looks wrong at a glance — a byte reader that takes its
width from the payload rather than a fixed 8 bits, a `to_string()` that
deliberately does not pre-reserve, a fallback that is deliberately last. Each
one records a measurement that justifies the choice. Changing that code without
reading the comment reintroduces a bug someone already paid for.

## The one failure mode that matters here

**"Decode errors: 0" means the decoder did not throw. It does not mean the
values are right.** Every expensive bug in this project's history has been a
plausible wrong value or a check that could not fail, never a crash. Known
shapes, all found in real data:

- a field typed `FString` that is actually `FText` — null on every row, no counter moved
- a field pinned to `FieldType::Raw` because name resolution failed — never decoded, so never counted as a failure
- a decoder exempted from the "payload fully consumed" check — leftover bits vanished with no error and no tally
- a comparison tool that printed a mismatch and exited 0
- a channel reused by a new actor inheriting the previous actor's schema — a destructible prop decoded as a smoke grenade, emitting plausible field names and typed values for the wrong class

Consequences for how you work:

- **A counter that cannot move is worse than one that reports a wrong number.**
  Print zeros. A line that appears only when non-zero cannot distinguish
  "nothing is wrong" from "this code stopped running".
- **A missing value renders as a visible absence, never as a plausible number.**
  `?`, not `0`.
- **Prefer a surfaced tally to silent tolerance, and to hard rejection.** The
  established pattern is `errors` / `truncations` / `skipped_bits` counters that
  reach the summary.
- **Fail loudly rather than guess.** A guessed scale drawing a plausible wrong
  picture is worse than no picture.

## Verifying your own work

- **Check the exit code, not the output.** `tools/check_docs.py` prints an
  identical-looking summary line on both the passing and the failing path. That
  has fooled someone here, in this repo, while they were fixing this exact class
  of bug.
- **A test that passes is not a test that works.** Break the property the test
  is named for and confirm it fails. Tests that stayed green while the
  implementation was mutated have shipped here more than once — including one
  that asserted a static fact about its own fixture.
- **Type nothing you have not seen decode.** After adding an overlay type, count
  non-null values on that column. `LocalizedStat` was typed `FString` on the
  strength of its name and produced null on 3,011 of 3,011 rows while
  `Decode errors: 0` held the whole time.
- **Two independent implementations agreeing is the strongest evidence** — a
  Rust decoder against a Python cross-check, or a derived figure against the
  game's own numbers. "The column is not null" is the weakest.
- **Check the reason you gave for not doing something.** More than one item here
  was skipped on a rationale that had never itself been checked.

## Traps that have cost real time

- **`crates/vrf-decode/src/table.rs` is generated.** Edit
  `tools/extract_descriptors.py` or `tools/apply_type_corrections.py` instead,
  then regenerate. The pipeline order is generate -> correct -> `cargo fmt`.
- **Some entries in that table are unreachable.** The four `LifeChangeEvents`
  member entries never appear as a top-level parameter. "Fixing" one compiles,
  passes tests, and changes no rows. The real typing happens in
  `crates/vrfkit/src/sink/rpc.rs`.
- **One name entry is dead by design.** `EquippablePickupProjectile_C`'s
  `MyEquippable` no longer matches: FName instance numbers are part of the name,
  so the wire says `MyEquippable_0`. It resolves through the
  `compatible_checksum` fallback until the generator learns the number.
- **`actors.event` has three values, not two** — `open` / `close` / `dormant`.
  Dormancy is not destruction; only `close` is a despawn.
- **MSRV 1.86 is not your local toolchain.** `let` chains have already broken a
  build this way. Run the sweep through `cargo +1.86.0`.
- **`pip install -r requirements.txt`.** `pyarrow` and `numpy` are real
  dependencies of `tools/`; without them eleven test modules fail to import and
  the suite silently runs a subset.

## Measuring

When you need a number, measure it and write down *how*, including the filters.
A figure whose method is not recorded cannot be reproduced, and this repo has
already shipped a comment quoting a denominator that matched no table on disk.

Corpus guards read their inputs from environment variables — see
`CONTRIBUTING.md`. Nothing in the tree points at one person's disk, and nothing
should start.

Game files are read-only. Copy what you need; never modify the installation.
