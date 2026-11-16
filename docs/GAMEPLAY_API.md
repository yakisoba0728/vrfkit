# Optional gameplay observations

The Rust `vrfkit::gameplay` module adds a small, explicit interpretation layer
on top of `ReplayReader`. It supports native round starts, character deaths,
spike plants, defuses and explosions, plus observed round-phase bytes from the
main replication stream. Calling the helpers opts in; ordinary Reader and CLI
paths do not call them or perform additional gameplay work.

This is an observation API. It does not assemble a complete match, resolve
accounts, award kill credit or infer missing events. Stateless projections
borrow source evidence. The optional `MainIdentityTimeline` retains identity
and lifecycle evidence for packet-scoped queries; raw Event payloads and field
rows remain unchanged.

```rust
use vrfkit::{ReplayEvent, ReplayReader};
use vrfkit::gameplay::{GameplayError, GameplayKind, native_event};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut deaths = 0_u64;
    let summary = ReplayReader::new().read_path("match.vrf", |event| {
        if let ReplayEvent::ContainerEvent(raw) = event {
            if let Some(observed) = native_event(&raw)? {
                if let GameplayKind::CharacterDeath { killer, victim } = observed.kind {
                    deaths += 1;
                    // These are actor GUIDs. status() says Null or Unresolved;
                    // no account, PlayerState or movement join was guessed.
                    let _ = (killer.status(), victim.status(), observed.time_ms());
                }
            }
        }
        Ok::<(), GameplayError>(())
    })?;
    println!("{deaths} native death declarations; {:?}", summary.block_verdict());
    Ok(())
}
```

Run the complete aggregate-only example, including phase observations:

```bash
cargo +1.86.0 run --release -p vrfkit --no-default-features \
  --example gameplay_events -- match.vrf
```

It prints no player identifiers. Recognized but invalid observations stop this
example through `ReadError::Consumer(GameplayError)`; a different consumer may
instead record the projection error and retain the raw observation.

## Contract

| Item | Meaning and limits |
| --- | --- |
| `native_event(&EventChunk)` | `Ok(Some(event))` for a supported validated declaration; `Ok(None)` for an unprojected group; `Err` for a supported group that fails validation. |
| `round_phase_event(&FieldRecord)` | Only use with main-stream `ReplayEvent::Records` fields. Other field names return `None`. A recognized phase field with unproven identity, absent/conflicting overlay or raw mismatch returns an error. |
| `GameplaySource::NativeContainer` | Native outer Event source with id, group, metadata, both header times, validated inner seconds and borrowed raw payload. There is no invented packet ID. |
| `GameplaySource::MainField` | Exact original field, including packet/channel, actor/subobject, schema identity, typed values and raw bits. It is a derived observation, not a native Event. |
| `GameplayEvidence` | `ValidatedNativePayload` or `ValidatedReplicationValue` describes exactly what was checked. It is not a probability or a claim of independent game-level confirmation. |
| `ActorReference::status()` | Zero is `Null`; every nonzero GUID remains `Unresolved`. No future GUID snapshot is treated as earlier identity evidence. |
| Time | Milliseconds come from the source. Zero remains a valid wire time. Native inner seconds must be finite and agree with `time1` within the shared measured 1.001 ms quantization bound. `time2` stays explicit and is not assumed equal to `time1`. |
| Publication | All callback observations are provisional until Reader completion. A later read error cannot roll back data saved by a consumer. Stage external output when all-or-nothing publication is required. |

The native projection reuses `vrf_container::parse_known_event_payload` and its
time validator. It also requires the declared payload length to agree, no
unexplained trailing chunk bytes, and a NUL-terminated UTF-8 or UTF-16 enum
name. These stricter interpretation rules do not change the raw parser's
tolerance or preservation behavior. Ultimate-use and side-switch Event payloads
remain unprojected: their words are not assigned additional semantics here.

## Phase observations and existing Python tools

The helper accepts only the observed
`/Game/GameModes/Bomb/BombGameState.BombGameState_C_ClassNetCache` actor field
`MulticastSetPhase.NewPhase`, checksum `3954716686`. Its sole typed overlay must
be a byte-valued integer and equal the exact eight-bit raw window.

