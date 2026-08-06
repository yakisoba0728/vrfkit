# Extractable data

What you can get out of a VALORANT replay with vrfkit. The export is six
Parquet tables plus `manifest.json`; every field's raw bits are always present
even when the type is unknown, so "untyped" below means *not yet decoded*, not
*missing*.

Legend: ✅ typed (value decoded) · ◐ raw or derivable · ❌ not in the replay.

---

## Player identity

| Data | Source | Status |
|---|---|---|
| Account UUID (subject) | `manifest.players.subject` / `BombPlayerState.Subject` | ✅ |
| Character NetGUID | `manifest.players.character_net_guid` / `SpawnedCharacter` | ✅ joins movement 10/10 on 71 of 71 replays |
| Agent (characterId) | `manifest` game_specific_data.playerLoadouts | ✅ |
| Two players on the same agent | disambiguated by `subject` (characterId alone can't) | ✅ |
| Display name | — | ❌ replays carry no display names, only the subject UUID |

That 10/10 was not free, and it is worth knowing why it can break.
`SpawnedCharacter` is replicated a second time as 0 when a player disconnects,
so a plain last-write-wins capture throws the real GUID away. It did: 9 players
across 5 replays came out with `character_net_guid == 0` while still appearing
in the manifest, so nothing looked wrong until the join was counted. The rule is
**last non-zero write wins** -- 0 is not a NetGUID. Before the fix 64 of 69
replays joined all ten and the worst managed 7; after it, 71 of 71 do.

## Economy

| Data | Source | Status |
|---|---|---|
| Current credits | `fields` `MoneyManagementComponent.Money` | ✅ Int32 |
| Start-of-round credits | `StartOfRoundMoney` | ✅ |
| Total granted | `TotalMoneyGranted` | ✅ |
| Team loadout value | `BaseTeamState.LoadoutValue` / `AverageLoadoutValue` | ✅ |
| Per-round spend | `StartOfRoundMoney` − `EndOfRoundMoney` (OwnerExclusivePlayerInfo) | ◐ derivable |
| ACS (combat score) | — | ❌ `PlayerScoreComponent` is not replicated |

## Purchases (full buy log)

| Data | Source | Status |
|---|---|---|
| What was bought (item) | `PurchasedItemComponent.Purchaseable` → `net_guids.path` / `equippable_table.py` | ✅ all 10 players; on `02d4d478` 576 purchases over 52 distinct item GUIDs, 20 of which resolve to a class path |
| Who bought it | `PurchasedItemComponent.PurchasingPlayerState` → `manifest.players.subject` | ✅ |
| When | `fields.time_ms` of the purchase row | ✅ |
| Which round | `time_ms` vs `events.roundStarted` | ◐ derivable |
| Cost | Money delta around the purchase, or per-round spend | ◐ derivable |
| Source (buy/ability/etc.) | `PurchasableTransactionSource` | ◐ partial (some rows) |
| Inventory slot → item | `ItemSlot.Contents`, `AresInventory.ItemSlots` | ✅ / ◐ (MultiItemSlot raw) |
| Charges purchasable this round | `EquipmentChargeComponent.TotalChargesAllowedToPurchaseThisRound` | ✅ |

**Purchase-history recipe:** filter `fields` to `group_path = PurchasedItemComponent`,
then per row join `Purchaseable` → `net_guids` (item path → display name via
`equippable_table.py`), `PurchasingPlayerState` → `manifest.players` (account
identity), and `time_ms` → nearest `roundStarted` (round number). One row per
purchase; all ten players' purchases are replicated.

## Combat — kills & deaths

| Data | Source | Status |
|---|---|---|
| K / D / A | `fields` CombatReport nested array | ✅ multiset-identical to the C# parser |
| Kill log (killer/killed NetGUID) | `events.characterDeath` word0/word1 + `MulticastNotifyKilledEnemy` RPC | ✅ 132/132 reconciled |
| Multikill level | `MulticastNotifyKilledEnemy.MultikillLevel` | ✅ single/double/triple/quad |
| Kill timeline | `events.characterDeath` time_ms | ✅ (recovers the +13 the C# parser lost) |

## Combat — damage

| Data | Source | Status |
|---|---|---|
| Damage dealt / received | CombatReport `DamageDealt` / `DamageReceived` | ✅ |
| Regional damage (head/body/leg) | `Interactions[].Regions[].Hits/Damage` | ✅ multiset-identical |
| Wallbang | `bIsWallPen` | ✅ |
| Damage source (weapon, location, bone) | `MulticastNotifyDamage` (EquippableUsed, ImpactLocation, ImpactBone) | ✅ |
| ADR | derived from CombatReport | ◐ +0.1–0.2 vs trackers (wire damage is fractional; not a bug) |
| Health / armour / overheal, absolute | `DamageableComponent` RPCs → `LifeChangeEvents[]` / `LifeChangeBySection[]` | ◐ **raw** — decodes cleanly, see below |

### Health is absolute, not a subtraction

The `DamageableComponent` RPCs carry an array whose elements hold
`ChangedComponent` (which damage section), `LifeResult` (**the absolute value
after the change**), `DeltaLife`, and `bAliveAfterChange`. Nothing has to be
accumulated. The array is still `raw_bits` -- typing it is listed under What's
next -- but it walks with the ordinary RepLayout dynamic-array framing, and the
decoded handles are the manifest handles with no offset.

Verified over 69 replays on build 13.02: 377,487 elements, zero parse errors,
zero residual bits, and every element carrying exactly four members (these RPC
parameters never send partial elements). Corroborated against separate decode
paths -- `sum(DeltaLife)` equals the RPC's own `DamageTaken`/`HealTaken`/
`DecayApplied` on 230,855 of 230,855, and `bAliveAfterChange` agrees with
`bAliveAfterDamage` on 61,045 of 61,045.

Three conventions a consumer has to get right, each found by a check failing:

- **Death is `bAliveAfterChange == False`, not `LifeResult == 0`.** Deduplicated
  per `(victim, RespawnNumber)` the first matches `events.characterDeath`
  9,362/9,362 across all 69 replays; the second misses on two, because a
  character really can sit at exactly 0 health and be alive (65 cases, all
  KAY-O). The flag is also re-reported after death, hence the RespawnNumber
  dedup.
- **Armour is `AttachedDamageSection`, not `ShieldDamageSection`.** The latter
  is an empty shell -- 67,316 elements, every `LifeResult` 0. The real armour
  section's outer is a `HeavyArmorItem_C` / `LightArmorItem_C` /
  `PlasmaArmorItem_C`, and its maximum reads 50.00 / 25.00 / 25.00, which is the
  game's own numbers and an outside confirmation that the f32 decode is right.
  Armour absorbs 2:1 against health on 12,747 of 12,747 hits where it survived.
- **`MulticastNotifyOverhealDecay` sends `DeltaLife` positive while life goes
  down.** Its magnitude matches `DecayApplied` 33,181/33,181, and the running
  chain only closes if the sign is flipped. `life += DeltaLife` runs overheal
  backwards.

Round starts anchor at 100: `LifeResult - DeltaLife == 100` on the first health
event of 10,981 of 10,996 lives. The 15 exceptions all read 200 and are all
Phoenix -- Run It Back, not a decode fault. On the reset broadcast
(`MulticastSectionLifeChange`) the `LifeResult` is trustworthy and the
`DeltaLife` is not an edge delta; ignore it there.

## Abilities

| Data | Source | Status |
|---|---|---|
| Ultimate cast | `events.characterUltimateUsed` (word0 = character) | ✅ |
| Cooldown / start time | `Comp_Ability_CooldownComponent` | ✅ Double |
| Ability cast count / cast log | `Comp_AbilityStatisticsReplicator.AbilityCastsThisRound[]` — `Player` (subject UUID), `Slot`, `Round`, `RoundPhase`, `CastTime`, `CastLocation` | ✅ one record per cast, all ten players; `Player` matches a manifest subject 352/352 |
| Ability state stream | `AbilitiesAndBuffsComponent` (`_cnc_h1`) | ◐ fc=34 brute-forced, inner decomposed (flag + u32 stream); semantics need game assets |
| GAS owner / avatar / attribute sets | `AresAbilitySystemComponent` (OwnerActor, AvatarActor, SpawnedAttributes, CachedAttributeSet) | ✅ via AbilitiesAndBuffsComponent->AresAbilitySystemComponent remap |
| Status effects on a player (nearsight / slow / detain / ...) | `EffectManagerComponent:MulticastPlayContinuousEffect` + `MulticastStopContinuousEffect`, on the **affected** player's actor | ✅ named, with start and end — see below |
| Active gameplay effects (GAS array) | `AresAbilitySystemComponent.ActiveGameplayEffects` | ❌ **not replicated.** Across 15 exports every such row has `bit_count == 0` and none sit on a player character; the GAS spec handles the group declares (`Def`, `Duration`, `StackCount`, `StartServerWorldTime`, ...) never appear on the wire at all |
| GAS attribute values | `AresAttributeSet.{BaseValue,CurrentValue}` per handle | ◐ **checkpoints only.** The live stream sends each attribute once when the channel opens and never updates it; `CurrentValue` does move (Reyna's ultimate puts handles at 1.1/0.9) but only checkpoint snapshots show it, and those are written at round transitions, so transient debuffs are gone by then |
| Persistent effect position (smoke/wall/molly/slow/trap) | `actors.parquet` class_path + spawn xyz | ✅ every spawned effect actor |
| Persistent effect lifetime | `actors.time_ms` paired across `event` `open`/`close` (non-fuel); `CurrentFuelLevel`+`WallActivated` (Viper) | ✅ |
| Smoke live position | `ReplicatedMovement` (x100) / `MulticastAddSmokeScreenPoint.Translation` | ✅ |
| Interaction progress (plant/defuse/orb pickup) | `UsableComponent.HighestProgress` (Float 0..1) / `bIsActive` | ✅ |

### `CastTime` is not measured from `roundStarted`

Its zero is the barrier drop -- the *end* of the buy phase -- while
`events.roundStarted` fires when the round begins, at the buy phase's start.
Joining a cast on `roundStarted + CastTime` therefore lands 30 seconds **early**,
or 45 on the first round of each half.

Measured on 10,460 casts over 20 replays, the residual
`(cast row's time_ms - roundStarted) / 1000 - CastTime` is, for the first
fourteen rounds:

| round | n | median residual |
|---|---|---|
| 1 | 394 | **44.99 s** |
| 2-12 | 6,247 | **29.88-29.91 s** |
| 13 | 347 | **44.89 s** |
| 14 | 364 | **29.89 s** |

Those are the buy-phase lengths the game uses -- 45 s on the first round of each
half, 30 s otherwise -- so this is confirmed against a constant the replay does
not carry, not fitted to the data. The correct absolute time is

    roundStarted + buyPhaseLength(round) + CastTime

The median is the statistic to use here, not the mean. `AbilityCastsThisRound`
is a replicated array that accumulates over the round, so a cast is re-sent on
every later replication and its `time_ms` drifts upward; the residual is exact
only on the first send. 60.1% of rows land within 29-31 s and the tail is all
re-replication.

### Status effects, and where they actually live

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

Over 71 demo replays the same pairing gives a much larger sample. These are not
corrections to the table above -- that one is honest about being a single
replay -- but they are the numbers to quote:

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
one-sided cast look like a debuff on the caster. Filter by suffix; the three
seen are `FXC_Vampire_4_NearsightAOE_Equip_C`,
`FXC_Wraith_Q_NearsightMissile_Equip_C` and `FXC_Sequoia_Q_FragileMissile_Cast_C`.

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
rate is 99/99; outside it is 72 in 122,356. The multiplier sits outside falloff,
so a Vandal headshot reads 320 = 40 x 4 x 2. Use `DamageDealt`, not
`DamageTaken` -- the latter is clamped to remaining life.

### Callout regions name half the maps

`CalloutRegionTrackingComponent.CurrentRegion` resolves through `net_guids` to a
region asset, which gives a player's position as a map callout rather than as
centimetres. Measured over 64 demo replays covering 12 maps, the asset names are
only useful on half of them:

| named | numbered |
|---|---|
| Ascent, Bonsai (Split), Infinity (Abyss), Port (Icebox), Rook (Corrode), Triad (Haven) | Canyon (Fracture), Foxtrot (Breeze), Jam (Lotus), Juliett (Sunset), Pitt (Pearl), Plummet (Summit) |

On the first group the leaf reads `CalloutRegion_ASite`, `BP_CalloutRegion_A_Lobby`,
`InfinityCallout_ABridge` and so on. On the second it is `BP_CalloutRegion10`,
`BP_CalloutRegion_C_0` -- an index with no name behind it. The prefix varies
independently of this (`BP_`, `InfinityCallout`, bare), so match on whether
letters survive after stripping it, not on the prefix.

The regions are still usable where they are numbered -- the id is stable within
a map and the spatial extent can be recovered by pooling player positions per
region -- but a consumer that wants to *label* the area has to supply its own
names for six of the twelve maps.

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

**Crouch is not in `movement_state`.** That column is 0 on every row of the
corpus. Crouch is `bCrouchHeld` on the character actor, or a ~19 cm drop in
`pos_z`, and crouch speed is ~190 cm/s.

## Movement & position

| Data | Source | Status |
|---|---|---|
| Position (cm) | `movement.parquet` pos_x/y/z | ✅ ≤0.0005 vs C# |
| Rotation (yaw/pitch) | movement | ✅ exact |
| Velocity | movement vel_x/y/z | ✅ exact |
| Time (128 Hz tick, resets per round) / global | movement `timestamp` / `time_ms` | ✅ — `timestamp` is a **tick counter**, not milliseconds |
| Posture (crouch) | `fields.bCrouchHeld` (not movement_state) | ✅ |
| Trajectory | movement time series per character | ✅ |

### The tick is 128 Hz by a 3:13 pattern, not by alternating

Consecutive per-character steps are 7 ms on 18.72% and 8 ms on 81.28% -- that is
3:13, or 3/16 and 13/16. The mean is 7.8128 ms, **127.995 Hz**, and sixteen
ticks come to 3x7 + 13x8 = **exactly 125 ms**. It is not a 1:1 alternation, so
do not assume 7,8,7,8 when reconstructing a clock; count ticks and multiply by
125/16. Measured over 37,775,664 steps on 20 replays.

`timestamp` increments by 1 per tick (Δ=1 on 37,938,231 steps, against 34 steps
of 3 or 4) and resets each round, which is why the round cut is found at the
point it drops rather than from a timer.

`time_ms` is the global scrub axis and is non-decreasing on all 71 replays --
though not strictly increasing, since many rows share a `time_ms`.
`duration_ms - max(time_ms)` lands within -1..8 ms.

### Minimap projection

The transform is not in the replay. It comes from **valorant-api.com**, which
publishes `xMultiplier`, `yMultiplier`, `xScalarToAdd` and `yScalarToAdd` per
map; join on `manifest.level_names_and_times[0].name`, which is that API's
`mapUrl`. Those constants are an external source and are not reproduced here.

**The axes cross.** What works is

    u = pos_y * xMultiplier + xScalarToAdd
    v = pos_x * yMultiplier + yScalarToAdd

`pos_y` drives the horizontal axis and `pos_x` the vertical. Of the four
sign/order variants only this one holds up: it puts 100.0000% of live positions
inside [0,1]² on eleven of twelve maps, while feeding `pos_x` to `u` collapses
to 0.9% on Haven and 3.1% on Fracture. Containment alone would not prove it --
a small enough scale contains everything -- so note also that the bounding
boxes fill roughly [0.01, 0.99], which a wrong scale would not.

Two things to handle first:

- **Park slot.** Hidden actors are parked at `pos_x ≈ -50000, pos_z ≈ -49900`.
  Filter on **both** x and z. Filtering on z alone misclassifies real falls.
- **Abyss is the exception, and not a decode fault.** It reaches 99.8416%: of
  2,482 out-of-range rows, 2,132 are already below z = -3000, and all 350 of the
  near-floor rows have negative `vel_z` (median -1693 cm/s, against 0 for
  in-range rows). The map has no floor, so players leave the minimap while
  falling. Nothing to fix -- clamp or drop by `vel_z`.

Containment was measured over 12 maps on 69 replays, 121,672,885 live movement
rows, on build 13.02.

## Weapons & loadout

| Data | Source | Status |
|---|---|---|
| Weapon instance class | `actors.parquet` class_path + `tools/equippable_table.py` (display name) | ✅ |
| Shot events (ammo, projectiles, vectors, seed, fire mode) | effect blobs | ✅ typed JSON |
| Magazine ammo over time | `AmmoComponent.AuthResourceAmount` (Int32) | ✅ via the `MagazineAmmo` remap; reads 0..100 |
| Reserve ammo over time | `AmmoComponent.AuthResourceAmount` (Int32) | ✅ via the `ReserveAmmo` remap -- same native component, second instance; reads 0..200, plus a 999 sentinel (below) |
| Equipped weapon (per player, over time) | `AresInventory.CurrentEquippable` / `NewCurrentEquippable` -> actor class | ✅ via InventoryComponent->AresInventory remap (resolve the NetGUID to its equippable actor) |
| Equipped weapon (on damage) | `MulticastNotifyDamage.EquippableUsed` | ✅ |
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
| Round result (winning team) | `RoundResults` struct blob | ✅ name-based (survives handle shifts) |
| Team score | derived / `BaseTeamState.Wins`/`Points` | ✅ (R1–R5 invariants verified) |
| Round number | `events.roundStarted` word0 / `RoundNumber` | ✅ |
| Half / side swap / overtime | `events.switchTeams` | ✅ |
| Per-round team economy | `TeamEconomy` (13.01) / `BaseTeamState` (13.02) | ✅ |

## Spike & objective

| Data | Source | Status |
|---|---|---|
| Plant / defuse / detonation | `events.spikePlanted` / `spikeDefused` / `spikeExploded` | ✅ |
| Spike carrier (who holds it, over time) | `BombEquippable_C.Owner` on the spike's own channel → `tools/extract_spike_carrier.py` | ✅ resolved to manifest `subject`; covers backpack, not just in-hand |
| Spike in hand (vs carried) | `AresInventory.CurrentEquippable` / `NewCurrentEquippable` == bomb GUID | ✅ the `in_hand` flag of the same view |
| Defuser | `TimedBomb.CurrentDefuser` (ObjectNetGuid) | ✅ |
| Planter | carrier at the `spikePlanted` timestamp (`extract_spike_carrier.py`) | ✅ from the Owner chain; the event payload itself carries no planter |
| Spike timer | `TimedBomb.TimeRemainingToExplode` / `DefuseProgress` | ✅ Double |
| Plant site (A/B) | `TimedBomb.PlantedAtSite` (EnumByte) + position derivation | ✅ absent handle = default site (UE default-value skip); 100% via spawn position |
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

Carrier resolution now misses no plant at all: `extract_spike_carrier.py`
reports `NO CARRIER` 0 times across the 71 exports, against 2 before the
disconnect fix above. Both of those were pawns whose `Owner` pointed at a
character the manifest had lost, and the tool said `NO CARRIER` rather than
guessing -- which is the only reason the failure was findable.

## Actor / GUID / structure

| Data | Source | Status |
|---|---|---|
| Every actor spawn/despawn + class/archetype/spawn location | `actors.parquet` | ✅ |
| GUID → object path | `net_guids.parquet` | ✅ |
| Containment chain (subobject → parent) | `net_guids.outer_net_guid` | ✅ |
| Full declared schema (475 groups, handle→name) | `manifest.net_field_export_groups` | ✅ |

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
| Game mode (Bomb / Swiftplay) | group_path (`GROUP_ALIASES` maps Swiftplay) | ✅ parser-side |

---

## Limitations (replay-format, not parser bugs)

- **Display names** — the replay carries no player names, only account UUIDs.
- **ACS** — `PlayerScoreComponent` is not replicated.
- **InventoryComponent** — RESOLVED: it replicates under its Blueprint class
  name, but the replay declares the property layout under the native parent
  `AresInventory`. The `KNOWN_SUBOBJECT_CLASS_PATHS` remap (InventoryComponent
  -> AresInventory, AbilitiesAndBuffsComponent -> AresAbilitySystemComponent)
  connects them, so the handles pick up names and types -- `CurrentEquippable`
  (equipped weapon / spike carrier) included. Remaining bare component groups
  (ZoomStateMachine, ReserveAmmo, CalloutRegionTracker, ...) need the same kind
  of Blueprint->native-parent map; their parents are not name-derivable and
  require the game's class hierarchy.
- **AbilitiesAndBuffsComponent** — the replay never declares its `_ClassNetCache`
  group, so `function_count` is brute-forced (fc=34). The outer RPC framing is
  fully recovered, and the inner payload is decomposed (a flag bit followed by a
  little-endian `u32` stream -- not the opaque blob it was once assumed to be).
  The stream is the GAS state-sync feed, not one RPC per ability cast, so it
  cannot attribute or count casts (use ability-actor spawns + `UltimateActive`
  for that). The later words' meaning is game-asset-dependent (the authoritative
  C# parser does not model this stream), so they stay in `raw_bits`.
- **spikeExploded** — not a limitation: `events.spikeExploded` is the canonical
  detonation signal and is always emitted. `RoundResults` records the round
  *win reason* (elimination/detonate/defuse), not whether the spike detonated,
  so it under-counts detonations and is not a reliable proxy.

---

## What's next (where to start)

Most of what this list used to hold is done. What remains is short, and two of
the three are limits rather than tasks.

1. **The next unnamed single handle** — `HANDLE_ADDITIONS` in
   `tools/apply_type_corrections.py` is currently empty: its one entry named
   `MagazineAmmo` handle 2 by hand, and the cooked game showed that group is an
   `AmmoComponent`, which the replay declares properly. The mechanism stays
   because the next bare handle will not necessarily have a native group to
   borrow from. Pin any new one in `crates/vrf-decode/src/tests/overlay.rs`.
2. **AbilitiesAndBuffs inner payload** — structurally decoded (`flag + u32`
   stream in `crates/vrf-decode/src/cnc.rs`), per-word meaning unknown.
   **Checked against the shipped game, not assumed:** the script object map has
   `/Script/ShooterGame.AresAttributeSet` as a class with *zero* members,
   because GAS attributes are `FGameplayAttributeData` fields declared in C++
   and cooked assets carry no member list for them. Unpacking the paks does not
   help. The fc=34 RPC timing and size are already exported.
3. **Keep the component remaps honest.** `KNOWN_SUBOBJECT_CLASS_PATHS` was read
   out of one build; a later one can rename a component and nothing here would
   notice on its own. Run `tools/check_component_remaps.py --export <dir>`
   against a replay from a new build. It needs no game install and no baseline.
4. **An `FText` decoder.** `FieldType` has none, which is why
   `Comp_AbilityStatisticsReplicator`'s `LocalizedStat` is deliberately
   untyped. Shifted one bit off the byte grid the payload reads `uint32 Flags=0,
   uint8 HistoryType=5` (`ETextHistoryType::StringTableEntry`), then a
   string-table asset path and a key, and the key is the statistic name. Nothing
   depends on it -- `Statistic` already carries the same fact as an enum -- so
   this is worth doing only when a second `FText` field turns up.

**Type nothing you have not seen decode.** `LocalizedStat` was typed `FString`
on the strength of the name and produced null on 3,011 of 3,011 rows while
`Decode errors: 0` held the whole time. A wrong type is not loud. After adding
one, count non-null values on the column before believing it.

### Done, and where the reasoning lives

| was | outcome |
|---|---|
| Remaining Blueprint components | done from the shipped game — "Reading component classes out of the game" below |
| `ReserveAmmo` | same pass; both ammo counters are `AmmoComponent.AuthResourceAmount` |
| Checksum propagation | built — `crates/vrf-decode/src/checksum_table.rs`, generated by `tools/extract_checksum_types.py`, consulted last in `resolve_entry` |
| Walk into `AbilityCastsThisRound[].Effects[]` | built — `ABILITY_CASTS_SCHEMA` in `crates/vrf-decode/src/array/schema.rs` |
| Exact ability cast count | the claim was wrong; see below |
| RPC signature aliasing | rejected with measurements — "Closed: RPC signature aliasing" |
| More name-resolved properties | rejected with measurements — "Closed: more name-resolved properties" |

**The cast-count entry is worth keeping as a lesson.** It said "confirmed
wire-limit: the GAS stream is state-sync, not one RPC per cast". The premise was
right and the conclusion did not follow --
`Comp_AbilityStatisticsReplicator` replicates one record per cast, with the
caster's subject UUID, slot, round, time and world location.

It went unseen through several sweeps for a reason that will recur: the rows
were always there and the array was already being flattened with every member
*named*, but no member had a type, so every value sat in `raw_bits` with the
`value_*` columns null. Each survey of "what is still untyped" ranked by row
count and read the top of the list; these fields sit at 300-800 rows each and
never made the cut. **A named field with no type is invisible to a scan that
starts from typed columns.**

### Reading component classes out of the game

The bare group names -- `ZoomStateMachine`, `ReserveAmmo`, `CalloutRegionTracker`
and a dozen others -- are Blueprint *component instance* names, not classes, so
nothing in a replay says what they are. The installed game does say, and it does
not need decryption: VALORANT's IoStore containers are `Compressed+Signed+
Indexed` with a zero encryption GUID, so the `Encrypted` flag is simply off.

The chain, for the record, since nothing in `tools/` reproduces it:

1. `.utoc` -> directory index -> the asset paths in each container.
2. A cooked Blueprint stores each component as a `<Name>_GEN_VARIABLE` export.
   Its `ClassIndex` is an `FPackageObjectIndex` of type `ScriptImport`, which is
   a hash rather than a name.
3. `global.ucas` holds the script object map -- a name batch followed by
   `FScriptObjectEntry` records -- which turns that hash into
   `/Script/ShooterGame.<Class>` by walking the outer chain.

Chunks are Oodle-compressed, so this needs an Oodle-capable reader; vrfkit
already depends on `oozextract` for replay chunks, which is what was used.

Every pair landed in `KNOWN_SUBOBJECT_CLASS_PATHS` in
`crates/vrfkit/src/sink/paths.rs`. The check on the method is that the same pass
independently reproduced `InventoryComponent -> AresInventory` and
`AbilitiesAndBuffsComponent -> AresAbilitySystemComponent`, which had been
inferred from handle shapes and are now confirmed.

It also corrected one guess. `MagazineAmmo` and `ReserveAmmo` are both
`AmmoComponent`, a group the replay declares with handle 2 as
`AuthResourceAmount` -- so the hand-written `AmmoCount` name, the one entry
`HANDLE_ADDITIONS` ever had, was in the right place with the wrong word. Both
ammo counters now read the real declaration and that mechanism is empty.

Effect on 02d4d478: unnamed handles 17,013 -> 2,460, decoded OK 702,149 ->
714,070, `Typed` 71.0% -> 72.2%, decode errors still 0. Corpus-wide, 215/215
replays with decode errors 0.

**This is the one thing here that a game patch can silently invalidate.** A
renamed component stops matching and its handles go quiet again, and the replay
never named it either, so no unit test can see it.

`tools/check_component_remaps.py` is what watches for that. It needs only an
export -- not the game, not a baseline -- so it works on a replay from a new
build, which is exactly when the question comes up. For each pair it compares
the rows still bare under the leaf against the rows that reached the native
group; healthy is ~0.1%, and a rename measured 15.6%. Asking only whether the
target has rows is not enough: nine leaves share
`EquippableStateMachineComponent`, so one going quiet leaves the other eight
covering for it. ClassNetCache rows are excluded, because the two RepLayout-only
remaps leave their RPC stream bare by design.

Three bare names are left, and the game says why none of them can be fixed this
way. `AttachedDamageSection` and `MapTargetingState` do name real classes --
`AttachedDamageSectionComponent`, `MapTargetingStateComponent` -- but **the
replay declares neither group**, so a remap would point at nothing; their
handles have no declaration to pick up anywhere in the file. `AresAttributeSet_2`
is a GAS attribute set rather than a component, which is the same wall as item 4
above. Nothing further to get from the paks for these.

### Closed: what the three mechanisms cannot reach

After the table, the engine-reference names and checksum propagation, 480,471
rows on 02d4d478 are still untyped across 1,511 `(group, field)` pairs. Sorted
by why:

| rows | why |
|---|---|
| 289,533 | declared `Raw`/`Skip` -- intentional, not a gap |
| 173,535 | has a `compatible_checksum`, but no declared field donates that checksum |
| 17,403 | no checksum at all (the unresolved `AbilitiesAndBuffs` payload) |

The first bucket is 60% of it and is nothing to fix: `BaseReplayController`'s
4kbit blob alone is 225,808 rows, and the per-agent
`ReplayLastTransformUpdateTimeStamp` rows are declared raw on purpose.

Two ordinary additions came out of the second bucket and are now typed --
`StopMovementTime` and `HandleNumber`, above. The largest remaining item does
not yield to a `FieldType` at all.

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

So the framing is settled and a dedicated decoder in the shape of `effect` or
`structs` could walk it. **What is missing is what any of it means.** Nothing
names the tags: the RPC carries only `PlayerID` beside it, no descriptor
declares the parameter, no checksum donor exists, and the cooked assets do not
help either -- this is engine-side input capture, not a Blueprint property. A
decoder written now would produce seven anonymous payloads, which is what
`raw_bits` already gives. Left alone until there is a source for the tag
meanings.

`ServerMovementTime` (4,654 rows) reads cleanly as Float and is still
deliberately untyped: the epoch is undocumented, so the values are not
interpretable. That decision is recorded with the other declined fields in the
`ADDITIONS` comment.

### Closed: RPC signature aliasing

Aliasing an undeclared RPC group to a declared sibling -- the `GROUP_ALIASES`
shape -- was measured across all 67 undeclared RPC groups and is **not worth
doing**. The 17 workable pairs would type 165,374 rows, but 139,222 of those are
already reachable by checksum, and the remaining 26,152 rest on nothing but a
shared parameter name, which is the rule this project has already rejected.
`Scale3D` is the example: its alias source is a group the replay's manifest does
not contain at all, so the type would come from an unrelated Blueprint's
same-named property.

Aliasing an RPC group also opens a hazard the class-level aliases do not, since
`resolve_entry` retries the *whole* order against the alias, handle fallback
included, and parameter handles do not correspond between two signatures.
Measured: aliasing `MulticastNotifyHeal` to `MulticastNotifyDamage_Base` reads
`LifeChangeBySection` through the other function's handle table as
`DamageDealt: Float`, on 1,918 rows, every one leaving 145 residual bits.

Bit-width agreement is not evidence here. Every candidate pair consumed its bits
exactly, including the wrong one above; a 1-bit `Bool` read as `EnumByte` also
passes. Width is a necessary condition and nothing more.

### Closed: more name-resolved properties

`ENGINE_OBJECT_REFS` typed 6,048 rows by resolving four `AActor` object
references by name, so the obvious next question was which other properties
deserve the same. The answer, after auditing every field name in the generated
table rather than only the ones this replay spawned, is **none**, and the reason
is worth keeping.

`Owner` was safe because its *encoding* is fixed, not because its name is
standard. `ReplicatedMovement` is just as standard a name and is declared three
different ways in the table -- `RepMovement{ByteComponents}` on 18 groups,
`RepMovement{ShortComponents}` on 6, `Skip` on 1. Those differ in width, so a
name rule there would not read a wrong value quietly; it would desync the block.
`RelativeScale3D` and `CosmeticRandomSeed` split the same way, and only on
groups this replay never spawned -- measuring against the wire alone would have
called both of them clean.

**RPC parameters are categorically excluded from a *name* rule.** A parameter
name is scoped to one function signature: 38 of the 211 parameter names in this
replay carry more than one `compatible_checksum`, which is exactly the same
statement the checksum makes. Typing them by name would merge signatures that
the schema itself distinguishes.

(An earlier revision of this section offered `AllianceFilter` as the
counterexample -- `EnumByte` on one group, `EnumRemainingBits` on another. It is
not one. Both groups declare it under checksum 2270825073 and every row is 3
bits wide, and `decode_byte` reads `bits_remaining()` for any width in 1..=8, so
the two declarations return the same value. It is an inconsistency in the table,
not a difference on the wire. The conclusion stands on the checksum evidence
above.)

Three names did clear the mechanical bar -- `AttachComponent` (declared `Raw`,
so it would produce no values at all), `PreventPickupCharacter` and
`OwningPrimaryDataAsset`. Together they would fill 59 rows of 1,255,920, and
each is a claim about Valorant's class hierarchy rather than about Unreal, so
each moves with a game patch. Not worth the standing risk.

Two loose ends found on the way, neither a live bug: `RemoteRole` is declared
`ObjectNetGuid` on one group and `Skip` on 36, and `StartTimeStamp` is `Double`
on one and `Float` on another. `RemoteRole` never appears on the wire in this
corpus, so nothing decodes through the odd entry.

The overlay table and all generated files are off-limits to hand-editing:
`tools/extract_descriptors.py` then `tools/apply_type_corrections.py` then
`cargo fmt` is the only path, and `python tools/check_docs.py` + the export
baseline must stay green.
