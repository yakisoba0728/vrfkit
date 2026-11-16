# Phase C independent QA recovery checkpoint - UNVERIFIED

Recorded on 2026-11-16 by independent QA role E after the execution environment became unavailable.

## Status and provenance

**This is a metadata-only recovery artifact. It is not Phase C acceptance.**

The four prepared QA source files listed below are **NOT preserved in this branch**. Their exact complete successful write/apply_patch inputs were no longer available in the agent's retained context after compaction, and the original filesystem was unavailable. No source has been reconstructed from descriptions. Recover the original files from the filesystem before attempting to resume them.

This document transcribes E's last observed findings and explicit limitations from the task conversation. It does not assert final local byte identity, a frozen QA source snapshot, or successful execution of Phase C acceptance gates. Historical Phase B exports are identified as historical throughout. A later recovery must re-read the underlying receipts and sources.

The authorized recovery branch is `phase-c/qa-recovery`, based directly on:

- Parent commit: `588be1effc8dfca2bed8810fd51a006828081c1b`.
- Parent tree, confirmed through the GitHub API during recovery: `5558934507cf1aab8c845fab54f70ca3435c8afc`.
- Repository: `yakisoba0728/vrfkit`.

This recovery operation creates documentation on the backup branch only. It does not create a PR, merge changes, advance main, or publish a release.

## Interruption and local state

Local QA worktree:

`/workspace/scratch/060a4b490a9f/phase-c-qa`

Local branch: `phase-c/independent-qa`, originally created at the parent commit above.

The final running local command reran the expanded FocusProjectiles evidence script and then attempted to cherry-pick A's accepted local commit:

`0e92e629cb1062de7348a9ec775fe82062ac22c0`

Polling local execution session `20097` returned:

> No usable execution environment is available.

Completion of the final Focus receipt rerun, completion of the cherry-pick, and the resulting worktree HEAD were **not observed**. Inspect the original worktree status and HEAD after recovery before continuing. No local execution was retried to create this remote metadata backup.

E had not launched any Cargo build. B held the exclusive build window when execution became unavailable. The intended independent build target was:

`/workspace/scratch/060a4b490a9f/targets/e`

The shared toolchain environment file was:

`/workspace/scratch/060a4b490a9f/runtime-env.sh`

It selected Rust/Cargo 1.86, one build job, disabled incremental compilation, and disabled test/dev debug information. The target must remain unique to E. Coordinate compilation with root because local memory and disk capacity were constrained. Do not run benchmarks or produce timing/performance claims.

The full task attachment was:

`/workspace/scratch/060a4b490a9f/upload/VRFKIT_Phase_C_Remaining_Engineering_Pro_Master_Prompt(1).md`

E had read that attachment, CLAUDE.md, and CONTRIBUTING.md before the interruption.

## Four source files requiring original filesystem recovery

All four were under:

`/workspace/scratch/060a4b490a9f/phase-c-qa/docs/evidence/phase_c/independent_qa/`

| File | Intended purpose | Last observed verification |
|---|---|---|
| `checkpoint_probe.rs` | Independent public Reader callback adapter to seven checkpoint Parquet tables; callback identity/order and complete counter coverage | Rustfmt completed; not compiled |
| `public_qa.py` | Fresh corpus invocation; export byte, manifest and counter comparisons; callback parity; existing independent gameplay checks | Python syntax check passed; corpus run not executed |
| `candidate_inventory.py` | Streaming aggregate candidate presence inventory without publishing raw identifiers | Historical six-fixture inventory completed |
| `focus_array_evidence.py` | Exact Clay schema/framing/member checks and malformed-input controls | Initial bounded run completed; final expanded receipt rerun unobserved |

The files were uncommitted at the last observed point. E had not recorded their final byte hashes. A descriptive table or this document cannot substitute for those missing bytes.

The checkpoint probe was designed to compare:

- Complete main and checkpoint network counter key sets and values.
- Complete main and checkpoint sink counter key sets and values.
- All integer checkpoint manifest summary counters.
- Exact bytes and callback row counts for the seven checkpoint Parquet tables.
- Checkpoint identity consistency, sequential indices, and callback sequencing.
- Movement/provenance vector alignment.
- Per-checkpoint partial source/identity and total partial bits.

