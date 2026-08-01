# Task brief for Codex

You are working **as a delegate** on the vrfkit VALORANT replay parser. A
main session owns this repository, set the direction, and will review and
integrate everything you produce. You are not taking over the project.

That framing matters in three practical ways:

1. **You work on a branch, never on `master`.** The main session may be
   committing to `master` while you work.
2. **Your deliverable is evidence, not just code.** Every number you report
   will be re-run and checked. Overclaiming costs more than it gains.
3. **"This cannot be done" is a successful outcome.** Two of the last three
   items on this project were closed by proving they should not be done, and
   those were the cheapest wins available. Do not ship a heuristic because
   you feel you must ship something.

**Use your own subagents aggressively.** Guidance in section 6.

---

## 1. What this project is

A from-scratch Rust parser (`vrfkit`) that replaces a C# reference parser
(`ValorantReplayParser`) for a Python analytics pipeline (`valplay`). It
reads VALORANT `.vrf` replays and writes Parquet.

Current state, all measured, all reproducible with the commands in section 4:

```
tests                236 passing, clippy 0 warnings, fmt clean
13.01 corpus         215/215 parse, malformed framing 0
13.02 demos          4/4 parse, malformed framing 0
oracle pass rate     97.49% - 99.68% (median 99.32%)
metrics parity       15 of 20 metric sections byte-identical to the C#
                     reference on ALL 11 replays with a reference bundle
                     (the harness prints 16/21; one of those keys is `note`,
                     a fixed provenance string that cannot fail)
```

The 5 sections that differ do so because vrfkit carries **more** data than
the C# parser, not less. Every difference is named and understood.

`PROJECT_STATUS.md` is the project's record. Read it after this file — but
see section 5 first, because its prose is reliable and its **figures are
not**.

---

## 2. Where things are

```
Parser (work here)  C:\Users\yakihyuk0728\Documents\GitHub\vrfkit
C# reference        C:\Users\yakihyuk0728\Documents\GitHub\ValorantReplayParser
                    READ ONLY. Contains the user's uncommitted work --
                    17 entries in git status. Never commit, stash, reset or
                    modify. If you must instrument it: add ONE clean file,
                    run it, `git checkout -- <that file>`, and verify the
                    status is still 17 entries.
valplay             C:\Users\yakihyuk0728\Documents\GitHub\valplay
                    READ ONLY. Run its scripts by absolute path only.
                    compute_metrics.py writes metrics.json into whatever
                    bundle directory you pass it, so always pass a directory
                    under vrfkit's out/ -- never one under valplay.
13.01 corpus        valplay\data\raw\vrf   215 files, ++Ares-Core+release-13.01
Reference bundles   valplay\pipeline\exports\<replay-id>\
                    12 exist; 11 have a matching .vrf in the corpus
13.02 demos         %LOCALAPPDATA%\VALORANT\Saved\Demos\*.vrf   4 files
```

Crates:

```
vrf-bitio      LSB-first bit reader, UE wire primitives
vrf-transform  per-build payload transforms, golden-verified
vrf-container  .vrf container, chunk stream, Oodle decompression
vrf-frame      DemoFrame iteration
vrf-schema     the replay's own dynamic field schema + NetGuidCache
vrf-net        Unreal replication pipeline (no skip path)
vrf-decode     primitive decoders, nested arrays, struct blobs, type overlay
vrf-movement   remote-character update protocol
vrf-export     columnar Parquet writers
vrfkit         CLI: inspect / validate / export
tools/         Python generators, the valplay adapter, verification harnesses
```

---

## 3. Rules you must not break

From `PROJECT_STATUS.md` section 8. These are load-bearing: breaking one
silently corrupts downstream output without any test failing.

**NO SKIP PATH.** Every field emits `(group_path, handle, name, bit_count,
raw_bits)` even when nothing is known about it. The overlay is additive:
typed values fill `value_*` columns, failure leaves them null with raw bits
intact.

**NO SILENT SUCCESS.** A block whose group cannot be resolved fails loudly
(`function_count = 0` returns `Err`, counted and named). Never guess a
capacity or group to make a number look better.

**NO HARDCODED NAMES IN THE PARSER.** Resolution uses data the replay itself
declares. Adding a list of component or weapon names to a Rust crate is an
automatic fail. Display-name mapping lives in the Python adapter, generated
by `tools/extract_equippables.py`.

