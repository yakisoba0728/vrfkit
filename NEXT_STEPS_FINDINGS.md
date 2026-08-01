# 7-A Re-scoping: verified findings

Written 2026-08-01. Supersedes PROJECT_STATUS.md section 7-A.
Every number below comes from a direct tool run against real data, not an estimate.

Build state: `cargo test` = 228 passed, 0 failed. Tree clean.
26 commits (PROJECT_STATUS header said 25, section 2 said 24 -- both were stale).

---

## Headline

PROJECT_STATUS section 7-A said:

> The shot EffectContainer carries a net GUID that refers to the equippable
> actor. [...] Estimated effort: 1-2 hours. No Rust change required.

**The premise is false.** But the conclusion is better than the document claims:
the resolution works at **100%**, via a different route, and the missing piece is
one small export addition.

| Claim in 7-A | Measured |
|---|---|
| EffectContainer carries the equippable GUID | `effect_equippable` set on **0 of 2,647** reference shots (0.0%) |
| Join it to actors.parquet | `firing_state` GUIDs match **0 of 2,475** actors.parquet rows |
| No Rust change required | A small Rust **export** change is required |

## The route that works, measured end to end

```
shot.firing_state (already emitted by the adapter)
  -> NetGuidCache guid_to_outer          <- NOT currently exported
  -> equippable actor GUID
  -> actors.parquet class_path           <- already exported, already correct
```

Probe run (temporary instrumentation, since reverted; export totals unchanged at
608,020 / 429,633 / 342,735 / 1,839,607):

```
guid table entries                      : 16,167   (14,480 have an outer)
distinct firing_state GUIDs in shots    : 175
  present in guid -> path               : 175 / 175  (100%)
  present in guid -> outer              : 175 / 175  (100%)

shots resolved to a weapon class_path   : 2,475 / 2,475  (100.00%)
class_path equal to C# reference        : 2,475 / 2,475  (100%)
```

The 28 apparent mismatches in the first scoring pass were an artifact of the
join key `(time_ms, actor_net_guid)`: exactly 28 keys carry two shots with
different weapons in the same millisecond. Verified by counting collisions --
28 ambiguous keys, 28 apparent mismatches. Not a resolution error.

Resolution is **one hop** for the sampled cases, not a deep walk:

```
FiringState      3086 -> outer 2910  (Sheriff)
FiringState      3692 -> outer 3220  (Ghost)
FiringState      3726 -> outer 1568  (Classic)
ZoomedFiringState 3748 -> outer 1010 (Headhunter)
```

And the destination table is already complete: all **157 / 157** reference
equippable GUIDs are in our `actors.parquet` with **byte-identical class_path**.

---

## How the C# parser resolves it (for reference)

`ValorantReplayParser/src/Replay.Valorant/Combat/ValorantShotEventEnricher.cs:123`
`ResolveShotEquippable`, three tiers in order:

1. `shot.EffectEquippable` -- GUID off the effect container.
   **Measured dead: 0 / 2,647.**
2. `ResolveFromFiringState` (:163) -- walk the `FiringState` GUID's outer chain
   (`TryGetOuterNetGuid`, max 16 levels) to an actor known to be an equippable.
   **This is the one that fires.**
3. `FiringPlayerState -> BombPlayerState.PossessedCharacter ->
   AresInventory.CurrentEquippable` (:78, :103). Not needed -- tier 2 is 100%.

---

## Revised plan

| Step | Where | Size | Status |
|---|---|---|---|
| **A2. Export netguid -> (path, outer)** | Rust, `vrf-export` + a `NetGuidCache` accessor | small | The only Rust work. Data already in `guid_to_outer` (`cache.rs:89`) and `guid_to_path` (:88); the exporter simply never emits it. |
| **A3. Outer-chain walk in the adapter** | Python, `to_valplay_bundle.py` | small | Mirror C# tier 2. Verified to hit 2,475/2,475. |
| **A4. class_path -> display name + category** | Python adapter | small | See invariant note below. |
| ~~A1. Resolve `InventoryComponent` -> `AresInventory`~~ | Rust, `vrf-schema` | large | **Not needed for 7-A.** Tier 3 is unnecessary. |

Export `path` alongside `outer` -- C# tier 2's own final fallback
(`ValorantEquippableResolver.Resolve(value, netGuidCache)`, :184) uses the path,
and `path` is what identifies a GUID as `FiringState` vs `ZoomedFiringState`.
The table must cover **all** GUIDs in the chain, not only ones that opened an
actor channel: the 175 firing_state GUIDs appear in zero rows of
`actors.parquet` and zero rows of `fields.parquet`.

Suggested shape: `net_guids.parquet` with `(net_guid, path, outer_net_guid)`,
16,167 rows for this replay -- trivially small.

---

## Invariant conflict to decide before A4

`ValorantReplayParser/src/Replay.Valorant/Combat/ValorantEquippableResolver.cs:20`
is 130 lines of hardcoded table:

```csharp
Define("/Game/Equippables/Guns/Sidearms/Revolver/RevolverPistol.RevolverPistol_C",
       "Sheriff", ValorantEquippableCategory.Sidearm),
```

valplay's `_actorindex.py` docstring claims the name is "a server-stored field"
and "nothing is hard-coded". True **only from valplay's side** -- the hardcoding
lives one layer down, in the C# parser.

So reproducing `shot.equippable.name` requires a hardcoded table somewhere, which
collides with PROJECT_STATUS section 8 / tradeoff 3 ("NO HARDCODED NAMES ANYWHERE").

Recommended: keep the Rust parser pure -- it emits `class_path`, which is
self-describing (`AssaultRifle_AK`) -- and confine display-name mapping to the
Python adapter, where it is presentation rather than a parsing rule. Record the
decision in the tradeoffs section rather than letting it happen by accident.

---

## Secondary findings

### 7-C's ceiling is confirmed real -- do not spend time there

Searched all 475 declared export groups: **zero** contain `AbilitiesAndBuffs`.
The document's assumption that no cache group is declared for it is now a
measured fact. That 91.7% of unattributed bits is unreachable until a future
game build declares the group.

### A1, if ever done, would help a family of groups

Top unnamed (`field_name = null`) group_paths in `fields.parquet`:

```
13043  InventoryComponent          <- tier 3 (not needed for 7-A)
 8042  ZoomStateMachine            <- fire mode / posture
 3124  MagazineAmmo                <- weapon_stats
 1782  CalloutRegionTracker
  746  MapTargetingState
  693  HealthDamageSection
  564  ReserveAmmo                 <- weapon_stats
  516  PMAimToolingPointsTarget
  470  VisionComponent
  464  AresAttributeSet_2
```

All actor **instance** names that never reached their declared component class
group. Note this is not a mechanical extension of `resolve_cnc_for_instance_name`
(`cache.rs:301`): no stem of `InventoryComponent` produces `AresInventory` -- the
class carries an `Ares` prefix the instance name lacks. It is a design problem,
which is why avoiding it for 7-A matters.

### Shot count gap: 2,647 reference vs 2,475 ours

The reference resolves `equippable` on exactly 2,475 of its 2,647 shots -- the
same number of shots our adapter emits. Our adapter filters on
`FiringState.FiringPlayerState` being present. If that filter is what produces
both numbers, then PROJECT_STATUS's "ray_count 2475/2475 exact" is a
self-selecting comparison and reads stronger than it is. Classify the 172:
pull their `source_id` / `fire_mode_evidence` from the reference file and
determine whether they are gun shots we drop or ability/melee effects that were
never gun shots. Not blocking for 7-A.