An ordering detail remains relevant: CLI export appends checkpoint partials after all main partials, while callbacks follow replay file order. Public fixtures had no partial rows in the prior evidence; synthetic coverage must establish that behavior rather than infer it from empty output.

The public harness was still pinned to six inputs. It had **not yet been extended** for the four newly added public fixtures, and the independent detailed lifecycle/movement identity probe had **not yet been written**.

## Completed source-review finding: repeated active actor opens

E found that reopening an already active actor GUID on the same channel could preserve the previous lifetime and leave an earlier SpawnedCharacter declaration eligible for resolution. A repeated open without a dormancy transition does not provide sufficient evidence to retain a unique uninterrupted lifetime.

E reported the finding to B. B changed the implementation to record a conflicting open when an active GUID is already held on either the same or another channel. Historical queries before the conflict remain unaffected; later resolution is unresolved until the lifetime is ended and a new valid epoch is established.

B added the author test:

`repeated_open_without_dormancy_is_unresolved_on_same_or_other_channel`

**E did not observe this Rust test execute.** The fix and test remain author-reported/source-reviewed at this recovery checkpoint. Obtain B's exact source commit and execution receipt before treating the issue as independently verified.

The identity API's reported conservative rules also need independent tests: strictly earlier packet declarations, explicit zero retirement, actor lifetime boundaries, dormancy, ambiguity from multiple current claims, partial-bunch initial/completion provenance, and concurrent changes. Native-event timestamps do not establish identity joins.

## Verified historical baseline attribution

E verified the SHA-256 of the accepted Phase B frozen CLI at:

`/workspace/scratch/a42b1381f63d/qa-evidence/frozen-binaries/vrfkit`

Binary SHA-256:

`29fa495b19b7dbce2bfe7bbedc1dc9be2492a4c6a5aca2a9d1468d3d74c657d4`

It matched the tracked receipt:

`docs/evidence/phase_b/independent_qa/build.json`

The receipt's source commit was:

`2fbead4f716f12f7c5d41ff067ff4ee63a55dc90`

All **133 recorded runtime source hashes** matched the Phase C starting baseline before the attempted A cherry-pick. This established an attributable old executable without a new baseline build. It did not validate any Phase C implementation.

Accepted historical six-fixture exports were located at:

`/workspace/scratch/a42b1381f63d/qa-evidence/fresh-oracles-all-six/corpus-work/<input-sha>/export/`

The existing six-fixture parity receipt was:

`docs/evidence/phase-b-cli-public-parity.json`

It records the historical export fingerprints; the intended comparison covers 78 Parquet files and canonical manifest content, excluding only the established volatile manifest values `source_file` and `elapsed_ms`. A decoder change must explicitly account for any justified additive difference rather than claim byte equality.

## Bounded historical Clay FocusProjectiles result

This result came from accepted historical Phase B **12.05 public** exports. It is a bounded independent raw-payload analysis, not a fresh Phase C decoder acceptance result.

Exact class:

`/Game/Characters/Clay/Clay_PC.Clay_PC_C`

| Declaration | Schema handle | Checksum |
|---|---:|---:|
| Parent FocusProjectiles | 45 | 957945936 |
| Member FocusProjectiles | 46 | 831657485 |

The encoded member handle was **47**. Both declarations were present in the main schema and all **18 checkpoint schema sets** examined.

### Observed raw payloads

| Scope | Parent rows | Populated parents | Empty parents | Member values |
|---|---:|---:|---:|---:|
| Main | 102 | 53 | 49 | 53 |
| Checkpoints | 18 | 0 | 18 | 0 |

All observed target payloads consumed fully under the bounded independent grammar.

| Main payload shape | Rows | Payload width |
|---|---:|---:|
| Empty capacity-zero arrays | 46 | 16 bits |
| Empty arrays with one zero trailer | 3 | 24 bits |
| Populated arrays with 16-bit IntPacked member GUIDs | 18 | 64 bits |
| Populated arrays with 24-bit IntPacked member GUIDs | 35 | 72 bits |

The main array capacity distribution was:

| Capacity | Rows |
|---|---:|
| 0 | 46 |
| 1 | 51 |
| 2 | 5 |