**GENERATED FILES ONLY VIA GENERATORS.**
```
crates/vrf-decode/src/table.rs                    <- tools/extract_descriptors.py
                                                     then tools/apply_type_corrections.py
crates/vrf-transform/src/sbox.rs                  <- tools/extract_sboxes.py
crates/vrf-transform/tests/data/golden_vectors.rs <- tools/extract_golden.py
tools/equippable_table.py                         <- tools/extract_equippables.py
```

**A CUSTOM C# DECODER MEANS THE TYPE IS UNKNOWN, NOT RAW.**
`extract_descriptors.py` cannot see through `.Decode(...)` in the C#
descriptors, so those fields land as `FieldType::Raw` — indistinguishable
from a field deliberately kept raw. Two real bugs came from this.

**ASCII ONLY IN RUST CODE AND COMMENTS.** The Windows cp949 console truncates
output at the first non-ASCII byte in a format string. This is a correctness
constraint on the diagnostics path, not style.

**NO UNSAFE.** `#![forbid(unsafe_code)]` everywhere.

**TDD.** Failing test first, watch it fail for the right reason, then
implement. A test that passes before you write the code proves nothing.

---

## 4. Verification protocol

Run all of it. Report the actual numbers — "tests pass" is not a report,
"236 passed, 0 failed" is. The main session will re-run these.

```powershell
cd C:\Users\yakihyuk0728\Documents\GitHub\vrfkit
$env:CARGO_TARGET_DIR = $null

# Build health
cargo test                                    # expect 236 passed, 0 failed
cargo clippy --all-targets -- -D warnings     # expect no output
cargo fmt --check                             # expect exit 0

# Single-replay regression -- must NOT change
cargo build --release -p vrfkit
.\target\release\vrfkit.exe export `
  "C:\Users\yakihyuk0728\Documents\GitHub\valplay\data\raw\vrf\02d4d478-1dfb-4412-9a77-29ca29105a9d.vrf" `
  --out out\nested
# content blocks 608020, fields 429633, RPCs 342735,
# movement 1839607, NetGUID rows 16167, decode errors 0

python tools\compare_combat_report.py         # ALL INTERESTING SHAPES MATCH

# Full 13.01 corpus (~27s, runs 16-wide)
python tools\validate_corpus.py .\target\release\vrfkit.exe `
  "C:\Users\yakihyuk0728\Documents\GitHub\valplay\data\raw\vrf"
# blocks 136,545,822  fields 98,883,979  rpcs 75,571,092
# malformed 0  skipped 1,972,080,670
# skipped bits may legitimately DROP if you resolve more groups -- report the
# new value and explain it. The other four must be exact.

# 13.02 guard
python tools\check_corpus_baseline.py --baseline tools\baselines\build_1302.json

# Metrics parity across all 11 replays with a reference bundle
python tools\validate_metrics_corpus.py --jobs 3
# sections exact on ALL : 16 / 21     <- must not regress
#   (15 real metric sections; `note` is a constant string)
```

If you touch `tools/to_valplay_bundle.py`, regenerate before checking parity:

```powershell
Remove-Item -Recurse -Force out\valplay_bundle -EA SilentlyContinue
python tools\to_valplay_bundle.py out\nested
python "C:\Users\yakihyuk0728\Documents\GitHub\valplay\pipeline\metrics\compute_metrics.py" `
  (Resolve-Path out\valplay_bundle\02d4d478-1dfb-4412-9a77-29ca29105a9d).Path
```

---

## 5. Read this before you trust anything written down

The last session fixed 8 bugs and **overturned 7 documented claims that were
false**. Every one had been written confidently and never measured. This is
the single most useful thing to know about this codebase.

| What the docs said | What was true |
|---|---|
| "All timestamp differences are exactly -1ms; a systematic bunch-boundary choice" | Truncation vs round-half-away-from-zero. Only ~half of timestamps moved. "All differences are -1" was true only of the rows that differed. |
| "the ability sections need a class-path-to-display-name table" | The reference does not use display names either. The difference was package path vs full object path. |
| "combat.per_player 270/270 exact" | 249 byte-exact; 21 differ by up to 3.6e-8 relative. The tolerance had been omitted. |
| "only fd816a35 has a reference bundle" | Eleven replays had both a bundle and a .vrf. |
| per-crate test counts | Wrong for 6 of 10 crates. |
| "tactical: 3 players differ" | 8 of 10 differ, and it is a reshuffle, not a gain. |
| **"malformed framing: 0"** | **The regex matched `Malformed:` while the oracle prints `Malformed framing:`. A non-matching pattern was silently skipped, so the counter had never been read at all.** It is genuinely 0 — but that was luck, not knowledge. |

