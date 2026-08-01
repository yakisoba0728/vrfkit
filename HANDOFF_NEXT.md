# Handoff: three remaining tasks on vrfkit

You are picking up a VALORANT replay parser that is in good shape. This
document is the whole briefing: what the project is, the rules you must not
break, how to prove your work, the three tasks, and — most importantly — the
failure mode that this codebase kept producing and how the last session
learned to catch it.

Read `PROJECT_STATUS.md` after this. It is the authoritative record; this
file only tells you where to start and what to be careful about.

---

## 0. TL;DR of what you are inheriting

A from-scratch Rust parser (`vrfkit`) that replaces a C# reference parser
(`ValorantReplayParser`) for a Python analytics pipeline (`valplay`).

Current state, all measured:

```
tests                236 passing, clippy 0, fmt clean
13.01 corpus         215/215 parse, malformed framing 0
13.02 demos          4/4 parse, malformed framing 0
oracle pass rate     97.49% - 99.68% (median 99.32%)
metrics parity       16 of 21 sections byte-identical to the C# reference
                     on ALL 11 replays that have a reference bundle
```

The 5 sections that differ do so because we carry MORE data than the C#
parser, not less. Every difference is named and understood. Nothing is
BLOCKED.

Section 7 of PROJECT_STATUS has no open items. Everything there is either
done or closed with a measurement showing it cannot or should not be done.

---

## 1. Where things are

```
Parser (this repo) C:\Users\yakihyuk0728\Documents\GitHub\vrfkit
C# reference       C:\Users\yakihyuk0728\Documents\GitHub\ValorantReplayParser
                   READ ONLY. Has the user's uncommitted work -- 17 entries
                   in git status. Never commit, stash, reset, or modify.
                   If you must instrument it: add ONE clean file, run it,
                   `git checkout -- <that file>`, verify status is still 17.
valplay            C:\Users\yakihyuk0728\Documents\GitHub\valplay
                   READ ONLY. Run its scripts by absolute path only.
                   Never write into it -- compute_metrics.py writes
                   metrics.json into whatever bundle dir you pass, so always
                   pass a directory under vrfkit's out/.
13.01 corpus       valplay\data\raw\vrf   215 files, all ++Ares-Core+release-13.01
Reference bundles  valplay\pipeline\exports\<replay-id>\
                   12 of them; 11 have a matching .vrf in the corpus
13.02 demos        %LOCALAPPDATA%\VALORANT\Saved\Demos\*.vrf   4 files
```

Crate layout:

```
crates/
  vrf-bitio       LSB-first bit reader, UE wire primitives
  vrf-transform   per-build payload transforms, golden-verified
  vrf-container   .vrf container, chunk stream, Oodle decompression
  vrf-frame       DemoFrame iteration
  vrf-schema      the replay's own dynamic field schema + NetGuidCache
  vrf-net         Unreal replication pipeline (no skip path)
  vrf-decode      primitive decoders, nested arrays, struct blobs, overlay
  vrf-movement    remote-character update protocol
  vrf-export      columnar Parquet writers
  vrfkit          CLI: inspect / validate / export
tools/            Python generators, adapters and verification harnesses
```

---

## 2. Rules you must not break

These are in PROJECT_STATUS section 8. They are load-bearing: breaking one
silently corrupts downstream output without any test failing.

**NO SKIP PATH.** Every field emits `(group_path, handle, name, bit_count,
raw_bits)` even when nothing is known about it. The overlay is additive:
typed values fill `value_*` columns, failure leaves them null with the raw
bits intact. A parser that silently drops data cannot be trusted even when it
looks correct.

**NO SILENT SUCCESS.** A block whose group cannot be resolved fails loudly
(`function_count = 0` returns `Err`, counted and named). Never guess a
capacity or a group to make a number look better. The oracle's honesty
matters more than its pass rate.

**NO HARDCODED NAMES IN THE PARSER.** Resolution uses data the replay itself
declares. Weapon display names live in the Python adapter
(`tools/equippable_table.py`, generated) because labelling is a presentation
concern; the Rust crates emit class paths only.

**GENERATED FILES ONLY VIA GENERATORS.**
```
crates/vrf-decode/src/table.rs                    <- tools/extract_descriptors.py
                                                     then tools/apply_type_corrections.py
crates/vrf-transform/src/sbox.rs                  <- tools/extract_sboxes.py
crates/vrf-transform/tests/data/golden_vectors.rs <- tools/extract_golden.py
tools/equippable_table.py                         <- tools/extract_equippables.py
```
Hand-editing these is how subtle bugs enter.

