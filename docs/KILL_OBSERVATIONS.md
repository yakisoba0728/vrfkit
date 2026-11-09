# KillData observations and the kill ledger

Two commands read the measured `KillData` array rows of one export made with
`--checkpoints` (both read main and checkpoint field, declaration and
reference tables):

```powershell
python tools/extract_kill_observations.py --export out/replay --out out/kill-observations.json
python tools/extract_kill_ledger.py --export out/replay --out out/kill-ledger.json
```

## Observations

`extract_kill_observations.py` writes serialized element updates (schema 1,
kind `vrfkit_killdata_observation_export`), not a deduplicated kill ledger. A
replay can repeat a payload, send several elements in one parent array, or
send a partial element holding only `bDidKillTriggerFinisher`, so a missing
member stays `null`/absent and is never copied from another record.

Each observation is keyed by source table, checkpoint identity when
applicable, physical `KillData` parent-row ordinal and element index. It keeps
packet/object coordinates, a parent-raw digest, exact member raw windows and
the measured values (victim, equippable class, weapon theme, assisting
players, damage type/value/region, game time, round timestamp/number,
finisher flag). `time_ms`, `GameTimeElapsed` and `RoundTimestamp` are separate
clocks, never converted into one another. References keep their numeric ID
and scoped resolution status: victim and assisting players against the
matching actor table, the rest against the NetGUID table; unresolved IDs stay
in the output.

The tool fails when declarations differ from the measured group/name/checksum
identities, array framing is incomplete, a nested handle is unexpected, a
typed child differs from its raw window, child coordinates cross scopes, or
children are not physically adjacent to their parent. `provenance` records the
manifest, input Parquet, extractor and `wire_bits.py` SHA-256.

## Ledger

`extract_kill_ledger.py` (schema 1, kind `vrfkit_character_death_ledger`)
keeps labelled `characterDeath` events, component-local KillData state and the
links between them, with the whole observation stream in
`source_observations`.

`state_projection.entities` is keyed by `(export_id, object_net_guid,
element_index)`. A complete main observation creates a base state; later
finisher-only updates to that key are revisions. `tools/kill_state.py` fails
on a partial update before a base, duplicate complete keys, conflicting
ownership, duplicate coordinates or an unmeasured partial shape. Checkpoint
observations are snapshots compared against the base and every revision; they
never create entities, and unmatched ones stay in
`checkpoint_snapshots.unresolved`.

Every `characterDeath` and `roundStarted` payload is validated whole (tag,
words, bounded FString, terminator, float value agreeing with `time1`);
invalid ones stay visible and cannot join. Death words name character pawns,
KillData names PlayerStates, so each pawn is bridged through its `PlayerState`
field within its actor lifetime (a reopen resets it; dormancy does not). A
match needs all of: both pawns resolving to the KillData owner/victim pair,
`round_number` equal to the unique strictly prior validated `roundStarted`
word, KillData's `time_ms` 0-50 ms after the event's `time1`, and exactly one
candidate on each side -- never a nearest timestamp. `same_player_state` and
`different_killer_same_victim_context` describe observed identities, not
suicide or kill-credit rules, and are not summed as kills. The inputs are
hashed before and after; a changed input fails the run.

## Measured builds

Both tools accept 11.06--12.09, 13.01--13.06, 13.08 and 13.09 (`MEASURED_BUILDS`) and refuse
12.10, 12.11, 13.00 and any future build before reading a row, even one whose
declarations happen to match: parser support is not enough, and those three
builds emit no KillData children.

| Exports | Validated deaths | Complete main entities | Joins | Unmatched deaths / observations | Finisher revisions | Checkpoint snapshots matched | Lag (ms) |
|---|---:|---:|---:|---:|---:|---:|---:|
| 48 legacy (11.06--12.09) | 7,381 | 7,335 | 7,332 | 49 / 3 | 12 | 73,518 | 8-35 |
| 714 (13.01--13.05) | 101,966 | 101,212 | 101,179 | 787 / 33 | 197 | 1,010,872 | 5-41 |
| 38 (13.06) | 5,408 | 5,373 | 5,370 | 38 / 3 | 8 | 53,563 | 7-30 |
| 1 (13.08) | 171 | 169 | 169 | 2 / 0 | 0 | 1,881 | 8-20 |
| 2 (13.09) | 308 | 307 | 307 | 1 / 0 | 0 | 3,079 | 8-11 |

In every run no join was ambiguous, no checkpoint snapshot was unresolved,
every finisher revision changed the state (all `false` to `true`), and every
unmatched observation kept same-round, same-victim context with a different
killer. A 100 ms lag cap changed no match. The 714-export run was also checked
by an independent identity reconstruction and join comparison; on 13.06 the
event row counts read straight from `events.parquet` and the standalone
observation document both matched.
