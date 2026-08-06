# 2D replay viewer — design

**Date:** 2026-08-07 · **Status:** design approved, not yet implemented
**Base:** `89cdb54`

## Purpose

A **parser verification instrument**, not a product viewer. It exists to answer one question:
*does the data vrfkit extracts actually describe a game of VALORANT?* Every design choice below
resolves in favour of making a wrong value visible, not in favour of looking good.

This is the reason it is worth building now. The typed share sits at 75.1% and the checksum
buckets say the remaining untyped rows are a coverage gap, not a resolution bug — so there is no
longer a code-quality path to improvement. What is unknown is whether the missing 25% is the 25%
a reproduction actually needs. Building the reproduction is what answers that.

## Non-goals

- Not a shippable product. No hosting, no accounts, no polish budget.
- Not a replacement for `valplay` (analysis + AI coaching). This renders; it does not advise.
- Not a batch tool. Rendering round 3 of all 215 replays onto a contact sheet is a genuinely
  useful *second* instrument and is explicitly deferred.

## Architecture

```
out/<replay>/            tools/build_replay_viewer.py        replay_<id>.html
  movement.parquet  ─┐                                        (one self-contained file)
  actors.parquet    ─┤    1. slice by round                    data  : base64 typed arrays
  events.parquet    ─┼─▶  2. downsample + measure at 125Hz  ─▶ minimap: data URI
  fields.parquet    ─┤    3. join effects / health / gear      JS+CSS: inline
  manifest.json     ─┘    4. map constants + image (cached)
                                   │
                         out/mapcache/   (gitignored, fetched once)
```

The builder is pure Python and follows the shape of the tools already in `tools/` — take an
export directory, derive a view, write one artefact. No server, no CDN, no framework: a strict
self-contained page is what makes a finding shareable as a single file.

## The one real risk: downsampling can hide the bug

Movement is sampled at **125 Hz** (median 8 ms gap, 1,839,607 rows on the reference replay).
Playback needs nothing near that. But a teleport — the exact defect this instrument should catch —
can fall between two downsampled frames and vanish.

**Playback and measurement are therefore separate passes.**

- **Playback** renders at 20 Hz (50 ms).
- **Measurement** runs over the **full 125 Hz** stream and ships its findings into the page.

An anomaly that is not drawn is still listed: *"round 3, 12.4 s, player_4, 1,850 cm in one tick"*,
clickable to seek there. The page states the playback rate next to the findings so nobody reads
"looks smooth" as "is smooth".

## Time base

Confirmed by measurement, not assumed: `movement`, `actors`, `fields` and `events` all share one
millisecond clock running 0 → ~1,772,000 on the reference replay. `roundStarted` lands at 62 /
92,033 / 227,108 / … and deaths interleave sensibly. No conversion is needed anywhere.

Rounds are cut on `events.roundStarted`. **Round-at-a-time playback is the default** — scrubbing
29 minutes as one timeline is not a useful verification act.

## Projection

The transform is not in the replay. It comes from valorant-api.com per map, joined on
`manifest.level_names_and_times[0].name`. Use the formula `docs/DATA.md` already validated over
12 maps and 121,672,885 rows — **the axes cross**:

```
u = pos_y * xMultiplier + xScalarToAdd
v = pos_x * yMultiplier + yScalarToAdd
```

Park slot (`pos_x ≈ -50000` **and** `pos_z ≈ -49900`) is filtered on **both** axes; filtering on
z alone misclassifies real falls. Filtered rows are **counted and displayed**, never silently
dropped.

If the map constants cannot be fetched, **the build fails**. It does not draw on a blank square at
a guessed scale — a plausible wrong picture is worse than no picture, which is the same rule the
decoder follows.

## Layers

Six, each independently toggleable.

1. **Players** — the 10 GUIDs in `manifest.players`, which carry 1,776,314 of 1,839,607 movement
   rows (96.6%). Dot, facing cone from `yaw`, team colour, alive/dead state.
