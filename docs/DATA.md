# Extractable data

What you can get out of a VALORANT replay with vrfkit. With checkpoints the
export is thirteen Parquet tables plus `manifest.json`. Unknown property payloads and unresolved
whole RPCs retain raw bytes, but successfully decoded movement and synthesized
child rows may not duplicate their input bytes. "Untyped" is not synonymous
with "lost"; check stream-loss counters separately.

Legend: ✅ typed (value decoded) · ◐ raw or derivable · ❌ unavailable in the
stated observation scope. Absence in a sample is not proof of format-wide absence.

Current validation is [BUILD_VERIFICATION.md](BUILD_VERIFICATION.md): one
acceptance rule on 1,018 replays of 24 builds. Structured-array children are
admitted per build and per route ([below](#structured-array-children)).
Physical row ratios are not semantic completeness.

---

## Player identity

| Data | Source | Status |
|---|---|---|
| Account UUID (subject) | `manifest.players.subject` / `BombPlayerState.Subject` | ✅ |
| Character NetGUID | `manifest.players.character_net_guid` (the last body) and `character_net_guids` (every body, in write order) / `SpawnedCharacter` | ✅ joins movement 10/10 on 71 of 71 replays |
| Agent (characterId) | `manifest` game_specific_data.playerLoadouts | ✅ |
| Agent GUID, replicated | `BombPlayerState` / Swiftplay player state `A`, `B`, `C`, `D` (checksums 988169428, 943211507, 965590766, 1032080829) | ✅ four `UInt32` words of one FGuid, non-negative in `value_i64`, main and checkpoint. Formatted `%08x-%04x-%04x-%04x-%04x%08x` from (A, B>>16, B&0xffff, C>>16, C&0xffff, D), they equalled the header `characterId` on all 4,112 comparable sets in a 30-replay check (0 mismatches; the sampled 11.06-11.09 replay headers carry no loadouts). Names stay `A`..`D` on the wire; the byte-shaped `B` fields are separate properties |
| Two players on the same agent | disambiguated by `subject` (characterId alone can't) | ✅ |
| Display name / Riot ID | — | ❌ not established by the available evidence |
| `ProfileName` | PlayerState replicated FString | ✅ exact string decoded; its purpose is not established as a display name or Riot ID |

`SpawnedCharacter` is replicated again as 0 when a player disconnects, so the
rule is **last non-zero write wins** -- 0 is not a NetGUID. Last-write-wins lost
9 players across 5 replays with nothing looking wrong (64 of 69 replays joined
all ten, the worst 7); with the rule, 71 of 71 do.

**A reconnect gives a player a second pawn**, and `character_net_guids` keeps
both: on 39c2bb2c (13.05) PlayerState 256 writes `SpawnedCharacter` 1510, then
0, then 45530, and pawn 1510 carries 1,854 effect records. The analysis tools
take player bodies from that history through `tools/player_identity.py`. Over
the 1,018-export audit corpus, 10,426 `SpawnedCharacter` rows name 10,250 pawns,
each by exactly one PlayerState, and every named pawn's own `PlayerState` names
the PlayerState that spawned it. A pawn's `PlayerState` alone is not a body
criterion: 1,236 pawns carry a player's PlayerState without being a
`SpawnedCharacter` value, every one Astra's `Rift_TargetingForm_PC_C`.

## Economy

| Data | Source | Status |
|---|---|---|
| Current credits | `fields` `MoneyManagementComponent.Money` | ✅ Int32 |
| Start-of-round credits | `StartOfRoundMoney` | ✅ |
| Total granted | `TotalMoneyGranted` | ✅ |
| Team loadout value | `BaseTeamState.LoadoutValue` / `AverageLoadoutValue` | ✅ |
| Per-round balance change | `StartOfRoundMoney` − `EndOfRoundMoney` (OwnerExclusivePlayerInfo) | ◐ net balance change; grants, rewards, refunds and purchases must be separated before calling it spend |
| K / D / A | `BasicCombatStatsComponent.Aggregate*` | ✅ Int32 cumulative counters |
| ACS (combat score) | `PlayerScoreComponent.Score / rounds` | ✅ Int32 cumulative score |

## Purchase and inventory observations

| Data | Source | Status |
|---|---|---|
| Reported item | `PurchasedItemComponent.Purchaseable` → `net_guids.path` / `equippable_table.py` | ◐ replicated item state; repeated rows do not establish separate purchases |
| Who bought it | `PurchasedItemComponent.PurchasingPlayerState` → `manifest.players.subject` | ✅ |
| Observation time | `fields.time_ms` of the state update | ✅ replication time, not a guaranteed transaction timestamp |
| Which round | `time_ms` vs `events.roundStarted` | ◐ derivable |
| Cost | Temporally matched credit changes and item evidence | ◐ ambiguous grants/refunds and unmatched candidates must remain explicit |
| Source (buy/ability/etc.) | `PurchasableTransactionSource` | ◐ partial (some rows) |
| Inventory slot → item | `ItemSlot.Contents`, `AresInventory.ItemSlots` | ✅ / ◐ (MultiItemSlot raw) |
| Charges purchasable this round | `EquipmentChargeComponent.TotalChargesAllowedToPurchaseThisRound` | ✅ |
| Inventory correction counters | `AresInventory.CorrectionIndex` / `LastSeenClientCorrectionIndex` | ✅ Int32; strictly increasing per inventory, `LastSeen` always below `Correction`. What they count is inferred from the names only |

Join `Purchaseable` to `net_guids`, `PurchasingPlayerState` to player identity,
and update time to the latest preceding round boundary. Fields arrive as
separate updates: assemble component state and distinguish initialization or
round-start re-emission from a new item transition. A complete purchase ledger
requires corroborating credit changes; state rows alone are not that ledger.
Not every credit decrease is a spend: the side switch at half time and in
overtime resets credits, and that reset replicates as an ordinary `Money`
write 7-10 ms after `switchTeams`, before any buy phase opens.
`extract_match_observations.py` keeps those decreases (5,616 in the 1,018-export
corpus) out of its purchase evidence.

## Combat — kills & deaths

| Data | Source | Status |
|---|---|---|
| K / D / A | `fields` CombatReport nested array | ✅ multiset-identical (on 13.01 -- see below) |
| Kill log (killer/killed NetGUID) | `events.characterDeath` word0/word1 + `MulticastNotifyKilledEnemy` RPC | ✅ 132/132 on 13.01; 9,677/9,677 over 71 replays on 13.02 by the same two-source join; Event structural overlay exact on 109,126/109,126 chunks across 527 replays |
| Multikill level | `MulticastNotifyKilledEnemy.MultikillLevel` | ✅ single/double/triple/quad |
| Kill timeline | `events.characterDeath` time_ms | ✅ |

Deaths count every `bDied`. After a resurrect (Clove's self-revive, Sage's
raise) Riot's trackers drop some, so deaths can exceed theirs by up to the
resurrection count; `Rounds[N].Reports[1]` existing marks such a round.

## Combat — damage

| Data | Source | Status |
|---|---|---|
| Damage dealt / received | CombatReport `DamageDealt` / `DamageReceived` | ✅ |
| Regional damage (head/body/leg) | `Interactions[].Regions[].Hits/Damage` | ✅ multiset-identical (on 13.01) |
| Wallbang | `bIsWallPen` | ✅ |
| Damage source (weapon, location, bone) | `MulticastNotifyDamage` (EquippableUsed, ImpactLocation, ImpactBone) | ✅ |
| One row per damage invocation | `tools/extract_damage_events.py` -> `damage_events.parquet` | ✅ derived view |
| Finisher effect on a kill | `MulticastNotifyDamage_{Point,Base}.DeathMontageEffectOverride` → `net_guids.path` | ✅ ObjectNetGuid; 0 (the null reference) except on some kills, where it names an `FXC_*_C` finisher effect class |
| Death-montage context | `MulticastNotifyDamage_{Point,Base}.DeathMontageEffectOverrideContext` → `actors.parquet` (a dynamic actor: `net_guids` has no path for it) | ✅ ObjectNetGuid; 0 (the null reference) except on kills, where it is a player-character pawn open at the event. Not established as the killer or the victim |
| ADR | derived from CombatReport | ◐ +0.1–0.2 vs trackers (wire damage is fractional; not a bug) |
| Health / armour / overheal, absolute | `DamageableComponent` RPCs → `LifeChangeEvents[]` / `LifeChangeBySection[]` | ✅ typed section updates; actor/section timelines require joins, see below |
| Heal / overheal-decay source references | `MulticastNotifyHeal` `HealCauser`, `EventInstigator`, `EventInstigatorPawn`; `MulticastNotifyOverhealDecay` `DecayCauser`, `EventInstigator`, `EventInstigatorPawn` | ✅ `ObjectNetGuid` by exact group/name/checksum. `EventInstigator` is the instigator's PlayerController: it never joins to `actors.parquet` (join through the pawn's `Controller`/`Owner`), and that is expected, not a decode fault. `DecayCauser` = 0 means no causer. No heal credit is implied |

**The "multiset-identical" figures compare against an independent parser on
build 13.01 or earlier.** A result from that fixture does not establish
agreement on a newer build.

### Health is absolute, not a subtraction

The `DamageableComponent` RPCs carry an array whose elements hold
`ChangedComponent` (which damage section), `LifeResult` (**the absolute value
after the change**), `DeltaLife`, and `bAliveAfterChange`. Nothing has to be
accumulated to read a reported section result. The parent array remains in
`raw_bits`, and its decoded members are emitted as typed child rows. The array
uses ordinary RepLayout dynamic-array framing; its inner handles come from
the corresponding parameter-group declaration.

`vrfkit export` emits one typed row per member beside the parent blob row,
named `<Function>.LifeChangeEvents[i].LifeResult` and so on. The amount
relation is route-dependent: damage compares the negative delta sum, healing
and decay the positive sum.

Two things to know before joining on them. The local handles differ per RPC --
the same four members sit at 10-13, 1-4 or 2-5 depending on which function
carries them -- and `MulticastNotifyHeal` and `MulticastNotifyOverhealDecay`
name their array `LifeChangeBySection`, not `LifeChangeEvents`. A filter on the
array's name alone silently drops more than half the calls.

Three tools build on these rows, each keeping raw evidence and unresolved
references visible ([SECTION_OBSERVATIONS.md](SECTION_OBSERVATIONS.md)):
`tools/extract_section_observations.py` (all five routes, parentless amounts
and checkpoint rows), `tools/extract_healing_observations.py` (serialized heal
amounts, not effective HP) and `tools/section_timeline.py` (exact
predecessors, time- and packet-ordered comparisons, ordering/lifetime gaps).
None establishes a game life, player credit or a continuous health timeline,
and the historical measurements below do not replace their raw validation.

Verified over 69 replays on build 13.02: 377,487 elements, zero parse errors,
zero residual bits, and every observed element carrying exactly four members
in that measurement. Corroborated against separate decode
paths -- the historical report recorded scalar agreement on 230,855 of 230,855
and alive-flag agreement on 61,045 of 61,045. The scalar relation requires the
damage sign correction above. It also requires an explicit accumulation rule:
f64 accumulation rounded once and iterative f32 addition can disagree. These
historical totals are not a fresh whole-corpus transition verification.

Three observations from that historical 69-replay comparison:

- **The alive flag needs separate death-event corroboration.** Deduplicated
  per `(victim, RespawnNumber)`, a false flag matched `events.characterDeath`
  9,362/9,362 across all 69 replays; a zero-value rule missed on two, because a
  character really can sit at exactly 0 health and be alive (65 cases, all
  KAY-O). The flag is also re-reported after death, hence the RespawnNumber
  dedup. This historical join does not authorize substituting RPC
  `RespawnNumber` for `VictimRespawnNumber`; the two differ in current data.
- **Armour is `AttachedDamageSection`, not `ShieldDamageSection`.** The latter
  is an empty shell -- 67,316 elements, every `LifeResult` 0. The real armour
  section's outer is a `HeavyArmorItem_C` / `LightArmorItem_C` /
  `PlasmaArmorItem_C`, and its maximum reads 50.00 / 25.00 / 25.00, which is the
  game's own numbers and an outside confirmation that the f32 decode is right.
  Armour absorbs 2:1 against health on 12,747 of 12,747 hits where it survived.
  The section's own replicated properties (`bAlive`, `LastKnownDamageOwner`,
  and `Life` in checkpoints) are exported under its Blueprint class
  `BasicArmorAttachedDamageSection_C` -- or, in replays that also carry
  Phoenix's `PreventDeathDamageSection`, under the native
  `AttachedDamageSectionComponent`; see "The armour section" below.
- **`MulticastNotifyOverhealDecay` sends `DeltaLife` positive while life goes
  down.** Its magnitude matches `DecayApplied` 33,181/33,181, and the running
  chain only closes if the sign is flipped. `life += DeltaLife` runs overheal
  backwards.

The historical round-start comparison found `LifeResult - DeltaLife == 100` on the first health
event of 10,981 of 10,996 lives. The 15 exceptions all read 200 and are all
Phoenix -- Run It Back, not a decode fault. On the reset broadcast
(`MulticastSectionLifeChange`) the `LifeResult` is trustworthy and the
`DeltaLife` is not an established edge delta. Preserve both reported values,
but do not use reset DeltaLife as an accumulated change or fill unknown starts
with 100.

## Abilities

| Data | Source | Status |
|---|---|---|
| Ultimate cast corroboration | `events.characterUltimateUsed` (word0 resolves to a character on 15,699/15,768 rows) | ◐ do not count Event rows as casts: they outnumber `UltimateActive` False→True transitions by 51.5%; use the transition as authority and Event only as a ±100 ms cross-check |
| Cooldown / start time | `Comp_Ability_CooldownComponent` | ✅ Double |
| Ability cast observations | `Comp_AbilityStatisticsReplicator.AbilityCastsThisRound[]` — `Player` (subject UUID), `Slot`, `Round`, `RoundPhase`, `CastTime`, `CastLocation` | ✅ cast records repeated in array snapshots; deduplicate by cast identity before counting. In the reference sample, `Player` matches a manifest subject 352/352. |
| Ability state stream | `AbilitiesAndBuffsComponent` (`_cnc_h1`); bodies recovered from RepLayout tails: `/Script/ShooterGame.AresAbilitySystemComponent` (`__vrfkit_chained_cnc_h1__`) | ◐ FastArray numeric structure: replication keys, deleted/changed item IDs and raw field boundaries. On 1,018 exports all 3,999,493 `_cnc_h1` main windows (22 builds) and all 250,053 main and 181,108 checkpoint chained windows (24 builds) were consumed exactly. Available through `extract_fastarray_observations.py` on those routes, streams and builds. 12.10 and 12.11 have only chained rows; `_cnc_h1` checkpoint rows were never observed and remain unvalidated. Ability/effect names and field meanings remain unverified. |
| GAS owner / avatar / attribute sets | `AresAbilitySystemComponent` (OwnerActor, AvatarActor, SpawnedAttributes, CachedAttributeSet) | ◐ Component remap exposes these names. OwnerActor/AvatarActor remain raw: all 629,578 measured packed values equal the enclosing actor GUID, not a separately established player owner. See [the reference audit](GAS_AND_PATCHVOLUME_INVESTIGATION.md). |
| Status effects on a player (nearsight / slow / detain / ...) | `EffectManagerComponent:MulticastPlayContinuousEffect` + `MulticastStopContinuousEffect`, on the **affected** player's actor | ✅ named, with start and end — see below |
| Active effects on a component | `EffectManagerComponent.ServerActiveEffects[i]` children (`FloatValues`, `ObjectValues`, `VectorValues`) | ✅ `FloatValues` / `ObjectValues` carry the effect decoder's JSON in `value_str`, raw bits kept; `VectorValues` stays raw |
| Active gameplay effects (GAS array) | `AresAbilitySystemComponent.ActiveGameplayEffects` | ◐ No named rows in the ordinary property group in a 714-file audit (checkpoints included). Entries with that name and child handles occur under `AresAbilitySystemComponent_ClassNetCache` (49,076 rows). CNC framing can carry custom-delta properties as well as RPCs, so this location does not prove a function or an absent replicated array. This named array's item decoding remains unverified. |
| GAS attribute values | `AresAttributeSet.{BaseValue,CurrentValue}` per handle | ◐ **checkpoints only.** The live stream sends each attribute once when the channel opens and never updates it; `CurrentValue` does move (Reyna's ultimate puts handles at 1.1/0.9) but only checkpoint snapshots show it, and those are written at round transitions, so transient debuffs are gone by then |
| Persistent effect position (smoke/wall/molly/slow/trap) | `actors.parquet` class_path + spawn xyz | ✅ every spawned effect actor |
| Ground-area volume footprint (molotov, slow, net and wire patches; one circular `X` patch) | GroundVolumeComponent `FragmentInfo` cells, raw in `fields.parquet` (`PatchVolume` and the declared class) | ◐ decoded by `extract_ground_volumes.py`, not typed in Parquet: 36,661 of 36,661 windows exact in 1,018 exports, 346,186 cell updates with world polygon, floor, ceiling and grid cell, checked against a second reader and the owner's spawn. `Status` and `bIsActive` meanings unestablished. See [GROUND_VOLUMES.md](GROUND_VOLUMES.md). |
| Persistent effect lifetime | `actors.time_ms` paired across `event` `open`/`close` (non-fuel; a `dormant` event does not end the instance); `CurrentFuelLevel`+`WallActivated` (Viper) | ✅ |
| Smoke live position | `ReplicatedMovement.location` (world units, [per-class level](#replicatedmovementlocation-is-world-units-at-a-per-class-level)) / `MulticastAddSmokeScreenPoint.Translation` | ✅ as a track: `extract_active_effects.py --tracks` |
| Raze ability items (owner, persistence, attachment, seed) | `Ability_Clay_{4,E,Q,X}_*`: `CreatedByCharacter`, `bInPersistentData`, `AttachComponent`, `RelativeScale3D`, `CosmeticRandomSeed` | ✅ exact group/name/checksum; `CreatedByCharacter` resolves to the `Clay_PC_C` actor on every non-null main row |
| Hush (Paprika) ability owner and projectiles | `Ability_Paprika_*_C.CreatedByCharacter`; `ReplicatedMovement` on `Projectile_Paprika_{E_RedirectPull,Q_RedirectFlash,X_FullBendPop}_C`; `Selected Point` on the Flashbulb and Displacement projectiles, overhand and underhand | ✅ exact group/name/checksum, measured on one 13.09 replay: `CreatedByCharacter` resolves to the `Paprika_PC_C` actor on all 53 rows; movement in whole units with byte rotators ([per-class level](#replicatedmovementlocation-is-world-units-at-a-per-class-level)); `Selected Point` is VectorDouble (world units), at a median 75-560 units from its own projectile's path against 400-2,971 from the others'. What the point selects is not established. The underhand projectiles' movement stays raw |
| Raze satchel attachment | `Projectile_Clay_Q_Satchel_Arming`: `AttachComponent`, `LocationOffset`, `RotationOffset`, `RelativeScale3D` | ✅ exact identity; offsets are relative to the attach component, which resolves to world geometry or a character capsule |
| Raze Boom Bot position | `Pawn_Clay_E_Boomba.ReplicatedMovement` (short rotation) / `bAIControlled` | ✅ location in centimetres, checked against spawn; `bAIControlled` is true on every observed row |
| Cypher trapwire armed, trap owner | `GameObject_Gumshoe_{E,4}_TripWire(_SecondWire)_C.Deployed`; `CreatedByCharacter` on the trapwire and cage ability items; the cage item's `RelativeScale3D` | ✅ on both sides of the 13.01 rename (`Ability_E` trapwire -> `Ability_4`, `Ability_4` cage -> `Ability_Q`; typed at the new paths). `Deployed` is sent once per wire, always true: false is the class default and is never replicated, so the row's `time_ms` is when that wire went live. `CreatedByCharacter` resolves to the `Gumshoe_PC_C` actor. The cage projectile's `HasStopped` has no 13.01 successor field |
| Sova (Hunter) bolt trail point | `Projectile_Hunter_{Q_RevealBolt,4_ExplosiveBolt}_C.TrailPosition`, and on 11.06-12.05 `Projectile_Hunter_4_ExplosiveBolt_PrototypeBalance_C.TrailPosition` | ✅ VectorDouble (world units), exact group/name/checksum. One to three values per bolt, the first in the packet that opens the bolt's channel; every value lies on that bolt's own path (its spawn point and `ReplicatedMovement` locations), 94-101 units away at most and 22-24 at the median, against a median 1,129-2,146 for another bolt's path. What the game draws from it is not established |
| Possession state (cameras, drones and other possessable pawns) | `PossessableActorComponent_C.IsPossessed` and `Rift_PossessableActorComponent_C.IsPossessed` (component rows, on the pawn's channel); Cypher's camera `Pawn_Gumshoe_E_PossessableCamera_C.Possessed` / `IsDeployed` (under `Ability_Q/Pawn_Gumshoe_Q_PossessableCamera_C` until 12.08) | ✅ Bool, exact identity. `IsPossessed` is carried by `Pawn_Gumshoe_E_PossessableCamera_C`, `Smonk_PostDeath_PC_C`, `Pawn_Hunter_E_Drone_C`, `Pawn_Guide_Q_PossessableScout_C`, `Pawn_Cashew_4_Spider_LockOn_C`, `Pawn_Gumshoe_Q_PossessableCamera_C`, `Pawn_Aggrobot_RollyPolly_C` and (the Rift component) `Rift_TargetingForm_PC_C`. The camera's `IsDeployed` is one row per camera, always true |
| Killjoy turret and alarmbot on the ability item | `Ability_Killjoy_{E_Turret,Q_Alarmbot}_C.DeployedActor` | ✅ ObjectNetGuid, exact identity: GUID 0 (null) or the `Pawn_Killjoy_E_Turret_C` / `Pawn_Killjoy_Q_StealthAlarmbot_C` actor of the same export, on every row |
| Charge of a charged ability | `Comp_Equippable_Charged_C.CurrentCharge` | ✅ Double 0.015..1.0, exact identity; carried by `Ability_Wraith_4_Smoke_C` and `Ability_Mage_E_WorldSmoke_C` channels. The unit is not established |
| Raze satchel, Paint Shells and rocket position | `ReplicatedMovement` on `Projectile_Clay_Q_Satchel_Arming_C`, `Projectile_Clay_4_*` (renamed at 13.01) and `Projectile_Clay_X_Rocket_C` | ✅ typed, whole units, main stream only ([per-class level](#replicatedmovementlocation-is-world-units-at-a-per-class-level)) |
| Guide (Gekko) E projectile flight | `Projectile_Guide_E_HawkFlash_C`: `ReplicatedMovement` (ByteComponents), `Banking` (Double), `PostControlVelocity` | ✅ typed, main stream only (11.06-13.06). `location` is world units (whole units, measured against spawn -- [per-class level](#replicatedmovementlocation-is-world-units-at-a-per-class-level)). Velocity is in world units; roll is never replicated and reads 0. `Banking` is an angle in degrees, -180..180; what it banks is not established |
| Interaction progress (plant/defuse/orb pickup) | `UsableComponent.HighestProgress` (Float 0..1) / `bIsActive` | ✅ |
| Ult orb pickup (who, which orb) | `BaseGameState_C:OrbPickedUpRPC.{Orb Gatherer, Collectable Orb}` on the Bomb and Swiftplay game states (ObjectNetGuid) | ✅ 1,976 calls on a 76-replay sample (11.06-13.06): `Collectable Orb` is an `UltPointOrb_C` actor on every one; the gatherer's `NumUltimatePoints` rises by one within 1 s on 1,944, the other 32 at 7 or 8 points already |

### `CastTime` is not measured from `roundStarted`

Its zero is the barrier drop: the round's `ClientBuyPhaseEnd` (`MulticastSetPhase`
4), which `tools/extract_rounds.py` writes as `buy_end_ms`. `events.roundStarted`
fires at the buy phase's start instead. The absolute time is

    buy_end_ms + CastTime

On 13.01 the residual `(cast row's time_ms - buy_end_ms) / 1000 - CastTime` has
median -0.000 s over 420 first sends (13.05: -0.002 s over 106; 13.06: 0.000 s
over 406), with `buy_end_ms` and the raw `ClientBuyPhaseEnd` rows picking the
same epoch for every cast row. Against `roundStarted` the same casts read
29.89 s, and 10,460 casts over 20 replays read 44.99 s on the first round of
each half and 29.88-29.91 s otherwise: the 45/30 s buy phases. Do not hardcode
them; the public fixtures' buy phases last 0.1-15.5 s.

Use the median of first sends, not the mean. `AbilityCastsThisRound` is a
replicated array that accumulates over the round, so a cast is re-sent on every
later replication and its `time_ms` drifts upward (median 0.066 s over all 579
cast rows on 13.01; 299 of the 420 first sends land within 0.5 s).

### Status effects, and where they actually live

For an executable view of these observations, use
`tools/extract_player_effects.py` (see [USAGE.md](USAGE.md#analysis-helpers)).
Effect replication can also target cameras, drones and decoys. The tool keeps
those observations but admits a player target only through a
`SpawnedCharacter` value -- the manifest's, or an earlier one the manifest
dropped when the player reconnected (see [Player identity](#player-identity));
possession alone is not a player body.

A debuff shows up as a continuous effect played **on the affected player's own
actor**, not on the caster's. `EffectManagerComponent`'s
`MulticastPlayContinuousEffect` carries an `EffectContainer` NetGUID that
resolves through `net_guids.parquet` to a named effect, and
`MulticastStopContinuousEffect` closes it by `EffectID`. On the reference replay
every one of the 55 nearsight applications has a matching stop, so start, end
and victim are all recovered.

The names say what the effect is and the measured durations match the game:

| effect | applications | median duration |
|---|---|---|
| `FXC_Wraith_Q_NearsightMissile_Nearsight_C` (Paranoia) | 13 | 2.37 s |
| `FXC_Vampire_4_NearsightAOE_Nearsight_C` (Leer) | 15 | 0.31 s |
| `FXC_Wushu_4_SmokeNearsight_C` | 15 | 0.95 s |
| `FXC_Deadeye_4_Trap_Slowed_C` (trap slow) | 2 | 2.01 s |
| `FXC_Aggrobot_X_DetainDebuff_C` (detain) | 1 | 3.15 s |

An AoE applies to several victims in the same tick, each as its own row, so
"who was affected" comes out per player rather than per cast.

Over 71 demo replays the Play/Stop pairs give a much larger duration sample.
These are not corrections to the table above -- that one is honest about being a
single replay -- but they are the numbers to quote. **Durations only:** this
pass paired by `EffectID` to measure length and did not account for every
unpaired Play, so it says nothing about the termination rate above.

| effect container | n | median |
|---|---|---|
| `FXC_Vampire_4_NearsightAOE_Nearsight_C` (Leer) | 823 | 0.43 s |
| `FXC_Wushu_4_SmokeNearsight_C` | 645 | 0.84 s |
| `FXC_Grenadier_Player_Suppressed_C` (suppress) | 369 | 8.00 s |
| `FXC_Deadeye_4_Trap_Slowed_C` (trap slow) | 299 | 2.23 s |
| `FXC_Global_ConcussedWavy_Prototype_C` | 273 | 2.60 s |
| `FXC_Wraith_Q_NearsightMissile_Nearsight_C` (Paranoia) | 216 | 2.32 s |
| `FXC_Thorne_4_PlayerMovingInSlowField_Production_C` | 170 | 4.89 s |

Suppress lands on 8.00 s, which is the figure the 215-replay pass below reached
by a different route -- two independent measurements agreeing is what makes the
rest of the column trustworthy.

**Exclude the caster-side containers.** `_Equip_C` and `_Cast_C` play on the
*caster's* actor and are the cast animation, not an application: counting them
inflates Leer by 1,365 and Paranoia by 390 over these replays, and would make a
one-sided cast look like a debuff on the caster. The three seen are
`FXC_Vampire_4_NearsightAOE_Equip_C`, `FXC_Wraith_Q_NearsightMissile_Equip_C`
and `FXC_Sequoia_Q_FragileMissile_Cast_C`. The `_Equip_C`/`_Cast_C` suffix is
the observed convention on those three, not a guarantee -- like the container
names above it is cosmetic-asset naming, so a caster-side effect under some
other suffix would not be caught by a suffix filter alone. Confirm against the
actor the effect plays on when it matters.

Two caveats. The container names are cosmetic-effect assets, so the same
gameplay state can arrive under more than one name and a pure-audio variant sits
beside the real one (`..._DetainDebuff_Audio_C`). And this is a *visual* effect
channel: strong evidence the state was applied, but not an authoritative flag --
no component carries an `IsVulnerable`-style boolean anywhere. A status is
always reconstructed as an interval from a start/stop pair.

Measured across all 215 replays:

| status | signal | coverage |
|---|---|---|
| suppressed | `FXC_Grenadier_Player_Suppressed_C` | 1,021 windows over 83 replays, zero unterminated; median 8.0 s |
| vulnerable | `..._Fragile*` effects (the internal name is **Fragile**, not Vulnerable) | 4.0 s duration; 82 of 215 replays show it by one signal or another |

**Vulnerable doubles damage, exactly.** `DamageDealt / FalloffMultiplier`
recovers each weapon's base damage, and over 123,008 gun and melee hits that
ratio is 1.0 on 122,284 and exactly 2.0 on 171. Inside a Fragile window the 2x
rate is 99/99; outside it is 72 in 122,356. A separate pass over the 71 demo
replays -- a different corpus, so a different denominator -- put the in-window
rate at 34 of 61 against 21 of 41,378 outside, a gap of the same three orders
of magnitude. Read the direction as settled and the in-window share as
depending on how completely the Fragile windows were paired.

The multiplier sits outside falloff,
so a Vandal headshot reads 320 = 40 x 4 x 2. Use `DamageDealt`, not
`DamageTaken` -- the latter is clamped to remaining life.

### Callout regions: the export names the actor, not the callout

`CalloutRegionTrackingComponent.CurrentRegion` resolves through `net_guids` to
the region actor a player stands in, which gives a position as a map callout
rather than as centimetres. What the export carries is correct: the actor's
object name, `CalloutRegion_A_Short`, whose outer chain ends at the map's
callout level, `/Game/Maps/Triad/Triad_Callout_Volumes`. That name is a level
designer's label, not the callout. The game's name for the region is the
actor's `RegionName`, a string-table text stored in that level; the replay
does not carry it, and nothing in this repository reads it.

Actor names come in two forms. The split was first measured over 64 demo
replays covering 12 maps; the 13.06 game files confirm it for all 13, Bind
included (160 region actors on the first seven maps, 140 on the other six):

| letters left after the prefix | numbered |
|---|---|
| Ascent, Bonsai (Split), Duality (Bind), Infinity (Abyss), Port (Icebox), Rook (Corrode), Triad (Haven) | Canyon (Fracture), Foxtrot (Breeze), Jam (Lotus), Juliett (Sunset), Pitt (Pearl), Plummet (Summit) |

The prefix varies independently (`BP_`, `InfinityCallout`, bare), so the
form is whether letters survive after stripping it. A numbered name
(`BP_CalloutRegion10`, `BP_CalloutRegion_C_0`) has nothing behind it. A
lettered name is not a label either: on 9 of the 160 lettered actors the
game's `RegionName` is another word.

| map | actor | `RegionName` |
|---|---|---|
| Ascent | `CalloutRegion_A_Link` | Tree |
| Ascent | `CalloutRegion_Back_B` | Boat House |
| Split | `BP_CalloutRegion_B_Main` | Garage |
| Haven | `CalloutRegion_A_Short` | Sewer |
| Haven | `CalloutRegion_C_Short` | Garage |
| Icebox | `BP_Callout_B_Angled_Container` | Yellow |
| Bind | `CalloutRegion_B_Lobby` | Fountain |
| Ascent, Haven | `CalloutRegion_Mid` | Courtyard |

Another word is not necessarily another place -- some may be synonyms -- but a
label read off the actor name is not the game's. Take names from `RegionName`
on every map; the two forms only say where the actor name is legible.
valorant-api.com lists the same names -- its callouts matched the game's
`RegionName` and super-region on all 300 regions of the 13 maps in 13.06 --
but keyed by a point location, not by the actor.

Method and provenance: the callout levels of the installed 13.06 game were
read statically and each region actor's `RegionName` resolved
through the map's string table; no game file or extract is in this
repository. Lettered means letters remain after stripping `BP_CalloutRegion`,
`CalloutRegion`, `BP_Callout`, `InfinityCallout` or `BP_`; another word means
neither name contains the other once the site letter and non-letters are
dropped (`AttackerSpawn` against "Spawn" counts as the same).

Spike sites have the same trap: a `BombDestination` actor's name does not
give its site. In Icebox's bomb-mode level, `BombDestination_A_0` has
`BombSite` B and `BombDestination_B2` has A. Use `TimedBomb.PlantedAtSite`,
which agreed with the game's plant volumes on all 50 plants of nine 13.06
replays.

The regions are still usable without names -- the id is stable within a map
and the spatial extent can be recovered by pooling player positions per
region.

### Slows are visible in the movement data itself

Independently of the effect channel above, a slow is legible from speed alone,
because VALORANT's horizontal speeds sit on a multiplicative lattice off
675 cm/s:

```
675.00 run   607.50 (0.90)   573.75 (0.85)   540.00 (0.80, rifle ADS)
513.00 (0.76)   506.25 (0.75)   405.00 (0.60)   324.00 walk
```

Inside a slow zone every one of those values appears **halved** -- 337.50,
303.75, 286.88, 270.00, 256.50, 253.13, 161.90. Measured across 70 replays and
128,324,174 movement rows, the multiplier is 0.500 to within 0.04%. It is a
multiplier and not a cap: values already below 337.5 are halved too.

Matching a speed against the halved lattice to within ~1% classifies a slow with
under 1% false positives on four negative-control zones (smokes, cages, molly,
heal pool). A plain threshold does not work -- walking is 324 and slowed running
is 337.5, 13.5 cm/s apart.

Effective radius is roughly 500-600 cm and the estimate is genuinely unstable
below that, so treat it as a range. The slow lingers about 0.3-0.5 s after
leaving. Sage's orb and Chamber's trap slow; Fade's Seize and Terra's time-slow
grenade showed no movement-speed effect at their actor's position.

**Crouch is not in `movement_state`.** That column is 0 on all
1,034,035,170 exported movement rows of a 527-replay 13.01-13.04 corpus. Crouch
is `bCrouchHeld` on the character actor, or a ~19 cm drop in `pos_z`, and
crouch speed is ~190 cm/s.

## Movement & position

| Data | Source | Status |
|---|---|---|
| Position (cm) | `movement.parquet` pos_x/y/z | ✅ ≤0.0005 against an independent parser (on 13.01) |
| Rotation (yaw/pitch) | movement | ✅ exact |
| Velocity | movement vel_x/y/z | ✅ exact |
| Time (128 Hz tick, resets per round) / global | movement `timestamp` / `time_ms` | ✅ — `timestamp` is a **tick counter**, not milliseconds |
| Posture (crouch) | `fields.bCrouchHeld` (not movement_state) | ✅ |
| Trajectory | movement time series per character | ✅ |
| Force modules on a character (tagging, knockback, movement modifiers) | `ForceModuleManagerComponent` RPCs: `NetMulticastApplyForceModule` -- `Module` (→ `net_guids`, a `ForceModule_*` class), `ModuleType`, `Character` (equals the row's actor), `RespawnNumber`, `NetTimestamp`, `HandleNumber`, `SourceLocation`, `Source`, `Duration`; `NetMulticastRemoveForceModule` -- `HandleNumber`, `ModuleType` | ✅ typed: `Source` and `Duration` by exact group/name/checksum, Remove's `ModuleType` through Apply's checksum, the rest by name. `ModuleType` names are unknown: 0 is most modules and 2 the six displacement ones on Apply; Remove's 1 has no established meaning. `NetTimestamp` is a per-actor/per-life clock, not replay time |
| Ability projectile, smoke, deployable, pawn (turret, bot, decoy) and dropped-weapon position | `fields.ReplicatedMovement` `location` / `linear_velocity` (JSON in `value_str`) | ✅ world units on the 89 classes the overlay types (26 in the table, 63 scoped) -- [see below](#replicatedmovementlocation-is-world-units-at-a-per-class-level) |

### The tick is 128 Hz by a 3:13 pattern, not by alternating

Consecutive per-character steps are 7 ms on 18.72% and 8 ms on 81.28% -- that is
3:13, or 3/16 and 13/16. The mean is 7.8128 ms, **127.995 Hz**. It is not a 1:1
alternation, so do not assume 7,8,7,8 when reconstructing a clock; count ticks
and multiply by 125/16. Measured over 37,775,664 steps on 20 replays.

3x7 + 13x8 = 125 ms is arithmetic on that ratio, not a measurement, and the two
part company at the tail: of 7,836,799 sixteen-tick windows, 95.93% come to
exactly 125 ms, and 0.45% of single-tick steps are neither 7 nor 8 ms (up to
234 ms). Use 125/16 as the nominal rate; do not assume any given window hits
it.

`timestamp` increments by 1 per tick (Δ=1 on 37,938,231 steps, against 34 steps
of 3 or 4) and resets each round, which is why the round cut is found at the
point it drops rather than from a timer.

`time_ms` is the global scrub axis and is non-decreasing on all 71 replays --
though not strictly increasing, since many rows share a `time_ms`.
`duration_ms - max(time_ms)` lands within -1..8 ms.

### Minimap projection

`tools/minimap.py` holds the projection and checks it against an export. The
constants are not in the replay: valorant-api.com publishes `xMultiplier`,
`yMultiplier`, `xScalarToAdd` and `yScalarToAdd` per map, keyed by `mapUrl` =
`manifest.level_names_and_times[0].name`, and the user supplies that file.
**The axes cross:**

    u = pos_y * xMultiplier + xScalarToAdd
    v = pos_x * yMultiplier + yScalarToAdd

Over 12 maps on 69 replays (121,672,885 live rows, build 13.02) this puts
100.0000% of live positions inside [0,1]² on eleven maps, with bounding boxes
filling roughly [0.01, 0.99] (containment alone proves nothing: a small enough
scale contains everything); feeding `pos_x` to `u` collapses to 0.9% on Haven
and 3.1% on Fracture. Abyss's symmetric constants cannot tell the two orders
apart. Hidden actors park at `pos_x ≈ -50000, pos_z ≈ -49900`; filter on both
x and z, since a fall passes through that z.

**Abyss reads 99.68-99.84%, and that is not a decode fault.** Of its
out-of-range rows about six in seven are already below z = -3000, and 99-100%
of the rest have negative `vel_z` (median about -1,600 cm/s, against 0 in
range): the map has no floor, so players leave the minimap while falling. Clamp
or drop by `vel_z`.

### `ReplicatedMovement.location` is world units, at a per-class level

Non-player actors -- ability projectiles, smokes, walls, pawns, dropped weapons --
report their position in `fields.parquet` rows named `ReplicatedMovement`, whose
`value_str` is a JSON object. Its `location` is in the same world units as
`actors.spawn_x/y/z` and `movement.pos_*`; `linear_velocity` is units per
second.

The wire packs `round(world * scale)` and one bit saying "scaled", never the
scale, and bit consumption does not depend on it, so a wrong divisor decodes
cleanly.

The scale is Unreal's `LocationQuantizationLevel`, a per-class choice the wire
does not carry, so every `RepMovement` entry states it beside the rotator width
(`FieldType::RepMovement { rotation, location }`).

**Method.** Over the 1,018 audited replays (24 builds; the one-replay 12.10,
12.11 and 13.00 fixtures carry no `ReplicatedMovement` rows). An independent Python reader of the raw bits,
which agrees with the Rust decoder on all 8,250,603 typed rows, supplies the
packed integers. Two checks per class:

- *Spawn join.* Each `open` row in `actors.parquet`, joined to the same actor's
  first `ReplicatedMovement` row at the same `time_ms` on the same channel, spawn
  position at least 50 units from the origin: `|packed integer| / |spawn xyz|` is
  the scale. Its median on every class and every build lands within 0.01% of 1
  or of 100, and the 1st-99th percentiles within 0.03%. Read at its level, the
  location is within 0.5 units of the spawn on every axis of every join (0.0502
  on SeekerNade) -- the rounding its level allows.
- *Speed.* Consecutive rows of one actor with `0 < dt <= 0.2 s` and
  `|velocity| >= 200`: `(|location step| / dt) / |velocity|` with the location at
  its measured scale. Near 1 means the velocity is whole units too.

`tools/check_rep_movement_levels.py` runs both checks, and the rotator width by
exact consumption, over any directory of exports.

| Class | Rotator | Location | Spawn joins | Builds | Ratio p1-p99 | Speed check (median, pairs) |
|---|---|---|---:|---:|---|---|
| `Pawn_Aggrobot_SeekerNade_C` | Short | **two decimals** | 932 | 15 | 99.9984-100.0014 | no moving rows |
| `EquippablePickupProjectile_C` | Byte | whole units | 288,644 | 21 | 0.9997-1.0002 | 1.01 (1,950,150) |
| `GameObject_Smonk_NewSmoke_C` | Byte* | whole units | 27,667 | 21 | 0.9999-1.0001 | no moving rows |
| `Projectile_Wraith_4_Smoke_C` | Byte | whole units | 14,935 | 17 | 0.9998-1.0002 | 1.00 (144,774) |
| `Zone_Wraith_4_Smoke_C` | Byte* | whole units | 14,902 | 17 | 0.9999-1.0001 | no moving rows |
| `Projectile_Vampire_4_NearsightAoE_C` | Byte | whole units | 13,892 | 21 | 0.9998-1.0002 | 0.97 (67,202) |
| `Projectile_Hunter_Q_RevealBolt_C` | Byte | whole units | 12,032 | 14 | 0.9998-1.0002 | 1.00 (170,773) |
| `Projectile_Wushu_4_Smoke_C` | Byte | whole units | 11,976 | 20 | 0.9999-1.0002 | 0.98 (530,104) |
| `Projectile_Guide_E_HawkFlash_C` | Byte | whole units | 8,265 | 15 | 0.9998-1.0001 | 0.99 (1,013,091) |
| `Projectile_E_BountyHunter_Divebomb_C` | Byte | whole units | 5,715 | 14 | 0.9999-1.0001 | 0.99 (120,908) |
| `Projectile_Hunter_4_ExplosiveBolt_C` | Byte | whole units | 5,280 | 5 | 0.9999-1.0001 | 1.00 (55,390) |
| `Projectile_Wraith_Q_NearsightMissile_C` | Byte | whole units | 4,062 | 17 | 0.9998-1.0001 | 1.00 (53,124) |
| `Projectile_Smonk_DecayNade_C` | Byte | whole units | 3,505 | 21 | 0.9999-1.0001 | 1.00 (46,775) |
| `GameObject_Smonk_Q_DecayExplosion_C` | Byte* | whole units | 3,490 | 21 | 0.9999-1.0001 | no moving rows |
| `GameObject_Smonk_NewSmoke_PDS_C` | Byte* | whole units | 3,320 | 21 | 0.9998-1.0002 | no moving rows |
| `Projectile_Neon_C_Tunnel_C` | Byte | whole units | 3,269 | 12 | 0.9998-1.0001 | 0.98 (54,749) |
| `Projectile_Phoenix_Q_FlameWall_ThroughWall_C` | Byte | whole units | 2,765 | 18 | 0.9999-1.0001 | 0.98 (179,899) |
| `Projectile_E_Aggrobot_DiscTurret_PowerWave_C` | Byte | whole units | 1,819 | 15 | 0.9998-1.0002 | 1.00 (30,501) |
| `Projectile_E_Aggrobot_OrbSpawner_C` | Byte | whole units | 1,812 | 15 | 0.9999-1.0002 | 1.06 (13,139) |
| `GameObject_Mage_E_WorldSmoke_C` | Byte* | whole units | 1,046 | 5 | 0.9999-1.0001 | no moving rows |
| `Projectile_Terra_C_TimeSlowGrenade_C` | Byte | whole units | 1,033 | 11 | 0.9999-1.0002 | 1.00 (10,280) |
| `GameObject_Terra_C_TimeSlowGrenade_Explosion_C` | Byte | whole units | 1,029 | 11 | 0.9998-1.0001 | no moving rows |
| `Projectile_Aggrobot_Zamboni_Rocket_C` | Byte | whole units | 1,007 | 15 | 0.9998-1.0002 | 1.00 (1,112) |
| `Projectile_Pandemic_E_SmokeScreen_NoCollision_C` | Byte | whole units | 703 | 8 | 0.9998-1.0001 | 1.00 (8,930) |
| `Projectile_Aggrobot_C_ExplodeyPatch_C` | Byte | whole units | 647 | 15 | 0.9999-1.0001 | 1.00 (9,052) |
| `Projectile_Mage_Q_Wall_C` | Byte | whole units | 596 | 5 | 0.9998-1.0002 | 0.98 (131,617) |

\* The five `Byte*` classes never replicate a rotation, so the wire cannot
tell byte from short components and the width is a prior, not a measurement:
the 13.06 game's class data, by which every observable native `AGameObject`
decodes as byte. The evidence, and the byte-identical re-export of all 903
replays that declare their movement, are at `GAME_OBJECT_BYTE_ROTATOR_GROUPS`
in `tools/apply_type_corrections.py`. A rotator flag, if one ever appears,
decides by exact consumption.

Every class the table types was observed, so no entry rests on the default
alone. Sixty-three more are exact scoped identities
(`tools/fixtures/scoped_type_evidence.json`, each entry with its own
figures): the Boom Bot, measured on the 1,018 replays, 59 measured by the
tool on 123 (every replay of the 11.06-12.09 folders, 15 per 13.x build),
where the Boom Bot reads the same, and three Hush projectiles measured on one
13.09 replay (18 joins, within 0.5; speed 0.98-1.00). The ten `Pawn_*`/`AIPawn_*` classes --
`Pawn_Killjoy_E_Turret_C` and `Pawn_Killjoy_Q_StealthAlarmbot_C` among them --
pack two decimals with short rotators (ratio 99.998-100.003, every join within
0.0504 of spawn); the other 53, Raze's satchel, Paint Shells and rocket among
them, pack whole units with byte rotators (0.9997-1.0004, within 0.5).

What the evidence does **not** cover:

- **SeekerNade's velocity level.** All 932 of its velocities are the zero vector,
  which reads the same at any divisor, so its speed check has nothing to measure.
  The reader uses whole units for every velocity.
- **Checkpoints.** No checkpoint table in the 1,018 exports holds a single
  `ReplicatedMovement` row, so every figure above is main-stream only.
- **Six classes left raw**, all whole units. `GameObject_Iris_E_Smoke_C`,
  `GameObject_Cashew_E_AirStrikeMortar_C`, `Zone_Gumshoe_4_Cage_C` and
  `Projectile_NetToss_Underhand_C` never send a rotation, so the wire cannot
  choose the rotator width. Cypher's cage projectile,
  `Projectile_Gumshoe_4_CageTrap_C`, is `Skip` in the table, and its 13.01
  successor `Projectile_Gumshoe_Q_CageTrap_C` (byte, like it) stays raw with
  it: typing one side alone breaks the move `check_entry_survival.py` follows.
- **Hush's underhand throws.** `Projectile_Paprika_{E_RedirectPull,Q_RedirectFlash}_Underhand_C`
  consume exactly as byte rotators, but none of their 32 opens has a first
  update at the same `time_ms`, so their level is unmeasured and they stay raw.

**New entries.** Every `RepMovement` entry in `table.rs` is whole units --
Unreal's own `FRepMovement` default -- except SeekerNade, which
`apply_type_corrections.py` pins to two decimals. A default is a prior, not a
measurement, so `tests::overlay` lists every group given a `RepMovement` type
-- by the table or by `scoped_types.rs` -- with its measured level and fails
on a group it does not list, and `check_rep_movement_levels.py` exits 1 when
a typed class measures another level. A `RepMovement` literal written without
a `location:` does not compile.

### RPC transforms: `249` is a rotation quaternion, not a rotator

Several RPCs and transition contexts replicate an `FTransform`, and the wire
flattens it into three members: `Translation` and `Scale3D` by name, and the
rotation under the hardcoded name index 249 (`Rotation`), which the export
spells `249`. All three are 192-bit `VectorDouble`s, rendered `(x,y,z)` in
`value_str`.

`Translation` is a location and `Scale3D` a scale. **`249` is neither an Euler
rotator nor a direction: it is the transform's `FQuat`, and the three numbers
are its X, Y and Z.** The checksum says so -- 747197698 reproduces as the member
`Rotation: FQuat` of `Transform: FTransform`, and not as `FRotator` or `FVector`
(`tools/tests/test_compatible_checksum_facts.py`); the 13.06 executable's
reflection has `Transform.Rotation` as a quaternion. W is not sent. Unreal's
`FQuat::NetSerialize` normalizes the quaternion, flips all four signs when W is
negative and writes only X, Y and Z, so a reader rebuilds
`W = sqrt(max(0, 1 - (x*x + y*y + z*z)))`. That convention is the public engine
source's; it was not verified in this binary. The data fit it: over all 1,018
replays (24 builds, main and checkpoint rows), the 12,698,371 rows under
747197698 are all 192 bits with `|xyz| <= 1` (the largest is exactly 1, none
exceeds `1 + 1e-6`), and 12,594,391 of them hold a negative zero -- what the
sign flip leaves on a zero component. No `W` column is exported; compute it
when you need the full quaternion.

| Carries it | `249` checksum | Parent | Rows (1,018 replays) |
|---|---|---|---|
| `EffectManagerComponent:MulticastPlay{Continuous,OneShot}Effect`, the weapons' `AresEquippable:MulticastPlay{Continuous,OneShot}EffectFromClient` | 747197698 | parameter `Transform` | in the 12,698,371 |
| `TransformTransitionContext`, `TransitionContext_Sequoia_X_TeleportInfo_C`, `StateContext_ActorTrailTargetingResult_C` (typed through the checksum table) | 747197698 | property `Transform` | in the 12,698,371 |
| `AresGameStateBase:MulticastResetForRespawn` | 1874998526 | parameter `SpawnTransform` | 183,577, `\|xyz\| <= 1` |
| Viper's and Phoenix's `MulticastAddSmokeScreenPoint`, Astra's `MulticastAddAnchor` | 177696787 | parameter `ValveSetTransform` | 58,598, `\|xyz\| <= 1`; left raw |

A different `249` travels on the effect-placement RPCs
(`ClientPlayOneShotEffectAtLocation`, `ReplayPlay*EffectAtLocation`,
`ReplayRecord*Effect`): checksum 2526428638, which reproduces as a top-level
`Rotation: FRotator`. That one is a rotator, typed `RotationShort` and rendered
in degrees. Same name, different property -- the checksum is what tells them
apart.

## Weapons & loadout

| Data | Source | Status |
|---|---|---|
| Weapon instance class | `actors.parquet` class_path + `tools/equippable_table.py` (display name) | ✅ |
| Shot events (ammo, projectiles, vectors, seed, fire mode) | effect blobs | ✅ typed JSON |
| Magazine ammo over time | `AmmoComponent.AuthResourceAmount` (Int32) | ✅ via the `MagazineAmmo` remap; reads 0..100 |
| Reserve ammo over time | `AmmoComponent.AuthResourceAmount` (Int32) | ✅ via the `ReserveAmmo` remap -- same native component, second instance; reads 0..200, plus a 999 sentinel (below) |
| Equipped weapon (per player, over time) | `AresInventory.CurrentEquippable` / `NewCurrentEquippable` -> actor class | ✅ via InventoryComponent->AresInventory remap (resolve the NetGUID to its equippable actor) |
| Equipped weapon (on damage) | `MulticastNotifyDamage.EquippableUsed` | ✅ |
| Holder of a weapon effect | `AresEquippable:MulticastPlay{Continuous,OneShot}EffectFromClient.EffectManagerComponent` → `net_guids` (`EffectManager`, whose outer is the holding pawn) | ✅ ObjectNetGuid; a just-dropped weapon can still name its previous holder |
| Weapon readying speed | `ReadyingStateComponent.AuthEquipSpeed` | ✅ EnumByte, {0, 1, 2} in the main stream (same values as `AutoEquipSpeed`); member names unknown. Always 0 in checkpoints, so not readying state there |
| Buyer team recorded on an equippable | `AresEquippableDataTracker.OriginalBuyerTeam` | ✅ FName as sent, `Red` or `Blue`; nothing maps it to attacker/defender or to a player |
| Skin / spray / charm | `manifest` playerLoadouts (per subject) | ✅ |

**999 means infinite reserve, not infinite ammo.** It appears on 53 rows over
71 replays, always on `ReserveAmmo` and never on `MagazineAmmo`, and it belongs
to `Gun_Sprinter_X_HeavyLightningGun_Production_C` -- Neon's ultimate. The
correspondence is exact: 22 replays carry a 999 and 22 replays carry that gun,
with no replay on either side of the pair alone. It is written once per gun
instance and never moves, while the same gun's magazine counts down and reloads
normally. Treat it as a sentinel, not a count -- a max() over the column
otherwise reports a 999-round reserve.

## Rounds & score

| Data | Source | Status |
|---|---|---|
| One row per played round (phases, `buy_end_ms`, result) | `tools/extract_rounds.py` -> `rounds.parquet` | ✅ derived view; a surrender pads `RoundResults` with awarded rounds nobody played (12 on each public fixture), which make no row |
| Round result (winning team) | `RoundResults` struct blob | ✅ name-based (survives handle shifts) |
| Per-team round results (13.09) | `RoundResults[round].TeamResults[index].{TeamName,TeamNumber,TeamRole,TeamOutcome}` | ✅ additive, sparse and declaration-driven; the enum outcome is kept numeric, and the raw parent blob is preserved |
| Team score | derived / `BaseTeamState.Wins`/`Points` | ✅ (R1–R5 invariants verified) |
| Round number | `events.roundStarted` word0 / `RoundNumber` | ✅ |
| Half / side swap / overtime | `events.switchTeams` | ✅ |
| Per-round team economy | `TeamEconomy` (13.01) / `BaseTeamState` (13.02) | ✅ |
| Round-loss streak | `BombGameState_C.CurrentLossStreak` / `LossStreakTeam` (Swiftplay's game state carries `LossStreakTeam` too) | ✅ exact identity: Int32, 0..2 on every observed row; ObjectNetGuid, GUID 0 or the `RedTeam` / `BlueTeam` object in `net_guids` |
| Match-timer override flag | `BombGameState_C.ShouldOverrideMatchTimer` (and Swiftplay's) | ✅ Bool, exact identity. Always sent in the same packet as `OverrideMatchTimerText`: true with its formatted-number form, false with its empty form, on every observed row |
| Round-end ceremony and its subject | `{Default,Ace,Clutch,Closer,Flawless,Thrifty,TeamAce}Ceremony_C.bShouldDisplayCeremony`; `ClutchCeremony_C.ClutchPlayer`, `CloserCeremony_C.CloserPlayer`; `FlawlessCeremony_C.FlawlessTeam`, `ThriftyCeremony_C.SalvagingTeam`, `TeamAceCeremony_C.TeamAcingTeam`; `ThriftyCeremony_C.{Blue,Red}TeamStartingAvgInventoryValue` | ✅ exact identity: Bool; ObjectNetGuid (GUID 0, or a `BombPlayerState_C` / Swiftplay player-state actor, or the `RedTeam` / `BlueTeam` object in `net_guids`); Int32. In every packet that carries both, the flag is true exactly when the ceremony's player or team is non-null. The ceremony actor chosen for a round is `ChosenCeremonyForRound` |
| Match-timer override text | `BombGameState_C.OverrideMatchTimerText` (and Swiftplay's) | ✅ `FTextTree`, exact identity: the FText history tree as JSON in `value_str` -- the empty history 255 (`{"flags":0,"history":255,"kind":"empty"}`) or history 4, a number the game formats itself (`kind: "as_number"`, a `source.double`, the `format` options -- always two integral and two fractional digits here -- and a `culture`). Observed source values 0.01..36.95. What the number counts down is not established. See [below](#history-4-a-formatted-number) |

## Spike & objective

| Data | Source | Status |
|---|---|---|
| Plant / defuse / detonation | `events.spikePlanted` / `spikeDefused` / `spikeExploded` | ✅ |
| Spike carrier (who holds it, over time) | `BombEquippable_C.Owner` on the spike's own channel → `tools/extract_spike_carrier.py` | ✅ resolved to manifest `subject`; covers backpack, not just in-hand |
| Spike in hand (vs carried) | `AresInventory.CurrentEquippable` / `NewCurrentEquippable` == bomb GUID | ✅ the `in_hand` flag of the same view |
| Defuser | `TimedBomb.CurrentDefuser` (ObjectNetGuid); `Comp_BombEvents_C:BombDefusedRPC.DefusingCharacter` (ObjectNetGuid) at the defuse | ✅ the RPC's character equals the latest non-null `CurrentDefuser` on 224/224 defuses (76-replay sample, 11.06-13.06) |
| Planter | `Comp_BombEvents_C:BombPlantedRPC.BombPlanter` (ObjectNetGuid); also the carrier at the `spikePlanted` timestamp (`extract_spike_carrier.py`) | ✅ the RPC's planter equals the spike's `BombEquippable_C.Owner` at the RPC on 873/873 plants (76-replay sample), Gekko's Wingman (`Pawn_Aggrobot_SeekerNade_C`) when it plants; the event payload itself carries no planter |
| Spike timer | `TimedBomb.TimeRemainingToExplode` / `DefuseProgress` | ✅ Double |
| Plant site (A/B) | `TimedBomb.PlantedAtSite` (EnumByte) + position derivation; `BombPlantedRPC.PlantSite` (EnumByte, 2 bits) and `PlantLocation` | ✅ absent handle = default site (UE default-value skip); 100% via spawn position. The RPC's `PlantSite` equals `PlantedAtSite` of the `TimedBomb` opened at the same `time_ms` on 873/873, and both are absent together |
| Detonation source | `events.spikeExploded` is canonical (always emitted) | ✅ `RoundResults` under-counts: it logs win-reason, not detonation |

The gap between 776 plants, 248 defuses and 59 detonations looks like loss and
is not. Checked against independent RPCs over 71 replays, `spikeExploded`
matches `ClientBombExplode` 59 to 59 and `spikeDefused` matches
`BombHasBeenDefused` 248 to 248, with no replay disagreeing;
`TimeRemainingToExplode` agrees too, reaching 0.00 exactly on the detonations
and stopping mid-count otherwise. The remaining 469 are rounds that ended in a
team wipe after the plant, so the fuse never ran out. `BombEquippable` actors
open 1,317 times against 1,317 `roundStarted` events -- one bomb per round,
exactly.

`extract_spike_carrier.py` resolves a carrier for every plant: through the
whole `SpawnedCharacter` history (`tools/player_identity.py`) it reports
`NO CARRIER` 0 times on all 1,018 audit exports, and an interval owned through
a player's earlier pawn carries `carrier_identity_provenance`. A carrier it
cannot resolve prints `NO CARRIER` rather than a guess.

## Actor / GUID / structure

| Data | Source | Status |
|---|---|---|
| Every actor spawn/despawn + class/archetype/spawn location and velocity | `actors.parquet` -- `event` is `open`/`close`/`dormant`; only `close` is a despawn; velocity in world units per second | ✅ |
| GUID → object path | `net_guids.parquet` | ✅ |
| Containment chain (subobject → parent) | `net_guids.outer_net_guid` | ✅ |
| Full declared schema (475 groups, handle→name) | `manifest.net_field_export_groups` | ✅ |

## Structured-array children

A measured array route expands one exact (group, parent, checksum) identity
into additive child rows that follow the raw parent, which is kept. A route
runs only on the builds `MeasuredArrayRoutes::for_branch`
(`crates/vrfkit/src/sink/measured_routes.rs`) admits: every route on
13.01--13.06, a measured subset on 11.06--13.00
([route table](LEGACY_BUILD_SUPPORT.md#measured-array-routes)).

| Array | Members | Column | Meaning |
|---|---|---|---|
| `SelectedV2` | `EquippableDataAsset`, `EquippableSkinDataAsset`, `EquippableSkinLevelDataAsset`, `EquippableSkinChromaDataAsset`, `EquippableCharmDataAsset`, `EquippableCharmLevelDataAsset` | `value_i64` | Object references into the GUID table |
| `SelectedV2[i].EquippableAttachments[j]` | `SocketAsset`, `AttachmentAsset` | `value_i64` | Object references into the GUID table |
| `KillData` | `Victim` | `value_i64` | A dynamic actor ID (`actors.parquet`), never in the static GUID table |
| `KillData` | `KillingEquippableClass`, `DamageType` | `value_i64` | Object references into the GUID table |
| `KillData[i].AssistingPlayers[j]` | `AssistingPlayers` | `value_i64` | Actor references |
| `KillData` | `DamageTaken`, `GameTimeElapsed`, `RoundTimestamp` | `value_f64` | f32 widened; units and attribution unverified |
| `KillData` | `RoundNumber` | `value_i64` | Signed 32-bit, observed 0-35 |
| `KillData` | `bDidKillTriggerFinisher` | `value_bool` | One bit; `false` is decoded, not absent |
| `KillData` | `DamageRegion` | `value_i64` | Unsigned byte, observed 0, 1, 2, 5; labels unverified |
| `KillData` | `WeaponTheme` | `value_str` | A true prefix, then a terminated FString (not an FName) |
| `RequestedIgnoreActors` | its child | `value_i64` | Packed NetGUID |
| `TrackedRewards` | `LocalizedRewardName` | `value_str` | The FText history tree as JSON, below |

A reference of 0 is a null reference; a null `value_i64` means nothing was
decoded. Checkpoint rows also join on `checkpoint_index`. A malformed window
stays raw and moves `array_leaf_decode_errors`. `TransitionContext`
(`EquippableStateMachineComponent`) is a packed NetGUID in `value_i64` too, and
all its checkpoint observations are 0.

### FText history trees

`LocalizedRewardName` (and the timer text below) keeps its raw bytes and adds
the whole text-history tree as JSON. History 11 is a string-table entry:

```json
{"flags":0,"history":11,"kind":"string_table","table":{"name":"/Game/GameModes/Bomb/BombMode_Strings.BombMode_Strings","number":0},"key":"Kill"}
```

History 3 is `kind: "format"` with a nested `source` tree and an ordered
`arguments` array; each argument keeps its name and wire tag, tag 0 as
`{"bits_u64":"1"}` (all 64 bits as a decimal string), tag 4 as a nested tree.
History 255 is `{"flags":0,"history":255,"kind":"empty"}` with a zero wire
length. Any other history or argument tag fails visibly and stays raw. The
reader bounds strings at 64 KiB, arguments at 128, depth below 16 and nodes at
256, and requires full bit consumption. No localized label is rendered.

### History 4: a formatted number

`FieldType::FTextTree` applies the same reader to a whole property:
`BombGameState_C.OverrideMatchTimerText` (and Swiftplay's copy), by exact
identity. It sends the empty form or a 376-bit history 4:

```json
{"flags":1,"history":4,"kind":"as_number","source":{"tag":3,"double":15.614009857177734},"format":{"always_sign":false,"use_grouping":true,"rounding_mode":0,"minimum_integral_digits":2,"maximum_integral_digits":2,"minimum_fractional_digits":2,"maximum_fractional_digits":2},"culture":""}
```

After the flags and history byte: the source type byte (only 3, a double, is
read; any other is refused), the double, an archive bool for whether
`FNumberFormattingOptions` follow, those options (two bools, the rounding-mode
byte, four i32 digit limits), and the culture FString. A non-finite double is
refused. On all 1,018 replays, 7,099 rows were history 4 and 14,005 empty, and
an independent reader (`validate_type_evidence.py`, `FTextTree`) agreed on
every row.

## Replay metadata

| Data | Source |
|---|---|
| Build / branch / version / changelist | `manifest` |
| Duration, timestamp (FDateTime), encryption/compression flags | `manifest` |
| Platform, build config, network checksum/GUID | `manifest` |
| Stats (packets / bunches / blocks / fields / RPCs / malformed / skipped) | `manifest` |
| playerLoadouts (subject → agent/skin/spray), matchID | `manifest` game_specific_data |

## Other

| Data | Source | Status |
|---|---|---|
| Ping / latency (ms) | `BombPlayerState.Ping` (16-bit, ms) | ✅ typed (SerializedInt{65536}) |
| Connection status | `ConnectionStatus` | ✅ |
| Movement-prediction reset on possession | `ShooterCharacter:ClientResetRemoteMovementPrediction.isPossess` (every character and pawn cache) | ✅ Bool; true on every one of 291,346 observed rows |
| Game mode (Bomb / Swiftplay) | group_path (`GROUP_ALIASES` maps Swiftplay) | ✅ parser-side |
| Other Blueprint properties typed by exact identity | `OnKillEffect_Base_C.Victim FXC` / `VictimFXC_Planted` (a `FXC_Finisher_*` class in `net_guids`); `FloatTransitionContext_C.Float`; `EquipRequestTransitionContext_C.TargetEquippable`; `ActivatableActorComponent_C.IsActivated` and `ActivatableActor_Selectable_Component_C.IsActivated`; the patches' `Has Succesfully Hit`; the removable objects' and reveal darts' `Target` (with the pre-13.01 net-toss and tracking-dart paths); `Ability_{Wraith_Q_NearsightMissile,Sequoia_Q_FragileMissilePrototypeEquipped}_C.Projectile` / `WarningActor`; the Cashew missile's `EndingStrikeLocation`, `RocketIndex`, `IsCurrentlyDoingInitialFlyOut` and its markers' `Seeking Missile Actor`; `Pawn_Killjoy_Q_StealthAlarmbot_C.IsArmed` / `IsBurrowed`; Breach's `Charge`, `ChargeDistance` and fissure `CharacterLocation`; `DefaultToTargetViewModeTargeting`; `Ability_Iris_Thumper_C.Is State Concuss`; `GameObject_Thorne_E_Wall_Segment_Fortifying_C.IsAlive_0`; `Actor_Sequoia_X_StandardArena_C.AssignedPlayspace`; `Switch_BlackMarket_2_C` lever state and times; `BP_Breakable_Simple_*_C.Destroyed`; the finisher effect objects' fields; `ActiveSlowTimeEffects` | ✅ each typed only at its exact group, name and checksum; the evidence per identity -- game-file property class, recomputed checksum, corpus decode and value checks -- is in `tools/fixtures/scoped_type_evidence.json`. Types say what the bits are, not what the game means by them |

---

## Limitations (replay-format, not parser bugs)

- **Display names** — the replay carries no player names, only account UUIDs.
- **Bare component groups** — a component such as `InventoryComponent`
  replicates under its instance name while the replay declares its layout under
  the native class (`AresInventory`). `KNOWN_SUBOBJECT_CLASS_PATHS` connects
  them (pinned in `crates/vrfkit/src/sink/paths.rs`). The map comes from the
  game's class hierarchy, not from names; a bare group on a new build needs
  "Reading component classes out of the game" below.
- **AbilitiesAndBuffsComponent** — the replay never declares its `_ClassNetCache`
  group, so `function_count` is brute-forced (fc=34). The outer RPC framing is
  fully recovered, and the inner payload is a FastArray body whose property
  meanings are unknown; the words stay in `raw_bits`
  ([FastArray observations](GAS_AND_PATCHVOLUME_INVESTIGATION.md)).
- **FName instance numbers are part of the name.** Number 0 means no suffix
  and N the suffix N-1, and the readers (`scalar::read_fname`) render it, so
  `MyEquippable` on `EquippableGroundPickup_C` and `MyEquippable_0` on
  `EquippablePickupProjectile_C` are distinct fields (as are `IsAlive` /
  `IsAlive_0` on Thorne's wall segments). `table.rs` spells the projectile's
  field `MyEquippable`, so that name entry never matches and the field
  resolves through `compatible_checksum` (`checksum_table.rs`, `ObjectNetGuid`).
- **spikeExploded** — not a limitation: `events.spikeExploded` is the canonical
  detonation signal and is always emitted. `RoundResults` records the round
  *win reason* (elimination/detonate/defuse), not whether the spike detonated,
  so it under-counts detonations and is not a reliable proxy.

---

## What's next (where to start)

Remaining work is [`FOLLOWUP.md`](FOLLOWUP.md). Start by bucketing the untyped
rows on `compatible_checksum` ([recipe](USAGE.md#fieldsparquet)). A new bare
handle is one sorted `OVERLAY_HANDLE_TABLE` line in
`crates/vrf-decode/src/table.rs`, pinned in `crates/vrf-decode/src/tests/overlay.rs`.

### Reading component classes out of the game

The bare group names -- `ZoomStateMachine`, `ReserveAmmo`, `CalloutRegionTracker`
and many others -- are component *instance* names, not classes, so nothing in a
replay says what they are. The installed game does say, and it does not need
decryption: VALORANT's IoStore containers are `Compressed+Signed+Indexed` with a
zero encryption GUID, so the `Encrypted` flag is simply off.

`tools/extract_component_classes` reads it (build and flags:
[USAGE.md](USAGE.md#reading-the-installed-game)). Given the `Paks` directory it
prints one row per component template -- the instance name the replay sends, the
package that owns it, and the class. The chain it follows:

1. `.utoc` -> chunk ids, compression blocks and the directory index. Only TOC
   version 5 is accepted, the one the 13.06 containers use.
2. Each `ExportBundleData` chunk starts with the package header -- name map and
   export map. Only the blocks the header spans are decompressed, with the same
   `oozextract` vrf-container uses for replay chunks, which keeps a full scan of
   the 30 GB of `.ucas` to seconds.
3. A component shows up in one of two export shapes. A Blueprint-added component
   is a `<Name>_GEN_VARIABLE` export; a component the C++ class creates is a
   subobject of the class default object (`Default__<Class>`) under its instance
   name. The first shape alone cannot reproduce the table: in 13.06,
   `InventoryComponent`, `AbilitiesAndBuffsComponent`, `CalloutRegionTracker`,
   `VisionComponent` and `DamageHandlerComponent` exist only in the second.
4. The export's `ClassIndex` is an `FPackageObjectIndex`. A `ScriptImport` is a
   hash, which `global.ucas`'s script object map -- a name batch followed by
   `FScriptObjectEntry` records -- turns into `/Script/<Module>.<Class>` by
   walking the outer chain. A `PackageImport` names a public export of another
   package; the tool follows it to that package's Blueprint class and on up the
   super chain to the first native class.

The game checks the result, which is what makes it more than a parse. On the
13.06 containers (17 containers, 379,670 TOC entries, 289,267 packages; the
tool prints each container's id, size and modification time, so a run can be
matched to the files it read):

- every one of the 80,800 script object paths the tool rebuilds hashes back to
  the index the game stores for it -- CityHash64 of the lowercased UTF-16 path,
  which is how the engine forms that index, so a wrong name or outer link
  cannot pass. The separators are not proved: the hash folds `.` and `:` alike
  into `/`, and the tool writes `/Script/<Module>.<Class>:<Sub>` and joins
  anything deeper with `.`, as the engine's `GetPathName` does;
- every package name hashes back to its chunk id, and agrees with the directory
  index's file name;
- every TOC file is consumed to its last byte, and 0 packages fail.

Anything the tool cannot resolve prints as `?`. UTF-16 names in a name batch
are **not** aligned to two bytes (aligning them misreads 26 packages).

What it does not read: the 17 legacy `.pak` files beside the containers. Their
indexes are encrypted, so a package stored only there would be invisible. Nothing
so far points at one -- every pair below came from the IoStore containers.

**The existing table reproduces** on 13.06: every pair a cooked asset can hold
came back with the class it already had, none contradicted, and
`AresAttributeSet_2` is in no package, as expected of a runtime subobject.
`MagazineAmmo` and `ReserveAmmo` are both `AmmoComponent`, whose handle 2 the
replay declares as `AuthResourceAmount`, so the hand-written `AmmoCount` handle
row was in the right place with the wrong word.

#### Pairs added from 13.06

Every bare group of at least 9,000 rows in the 1,018-replay corpus inventory
(builds 11.06-13.06) was looked up in the tool's output. A pair went in only
when all of these held,
measured over every replay's main-stream rows and, separately, every
checkpoint's rows:

- **one class.** The instance name has exactly one class across every package
  and both export shapes;
- **declared.** The replay declares that class's group wherever the bare group
  carries RepLayout rows -- rows whose `field_name` is null or carries no
  `_cnc_h` / unresolved-payload marker;
- **handles fit.** The handles those rows use are a subset of the handles the
  target declares in the same replay, and checkpoint rows of the handles their
  own checkpoint declares;
- **widths agree** wherever the target group also has rows of its own in the
  same replay, handle for handle.

All are RepLayout-only, like every pair before them, so their ClassNetCache rows
stay bare by design.

| leaves | class | RepLayout rows, main | checkpoint | ClassNetCache rows left bare |
|---|---|---:|---:|---:|
| `Resume_StateMachine`, `Sprint_StateMachine`, `Slide_StateMachine`, `ProjectileStateMachine`, `EquipStateMachine`, `PrimaryTriggerActionStateMachine`, `EquippableStateMachine_Activate`, `LaserStateMachine`, `SelfResStateMachine`, `Ability State Machine (EquippableStateMachine)`, `TimerStateMachine`, `CloakStateMachine`, `SpontaneousEquip_StateMachine`, `EquippableStateMachine_Dart`, `EquippableStateMachine_Attack`, `EquippableStateMachine_PickUpOnCooldown`, `SwapCameras_StateMachine` | `EquippableStateMachineComponent` | 1,786,888 | 259,401 | 107,750 |
| `ShieldDamageSection`, `OverhealDamageSection` | `ChildDamageSectionComponent` | 45,157 | 246,317 | 0 |
| `PreventDeathDamageSection` | `AttachedDamageSectionComponent` | 572 | 10,087 | 10,115 |
| `PMAimToolingTarget` | `/Script/InputTooling.AimToolingSkeletalTargetComponent` | 34,662 | 543,324 | 0 |
| `Usable_PickUp` | `UsableComponent` | 18,512 | 0 | 0 |
| `StealthComp` | `SimpleVisualTimelineStealthComp` | 23,400 | 0 | 0 |
| `StealthV1AddedForAISight` | `StealthComponent` | 11,564 | 0 | 0 |
| `Collision Static Mesh` | `/Script/Engine.StaticMeshComponent` | 10,924 | 1,698 | 0 |
| `Comp_Ability_CooldownComponent1` | Blueprint `Comp_Ability_CooldownComponent_C` | 35,381 | 21,945 | 0 |
| `DamageSection_Vampire_Q_BloodArmor` | Blueprint `DamageSection_Vampire_Q_Heal_BloodArmor_C` | 13,055 | 19,396 | 20,967 |
| `ChooseTeleportSpot_StateComponent` | Blueprint `ChooseMapLocationOnNavMesh_StateComponent_C` | 14,866 | 1,939 | 3,505 |
| `AresAttributeSet_1` | `AresAttributeSet` (wire evidence, below) | 131,014 | 2,550,032 | 0 |
| `AttachedDamageSection` | Blueprint `BasicArmorAttachedDamageSection_C` (its own review, [below](#the-armour-section-attacheddamagesection)) | 212,885 | 34,124 | 607,676 |

Classes without a module are `/Script/ShooterGame`; the four Blueprint classes
are declared by the replay under their full `/Game/..._C` paths, which is what
the pairs name. In all, the 29 pairs of the first pass move 2,125,995
main-stream and 3,654,139 checkpoint RepLayout rows of the corpus inventory
from a bare group into a declared one; the armour pair, added after its own
review, moves 212,885 and 34,124 more.

`AresAttributeSet_1` is not a component, so it is held to `AresAttributeSet_2`'s
standard instead of the tool's: over all 536 replays that carry it, every
main-stream handle it uses (116 per replay) is declared by the native group in
that replay, every checkpoint's by that checkpoint (10,455 checkpoints), and all
2,681,046 rows are 32 bits wide, the width the named instance has on each handle
(124,280 per-replay comparisons, none different). The only pair whose widths
differ from its target's own rows is `PMAimToolingTarget`, on handle 2:
`AttachParent` is a packed object reference, so 16 bits against 24 is the size
of the NetGUID it carries. For four pairs of the first pass the target has no
rows of its own in any replay, so the width condition says nothing about them
and the other three carry them alone: `StealthComp`, `Collision Static Mesh`,
`DamageSection_Vampire_Q_BloodArmor` and `ChooseTeleportSpot_StateComponent`.
The armour pair is in the same position and was held to the declared property
types instead (below).

Re-exported on 92 replays against the parent commit, every row keeps its
identity in both field tables, only the remapped groups' rows change (all
named), every other file is byte-identical, and `check_component_remaps.py`
reads the new pairs `ok` or `absent` where it read the parent's exports
`broken`. The newly named rows without an overlay type (`CursorWorldLocation`,
`RelativeLocation`, `TargetID`, the stealth flags, a section's `Life`) stay
untyped until they are seen to decode.

Not added, and why:

| group | rows | why not |
|---|---:|---|
| `PatchVolume` | 113,654 | Class `/Script/DynamicVolume.GroundVolumeComponent`, declared in 955 replays -- but in 915 of them its RepLayout rows include handle 0 (the preserved tails, 2,166 to 36,538 bits), which the target never declares, and handles 20-22 change meaning between builds. Not a name remap. It is a lead for the [PatchVolume investigation](GAS_AND_PATCHVOLUME_INVESTIGATION.md): the declared handle set contains exactly the twelve FastArray item handles that investigation left unassigned (23, 24, 25, 26, 30, 33, 36, 39, 40, 41, 42, 43). |
| `DefenderAnnouncer`, `AttackerAnnouncer` | 52,239 / 41,964 | Blueprint `AnnouncerVOComponent_C`, declared in none of 1,018 replays; every row is ClassNetCache. |
| `BeforePostRoundTransitionSyncTimer` | 36,376 | `SyncedTimerComponent`, declared in 2 of 1,018 replays; every main-stream row is ClassNetCache. |
| `WaitForInitialKillOrAssistState` | 15,993 | Blueprint `StateComponent_WaitForKillOrAssist_Smonk_Child_C`, declared in none; ClassNetCache only. |
| `Comp_Equippable_Subequippable` | 36,702 | No export by that name in 13.06; the group is absent from 13.05 and 13.06 replays too. |
| `Switch_BlackMarket_5`, `RespawningPlummetShootable3_UAID_*` (two), `B_Site_Door_Switch_0`, `RespawningWallPlate2_2`, `RespawningWallPlate2_7`, `Drawbridge6` | 9,607-31,254 | Actors placed in a map, not components: no component export carries these names. |
| `MapTargetingState` | 743,457 | Two classes: the Blueprint `StateComponent_RangeLimited_MultiMapTargeting_C` in 9 packages, native `MapTargetingStateComponent` in 7. No name remap can be right for both. |

**This is the one thing here that a game patch can silently invalidate.** A
renamed component stops matching and its handles go quiet again, and the replay
never named it either, so no unit test can see it.

`tools/check_component_remaps.py` is what watches for that. It needs only an
export -- not the game, not a baseline -- so it works on a replay from a new
build, which is exactly when the question comes up. For each RepLayout pair it
counts the RepLayout rows still bare under the leaf, and any at all is `broken`:
across the 92 re-exported replays not one of the 48 RepLayout pairs left a
single row bare, so healthy is exactly zero. ClassNetCache rows are excluded,
because the RepLayout-only remaps leave their RPC stream bare by design.

The four ClassNetCache effect pairs are judged on the rows they route -- the
leaf's ClassNetCache rows against `<class>_ClassNetCache`, any bare row
`broken`; RepLayout rows under a ClassNetCache-only leaf are a printed count,
not a failure.

What the checker cannot see is a rename itself: the old leaf simply vanishes.
The renamed component arrives under its new name, which is why the checker
prints the bare groups no pair claims on every run. When that list or a
`broken` verdict points at a component, re-read its class from the new build's
game with `tools/extract_component_classes` rather than guessing a name.

#### The armour section, `AttachedDamageSection`

The bare leaf is Blueprint `BasicArmorAttachedDamageSection_C` (native
ancestor `AttachedDamageSectionComponent`), the only class four armour-item
packages give that name. Its rows sit on objects under `PlasmaArmorItem_C`,
`HeavyArmorItem_C` and `LightArmorItem_C` actors. The replay declares handle 2
`bAlive` (checksum 622178691) and handle 5 `LastKnownDamageOwner` (205313645),
and checkpoints handle 3 `Life` (962760191) as well; every row uses a declared
handle. `bAlive` is 1 bit on every row and is typed `Bool` through the checksum
table (every damage-section class declares that checksum).
`LastKnownDamageOwner` is a packed NetGUID that consumes its window exactly --
0, or the `DamageHandlerComponent` whose outer is the armour item's
`Instigator` -- and `Life`, read as a float, never exceeds the item's armour
(50 / 25 / 25); both stay named but untyped. The leaf's ClassNetCache rows
stay bare by design.

It does not reach every armour block. In the 486 replays that also declare the
native `/Script/ShooterGame.AttachedDamageSectionComponent` group (exactly those
with Phoenix's `PreventDeathDamageSection` rows), `unique_leaf_match` resolves
the armour blocks to the native parent first, which declares `bAlive` only, so
handles 5 and 3 stay unnamed there. A consumer that wants the section's
properties reads both paths; moving those rows is a resolution-order change,
not a table entry.

### Closed: what the three mechanisms cannot reach

Most rows still untyped after the table, the engine-reference names and
checksum propagation are declared `Raw`/`Skip` on purpose
(`BaseReplayController`'s 4-kbit blob alone is 225,844 rows on 02d4d478); the
rest carry a checksum no declared field donates, or none at all (the
unresolved `AbilitiesAndBuffs` payload). The largest remaining item does not
yield to a `FieldType` at all:

`ClientReplayReceiveInputEventProcessingCapture.InputEventData` (53,605 rows,
one per `PlayerID` row) is a **tagged union**, not a scalar. The leading byte's
top 7 bits are a tag, and the tag fixes the width exactly -- seven tags, five
widths, no exceptions across all 53,605 rows:

| tag | width | rows | | tag | width | rows |
|---|---|---|---|---|---|---|
| 41 | 64 | 18,288 | | 12 | 32 | 3,093 |
| 15 | 32 | 14,724 | | 20 | 40 | 2,961 |
| 6 | 24 | 11,486 | | 28 | 48 | 2,522 |
| | | | | 13 | 32 | 531 |

The framing is settled; **what is missing is what any of it means.** Nothing
names the tags -- the RPC carries only `PlayerID` beside it, no declaration or
checksum donor exists, and this is engine-side input capture, not a Blueprint
property -- so a decoder now would produce seven anonymous payloads, which is
what `raw_bits` already gives.

Two time fields are `Float`, in `value_f64` with their raw payloads kept:

- `ReplayLastTransformUpdateTimeStamp` tracks server time in seconds with a
  file-specific offset from replay time: near 10 s in 672 of 714 files,
  10.8--110.1 s in the rest, so never hardcode subtracting 10. Continuity
  across checkpoint restoration is unverified.
- `ServerMovementTime` (FiniteSpeed, Spline, FloatCurve and Precalculated
  movement groups) is an actor-relative movement clock in seconds;
  channel-open time only approximates its zero.

### Closed: RPC signature aliasing

Aliasing an undeclared RPC group to a declared sibling (the `GROUP_ALIASES`
shape) is **not worth doing**: of the 165,374 rows the 17 workable pairs would
type, 139,222 are reachable by checksum already and the rest rest on a shared
parameter name alone. It is also unsafe: `resolve_entry` retries the whole
order against the alias, handle fallback included, and parameter handles do not
correspond between signatures -- aliasing `MulticastNotifyHeal` to
`MulticastNotifyDamage_Base` reads `LifeChangeBySection` as `DamageDealt:
Float` on 1,918 rows, each leaving 145 residual bits.

Bit-width agreement is not evidence here. Every candidate pair consumed its bits
exactly, including the wrong one above; a 1-bit `Bool` read as `EnumByte` also
passes. Width is a necessary condition and nothing more.

### Closed: more name-resolved properties

Only the four `AActor` object references in `ENGINE_OBJECT_REFS` resolve by
name, and no other property should. `Owner` is safe because its *encoding* is
fixed, not because its name is standard: `ReplicatedMovement` is as standard a
name and is declared with two rotator widths and two location levels, so a name
rule would desync blocks or read a value 100x off without an error -- the
failure the
[per-class level](#replicatedmovementlocation-is-world-units-at-a-per-class-level)
prevents. `RelativeScale3D` and `CosmeticRandomSeed` split the same way.

**RPC parameters are categorically excluded from a *name* rule.** A parameter
name is scoped to one function signature: 38 of the 211 parameter names in this
replay carry more than one `compatible_checksum`, which is exactly the same
statement the checksum makes. Typing them by name would merge signatures that
the schema itself distinguishes.

`AllianceFilter` is not a counterexample: every group declares it under
checksum 2270825073 as 3 bits, and `apply_type_corrections.py` declares its
third donor `EnumByte` so the checksum learner keeps it.

A type change to the table goes through `apply_type_corrections.py`, as
[CONTRIBUTING.md](../CONTRIBUTING.md#generated-files--never-hand-edit) describes.
