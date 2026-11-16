# Phase C independent QA - resumed work in progress

The original QA worktree and four source files were recovered after the
execution environment reconnected. Their exact recovered bytes are committed
in `9f2b2de02026b4d971ce52ad918f5abbacb1ebd2`. This supersedes the historical
metadata-only recovery branch's source-availability limitation. That earlier
recovery document remains an accurate record of the interruption.

**This directory is not an acceptance receipt yet.** Prepared harnesses and
synthetic tests must execute against frozen, source-corresponding final
binaries before any Phase C acceptance claim is made.

## Observed checks after reconnection

- The expanded independent Clay FocusProjectiles checker completed against
  accepted historical Phase B public 12.05 exports. The aggregate receipt is
  `focus-array-historical.json`. This includes the actor-open corroboration
  whose final combined receipt rerun had been interrupted before recovery.
- Four newly pinned public inputs were validated and exported with the frozen
  accepted Phase B CLI. `extra-baseline.json` records those fresh old-code
  outputs separately from the historical six-fixture baseline. It reports
  23 checkpoints, 833 independent typed/raw values, and no audit failures.
- An independent raw re-read of the historical six exports validated all 33
  SpawnedCharacter rows: 32 with 16-bit packed GUIDs and one with 8 bits. Exact
  supported group, checksum 1413303948, actor-only address, sole u32 overlay,
  full raw consumption, and raw/typed equality were checked. Fresh final
  comparisons remain necessary.

The expanded scope is ten public replay inputs across seven build labels and
99 observed baseline checkpoints. No private replay inputs are included.

## Prepared gates

`freeze_build.py` compiles and freezes the CLI, public-API probes, and separate
independent synthetic contracts. It records runtime source hashes, the exact
commit/tree, compiler profiles, artifact hashes, and temporary probe hashes.
The Cargo test profile has opt-level 2 with debug assertions; this is a
functional verification build, not a performance measurement.

`public_qa.py` runs fresh validation/export, complete raw/typed audit, seven
checkpoint callback table comparisons, complete main/checkpoint counter maps,
the existing native/phase gameplay oracle, and every movement's independent
packet/lifecycle identity join. It requires explicit six-fixture historical
exports and four-fixture fresh old-code baseline paths.

`decoder_delta.py` predicts the narrow 12.05 Clay change from the old raw parent
bytes. It requires unchanged original rows and exactly 53 additive child rows
in their observed order. It normalizes only the documented counters after
checking their exact increments. All other table bytes, manifest values, and
summary counters must remain equal. `--focus-route` enables this explicit
comparison once the decoder implementation is present.

`identity_oracle.py` constructs packet intervals from exported actor lifecycle
rows and independently decoded raw SpawnedCharacter fields. It compares every
movement value, exact proof tuple, and explicit resolution status from the
Rust probe. It does not use time proximity, player account information, or
final GUID paths to infer earlier identity.

`contracts.rs` contains seven independently authored synthetic tests covering
strict packet boundaries, retained owned evidence, null retirement, ambiguity,
close/reopen, dormancy, conflicting opens, fragment ranges, missing/misaligned
provenance, malformed raw/schema evidence, and checkpoint consumer errors.
Successful execution and meaningful source mutations are still pending.

## Data handling and reproducibility

Raw probe streams, full manifests, detailed logs, and Parquet exports remain
local. Publish only reviewed aggregate receipts and synthetic sources. The
local export directories are necessary to resume validation; their presence
does not imply that an interrupted or unexecuted gate passed.

The extra baseline command must use pre-Focus-route tool sources, because the
new CLI summary adds counters that the frozen old executable cannot print.
Its saved receipt identifies the actual runner source commit and script hash.
Do not relabel an old executable as a current-source build.

No benchmark commands, throughput, timing comparison, CPU, or memory
improvement measurements are part of these gates.
