# Legacy build support (11.06--13.00)

All sixteen builds from 11.06 through 12.09 have verified payload transforms:
their 48 available replays pass ReplayData validation and checkpoint-enabled
export, and each build's native reader matches Rust on 79 cases (1,264 native
cases in `native_vectors.rs`, beside the 88 golden vectors; the file also
holds 79 cases each for 13.08 and 13.09). Unknown branches
fail closed. The common all-build audit is
[BUILD_VERIFICATION.md](BUILD_VERIFICATION.md).

## Measured array routes

The checksum-gated structured-array routes are gated per build and per route
in [`measured_routes.rs`](../crates/vrfkit/src/sink/measured_routes.rs). A
route is admitted on a build only when, on every available replay of it:

- the replay declares every handle the route's tables are keyed on (the
  parent, the typed members, the nested container and its members) with the
  same name and checksum as the measured 13.x layout;
- every array counter stays zero on the main stream and in checkpoints:
  errors, truncations, implicit terminations, unconsumed root and nested
  bits, and array-leaf decode errors;
- an independent Python re-parse of every parent reproduces every emitted
  child: count, physical adjacency, name, handle, context, raw window and
  typed value, with no refused parent, no refused nested array, no
  zero-width member and no measured member left null;
- the route emits at least one child on that build. A route never observed
  there is unobserved, not verified, and stays off.

| Build | Replays | Admitted routes | Main rows added (typed) | Checkpoint rows added (typed) |
|---|---:|---|---:|---:|
| 11.06 | 3 | KillData, ServerActiveEffects, RequestedIgnoreActors | 31,342 (26,504) | 57,320 (50,918) |
| 11.07 | 3 | as 11.06, plus the projectile path | 26,854 (23,359) | 58,851 (52,325) |
| 11.08 | 3 | as 11.07 | 28,637 (24,731) | 63,384 (56,378) |
| 11.09 | 3 | as 11.07, plus SelectedV2 | 42,633 (33,546) | 327,309 (221,774) |
| 11.10 | 3 | as 11.09 | 34,076 (26,862) | 281,718 (188,373) |
| 11.11 | 3 | as 11.09 | 50,801 (40,960) | 353,780 (240,317) |
| 12.00 | 3 | as 11.09 | 35,850 (28,355) | 326,040 (219,319) |
| 12.01 | 3 | as 11.09 | 37,360 (29,668) | 304,730 (204,409) |
| 12.02 | 3 | as 11.09 | 40,056 (31,735) | 333,768 (225,019) |
| 12.03 | 3 | as 11.09 | 35,325 (28,012) | 308,703 (206,901) |
| 12.04 | 3 | every route but ActiveBlinds | 80,512 (48,281) | 464,924 (306,815) |
| 12.05 | 3 | every route but ActiveBlinds | 66,178 (39,720) | 341,460 (220,047) |
| 12.06 | 3 | every route but ActiveBlinds and the path | 61,880 (38,151) | 280,502 (180,526) |
| 12.07 | 3 | every route but ActiveBlinds and the path | 66,924 (40,451) | 363,603 (235,470) |
| 12.08 | 3 | every route but ActiveBlinds | 73,881 (46,395) | 340,929 (219,927) |
| 12.09 | 3 | every route but ActiveBlinds | 70,625 (42,852) | 347,023 (224,619) |
| 12.10 | 1 | AllPlayersObfuscatedPlayerInformation, TrackedRewards, SelectedV2, ServerActiveEffects | 524 (299) | 3,067 (1,875) |
| 12.11 | 1 | as 12.10 | 510 (294) | 2,627 (1,606) |
| 13.00 | 1 | as 12.10 | 510 (294) | 2,639 (1,614) |
| 13.01--13.06 | -- | every route | -- | -- |
| 13.08 | 1 | every route but the path | 26,121 (16,363) | 144,435 (94,792) |
| 13.09 | 2 | every route | 51,670 (33,059) | 252,290 (164,660) |

"The path" is `MulticastSetPath.NetworkedProjectilePath`. Across the 51
11.06--13.00 replays the admitted routes add 784,478 main and 4,562,377 checkpoint rows,
every one an additive child of a parent still exported whole. Rows added per
route, main / checkpoint:

| Route | Rows added | Typed |
|---|---:|---:|
| `AllPlayersObfuscatedPlayerInformation` | 7,587 / 108,402 | 5,778 / 72,268 |
| `TrackedRewards` | 173,562 / 154,630 | 62,224 / 55,225 |
| `SelectedV2` | 161,760 / 3,283,500 | 100,236 / 2,034,584 |
| `KillData` | 78,144 / 783,354 | 75,887 / 760,599 |
| `ServerActiveEffects` | 145,513 / 232,491 | 88,432 / 135,556 |
| `RequestedIgnoreActors` | 195,640 / 0 | 195,640 / 0 |
| `MulticastSetPath.NetworkedProjectilePath` | 22,272 / 0 | 22,272 / 0 |

"Typed" counts rows with a non-null value column and includes the nested
`EquippableAttachments` and `AssistingPlayers` references. It was measured
before the effect decoder filled `value_str` on `ServerActiveEffects[i]`
`FloatValues` / `ObjectValues` children, so the ServerActiveEffects and total
typed figures are low; "Rows added" is unchanged. The 12.10, 12.11 and 13.00
entries rest on one short public fixture each, where KillData,
RequestedIgnoreActors, ActiveBlinds and the path never occur.

