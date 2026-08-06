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
| Character NetGUID | `manifest.players.character_net_guid` / `SpawnedCharacter` | ✅ joins movement 10/10 |
| Agent (characterId) | `manifest` game_specific_data.playerLoadouts | ✅ |
| Two players on the same agent | disambiguated by `subject` (characterId alone can't) | ✅ |
| Display name | — | ❌ replays carry no display names, only the subject UUID |

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

## Abilities

| Data | Source | Status |
|---|---|---|
| Ultimate cast | `events.characterUltimateUsed` (word0 = character) | ✅ |
| Cooldown / start time | `Comp_Ability_CooldownComponent` | ✅ Double |
| Ability cast count | ability-actor spawns (over-counts) / `characterUltimateUsed` (ultimates exact) | ◐ no exact per-cast count on the wire |
| Ability state stream | `AbilitiesAndBuffsComponent` (`_cnc_h1`) | ◐ fc=34 brute-forced, inner decomposed (flag + u32 stream); semantics need game assets |
| GAS owner / avatar / attribute sets | `AresAbilitySystemComponent` (OwnerActor, AvatarActor, SpawnedAttributes, CachedAttributeSet) | ✅ via AbilitiesAndBuffsComponent->AresAbilitySystemComponent remap |
| Active gameplay effects (buffs/debuffs) | `AresAbilitySystemComponent.ActiveGameplayEffects` (FastArray elements) | ◐ array framing recovered; per-effect semantics need game assets |
| Persistent effect position (smoke/wall/molly/slow/trap) | `actors.parquet` class_path + spawn xyz | ✅ every spawned effect actor |
| Persistent effect lifetime | `actors.time_ms` paired across `event` `open`/`close` (non-fuel); `CurrentFuelLevel`+`WallActivated` (Viper) | ✅ |
| Smoke live position | `ReplicatedMovement` (x100) / `MulticastAddSmokeScreenPoint.Translation` | ✅ |
| Interaction progress (plant/defuse/orb pickup) | `UsableComponent.HighestProgress` (Float 0..1) / `bIsActive` | ✅ |

## Movement & position

| Data | Source | Status |
|---|---|---|
| Position (cm) | `movement.parquet` pos_x/y/z | ✅ ≤0.0005 vs C# |
| Rotation (yaw/pitch) | movement | ✅ exact |
| Velocity | movement vel_x/y/z | ✅ exact |
| Time (128 Hz tick, resets per round) / global | movement `timestamp` / `time_ms` | ✅ |
| Posture (crouch) | `fields.bCrouchHeld` (not movement_state) | ✅ |
| Trajectory | movement time series per character | ✅ |

## Weapons & loadout

| Data | Source | Status |
|---|---|---|
| Weapon instance class | `actors.parquet` class_path + `tools/equippable_table.py` (display name) | ✅ |
| Shot events (ammo, projectiles, vectors, seed, fire mode) | effect blobs | ✅ typed JSON |
| Magazine ammo over time | `MagazineAmmo.AmmoCount` (Int32, per weapon) | ✅ typed via handle addition (3..25, depletion ramp) |
| Equipped weapon (per player, over time) | `AresInventory.CurrentEquippable` / `NewCurrentEquippable` -> actor class | ✅ via InventoryComponent->AresInventory remap (resolve the NetGUID to its equippable actor) |
| Equipped weapon (on damage) | `MulticastNotifyDamage.EquippableUsed` | ✅ |
| Skin / spray / charm | `manifest` playerLoadouts (per subject) | ✅ |

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

Roughly in value order. Each names the file to touch first.

1. **Remaining Blueprint components** — `ZoomStateMachine`, `ReserveAmmo`,
   `CalloutRegionTracker`, `VisionComponent`, `*StateMachine`, ... resolve to a
   bare Blueprint class name instead of their native parent group, exactly as
   `InventoryComponent` did before the remap. Two ways to get the
   Blueprint->native parent map:
   - **Authoritative**: open the VALORANT paks in FModel/UModel, read each BP
     component's parent class, and add `(leaf, native_path, RepLayout)` rows to
     `KNOWN_SUBOBJECT_CLASS_PATHS` in `crates/vrfkit/src/sink/paths.rs`.
   - **Inferred**: for each bare group, match its handle/bit-width structure
     against the manifest's declared native groups (the handles and types line
     up 1:1) -- no game files needed, but it is confirmation, not authority.
2. **Checksum propagation** — the largest typed-rows-per-unit-of-work left:
   about 192,800 rows over 38 `(group, parameter)` pairs, all of them RPC
   parameters. Every `NetFieldExport` carries a `compatible_checksum`, and
   Unreal hashes the property's *type* into it alongside its name: across the
   84 RPC groups here, exactly one pair of distinct names shares a checksum
   (`AKSwitchArray`/`AkSwitchArray`), while 38 of 211 parameter names carry more
   than one. So a checksum identifies a property in a way a name cannot, and a
   parameter the descriptors never declared can take its type from a declared
   field with the same checksum.

   Spot-checked at the value level, not just the bit width: `PlayerID` (53,605
   rows) read as `Int32` yields exactly {256..265}, the same ten values as the
   already-typed `BombPlayerState_C.PlayerId` rows in the same export;
   `StartMovementTime` (17,818) is exactly -1.0 throughout.

   **Built** -- `crates/vrf-decode/src/checksum_table.rs`, generated by
   `tools/extract_checksum_types.py` and consulted last in `resolve_entry`. The
   ordering worry that made this look hard dissolved: a checksum is derived from
   the property, not the replay, so the map is *generated* rather than learned
   at run time and nothing depends on when a group arrives. Verified stable on
   all five supported builds. Wired on the RPC path only, where the schema
   lookup is already being made; the property path would need a second lookup on
   its hottest loop to reach 4% as many rows.

   Remaining after it: about 50.2M rows corpus-wide, the largest single item
   being `ClientReplayReceiveInputEventProcessingCapture.InputEventData`
   (53,605 rows on 02d4d478, 32 bits), whose checksum no declared field donates.
3. **`HANDLE_ADDITIONS` for the next unnamed single handle** — the mechanism
   added for `MagazineAmmo` generalizes. `ReserveAmmo` (reserve bullets) is the
   obvious next candidate once its group is resolved (it may fall out of item 1).
   Add to `HANDLE_ADDITIONS` + `ADDITIONS` in `tools/apply_type_corrections.py`,
   pin in `crates/vrf-decode/src/tests/overlay.rs`.
4. **AbilitiesAndBuffs inner payload** — structurally decoded (`flag + u32`
   stream in `crates/vrf-decode/src/cnc.rs`), but the per-word meaning needs the
   GAS C++ serializer, which is compiled and not in the assets. Effectively
   blocked without game source. The fc=34 RPC timing/size is already exported.
5. **Exact ability cast count** — confirmed wire-limit: the GAS stream is
   state-sync, not one RPC per cast. No on-wire work helps; the approximation is
   ability-actor spawns + `characterUltimateUsed`/`UltimateActive`.

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