Rules that fall out of this. Apply them:

1. **Re-measure any figure before quoting it.** PROJECT_STATUS is honest
   about intent and unreliable about numbers.
2. **A guard you have not seen fail is not a guard.** Break something on
   purpose and confirm the check reports it.
3. **Check a metric's direction, not just its value.** The movement bug was
   found because a distance was *lower* while sample count was *higher* —
   arithmetically impossible, therefore findable.
4. **"We emit more rows than the reference" is not self-evidently harmless.**
   It broke a downstream distance calculation.
5. **A share of the failures is not a share of the whole.** 7-C's "91.7%" is
   91.7% of the unattributed bits, which are 2.1% of the replay.
6. **Prefer a cheap discriminating check over inspecting values.** One bug
   was diagnosed in two queries: parity (all our values odd, all the
   reference's even — dynamic NetGUIDs must be even) and "does it resolve to
   a real actor" (1/115 vs 114/115). A rank-order pairing that looked
   meaningful was worthless.
7. **Verify what you built AND the sections it was meant to unblock.** A
   feature landed at 100% while a second field silently halved a downstream
   metric. The aggregate looked perfect.

---

## 6. Use your own subagents

This work parallelises well. Do it.

- **Fan out read-only investigation** when questions are independent. Two
  agents answering "what are these extra rows" and "are these claims true"
  ran concurrently last session and both changed the plan.
- **Use isolated worktrees for anything touching Rust**, so parallel agents
  cannot conflict.
- **Give each agent the invariants (section 3) and the verification protocol
  (section 4).** They follow them when told.
- **Explicitly authorise "not solvable" as success**, or an agent will
  manufacture a heuristic.
- **Give an agent the hypothesis you would test first and ask it to test that
  first.** One was told "check whether the data is merely unexported before
  treating this as a naming problem" — it ran that check first and reported
  it negative, which made the rest of its report credible.
- **Do not take agent conclusions at face value.** Re-verify anything
  load-bearing before passing it up.

---

## 7. The tasks

Two live (A and B); C is withdrawn -- see below. If a task turns out to be
blocked on something only the user can supply (replay files), say so and
proceed with the other rather than stalling.

TASK C being withdrawn is itself the lesson this brief keeps making: its
premise was written down confidently, never measured, and was false. Section
5 exists because that keeps happening here.

---

### TASK A — Do non-Bomb game modes parse?

**Never attempted. Zero evidence either way. Highest value.**

All 215 corpus replays and all 4 local demos are Bomb mode (standard
competitive/unrated). Deathmatch, Spike Rush, Escalation, Team Deathmatch,
Swiftplay and Premier have never been run through the parser.

What is expected but unmeasured: the container, transform, DemoFrame and
replication layers implement Unreal's wire format, not VALORANT's game rules,
so a non-Bomb replay *should* parse. Nothing has confirmed that.

Out of scope: metrics parity. valplay's `compute_metrics.py` reads
`BombGameState`, `BombPlayerState` and round results; other modes are outside
its design. The question here is purely whether the **parser** holds.

Steps:

1. Non-Bomb replays land in `%LOCALAPPDATA%\VALORANT\Saved\Demos`. If none
   are present, **ask the user to supply them and move on to B and C** — do
   not stall, and do not fabricate a substitute.
2. For each replay obtained, run `vrfkit validate`. The bar: does it parse,
   is malformed framing 0, what is the oracle pass rate, does the branch
   detect correctly.
3. If framing breaks, that is a significant finding — it would mean the wire
   format differs by mode, which nothing currently predicts. Investigate
   fully before concluding.
4. If it parses, characterise the differences: which export groups appear
   that Bomb never has, which Bomb groups are absent, what the
   unattributed-bit profile looks like versus Bomb.
5. Pin whatever you obtain with
   `python tools\check_corpus_baseline.py --baseline tools\baselines\<name>.json --update`
   so a future transform change cannot silently break it.

Deliverable: a measured answer to "does vrfkit parse non-Bomb replays", with
per-mode figures, plus a committed baseline if replays were available. "No
replays were obtainable" is an acceptable result stated plainly.

---

### TASK B — Build coverage beyond 13.01

**Partial. Transforms exist for five builds; only two have replays.**

```
build   seed        addend  op        sbox   replays
12.10   0x12fd0ee5  0x1b    subtract  no     NONE
12.11   0x409d36a3  0x23    ADD       no     NONE
13.00   0x2949b6ef  0x11    subtract  yes    NONE
13.01   0xe62fcd5c  0x24    subtract  no     215  (fully validated)
13.02   0x9e81a37c  0x04    subtract  yes    4    (pinned and guarded)
```

`TAIL_XOR == SEED_ADDEND & 0xFF` for all five, pinned as a test. S-boxes are
shared between 13.00 and 13.02.

The three older builds are golden-vector verified in `vrf-transform`, but no
real replay has ever gone through them end to end. **A golden vector proves
the transform maths; it does not prove the container, schema, replication and
decode path survive that build's stream shape.**

Steps:

1. Determine whether replays for 12.10 / 12.11 / 13.00 exist anywhere
   reachable. Search `valplay\data\` broadly and any archive directories, and
   ask the user. Do not assume the corpus directory is the only source — last
   session found eleven reference bundles where the docs claimed one.
2. For every build you can obtain a replay for: run the full export, then
   `check_corpus_baseline.py --update` into `tools/baselines/build_<n>.json`
   and commit the baseline.
3. For builds with no obtainable replay, record that explicitly in
   PROJECT_STATUS along with **what you searched**, so nobody repeats it.
4. **Independently of whether you find replays**, verify that an
   unrecognised build fails loudly rather than silently selecting the wrong
   transform. A wrong transform would produce garbage that framing checks
   might not catch. Add a test if one does not exist.

Deliverable: baselines for every build you obtained a replay for, an explicit
record of which remain unvalidated and why, and a test proving an
unrecognised build fails loudly.

---

### TASK C — WITHDRAWN. Do not work on this.

This task asked you to recover the ~5% of unattributed bits that
PROJECT_STATUS 7-C attributes to `MeleeAttackState1/2/3/4`.

**An audit measured it after this brief was written and the premise is
false.** MeleeAttackState contributes **zero** failing blocks across all 215
replays. `/Script/ShooterGame.MeleeAttackStateComponent_ClassNetCache` IS
declared in the schema, the existing digit-stripping resolution already
reaches it, and it emits 473 field rows on 02d4d478. There is nothing to
recover.

7-C's whole breakdown was wrong, because it could not be derived from any
committed tool -- `MAX_STREAM_FAILURE_RECORDS` in
`crates/vrfkit/src/sink.rs` caps the diagnostic at 32 lines, so the
percentages had been eyeballed from a truncated sample. Re-derived with the
cap raised, and cross-checked against the oracle's own totals:

```
                             7-C said   02d4d478   corpus
  AbilitiesAndBuffsComponent   91.7%      98.61%    97.28%
  MeleeAttackState1/2/3/4      ~5%         0.00%     0.00%   (0 blocks)
  RespawningWallPlate2_7       ~2%         0.02%     0.005%
  PatchVolume                  unlisted    0.66%     1.55%   (2nd largest)
```

If you want a task in this area, the useful one is **making that breakdown
derivable**: the 32-line cap means nobody can reproduce these numbers from a
committed tool, which is why a wrong breakdown survived. That is a small,
well-defined change to the diagnostics path. Confirm with the main session
before starting it.

---

## 8. Working agreement

**Branch.** Create `codex/<task>` off current `master` and work there. Do not
commit to `master`. If you need isolation for parallel subagents, use git
worktrees off your branch.

**Commits.** One logical change each. Match the existing style — read
`git log`. Messages state what changed, why, and the measured before/after.

**Docs.** Update `PROJECT_STATUS.md` as part of the work, not afterwards. If
you disprove something it says, say so explicitly in the document; the
corrections are as valuable as the fixes.

**Report back** with, per task:
- what you changed, or why nothing changed
- every figure from section 4, before and after
- for anything you concluded is impossible: the specific measurements that
  establish it, so it is not re-litigated
- anything you found that was **not** in scope but looks wrong — last session
  found three separate bugs that way

**Do not** modify valplay or ValorantReplayParser. Read them freely.

**Assume you will be checked.** The main session re-runs the corpus, the
guards and the parity harness on everything you hand back. State uncertainty
where you have it; it is cheaper than a retraction.