### 13.08 and 13.09

Measured on three private replays (one 13.08, two 13.09; kept out of the
tree) with the same four criteria, main stream and checkpoints. Every route
handle is declared with its 13.x name and checksum in the main manifest and in
every checkpoint. The 13.08 replay declares no `MulticastSetPath` group, so the
path is unobserved there and stays off. A probe with the gate open on both
branches left every array counter at zero (errors, truncations, root/nested
residuals, implicit ends, leaf errors) in both streams; against the closed
gate every base row survives in order and the only added rows are route
children, equal to the `Route children` totals. An independent Python checker
(`wire_bits.parse_array` and `validate_type_evidence` decoders, with each
checkpoint's own declarations) re-derived every child of every parent: zero
mismatches, and it failed on a changed value, a deleted child and a corrupted
raw window. `validate_ability_array_evidence.py --compare-typed
--require-routes` passes (all 172 ActiveBlinds `SourceID` windows are the
297-bit inline form), and `verify_build_corpus.py` passes 3/3.
`extract_kill_observations.py` and `extract_kill_ledger.py` then join 476 of
479 character deaths (13.08: 169/171, 13.09: 307/308), with no identity or
checkpoint snapshot unresolved. "Typed" in the table counts the children the
route types, effect-array JSON and projectile-path values included.
ActiveBlinds has no checkpoint children on either build: all 477 checkpoint
parents are empty arrays.

### Why routes stay off

- **`AllPlayersObfuscatedPlayerInformation` and `TrackedRewards`, before
  12.04.** Two members (`StartOfRoundMoneyCache`,
  `StartOfRoundLoadoutValueCache`) sit at `OwnerExclusivePlayerInfo`
  handles 19--20, so `TrackedRewards` is at 21 instead of 18 and every later
  member moves by +3, with unchanged names and checksums. The member tables
  are keyed by handle, so none of the 26,325--32,416 main-stream
  `TrackedRewards` children per build would be typed.
- **`SelectedV2`, 11.06--11.08.** `DynamicMappings` (checksum 149045687) is
  declared at handle 13 and arrives with zero-width payloads, moving the
  nested `EquippableAttachments` array to handle 14, and the walker skips
  zero-width members without a counter.
- **`ActiveBlinds`, every legacy build.** Through 12.04 `SourceID` and
  `EffectID` swap handles 4 and 5. From 12.05 some `SourceID` values arrive
  as the 9-bit hardcoded-name form (flag 1, index 0) instead of the 297-bit
  inline string. The route refuses both and counts each refusal in
  `array_leaf_decode_errors`; `validate_ability_array_evidence.py` rejects
  the same parents.
- **Unobserved.** The path on 11.06, 12.06, 12.07 and 13.08, and four routes in
  each fixture, never produced a child.

Admitting a held-back route needs: the `OwnerExclusivePlayerInfo` and
nested `SelectedV2` member typing keyed by declared name and checksum (as
TeamEconomy selects its 12.01--12.05 handles); for ActiveBlinds,
declaration-keyed widths for the 11.06--12.04 swap and a rule for the 9-bit
`SourceID`, with the matching change in `validate_ability_array_evidence.py`.

## Reproducing 11.06--12.09

Lay out each build as `<root>/<build>/ShooterGame/Binaries/Win64/`, holding the
archived `VALORANT-Win64-Shipping.exe` and, for the seven protected builds
(11.06--12.00), the matching `stub.dll`; the recovered root is separate and
used only for those seven. Recovery needs optional `pefile`, `unicorn` and
`numpy`, native capture `pefile` and `unicorn`; ordinary Rust tests use the
committed vectors and need no game binary or emulator. The input and
recovered hashes are pinned in `tools/fixtures/native_recovery.json` and
`tools/fixtures/native_transform_readers.json`. The catalog also pins the
13.08 and 13.09 executables, which are not protected: `--check` reads them from
`<archive-root>/13.08/` and `<archive-root>/13.09/` like the other archived builds.

```powershell
python tools/recover_native_binaries.py --binaries '<archive-root>' --output '<recovered-root>'
python tools/capture_native_transforms.py --binaries '<archive-root>' --recovered-binaries '<recovered-root>' --check
cargo +1.86.0 test -p vrf-transform --locked
cargo +1.86.0 build --release -p vrfkit --locked
vrfkit validate '<replay-root>/12.01/sample-1.vrf' --diagnostics
vrfkit export '<replay-root>/12.01/sample-1.vrf' --out '<exports>/12.01/sample-1' --checkpoints
python tools/validate_type_evidence.py '<exports>' tools/fixtures/public_fixture_type_evidence.json --compare-typed
```

Repeat validate/export for samples 1--3 of every build; the required-counter,
nonzero-work and reconciliation checks are `check_decode_errors_corpus.py`'s.

The 48 samples are from
[`Matthias1590/unsupported-replays`](https://github.com/Matthias1590/unsupported-replays/tree/c5ba8419b96afa8e88e7afb3e594cf4cce9c75fe):
three per build, 11.06--11.11 and 12.00--12.09, renamed `.vrf` without
changing their bytes; that repository's SHA-256 manifest identifies them.