2. **Ability pawns** — the 30 non-manifest GUIDs, all of which resolve through
   `actors.parquet` (`actor_net_guid` → `class_path`): `Pawn_Hunter_E_Drone` (Sova drone),
   `Pawn_Aggrobot_RollyPolly` (Raze boombot), `Pawn_Aggrobot_SeekerNade`. Real gameplay, drawn
   distinctly and labelled by class.
3. **Post-death cameras** — `*_PostDeath_PC`. **Off by default.** These are spectator cameras, not
   positions; drawing them produces a dead player apparently walking around. This is a trap the
   design names because the data does not.
4. **Effects** — smoke / wall / molly / slow / trap / recon / orb, as circles at their spawn
   position with their lifetime. Derived by importing `extract_active_effects.build_with_tally`
   rather than reimplementing it, so there is one implementation of the pairing rule — including
   that a `dormant` event does **not** end an instance.
5. **Events** — deaths as a killer→victim line, spike planted / defused, round boundaries.
6. **Health** — per-player bar.

### Health conventions, encoded rather than rediscovered

`docs/DATA.md` records three rules, each of which was found by a check failing. The viewer
encodes all three, and the spec names them so an implementer does not re-derive them wrongly:

- Death is `bAliveAfterChange == False`, **not** `LifeResult == 0`. A character really can sit at
  exactly 0 health and be alive (65 cases, all KAY-O).
- Armour is `AttachedDamageSection`, **not** `ShieldDamageSection` — the latter is an empty shell
  whose every `LifeResult` is 0.
- `MulticastNotifyOverhealDecay` sends `DeltaLife` positive while life goes down.

Also: the local handles differ per RPC (10-13, 1-4 or 2-5), and `MulticastNotifyHeal` /
`MulticastNotifyOverhealDecay` name their array `LifeChangeBySection`, not `LifeChangeEvents`.
Filtering on the array name alone silently drops more than half the calls.

## The checks

The builder runs these over the **full-rate** data and ships the results into the page as a
findings list. Each entry seeks to its moment on click. All eight print their count including zero,
for the reason the export summary already prints its zeros: a line that appears only when non-zero
cannot distinguish "nothing wrong" from "the check stopped running".

1. Inter-sample displacement above a threshold (teleport).
2. Projected position outside [0,1]², excluding the park slot.
3. Park-slot rows filtered (count).
4. A manifest player with zero movement rows inside a round.
5. A `characterDeath` whose victim has no movement row near that time.
6. A movement GUID that resolves to neither a manifest player nor a known pawn class.
7. An effect whose spawn position projects outside the map.
8. Health increasing without a heal or overheal RPC in the same window.

## Failure rules

- A missing input table fails the build, naming the file.
- Unavailable map constants fail the build.
- Every row the builder drops or filters is counted, and the count reaches the page.
- The page states its playback rate wherever it might be mistaken for the sample rate.

## Testing

The builder is where the logic lives, so that is where the tests go — synthetic Parquet fixtures,
the same shape as the existing `tools/tests/`:

- projection, including that the axes cross and that a swapped variant fails
- park-slot filtering on both axes, and that z-alone would misclassify
- **that downsampling does not hide an injected teleport** — the measurement pass must still
  report it, which is the single most important test in the suite
- round slicing on `roundStarted`
- each of the eight checks, fired and not fired
- effect pairing delegated to `extract_active_effects`, including the `dormant` rule

The HTML stays thin deliberately. It gets a smoke test: the emitted file parses, and its embedded
arrays have the lengths the builder reported.

## Open questions

None blocking. Two to settle during implementation with a measurement rather than a guess:

- The teleport threshold in cm/tick. Pick it from the reference replay's own distribution rather
  than inventing a round number.
- Whether 20 Hz playback is comfortable at round length; adjust after seeing one round render.