**A CUSTOM C# DECODER MEANS THE TYPE IS UNKNOWN, NOT RAW.**
`extract_descriptors.py` cannot see through `.Decode(...)` in the C#
descriptors, so any field with a custom decoder lands as `FieldType::Raw` --
indistinguishable from a field we deliberately keep raw. Two real bugs came
from this. When new descriptors land, diff the `.Decode()` call sites against
the `Raw` entries in `table.rs` before trusting them.

**ASCII ONLY IN RUST CODE AND COMMENTS.** The Windows cp949 console truncates
output at the first non-ASCII byte in a format string. This is a correctness
constraint on the diagnostics path, not a style rule.

**NO UNSAFE.** `#![forbid(unsafe_code)]` everywhere.

**TDD.** Write the failing test first, watch it fail for the right reason,
then implement. If a test passes before you write the code, it is testing
existing behaviour and proves nothing.

---

## 3. How to prove your work

Run all of this. Report the actual numbers, not "it passed".

```powershell
cd C:\Users\yakihyuk0728\Documents\GitHub\vrfkit
$env:CARGO_TARGET_DIR = $null

# 1. Build health
cargo test                                        # expect 236 passed, 0 failed
cargo clippy --all-targets -- -D warnings         # expect no output
cargo fmt --check                                 # expect exit 0

# 2. Single-replay regression -- these must NOT change
cargo build --release -p vrfkit
.\target\release\vrfkit.exe export `
  "C:\Users\yakihyuk0728\Documents\GitHub\valplay\data\raw\vrf\02d4d478-1dfb-4412-9a77-29ca29105a9d.vrf" `
  --out out\nested
# content blocks 608020, fields 429633, RPCs 342735,
# movement 1839607, NetGUID rows 16167, decode errors 0

python tools\compare_combat_report.py             # ALL INTERESTING SHAPES MATCH

# 3. Full 13.01 corpus (~27s, runs 16-wide)
python tools\validate_corpus.py .\target\release\vrfkit.exe `
  "C:\Users\yakihyuk0728\Documents\GitHub\valplay\data\raw\vrf"
# blocks 136,545,822  fields 98,883,979  rpcs 75,571,092
# malformed 0  skipped 1,972,080,670
# NOTE: skipped bits may legitimately DROP if you resolve more groups.
#       Report the new value and explain it. The other four must be exact.

# 4. 13.02 guard
python tools\check_corpus_baseline.py --baseline tools\baselines\build_1302.json
# OK: 4 replays match the baseline

# 5. Metrics parity across all 11 replays with a reference bundle
python tools\validate_metrics_corpus.py --jobs 3
# sections exact on ALL : 16 / 21     <- must not regress
```

If you change the adapter (`tools/to_valplay_bundle.py`), regenerate and
recompute before checking parity:

```powershell
Remove-Item -Recurse -Force out\valplay_bundle -EA SilentlyContinue
python tools\to_valplay_bundle.py out\nested
python "C:\Users\yakihyuk0728\Documents\GitHub\valplay\pipeline\metrics\compute_metrics.py" `
  (Resolve-Path out\valplay_bundle\02d4d478-1dfb-4412-9a77-29ca29105a9d).Path
```

---

## 4. THE MOST IMPORTANT SECTION: how this codebase lies to you

The last session fixed 8 bugs. It also overturned **7 documented claims that
turned out to be false**, and that was the more valuable half of the work.
Every one of them had been written down confidently and never measured.

What was found:

| Claim in the docs | Reality |
|---|---|
| "All timestamp differences are exactly -1ms; systematic bunch-boundary choice" | Truncation vs round-half-away-from-zero. Only ~half of timestamps moved. "All differences are -1" was true only of the rows that differed. |
| "ability sections need a class-path-to-display-name table" | The reference does not use display names either. The whole difference was package path vs full object path. |
| "combat.per_player 270/270 exact" | 249 byte-exact; 21 differ by up to 3.6e-8 relative. The tolerance had been omitted. |
| "only fd816a35 has a reference bundle" | Eleven replays had both a bundle and a .vrf. |
| per-crate test counts | Wrong for 6 of 10 crates. |
| "tactical: 3 players differ" | 8 of 10 differ, and it is a reshuffle, not a gain. |
| **"malformed framing: 0"** | **The regex matched `Malformed:` while the oracle prints `Malformed framing:`. A non-matching pattern was silently skipped, so the counter had never been read.** It is genuinely 0 -- but that was luck, not knowledge. |

The last one is the archetype. It was quoted as the primary evidence that
framing is exact.

**Practical rules that fall out of this:**

1. **Never quote a number from PROJECT_STATUS without re-measuring it.** The
   document is honest about intent and unreliable about figures.

2. **A guard you have not seen fail is not a guard.** Before trusting any
   check, break something on purpose and confirm it reports the breakage.
   The 13.02 baseline guard was validated this way.