The three 24-bit empty payloads had capacity 1 or 2 and no changed elements, followed by a single zero-byte trailer after the root terminator. Every checkpoint target payload was an empty 16-bit, capacity-zero array.

The bounded decoder required exact member identity, supported member bit widths, array bounds, nested and root terminators, and complete consumption. Its trailer allowance was a single zero byte for empty arrays only. No raw GUIDs or replay payload bytes are reproduced in this document.

### Independent framing controls

The completed bounded check accepted **3 positive synthetic cases** and rejected **10 malformed synthetic cases**:

1. Nonzero empty trailer.
2. Truncated empty trailer.
3. Truncated root terminator.
4. Wrong member handle.
5. Member width overrun.
6. Array index outside capacity.
7. Suffix after a populated array.
8. Missing nested terminator.
9. Runaway IntPacked GUID.
10. Overflowing IntPacked GUID.

These were malformed-input controls. They were **not Rust source mutations**, and they do not establish that a proposed Rust sink route rejects those cases.

### Actor-open corroboration and interrupted receipt distinction

A subsequent bounded comparison with the historical actor table found all **53 member GUIDs** represented by actor-open rows. The aggregate archetype counts were:

| Archetype | Members |
|---|---:|
| Default__Pawn_Clay_E_Boomba_C | 12 |
| Default__Projectile_Clay_4_ProjectilePrimary_C | 17 |
| Default__Projectile_Clay_Q_Satchel_Arming_C | 21 |
| Default__Projectile_Clay_X_Rocket_C | 3 |

Those GUIDs were absent from the final net_guids map. The actor-open comparison supplied the corroboration.

**Distinction:** the actor-table comparison was observed separately. E then patched the evidence script to include it in the combined receipt. Completion of that final combined rerun was not observed because execution became unavailable. Do not describe the expanded JSON receipt as confirmed until it is recovered, inspected, and rerun.

The intended receipt location was:

`/tmp/vrfkit-060a-phase-c-evidence/focus-array-historical.json`

It may contain the earlier receipt, an incomplete rerun, or the completed expanded receipt; its final state and bytes are unknown. The path alone is not execution evidence.

No identity attribution to players, damage credit, or gameplay event inference follows from the array-member/actor-open match.

### Recommendation recorded for decoder role C

The historical evidence supports considering an additive child-field route gated by the exact Clay class and both schema declaration checksums, with the evidence explicitly scoped to 12.05. Preserve the original raw parent and provenance.

Do not broaden the generic array grammar or apply a universal same-name property rule. The generic object-array decoder already admitted the observed narrow zero trailer; source review suggested the missing connection was the sink route. C must independently verify the exact existing code and any new route.

On this historical fixture, that route would add **53 main child rows**. Acceptance must account for the explicit bounded difference. Fresh final exports, schema/build identity verification, malformed-input rejection by the actual implementation, and checks against the additional public fixtures remain required. Other decoder families retain their evidence-blocked status.

## Historical candidate inventory

The completed streaming inventory scanned the historical accepted six-fixture exports for the bounded candidate names:

FocusProjectiles, ReplicatedMovement, InputEventData, StopEffectType, OwnerActor, AvatarActor, ActiveGameplayEffects, ActivatableAbilities, AbilitiesAndBuffs, Healing, HealingDone, HealingTaken.

The inventory aggregated names, checksums, widths, raw presence, and typed presence. It did not publish raw values or player identifiers. E's retained result includes these all-group FocusProjectiles counts:

| Build label | Main rows | Checkpoint rows |
|---|---:|---:|
| 12.05 | 391 | 159 |
| 12.08 | 288 | 141 |
| 12.09 | 394 | 181 |

These broader counts are distinct from the exact Clay class/checksum counts above. They do not justify broadening the Clay route.

The historical aggregate receipt was written to:

`/tmp/vrfkit-060a-phase-c-evidence/historical-candidates.json`

That file's final byte hash was not captured and its current availability is unknown. This branch preserves only the counts and qualifications stated here.

## Public corpus scope to recover

Original public fixture root:

`/workspace/scratch/d2a245f4874f/corpus-public/`