| Wire value | Rust observation | Existing Python relation |
| --- | --- | --- |
| 2 | `Reset` | `reset_ms`, corroborated by `ClientResetRound` in the full round extractor |
| 3 | `RoundStart` | `start_ms`, corroborated by `ClientRoundStart` |
| 4 | `BuyPhaseEnd` | `buy_end_ms`, corroborated by `ClientBuyPhaseEnd` |
| 5 | `UnnamedPostRound` | `post_round_ms`; no RPC provides a semantic name |
| 6 | `SideSwitch` | `side_switch_ms`, corroborated by `Multicast Side Switch Event` |
| Other byte | `Unknown(value)` | Preserved without a game meaning |

Repeated observations remain repeated. No previous state is retained, so the
API does not invent a transition before the first observation or deduplicate
repeated phases. Source timestamps are not sorted or used to guess order.
Checkpoint fields must not be passed as main fields. The default Reader calls
skip checkpoint snapshots and report their count separately; optional
checkpoint delivery remains a distinct source and state scope.

The Rust projection reuses the existing field decode and the Python phase
mapping's meaning. It deliberately does not duplicate the richer algorithms
in `tools/extract_rounds.py`, `extract_damage_events.py`, `extract_kill_ledger.py`
or `player_identity.py`. Those tools retain full-round RPC corroboration,
ambiguous-reference handling and independent kill evidence. For example:

```bash
python tools/extract_rounds.py --export out/match --out out/rounds.parquet
python tools/extract_damage_events.py --export out/match --out out/damage.parquet
```

Full round-end results, account identity, damage details and ability semantics
remain outside this API. The optional main-stream body relation and movement
association below do not infer those meanings. Spike native
payloads contain no planter, defuser or site; this API supplies none.

## Packet-scoped body identity and movement

`spawned_character_observation(&FieldRecord)` projects only the measured
BombPlayerState `SpawnedCharacter` identity and its canonical Swiftplay alias,
checksum `1413303948`, on the actor itself. The sole integer overlay must be a
u32 and equal a completely consumed packed NetGUID raw window. Zero remains
an explicit null declaration. An unknown group, checksum, nested object,
conflicting overlay, truncated GUID or extra raw tail is an error; other field
names return `None`. The exact borrowed row is available as `source`.

The body criterion comes from `tools/player_identity.py` and the existing
typed overlay. A pawn's `PlayerState`, `Owner` or `PossessedCharacter` is not
accepted as independent proof that it is a player's body. The new Rust layer
does not copy the Python tool's whole-export account join: it records what
was observed before an explicit packet boundary. No final `net_guids`
snapshot, manifest account, coordinate proximity or future write is used.

```rust
use vrfkit::{ReplayEvent, ReplayReader};
use vrfkit::gameplay::{IdentityError, IdentityResolution, MainIdentityTimeline};

let mut identities = MainIdentityTimeline::new();
let mut resolved_moves = 0_u64;
let summary = ReplayReader::new().read_path("match.vrf", |event| {
    if let ReplayEvent::Records(batch) = event {
        identities.observe_records(&batch)?;
        for index in 0..batch.movement.len() {
            let association = identities.movement_identity(&batch, index)?;
            if let IdentityResolution::Resolved(body) = association.identity {
                resolved_moves += 1;
                // This is the observed PlayerState/body relation, not an account.
                let _ = (body.player_state_net_guid, body.declaration.packet_id);
            }
        }
    }
    Ok::<(), IdentityError>(())
})?;
// Publish only after Reader success; all prior observations were provisional.
println!("{resolved_moves} associated moves; {:?}", summary.block_verdict());
# Ok::<(), Box<dyn std::error::Error>>(())
```

Use a fresh timeline per replay and feed every main `Records` callback once,
in callback order. Checkpoint callbacks remain independent and must not be
fed to this main timeline. `observe_packet` is the equivalent entry point for
owned main actor/field rows; it requires the complete packet vectors in their
original per-vector order. It does not infer ordering between those vectors.
Partial-only final flushes do not add evidence. An invalid observation or
non-increasing packet invalidates the timeline, preventing an ignored error
from silently leaving an older relation active.