3. **Check the direction of a metric, not just the value.** The movement bug
   was found because `posture.distance_m` was *lower* than the reference
   while we had *more* samples. Finer sampling cannot shorten a path, so the
   value was impossible, which made it findable.

4. **"We emit more rows than the reference" is not self-evidently harmless.**
   It broke a downstream distance calculation.

5. **A share of the failures is not a share of the whole.** 7-C's "91.7%" is
   91.7% of the unattributed bits, which are 2.1% of the replay. The document
   led with the scary number for a long time.

6. **Prefer a cheap discriminating check over staring at values.** The
   `EquippableUsed` bug was diagnosed in two queries: parity (all our values
   odd, all the reference's even -- dynamic NetGUIDs must be even) and
   "does it resolve to a real actor" (1/115 vs 114/115). A rank-order pairing
   that looked meaningful was worthless.

7. **Verify what you built AND the sections it was supposed to unblock.**
   Weapon identity landed at 100% while a second field (`fire_mode`) was
   silently halving `spray_control`. The aggregate looked perfect.

---

## 5. Use subagents aggressively

The last session used them well. Concretely:

- **Dispatch read-only investigation agents in parallel** when questions are
  independent. Two agents investigating "what are these extra movement rows"
  and "are the ours-better claims true" ran concurrently and both came back
  with measurements that changed the plan.

- **Use isolated worktrees for anything that touches Rust**, so parallel
  agents cannot conflict. Two agents worked in separate worktrees on
  different items; both committed docs-only results that merged cleanly.

- **Tell agents the invariants and the verification protocol.** They will
  follow them. Both worktree agents ran the full corpus and reported exact
  figures.

- **Explicitly authorise "this is not solvable" as a successful outcome.**
  Both closed items came back negative with evidence, which is what made them
  cheap. An agent that thinks it must ship something will ship a heuristic.

- **Give agents the hint you would test first, and ask them to test it
  first.** One was told "check whether the data is merely unexported before
  treating this as a naming problem" -- it ran that check first and reported
  it negative, which is what made the rest of its report credible.

- **Do not take agent conclusions at face value.** Re-verify anything
  load-bearing. One agent's own report corrected a framing error in the
  claim it was asked to confirm.

---

## 6. THE THREE TASKS

They are independent. Run them in parallel if you can.

---

### TASK A — Other game modes (biggest unknown, highest value)

**Status: never attempted. Zero evidence either way.**

All 215 corpus replays and all 4 local demos are Bomb mode (standard
competitive/unrated). Deathmatch, Spike Rush, Escalation, Team Deathmatch,
Swiftplay and Premier have **never been run through the parser**.

What is likely and what is unknown:

- The container, transform, DemoFrame and replication layers are mode-
  agnostic in principle — they implement Unreal's wire format, not VALORANT's
  game rules. A non-Bomb replay should *parse*. This is an expectation, not a
  measurement.
- The metrics layer is Bomb-specific on both sides. valplay's
  `compute_metrics.py` reads `BombGameState`, `BombPlayerState`, round
  results. Other modes are outside its scope, so metrics parity is not the
  goal here.

**What to do:**

1. Obtain at least one replay per non-Bomb mode. The user will need to play
   or supply them; they land in `%LOCALAPPDATA%\VALORANT\Saved\Demos`. **Ask
   the user for these — do not stall waiting.** If none are available, say so
   and move to the other tasks rather than inventing a workaround.
2. Run `vrfkit validate` on each. The bar is: does it parse, is malformed
   framing 0, what is the oracle pass rate, and does the branch detect
   correctly.
3. If framing breaks, that is a genuine finding worth a full investigation —
   it would mean the wire format differs by mode, which nothing currently
   predicts.
4. If it parses, characterise what is *different*: which export groups appear
   that Bomb never has, which Bomb-specific groups are absent, what the
   unattributed-bit profile looks like.
5. Pin whatever you learn with `check_corpus_baseline.py --update` against a
   new baseline file, so the next transform change cannot silently break it.

**Deliverable:** a measured answer to "does vrfkit parse non-Bomb replays",
with per-mode figures, plus a baseline if any replays were available. A
well-evidenced "no replays were obtainable" is acceptable and should be said
plainly rather than papered over.

---

### TASK B — Build coverage beyond 13.01

**Status: partial. Transforms exist for five builds; only two have replays.**

```
build   seed        addend  op        sbox     replays available
12.10   0x12fd0ee5  0x1b    subtract  no       NONE
12.11   0x409d36a3  0x23    ADD       no       NONE
13.00   0x2949b6ef  0x11    subtract  yes      NONE
13.01   0xe62fcd5c  0x24    subtract  no       215  (fully validated)
13.02   0x9e81a37c  0x04    subtract  yes      4    (pinned, guarded)
```

`TAIL_XOR == SEED_ADDEND & 0xFF` for all five, pinned as a test. S-boxes are
shared between 13.00 and 13.02.

The three older builds are golden-vector verified (`vrf-transform`, 74 lines
of vectors) but **no real replay has ever been run through them end to end**.
A golden vector proves the transform maths; it does not prove the container,
schema, replication and decode path survive that build's stream shape.

**What to do:**

1. Determine whether replays for 12.10 / 12.11 / 13.00 exist anywhere
   reachable. Check `valplay\data\` broadly, any archive directories, and ask
   the user. Do not assume the corpus directory is the only source — last
   session found eleven reference bundles where the docs claimed one.
2. For any build you can obtain a replay for: run the full export, then
   `check_corpus_baseline.py --update` into `tools/baselines/build_<n>.json`
   and commit the baseline.
3. If a build has no obtainable replay, record that explicitly in
   PROJECT_STATUS with what you searched — so the next person does not repeat
   the search.
4. Consider whether `vrfkit inspect` correctly identifies each build from the
   header, and whether an unknown future build fails loudly rather than
   silently picking the wrong transform. **That last point is worth a test
   even if you find no replays**: a wrong transform on an unrecognised build
   would produce garbage that framing checks might not catch.

**Deliverable:** baselines for every build you can get a replay for, an
explicit record of which builds remain unvalidated and why, and a test that
an unrecognised build fails loudly.

---

### TASK C — The MeleeAttackState variants (7-C's tractable 5%)

**Status: identified, never attempted. Read the caution below carefully.**

PROJECT_STATUS 7-C says roughly 5% of unattributed bits are
`MeleeAttackState1/2/3/4`, and:

> The MeleeAttackState variants are tractable: if
> MeleeAttackState1_C_ClassNetCache etc. are added to the schema lookup
> logic, those would be recovered.

**Do not take that at face value. It has the exact shape of the claims that
turned out to be wrong.** Specifically, 7-C also says:

> digit suffix is a class variant, not an instance suffix; each has a
> distinct function table

and `crates/vrf-schema/src/cache.rs` (see `resolve_cnc_for_instance_name`,
around the trailing-digit fallback) currently **strips trailing digits**, so
`MeleeAttackState1` would resolve to `MeleeAttackState`. If each variant
genuinely has a distinct function table, that stripping is either already
producing a wrong `function_count` somewhere, or these blocks are failing
loudly instead — those are very different situations and the document does
not distinguish them.

Also relevant, from the 7-H investigation:
`MeleeAttackStateComponent_ClassNetCache` is reached from **five distinct
object names**. Instance name to class is many-to-one, so no string rule can
be correct in general.

**What to do, in this order:**

1. **Measure before designing.** For a replay where these appear, dump the
   actual failing blocks: what instance names, how many blocks, how many
   bits, and — critically — whether the schema declares
   `MeleeAttackState1_C_ClassNetCache` (or similar) at all. Search all
   declared export groups the way 7-C's `AbilitiesAndBuffs` check was done.
   If the groups are not declared, this is the same dead end as
   `AbilitiesAndBuffsComponent` and the task is closed with a measurement.
2. **Check the current behaviour.** Does the digit-stripping fallback fire
   for these names today? If it does, what group does it land on, and is that
   group's `num_exports` the same as the variant's real one? A wrong
   `function_count` changes the handle read width and desynchronises the
   stream — section 9 explains why this is destructive.
3. **Only then** decide whether a fix exists that does not violate NO
   HARDCODED NAMES. Adding `MeleeAttackState1` to a list is a hardcoded name
   and an automatic fail. A rule derived from what the replay declares is
   acceptable.
4. Whatever you conclude, the skipped-bit total should move (down if you
   recover blocks) or provably not move. Report the before and after.

**Deliverable:** either a fix with the corpus skipped-bit delta, or a
measured "not solvable without hardcoding" recorded in 7-C the way 7-H was
recorded. Both are good outcomes. A heuristic that works on one replay is
not.

---

## 7. Working agreement

- **Commit as you go**, one logical change per commit. Follow the existing
  message style: what changed, why, and the measured before/after. Look at
  `git log` for the shape.
- **Update PROJECT_STATUS** as part of the work, not afterwards. If you
  disprove something it says, say so explicitly in the document — the
  corrections are as valuable as the fixes.
- **If a task turns out to be impossible, that is a success.** Two of the
  three items closed last session were closed by proving they should not be
  done. Write down what you measured so nobody re-litigates it.
- **Do not touch valplay or ValorantReplayParser.** Read them freely.
- **Report actual numbers.** "Tests pass" is not a report. "236 passed, 0
  failed" is.

Good luck. The codebase is honest about its data and unreliable about its
prose — trust the former and measure the latter.
