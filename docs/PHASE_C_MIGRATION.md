# 0.6.0 migration: output counters and Reader API

This guide is for consumers upgrading to 0.6.0 from 0.5.x or from an earlier
source revision. 0.6.0 is a minor version bump because the native
movement-counter migration and the new required FocusProjectiles counters are
not compatible with consumers that require the older manifest and summary
schema. It also adds the Reader's checkpoint callbacks, optional body
identity, and bounded nonseekable input. Pin the `v0.6.0` tag (or another
exact revision) before adopting these APIs. The
[error, network and publication changes](FINAL_CONSOLIDATION_MIGRATION.md)
are a separate guide.

## Reader checkpoints

Existing `read(&bytes, consumer)` and `read_path(path, consumer)` calls retain
their default behavior: main records and native Events are emitted, while
checkpoint chunks are counted as skipped. Call `read_with_options` or
`read_path_with_options` with `ReadOptions::default().with_checkpoints(true)` to
receive `ReplayEvent::Checkpoint` callbacks. Keep any existing custom
`schema_limits` when constructing the options; opting into snapshots should
not silently replace the caller's resource policy.

Checkpoint events expose their own identity, metadata, declarations, records,
content-block boundaries and summary. Keep snapshot records separate from
the main timeline and from other snapshots. Retain borrowed data by copying
the records needed by your application; no callback borrow may outlive the
callback. Stage irreversible external output until the whole read succeeds.
The Reader can emit provisional callbacks before detecting a later error.

The error enum includes a typed checkpoint-schema error with checkpoint
provenance. Downstream error handling should preserve the cause and distinguish
input/decode errors from the application's callback error. `ReplayError` is
`#[non_exhaustive]`, so a match needs a fallback arm; the
[Rust API section](FINAL_CONSOLIDATION_MIGRATION.md#rust-api) of the other
guide lists its variants. Existing extensible public event and summary types
are non-exhaustive as well; downstream matches need a fallback arm. The [Rust API contract](RUST_API.md) specifies ordering and
error precedence. CLI `export --checkpoints` retains its separate snapshot
tables and main-pass error precedence.

```bash
cargo +1.86.0 run -p vrfkit --no-default-features --locked \
  --example stream_checkpoints -- match.vrf
```

## Optional body identity and movement association

Create one `MainIdentityTimeline` for one replay and feed **every main
`Records` batch exactly once in callback order**. Do not feed checkpoint
records into it. Preserve the original actor/field vector order when using
the lower-level packet method. The timeline uses packet/lifecycle evidence,
not a sort by timestamps or a final GUID-cache backfill.

`spawned_character_observation` accepts only the proven declaration identity.
`IdentityResolution` preserves `Null`, `Unresolved` and `Ambiguous` outcomes;
successful `BodyIdentity` retains its declaration and actor-open evidence.
Null/replacement writes and actor lifetime boundaries affect current
resolution without erasing earlier historical evidence. A retired actor's
old declaration must not make a later, separately observed lifetime ambiguous.
Same-packet changes that lack a defensible ordering remain unresolved or
ambiguous, according to the observed conflict.

`movement_identity(&batch, index)` refers to the row at that exact index and
its own transport provenance. The complete batch must already have been
observed. Resolution requires the relationship to precede the initial
fragment and remain unchanged through the completing packet, inclusive.
It uses the movement row's wire character GUID; a controller's source actor
is not automatically the character. This associates a transport window, not
the movement server tick or a native Event's timestamp.

The existing `native_event` and `round_phase_event` helpers remain useful on
their own. The new timeline does not join deaths to accounts, assign kill
credit or reconstruct a full match. See [Gameplay API](GAMEPLAY_API.md) for
the exact identities, lifecycle rules and result types.

```bash
cargo +1.86.0 run -p vrfkit --no-default-features --locked \
  --example gameplay_identity -- match.vrf
```

## Finite nonseekable inputs

`ReplayReader::read_bounded(&mut input, max_input_bytes, options, consumer)`
accepts a `Read` input with an explicit cap. It buffers the complete encoded
input through EOF, then uses the same parser/options contract as the slice
API. Prefer `read_path` for a stable seekable file when chunk buffering is
wanted; a pipe does not turn this method into incremental packet streaming.

The cap is required and zero never means unlimited. At exactly the cap, a
one-byte probe distinguishes EOF from excess data; it may block waiting for
the next byte. An over-limit input returns `ReadError::InputLimit { limit }`
after consuming that probe byte. An empty input within a zero-byte cap still
undergoes ordinary replay-header validation. `Interrupted` reads are retried;
other I/O failures retain their cause. Neither failure nor callback rejection
rewinds the input.

The cap bounds retained encoded bytes/requested vector capacity, with an
additional 8 KiB read buffer. Allocator overhead, decompression, parser state
and caller-retained output are extra; it is not a process-memory limit.
Allocation is incremental and allocation refusal becomes an I/O error.
No temporary file is created. Timeouts and cancellation belong in the input
wrapper because callbacks cannot interrupt the initial buffering phase.
See [the complete bounded-input contract](RUST_API.md#nonseekable-input-with-an-explicit-byte-cap).

```bash
cargo +1.86.0 run -p vrfkit --no-default-features --locked \
  --example read_bounded_stdin -- 67108864 --checkpoints < match.vrf
```

## Export and counter migration

The earlier [native movement-component migration](NATIVE_MOVEMENT_COMPONENTS.md#counter-and-source-api-migration)
replaces the old tail/trailer interpretation with nine native counters. Keep
all nine in CLI/manifest quality checks. 0.6.0 additionally requires these
sink counters in main and checkpoint data, including their zero values:

| Counter | Meaning |
| --- | --- |
| `route_children_focus_projectiles` | Child rows emitted by the exact evidence-qualified 12.05 Clay route |
| `focus_projectiles_empty_trailers` | Empty deltas whose exactly one final zero byte is preserved in the raw parent while the measured array window is decoded |
| `focus_projectiles_declaration_refusals` | 12.05 parents routed by group, name and checksum but refused because the parent handle is not 45 or the handle-46 member declaration differs; the raw parent remains and no child row is emitted |

The summary prints them as the final `N focus projectiles` item of
`Route children:`, `FocusProjectiles trailers: N empty deltas` and
`FocusProjectiles refused: N parent updates with a mismatched declaration`, each with a
`Checkpoint` twin.

The route adds typed child rows before their unchanged raw parent. On the
measured 12.05 public fixture this produces 53 new main child rows, with no
new checkpoint child rows. Existing typed values, raw bytes and source
provenance must remain unchanged. Consumers must not assume a fixed historical
field-row count or discard the raw parent when a child is present.

Strict baseline validators now require the new keys; treating a missing key
as zero would conceal an older executable or incomplete export. Executables
that predate these keys, 0.5.1 included, therefore fail the current harness
even on a replay with no FocusProjectiles rows. Use an explicitly
schema/output-matched baseline and retain all equality gates.

## Upgrade checklist

- [ ] Replace the eight removed movement tail/trailer counters with the nine
  native component counters in every CLI/manifest check; do not convert old
  values into new ones.
- [ ] Accept the FocusProjectiles child rows and the three FocusProjectiles
  counters, and the other new keys and summary items listed in the
  [changelog](../CHANGELOG.md).
- [ ] Review source API consumers for checkpoint events and errors, schema
  limits, timeline ordering and explicit bounded-input caps.
- [ ] Re-export a baseline with the 0.6.0 executable before comparing against
  it; keep historical corpus evidence separate.

No release note for 0.6.0 claims improved wall time, CPU, utilization, RSS,
allocation behavior or throughput.