Root reported SHA verification of all six inputs against the Phase B pin receipt:

| Build label | Input SHA-256 |
|---|---|
| 12.05 | f5512818e79bd61d30ed8b7bbc34703e49f0e5a762acc96bf83fed08868c1b8b |
| 12.08 | 78e706843ea6992e5502d58f9b41c23ac5470a9e36e810d45e8724fb47a91201 |
| 12.09 | 925fe2d1dfce6dff071df724bff03faa7220ffd209bdf795b76888b65157efff |
| 12.10 | a4ce1b72f9bdf99492162013c1c909e6994a0d22bef1899e687fde71fbc86606 |
| 12.11 | 7a7a5492ddf286bb04413da96f0d3b216f91150e8174a3a4397493529e17ebdd |
| 13.00 | fd49091dd43171bb060eb6bbae50ed6677aa1077344572c5bf65f0c6fe2b4c1a |

The corpus auditor discovers .vrf files; the first three original public fixtures used .replay extensions. E's plan was to use read-only .vrf aliases while checking exact input SHA sets.

Root added four small public fixtures under:

`/workspace/scratch/060a4b490a9f/public-extra/`

The new input receipt is `receipt.json` in that directory. Root reported that Git blob SHA-1 identities matched the public reference tree at commit:

`b51d67423b7b4952d59051cf91e55efa1c42da05`

The four added inputs comprise one further 12.08 fixture and harbor, omen, and viper_wall fixtures from 13.01. Full new SHA-256 values were not retained in E's summary; recover the pin receipt rather than reconstruct them.

The final requested coverage is **10 inputs across 7 build labels**:

- Six inputs compared with the correctly attributed historical accepted baseline.
- Four additional inputs compared with fresh exports from the frozen Phase B CLI.
- Fresh final Phase C exports for all ten inputs.

E had not run the baseline CLI on the new four inputs or the final CLI on any input.

## Acceptance gates still incomplete

| Gate | E status at interruption |
|---|---|
| E Rust probe compilation | Not started |
| Fresh Phase C ten-input exports | Not run |
| Fresh four-input Phase B baseline exports | Not run |
| Callback/table/counter equivalence | Harness prepared, not executed |
| Exact lifecycle/movement identity oracle | Not implemented or executed |
| Independent raw validation of 33 historical SpawnedCharacter rows | Planned, not executed |
| Rust source mutation controls | Not run |
| Clean final executable freezing and source correspondence | Not established |
| Final source/file hashes and exact test count receipt | Not established |
| Phase C acceptance | **NOT GRANTED** |

B reported a historical set of 33 SpawnedCharacter rows with checksum 1413303948 and widths 8/16 bits for the requested raw oracle. E did not independently execute that check before interruption. Existing author tests and earlier historical gameplay oracles must not be relabeled as fresh independent Phase C evidence.

## Resumption sequence

1. Recover the original filesystem and inspect the interrupted session outcome, worktree status, HEAD, and four source files. Record hashes before editing.
2. Inspect and rerun the Focus receipt; retain the historical scope and the separate actor-check/final-rerun distinction.
3. Obtain exact accepted A/B/C/D source commits and source-correspondence receipts before assembling the final candidate.
4. Extend the harness to keep the six historical baselines separate from four fresh old-CLI baselines.
5. Run the verified frozen old CLI on the four new pinned public inputs.
6. Coordinate an exclusive compilation window; use E's unique on-disk target.
7. Compile and freeze clean candidate/probe executables before any source mutation.
8. Generate fresh final exports and compare exact tables, raw/typed/native output, manifests, and full counters for all ten inputs.
9. Execute Reader callback parity and independently test nonempty synthetic callback/order/error contracts.
10. Implement the independent lifecycle/movement oracle, raw SpawnedCharacter validation, and exact join comparison.
11. Run meaningful negative/source mutation controls, record observed failures, restore source, and verify restoration.
12. Record exact SHAs, trees, executable hashes, commands, actual test counts, and remaining limitations.
13. Commit only reviewed source and sanitized aggregate evidence through the root's authorized workflow.

Stop after the required gates are sufficiently verified. Do not expand into a broad audit, execute benchmarks, publish private replay data, or infer success from an interrupted command.
