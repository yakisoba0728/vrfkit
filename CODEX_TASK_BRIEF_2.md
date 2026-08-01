# Codex task brief #2 — the four open "needs work" items

## Who you are in this

You are a **delegate**, not a handoff target. A Claude session remains the main
worker on this repository and will review, verify and merge what you produce.
Work on your own branch, commit there, and report back. Do not push to `master`
and do not merge anything yourself.

Use subagents aggressively. Every task below decomposes into independent
investigation and implementation, and several of them are search problems where
fanning out is strictly better than one linear pass.

---

## The repository in one paragraph

`vrfkit` is a from-scratch Rust parser for VALORANT replay files (`.vrf`),
written to replace a C# reference parser (`ValorantReplayParser`) that a Python
analytics pipeline (`valplay`) depends on. It parses the replay to Parquet
(`fields`, `movement`, `actors`, `net_guids`), and a Python adapter
(`tools/to_valplay_bundle.py`) converts that into the event-bundle shape
`valplay/pipeline/metrics/compute_metrics.py` already consumes. Correctness is
measured by comparing our output to the C# reference's, field by field.

Read `PROJECT_STATUS.md` before touching anything. Sections 8 (invariants),
13 (this week's fixes and the lessons attached to them) and the QUICK START
block are the load-bearing parts.

---

## THINGS YOU MUST NOT DO

### Do not try to make `combat`, `economy_detail`, `weapon_stats`, `tactical` or `kast` match the reference

These five metric sections differ from the reference. **That is not a bug and
you must not "fix" it.** Any change that moves them toward the reference is a
regression, and it will be rejected.

- `combat` / `economy_detail` / `weapon_stats` differ because **we recover data
  the reference drops** — 13 kill RPCs the C# never emits, 496 of 496 purchase
  buyers resolved against its 151, one damage record it discards. Direction
  established and documented.
- `tactical` / `kast` differ because the **published reference bundles were
  built by the C# parser with a bug in it** — Gekko's descriptor path had a
  casing typo (`Aggrobot` vs `AggroBot`), so the reference is missing that
  character's kills entirely. The typo is now fixed at source, but the bundles
  predate the fix and cannot be regenerated without invalidating every
  comparison figure in `PROJECT_STATUS.md`. Section 13-C has the full
  derivation and section 6 pins the exact values.

If you find yourself editing metric logic to close a gap with the reference,
stop and report instead.

### Do not touch these

- `C:\Users\yakihyuk0728\Documents\GitHub\valplay` — **read only.** Run its
  scripts by absolute path. `compute_metrics.py` writes `metrics.json` into
  whatever directory you hand it, so **always pass a directory under vrfkit's
  `out/`**, never one inside valplay.
- `ValorantReplayParser`'s `main` branch — **must stay at `2d2e05e`.** That is
  the commit the reference bundles were built from (they stamp
  `parser_version: 1.0.0+2d2e05e8`). Do not merge, do not pull, do not rebase.
  Descriptor work happens on `local/vrfkit-descriptors` (currently `f67ea66`),
  which is also the branch that must be checked out to regenerate `table.rs`.
- Generated files by hand: `crates/vrf-decode/src/table.rs`,
  `crates/vrf-transform/src/sbox.rs`, `golden_vectors.rs`,
  `tools/equippable_table.py`. Change the generator and regenerate.

---

## Environment constraints (you will hit all of these)

- **Windows 11, PowerShell 5.1 primary.** PowerShell here-strings and heredocs
  break on this box repeatedly. For anything multi-line, write a script file
  and run it, or use a Bash tool if you have one.
- **Python paths in `python -c` need raw strings.** `C:\Users\...` contains
  `\U` and will raise `SyntaxError: truncated \UXXXXXXXX escape`. Write a
  `.py` file instead of fighting the quoting.
- **`pytest` is NOT installed and you must not add it.** The repo's convention
  for verification is a self-checking script with a `--check` mode — see
  `tools/apply_type_corrections.py` and `tools/check_export_baseline.py`. Both
  fail loudly and both were deliberately broken to prove they fail. Follow that
  pattern; stdlib `unittest` is acceptable if a script does not fit.
- **`cargo fmt` after regenerating `table.rs`.** The generator emits a compact
  one-line-per-field form; the committed file is rustfmt'd. Skip this and you
  get a phantom 5,000-line diff that hides the four lines that actually changed.
- **ASCII only in Rust code AND comments.** The Windows console is cp949 and
  truncates output at the first non-ASCII byte, which is a correctness
  constraint on the diagnostics path, not a style preference.
- `#![forbid(unsafe_code)]` everywhere. TDD. No skip path, no silent success.

---

## Which number means what (do not try to reconcile these)

You will meet three different counts and they are **not** meant to sum. Getting
this wrong costs a full cycle.

    1,240,444   rows in fields.parquet (02d4d478)
      988,979   rows offered to the type overlay
      871,595   rows in fields.parquet with no decoded value in any value_* column
      525,839   overlay "not in table"
       71,427   overlay "raw or skip" (typed as deliberately-opaque)
       33,529   overlay "no field name" (unmapped handle)
      358,184   overlay "decoded ok"

The 1,240,444 vs 988,979 gap is rows that never reach the overlay at all
(flattened array leaves among them). **Task B is measured against the 871,595
figure**, grouped by `group_path`. State which denominator you are quoting,
every time.

---

## Task A — the live effect decoder has zero tests

**Size: small. Do this first; it is the cleanest win.**

`tools/to_valplay_bundle.py` contains a Python port of the shot-effect blob
decoder (`_read_effect_float`, `_read_effect_object`, `_read_effect_vector`,
`_decode_effect_elements`, `_decode_effect_blob`, `_decode_rotation_short`,
around lines 486-620). It is the **live path** for four metric sections and has
no executable test of any kind.

`crates/vrf-decode/src/effect.rs` is a Rust implementation of the same wire
format. It is **not wired into the pipeline** (see the module docs for why) but
it carries **eight pinned hex vectors** in its test module — the repo's only
executable specification of this format on either side.

**Do:** port those eight vectors to a self-verifying Python script that
exercises the live decoder, in the repo's `--check` style. Then find at least
two cases the eight vectors do not cover and add them, taking the expected
values from the reference bundle rather than from our own output.

**Watch for:** the two implementations have deliberately different failure
contracts — Rust returns `Err` and discards the array, Python breaks out and
returns a partial list. Your tests must pin the **Python** contract as it
actually behaves, not assume the Rust one. If you find a case where they
disagree on a *successful* decode, that is a real finding; report it loudly.

**Done when:** the script fails when you deliberately corrupt a byte of any
vector (demonstrate this, do not assert it), and passes clean afterwards.

---

## Task B — where the untyped rows actually are

**Size: medium. Investigation first, code second.**

871,595 of 1,240,444 rows on 02d4d478 carry no decoded value. That is **not a
406-group problem**. Three groups are 65% of it:

     333,022  /Game/Characters/_Core/BaseReplayController.BaseReplayController_C_ClassNetCache
     124,744  /Script/ShooterGame.LocationalEffectManagerComponent_ClassNetCache
     110,508  /Script/ShooterGame.EffectManagerComponent_ClassNetCache
      23,275  /Script/ShooterGame.ReplayEffectComponent_ClassNetCache
      20,898  /Game/GameModes/Bomb/BombPlayerState.BombPlayerState_C

**The question to answer, per group, is binary:**

1. **The C# has a descriptor and our extractor fails to read it.** That is a
   generator bug and worth fixing. There is a worked precedent: commit
   `4f78f6d` found that `AddPropertyHandle`'s handle argument had to be a
   literal, so a descriptor that factored a run of handles into a helper
   (`AddDeathFields(uint firstHandle)`, called as `AddDeathFields(32)`) was
   invisible. Four fields came back. **Look for more shapes like that** — that
   is the highest-value thing in this task.
2. **The C# genuinely has no descriptor for it.** Then the reference cannot
   decode those rows either, and writing one is new upstream work, not a fix.
   Say so and stop. Do not write speculative descriptors.

Verify (2) rather than asserting it: check whether the reference bundle emits
anything for that group. If it does and we do not, it is case (1).

**Do:** produce a table — group, row count, verdict, evidence. Then fix every
case (1) you find, regenerate `table.rs` through the generator, `cargo fmt`,
and show the diff is exactly the entries you expect and nothing else.

**Done when:** each of the top five groups has a verdict backed by a command
you ran, and every case (1) is fixed and measured.

---

## Task C — unattributed block payloads are lost, not merely unnamed

**Size: medium. MEASURE FIRST, then stop and report before implementing.**

`crates/vrf-net/src/field.rs`, `parse_class_net_cache`: when
`function_count == 0` the export group could not be resolved, so the handle
width is unknown and the record stream cannot be walked. It returns
`Err(NetError::UnresolvedFunctionCount)` **before reading any bits**. The caller
only invokes `on_stream_failure`, which pushes a diagnostic string capped at 32
lines. No `on_field` or `on_rpc` fires, so **no Parquet row is written at all**.

The consequence, which `PROJECT_STATUS.md` 7-C states plainly: the raw bits are
not preserved anywhere. An archived `fields.parquet` **cannot** be reinterpreted
later when the missing descriptor arrives. The source `.vrf` must be kept.

Scale on 02d4d478: 6,365 of 608,020 blocks fail (1.05%), 17,507,210 bits (2.1%
of what we read). `AbilitiesAndBuffsComponent` alone is 17,264,706 of those
bits. That component carries ability and buff/debuff **state** — charge counts
over time, who was blinded or slowed and for how long, ult gauge between
casts — which is the difference between "this ability was used" (we have it)
and "this ability affected these players for this long" (we do not).

**Step 1, measure and report before writing any implementation:**
what would preserving those payloads cost? Rows added to `fields.parquet`, bytes
added after ZSTD, and the effect on export wall-clock. Measure on 02d4d478 and
on at least two more replays.

**Step 2, only after reporting:** if the cost is modest, implement preservation.
The block already knows its `group_path`, `actor_net_guid` and `bit_count`; what
it lacks is a per-record split, which is exactly what an unresolved group makes
impossible. So preserve the **whole block payload as one row** with a null field
name, not a fabricated per-field split. Fabricating structure you cannot read is
the failure mode this repo has hit three times in one week (13-A, 13-E, 13-I).

**Do not** silently change existing counters. `check_export_baseline.py` will
fire; explain every drift line before passing `--update`.

**Done when:** the cost is on the table with the commands that produced it, and
either the feature is implemented and a round-trip is demonstrated (write the
blob, read it back, confirm the bits are byte-identical to the source), or you
have reported why it is not worth it.

---

## Task D — 510 non-ASCII lines, and files whose encoding was mangled once already

**Size: small, mechanical. Do this last.**

The "ASCII only in code and comments" invariant is currently enforced only on
string literals; comments were never swept. 44 files, 510 lines. Worst
offenders:

     88  crates/vrf-movement/src/lib.rs
     52  crates/vrf-frame/src/lib.rs
     48  crates/vrf-container/src/lib.rs
     41  crates/vrf-container/src/header.rs
     38  crates/vrf-movement/src/decoder.rs

Separately: `crates/vrf-net/src/{field,lib,pipeline}.rs` carry doc comments with
literal `??` where em dashes used to be. Something mangled those files' encoding
once already; the damage is cosmetic (never printed) but it is real corruption
and the text should be restored to what it meant.

**Do:** replace non-ASCII characters with ASCII equivalents that preserve
meaning (`—` to `--`, `→` to `->`, `±` to `+/-`, `×` to `x`, and so on). Repair
the `??` sequences by reading the surrounding sentence and restoring what it
said. **Do not** delete a comment to satisfy the rule.

Then add a guard so this cannot silently return: a script that scans and fails,
in the same `--check` style as the rest of `tools/`. **The per-line scan already
missed a case once** — a multi-line USAGE literal in `cli.rs` — so scan file
content, not line by line, and prove the guard fails by planting a violation.

**Done when:** the guard fails on a planted violation, the tree is clean, and
`cargo test`/`clippy`/`fmt` are unaffected.

---

## How to work

**Isolation.** Create your own branch and worktree off `master`. Commit there.
The main session works in the same repository and will merge you.

    git worktree add ../vrfkit-codex-2 -b codex/needs-work

**Order.** A, then B, then C's measurement, then D. B is the highest value; C's
measurement gates a decision that is not yours to make alone.

**Commit granularity.** One commit per coherent fix, with the measurement in the
message. Look at `git log` for the house style: what was wrong, what the
evidence was, what the numbers are before and after.

---

## What "done" means here, and it is stricter than usual

This repository has caught **twelve** documented claims that turned out to be
false — seven from its own docs, five from a single session's new claims. The
review you will get is calibrated to that. Specifically:

1. **Every figure comes with the command that produced it.** A number without a
   command is treated as unverified.
2. **State which fields a comparison read.** Finding 13-I happened because a
   comparison read `location` and nothing else, and two rounds of "spawns match"
   shipped two wrong fields. If you compare against the reference, enumerate the
   fields you compared.
3. **A claim about what the data contains is not established by the code that
   produces it.** Finding 13-A: a fix rested on "there are zero genuine (0,0,0)
   spawns", which was never checked against the reference. There were 66. The
   "fix" dropped two metric sections out of exact and nothing noticed.
4. **Never trust a guard you have not seen fail.** Break it deliberately, show
   the failure output, revert, show it passes. Every guard in `tools/` was
   validated this way and one of them was silently applying nothing for days.
5. **Direction checks are cheap and decisive.** A metric that goes *down* when
   you feed it *more* data is impossible; that check found a real bug. Parity
   checks likewise — dynamic NetGUIDs are always even.
6. **Report what you did not do.** Scope you dropped, a case you could not
   verify, a measurement you skipped — say it. Under-reporting is the failure
   the user has already called out once.

**Full verification sweep before you report anything as done:**

```powershell
cargo test --workspace            # 242 passed, 0 failed
cargo clippy --workspace --all-targets -- -D warnings   # exit 0
cargo fmt --check                 # exit 0

cargo build --release
python tools\validate_corpus.py .\target\release\vrfkit.exe `
  "C:\Users\yakihyuk0728\Documents\GitHub\valplay\data\raw\vrf"
# 215/215, malformed 0, blocks 136,545,822 fields 98,883,979 rpcs 75,571,092

python tools\check_export_baseline.py --baseline tools\baselines\export_02d4d478.json
python tools\check_corpus_baseline.py --baseline tools\baselines\build_1210.json
python tools\check_corpus_baseline.py --baseline tools\baselines\build_1211.json
python tools\check_corpus_baseline.py --baseline tools\baselines\build_1300.json
python tools\check_corpus_baseline.py --baseline tools\baselines\build_1302.json
python tools\compare_combat_report.py     # ALL INTERESTING SHAPES MATCH
python tools\validate_metrics_corpus.py --jobs 3
# sections exact on ALL : 16 / 21  -- this must NOT go down
```

`validate_metrics_corpus` dropping below 16/21 means you broke something, even
if every other check is green. That is exactly how the spawn-location
regression was caught.

## Report back with

- What each task produced, with the numbers and the commands.
- Your branch name and HEAD commit.
- The before/after table for the full sweep above.
- Anything you found that contradicts `PROJECT_STATUS.md`. The document is
  wrong more often than it is right about figures, and correcting it is part of
  the job, not a distraction from it.
