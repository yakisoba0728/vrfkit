# Upstream delta audit — 2026-11-09

Continues [`UPSTREAM_AUDIT_2026_10.md`](UPSTREAM_AUDIT_2026_10.md) with the same method. A bounded cross-project review, **not** a claim that C# upstream and Rust vrfkit have identical semantics or that all upstream behavior was ported.

- Upstream: [michel-giehl/ValorantReplayParser](https://github.com/michel-giehl/ValorantReplayParser)
- Previous audited revision: [`09a8e1e1d81c48840b8810f523f1595e539b7d55`](https://github.com/michel-giehl/ValorantReplayParser/commit/09a8e1e1d81c48840b8810f523f1595e539b7d55) (2026-10-27)
- Audited upstream revision: [`400f7295964a7f6486378eac6decda3cefe42acb`](https://github.com/michel-giehl/ValorantReplayParser/commit/400f7295964a7f6486378eac6decda3cefe42acb) (2026-11-07)
- **13 commits** (6 non-merge, 7 merges), **47 net changed paths**: 15 added, 32 modified, 0 removed.
- Method: list every commit with `git log 09a8e1e..400f729` in a full clone, then compare the two complete Git trees by blob SHA (`git diff --name-status --no-renames 09a8e1e 400f729`; 621 and 636 blobs). A local tree diff has no API path limit. Each merge was checked against its first parent: none adds content beyond its branch's commits. Paths changed and reverted inside the range would not appear in this final-state inventory.
- Each entry below is a path-level change, not proof of line-by-line behavioral equivalence.

## Semantic triage (Rust vrfkit versus C# upstream)

| Upstream commit | Decision | Evidence |
|---|---|---|
| `5907abd` 12.09 damage RPCs (`KillCreditPlayerState` absent before 12.10, later handles one lower) | **Not ported; the fixed-handle bug does not apply to named parameters. Needs private 12.09 evidence for unnamed ones.** | See [12.09 damage handles](#1209-damage-handles-5907abd). |
| `5907abd` RoundResults 12.11 handle layout from 12.09 | **Already in vrfkit.** | `decode_round_results` selects members by declared name, never by handle. On the public 12.08, 12.10, 12.11 and 13.00 fixtures every `RoundResults[i]` member row is typed (0 null) with `Struct blobs: N decoded / 0 failed`, 12.08 included, which upstream's old layout did not cover. |
| `c5035c8` Hush Flashbulb as a flash, Suspension's field, new projectile registrations | **Ported where it decodes** (exact scoped identities); the flash-event enricher is not ported. | See [Hush](#hush-c5035c8). |
| `b70629b` crosshair settings (159-leaf positional binding, Legacy/Current orderings) | **Not ported.** vrfkit already types the leaves by name (`apply_type_corrections.py` ADDITIONS, `BombPlayerState`), and a name has one type in both orderings, so the reordering upstream found changes no vrfkit value. What upstream adds is section attribution (which `LineThickness` is primary, ADS or sniper), a derived view. | Needs replays of 12.08/12.10 and 12.11+ that carry crosshair blocks to verify the two orderings before any Python view is built on them. |
| `b0f0be0` `ValorantRoundPhaseChanged` | **Not applicable.** A C# enricher event mapping `ClientResetRound` / `ClientBuyPhaseEnd` / `MulticastEndRound` to phases. vrfkit exports those RPC rows and `MulticastSetPhase.NewPhase` (EnumByte); `tools/extract_rounds.py` derives round boundaries from them. | — |
| `d2ded83` `ActorOwnerReferences` / `ValorantActorOwnerObserved` | **Not applicable as an API.** vrfkit types `Owner` / `Instigator` on every group (`ENGINE_OBJECT_REFS`). The third reference upstream reads, `CreatedByCharacter`, is typed per exact identity; Hush's five ability items were added here (see below). | — |
| `d8f11f0` China 13.09 payload transform | **Not applicable to `main`.** China builds are not supported: `TransformVersion::from_branch` matches exact `++Ares-Core+release-NN.NN` strings, so `release-china-13.09` is refused with `UnsupportedBranch`, never decoded with a global transform. China work lives on [`BakaFT/vrfkit`](https://github.com/BakaFT/vrfkit) branch `china-release-support`. | Needs China-native vectors and a full replay, as for China 13.05. |
| C# descriptor/binder plumbing (`FieldDescriptor.HandleResolver`, `ReplayExportBinder`, `DecodedValueAssigner`), CHANGELOG/README, C# tests | Architecture-specific; no Rust counterpart. | — |

### 12.09 damage handles (`5907abd`)

Upstream binds every `FNetworkedDamageResponseData` member to a fixed handle, so a build without `KillCreditPlayerState` (handle 17) shifted every later member one handle. vrfkit names a parameter from the replay's own NetFieldExportGroup and types it by that name (`resolve_in_group`: declared name, then its `b`-prefixed spelling, then the handle). A declared name therefore cannot be shifted. On one local 13.09 replay every `MulticastNotifyDamage_{Base,Point}` and `MulticastReceivePlayerTemporaryDeathEvent_{Base,Point}` parameter arrived named (1,005 damage calls, 0 `_h{N}` rows).

The residual exposure is `OVERLAY_HANDLE_TABLE`. It maps `MulticastNotifyDamage_{Base,Point}` handles to the 13.x layout (`17 = KillCreditPlayerState`, `18 = RegionalDamage`, ...) and is consulted only when the wire gives no name or a bare decimal FName index. A different real name is refused and counted (`Handle conflicts`). If a 12.09 replay ever sent an unnamed damage parameter at handle 17 or above, it would be typed as the 13.x member at that handle, with no counter moving.

This could not be measured here. The public 12.08/12.10/12.11/13.00 fixtures declare no damage RPC group at all, and no 12.09 replay was available. Two things are known:

- `tools/fixtures/build_verification.json` records `main_overlay_handle_conflicts_refused` of 1-5 per 3-replay sample on every build 11.06-12.09 and 0 on every build 12.10 onward. Upstream's boundary is the same. The counter has no per-group breakdown, so whether those refusals are damage parameters is not known.
- `typed_mismatches: 0` in that file compares against an independent decoder that uses the **same** type assignment. It cannot catch a wrong-name/right-width row and is not evidence here. `KILL_OBSERVATIONS.md` reads `KillData`, not the damage RPCs, so it is not evidence either.

To close this, run on each private 12.09 (and 11.06-12.08) export:

```python
import collections, re, pyarrow.parquet as pq
t = pq.read_table("<export>/fields.parquet",
                  columns=["field_name", "value_i64", "value_f64", "value_bool", "value_str"]).to_pylist()
pat = re.compile(r"^(MulticastNotifyDamage_(?:Base|Point)|MulticastReceivePlayerTemporaryDeathEvent_(?:Base|Point))"
                 r"\.(?P<p>[^.\[]+)$")
c = collections.Counter()
for r in t:
    m = pat.match(r["field_name"] or "")
    if m:
        unnamed = m["p"].startswith("_h") or m["p"].isdigit()
        typed = any(r[k] is not None for k in ("value_i64", "value_f64", "value_bool", "value_str"))
        c[(m[1], m["p"], unnamed, typed)] += 1
for k, n in sorted(c.items()): print(n, *k)
```

vrfkit is immune on that corpus if no row has `unnamed=True, typed=True`. No `KillCreditPlayerState` row on 12.09 independently confirms upstream's claim. If unnamed typed rows exist, check their widths against the shifted layout before trusting them.

### Hush (`c5035c8`)

Upstream registers Hush's Flashbulb (overhand and underhand), Displacement (overhand and underhand) and Suspension projectiles, and Suspension's `GameObject_Paprika_X_LevitateZone_C`, with `Owner`, `Instigator`, `ReplicatedMovement` and `Selected Point`. In vrfkit, `Owner` / `Instigator` were already typed on all of them, the levitation zone included (`ENGINE_OBJECT_REFS`). The zone's `PlayerLevitatePositions` is variable-width and stays raw.

Measured on one local 13.09 replay (the only build with Hush), with `validate_type_evidence.py` and `check_rep_movement_levels.py`, then added as exact group/name/checksum identities to `tools/fixtures/scoped_type_evidence.json`:

| Identity | Rows | Check |
|---|---:|---|
| `ReplicatedMovement` on `Projectile_Paprika_{E_RedirectPull,Q_RedirectFlash,X_FullBendPop}_C` | 498 | ByteComponents consumes all; ShortComponents fails 157. 18 spawn joins at /1, each within 0.5; speed check 0.98-1.00. |
| `Selected Point` on the four Flashbulb/Displacement projectiles | 202 | All 192 bits, three finite f64 in map range; median distance to the projectile's own path 75-560 units, to other projectiles' paths 400-2,971. |
| `CreatedByCharacter` on the five `Ability_Paprika_*_C` items | 53 | 16-bit IntPacked; 53/53 resolve to `Paprika_PC_C`. |

The re-export moved exactly 753 rows from `Not in table` to `Decoded OK`, with `Decode errors: 0`. `validate_type_evidence.py --compare-typed` reported 0 failures, 0 mismatches and 0 missing identities.

**Not typed:** `ReplicatedMovement` on the underhand Flashbulb and Displacement, which consume as byte but have no spawn join, so their location level is unmeasured. `Actor_Paprika_X_ForceModuleProxy_C` movement (short-only rotator, not in the upstream delta). `LocationOffset` / `RotationOffset`, which upstream does not register. The flash-event semantics (windup-end detonation, `BlindObserved`) are C# enrichment with no Python counterpart yet.

## Revalidation needed before expanding the port

1. Run the 12.09 damage query above on the private legacy corpus (11.06-12.09) and record the counts here.
2. Re-check the Hush identities on more 13.09 (and later) replays, ideally one where an underhand projectile is thrown free so a spawn join exists.
3. Crosshair section attribution needs replays from both orderings before a Python view is built.
4. China 13.09 stays off `main` until a verified transform with native vectors and a full replay exists.

## Complete commit ledger (13)

| # | Commit | Subject | Author date |
|---:|---|---|---|
| 1 | [`b0f0be09e`](https://github.com/michel-giehl/ValorantReplayParser/commit/b0f0be09e0ca9046c577c5fd318eaf09dcb6d419) | feat: emit ValorantRoundPhaseChanged for round RPCs | 2026-11-01 |
| 2 | [`d2ded8320`](https://github.com/michel-giehl/ValorantReplayParser/commit/d2ded8320a239714ff6355650f31bb5c39c8e289) | feat: read actor owners in one place and emit ValorantActorOwnerObserved | 2026-11-01 |
| 3 | [`5907abd61`](https://github.com/michel-giehl/ValorantReplayParser/commit/5907abd61a7a7e75f865470f208a2e7a2f61aaac) | fix: decode 12.09 damage RPCs and round results | 2026-11-02 |
| 4 | [`d8f11f06d`](https://github.com/michel-giehl/ValorantReplayParser/commit/d8f11f06d1e07fd28fc96ffae6dfaba73dfedff3) | feat: support China 13.09 payloads | 2026-11-02 |
| 5 | [`0e9130065`](https://github.com/michel-giehl/ValorantReplayParser/commit/0e913006530b1f99608041fec961f8670d357db4) | Merge pull request #22 from michel-giehl/release-1209-and-china-1309 | 2026-11-02 |
| 6 | [`157f376b2`](https://github.com/michel-giehl/ValorantReplayParser/commit/157f376b26bedc58e996b8bd9d3ea50adeac5aaa) | Merge branch 'analyser-hardening' | 2026-11-04 |
| 7 | [`c5035c824`](https://github.com/michel-giehl/ValorantReplayParser/commit/c5035c82405c10ac6ee0e8cac8e05bd7579c838d) | Decode Hush's Flashbulb as a flash and register Suspension's field | 2026-11-05 |
| 8 | [`04dcbd928`](https://github.com/michel-giehl/ValorantReplayParser/commit/04dcbd92806cbb3005323ec987e4fa7c95ab6a1f) | Merge branch 'hush-abilities' | 2026-11-05 |
| 9 | [`b70629b0d`](https://github.com/michel-giehl/ValorantReplayParser/commit/b70629b0d9b7bb295f80ae55f08f77b0cc9cfff6) | Decode player crosshair settings from replays | 2026-11-06 |
| 10 | [`97fdb1867`](https://github.com/michel-giehl/ValorantReplayParser/commit/97fdb1867f45a889a13303b7003ee2a557e19746) | Merge crosshair-settings into preview | 2026-11-06 |
| 11 | [`762cc720e`](https://github.com/michel-giehl/ValorantReplayParser/commit/762cc720e173d4d511b3f1d5c4b32df6a184fa80) | Merge crosshair-settings: decode player crosshair settings from replays | 2026-11-06 |
| 12 | [`288878767`](https://github.com/michel-giehl/ValorantReplayParser/commit/2888787675904e2cd9c342ba0bfeb01e392bfd63) | Merge branch 'preview' into Main | 2026-11-07 |
| 13 | [`400f72959`](https://github.com/michel-giehl/ValorantReplayParser/commit/400f7295964a7f6486378eac6decda3cefe42acb) | Merge pull request #23 from michel-giehl/Main | 2026-11-07 |

## Complete final-state changed-path inventory (47)

`A` = added, `M` = modified, `D` = removed relative to the previous audited revision, from complete Git trees compared by blob SHA.

```text
M  CHANGELOG.md
M  README.md
M  src/Replay.Encoding/PayloadEncryption/PayloadTransformRegistry.cs
A  src/Replay.Encoding/PayloadEncryption/VersionedTransforms/ValorantSeededTransformChina13_09.cs
A  src/Replay.Models/Descriptors/ActorOwnerReferences.cs
M  src/Replay.Models/Descriptors/ExportGroupDescriptor.cs
M  src/Replay.Models/Descriptors/FieldDescriptor.cs
M  src/Replay.Models/Descriptors/FieldDescriptorBuilder.cs
M  src/Replay.Unreal/Parsing/DecodedValueAssigner.cs
M  src/Replay.Unreal/Parsing/FieldBinding.cs
M  src/Replay.Unreal/Parsing/FieldBindingCompiler.cs
M  src/Replay.Unreal/Parsing/ReplayExportBinder.cs
A  src/Replay.Valorant.Events/Crosshair/ValorantCrosshairEventEnricher.cs
A  src/Replay.Valorant.Events/Crosshair/ValorantCrosshairEvents.cs
M  src/Replay.Valorant.Events/Flashes/ValorantFlashEventEnricher.cs
M  src/Replay.Valorant.Events/Flashes/ValorantFlashEvents.cs
A  src/Replay.Valorant.Events/Ownership/ValorantActorOwnerObserved.cs
A  src/Replay.Valorant.Events/Rounds/ValorantRoundPhaseChanged.cs
M  src/Replay.Valorant.Events/ValorantEventEnricher.cs
M  src/Replay.Valorant/Combat/DamageParameters.cs
M  src/Replay.Valorant/Combat/DamageableComponentClassNetCacheDescriptor.cs
M  src/Replay.Valorant/Combat/MulticastNotifyDamageBaseParameters.cs
M  src/Replay.Valorant/Combat/MulticastNotifyDamagePointParameters.cs
M  src/Replay.Valorant/Combat/PointDamageParameters.cs
A  src/Replay.Valorant/Crosshair/CrosshairBlockBinding.cs
A  src/Replay.Valorant/Crosshair/CrosshairColor.cs
A  src/Replay.Valorant/Crosshair/CrosshairSettings.cs
A  src/Replay.Valorant/Crosshair/CrosshairSettingsUpdate.cs
A  src/Replay.Valorant/Crosshair/CrosshairWireLayout.cs
A  src/Replay.Valorant/Crosshair/CrosshairWireVariant.cs
M  src/Replay.Valorant/Descriptors/Agents/AbilityActorDescriptor.cs
M  src/Replay.Valorant/Descriptors/Agents/Paprika/PaprikaDescriptors.cs
M  src/Replay.Valorant/Flashes/Descriptors/FlashPaths.cs
M  src/Replay.Valorant/GameState/AresRoundResults.cs
M  src/Replay.Valorant/GameState/BombGameStateClassNetCacheDescriptor.cs
M  src/Replay.Valorant/GameState/BombGameStateDescriptor.cs
M  src/Replay.Valorant/GameState/BombPlayerStateDescriptor.cs
M  src/Replay.Valorant/GameState/MulticastReceivePlayerTemporaryDeathEventBaseParameters.cs
M  src/Replay.Valorant/GameState/MulticastReceivePlayerTemporaryDeathEventPointParameters.cs
M  tests/Replay.Encoding.Tests/PayloadEncryption/ValorantSeededTransformTests.cs
A  tests/Replay.Valorant.Events.Tests/Crosshair/ValorantCrosshairEventEnricherTests.cs
M  tests/Replay.Valorant.Events.Tests/Flashes/ValorantFlashEventEnricherTests.cs
M  tests/Replay.Valorant.Events.Tests/ValorantEventEnricherTests.cs
A  tests/Replay.Valorant.Tests/Crosshair/CrosshairSettingsTests.cs
M  tests/Replay.Valorant.Tests/Descriptors/ValorantDescriptorsTests.cs
M  tests/Replay.Valorant.Tests/Flashes/FlashDescriptorTests.cs
A  tests/Test.Integration/CrosshairIntegrationTests.cs
```