| Result | Exact meaning |
| --- | --- |
| `Null` | The queried character GUID is zero. |
| `Unresolved(reason)` | No current preceding declaration, unobserved packet boundary, missing/ended/conflicting actor lifecycle, prior-lifetime declaration, indeterminate same-packet order, or invalid timeline. |
| `Ambiguous { declarations }` | Multiple current PlayerState declarations name this character. All source declarations remain available in PlayerState GUID order. |
| `Resolved(BodyIdentity)` | One current declaration names this character and both actor lifetimes are observed and compatible. The result borrows the complete declaration and both original actor opens. It is not account resolution or kill attribution. |

`resolve_before_packet(reference, packet)` considers only writes and lifecycle
events in packets strictly smaller than `packet`. Timestamps, including zero
and regressions, remain evidence but do not define ordering. The query can
reach at most the boundary immediately after the latest observed packet;
it cannot predict a later unseen stream. A later write cannot backfill an
earlier query. A null write or new body retires the current relation while
historical queries still see the previous relation. This differs intentionally
from the Python whole-export history, which keeps earlier reconnect bodies.

Only an actor `close` or replacement on an active channel ends its observed
lifetime. `dormant` releases the channel without destroying the actor, and a
later reopen preserves that lifetime. An open while the GUID is still active
on a channel is conflicting lifecycle evidence, not proof of continuity.
After close/reuse a fresh body declaration is required; a declaration in the
same packet as a close cannot be assigned to one side of that close.

`movement_identity(&batch, index)` uses the exact same-index movement row and
`MovementProvenance`. Its transport completion packet must equal the movement
packet, the initial packet cannot be later, and the complete batch must already
have been observed. The source's actor may be the replay controller: identity
uses the movement row's wire character GUID and does not equate those actors.
The returned structure borrows both original records unchanged.

The association requires a body relation established before the initial
fragment packet and no relevant reference write or lifetime change through
the completion packet, inclusive. A change in that interval stays
`Unresolved(ConcurrentChange)`, including an otherwise valid same-packet
write. The Reader's separate row vectors cannot establish its order with the
movement RPC. This deliberate loss of coverage prevents a plausible but
unproven association. Native death chunks have no main packet identity;
this layer provides no timestamp-based death/player join.

The timeline retains only identity declarations and actor lifecycle evidence,
not all field and movement rows; its memory grows with that retained history.
It is opt-in, can be dropped by the caller, and adds no work to ordinary Reader
or CLI export. No performance improvement is claimed.

An aggregate-only example prints all four identity counts, including zeros,
without player identifiers:

```bash
cargo +1.86.0 run --release -p vrfkit --no-default-features \
  --example gameplay_identity -- match.vrf
```

## Reference comparison

The read-only comparison source is `ValorantReplayParser` commit
`fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8`.
Its [ValorantEventEnricher](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/Replay.Valorant.Events/ValorantEventEnricher.cs)
is an optional per-replay sink that forwards native and derived events and
flushes deferred enrichers after successful completion. It supports movement,
shots, flashes, nearsights, walls, footsteps, crosshair and ownership features
beyond this Rust module's scope. Its
[round phase projection](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/Replay.Valorant.Events/Rounds/ValorantRoundPhaseChanged.cs)
uses named RPCs. vrfkit's initial Rust projection uses validated native Event
chunks and explicit observed phase values, while its existing Python layer
continues the richer analytical joins. These are different contracts; no
event-coverage or performance superiority claim follows from this API.

## Phase B validation (historical)

The [reproduction record](evidence/phase_b/gameplay/README.md) documents six
fresh Reader executions and an independent Python/raw-byte comparison:
560 native projections and 302 phase observations, with zero differences in
values, multiplicity or order within each source stream. The native set
includes 424 death declarations, 76 round starts and 60 spike events. The
oracle exports are explicitly attributed to their historical Phase A source.
Twelve focused tests also exercise malformed inputs, raw preservation and
cancellation. This evidence does not resolve actors to players, validate
checkpoint gameplay or establish performance superiority.
