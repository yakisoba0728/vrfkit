# Interpreting reference-parser comparisons

`tools/compare_with_csharp.py` compares exports of the **same input replay**.
It reports differences in metadata, field/RPC coverage and sampled movement
values. A successful exit means the comparison observed overlapping field
coverage; it does not mean every compared value matched or either parser is
complete. Keep the report and both exports private when the input is private.

```bash
python tools/compare_with_csharp.py /private/csharp-export /private/vrfkit-export
```

## Integration contracts at the pinned revision

The reference for this comparison is ValorantReplayParser
[`fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8`](https://github.com/michel-giehl/ValorantReplayParser/tree/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8),
the default-branch revision rechecked on 2026-11-16 in the
[upstream audit](UPSTREAM_AUDIT_2026_11_16.md). Every reference link below pins
that source. vrfkit's column describes this integration branch's API; the
measured comparisons later in this document retain their original vrfkit
revision and replay scope.

| Contract | vrfkit | ValorantReplayParser at the pinned source |
| --- | --- | --- |
| Library integration | Rust 1.86, borrowed `ReplayReader` callbacks; source dependency with default features disabled removes the Parquet writer graph. | .NET 10, synchronous `IReplayEventSink` and a detached read result. The pinned [README](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/README.md) documents an experimental NuGet package and an unreleased event-project migration. |
| Input ownership | `read_path` uses seekable chunk input; `read(&bytes, ...)` borrows a supplied snapshot. Large payloads and parser/consumer state still cost memory. | The [base archive stream constructor](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/Replay.Encoding/Archives/ByteArchiveReader.cs) buffers the remaining input. This is an input contract, not a measured whole-process RSS comparison. |
| CLI data product | Typed/raw Parquet tables and manifest; the separate JSONL converter preserves selected cells. | The [JSON export runner](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/CliReader/JsonExport/ReplayExportRunner.cs) opts into enrichment and writes NDJSON plus a manifest. The event schema and vrfkit's table JSONL envelopes are different formats. |
| Container Event chunks and checkpoints | Native Event chunks are exposed and exported; checkpoint tables require `export --checkpoints`. The Rust Reader skips checkpoints. | The [chunk dispatcher](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/Replay.Unreal/Chunks/ReplayChunkDispatcher.cs) skips both chunk kinds. Its native parser events are a different concept from these container Event chunks. |
| Gameplay layer | Optional helpers expose validated native round/death/spike declarations and phase observations; Python tools provide separate richer joins. Actor references in the Rust projections remain unresolved. | The optional [event enricher](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/Replay.Valorant.Events/ValorantEventEnricher.cs) adds movement, shots, flashes, nearsights, walls, footsteps, crosshair, phase and ownership observations. Deferred enrichers flush after a successful read. |
| Preservation and counting | Available raw field windows remain beside the typed overlay; unknown whole payloads can be retained. Nullable raw cells, synthesized children, successful movement output and explicit failure counters limit reconstruction. | Typed descriptors expose decoded-property state and selected speculative decoders have raw fallback. The [manifest contract](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/CliReader/JsonExport/ReplayExportManifestWriter.cs) says its default export omits undecoded groups and empty decoded shells, and emits the latest move per remote-character update. |

Use [the adoption guide](ADOPTION.md) to choose a vrfkit entry point and
[Gameplay API](GAMEPLAY_API.md) for its exact observation scope. Neither
implementation's successful read establishes full game semantics. Both expose
provisional callbacks before completion, so a consumer owns its publication
policy. Different defaults, native-event terminology and movement row counts
must be reconciled before comparing coverage or work.

No speed superiority follows from this source comparison. vrfkit's profiling
and input-path reports measure named builds and replay inputs independently;
they are not matched-work performance measurements against the reference.

## Count units must agree

The C# manifest's `stats.bunch_count` and `stats.packets_with_bunches` measure
different things: a single packet can contain multiple bunches. The report's
**Bunches** row compares `bunch_count` with vrfkit's `bunch_count`. If the C#
manifest lacks that counter, the report shows `?`; packets containing bunches
cannot substitute for a missing bunch count. A measured zero remains zero.

Both counters are independently emitted by the reference parser's
[schema-v9 manifest writer](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/CliReader/JsonExport/ReplayExportManifestWriter.cs).
The regression fixture uses 8 packets containing 21 bunches to distinguish
the units. Those values are synthetic, not corpus measurements.

## Movement sample boundaries

When both exports contain the metadata, movement values are joined by
**packet ID, character GUID and movement timestamp**. Display time alone is
not a unique sample identity: several moves can share a packet or millisecond,
and the reference emits only the latest move per update. Joining the wrong
samples can manufacture position and velocity discrepancies.

Strong identities are reserved first. An unmatched strong identity is reported
as unmatched; it never falls back to a different move at a similar display
time. Multiple candidates are reported as ambiguous and excluded from numeric
comparison. All candidates for an ambiguous strong identity are also reserved
from legacy matching. The tool never chooses a row because its position or
velocity looks closest to the reference.

The Parquet read includes every sampled strong packet ID, regardless of its
display time. An exact identity outside the display-time window must remain
available; the window applies only to legacy joins. Synthetic regressions
exercise earlier and later display times, a maximum unsigned 64-bit packet ID,
and ambiguous identity rows that must not be reassigned to a legacy sample.

Older exports without the identity metadata use display timestamp and
character GUID, first exactly, then within one millisecond. The Parquet read
window includes both margins, so the earliest sampled C# row can match a Rust
row one millisecond earlier. The lower bound is clamped to zero because the
Parquet display timestamp is unsigned. Each Rust row is consumed at most once.
Ambiguous legacy candidates, including two neighbors around a missing exact
time, are also excluded instead of silently paired in file order.

The boundary regression uses a synthetic C# timestamp of 100 ms and Rust
timestamp of 99 ms. It proves the advertised join tolerance works at the
sample boundary; it does not establish that this offset occurs in a real
replay.

### Observed comparison scope

The identity join was also exercised on three public fixtures using reference
revision `fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8` and vrfkit revision
`67c0a2d94fbcbeeefa56761f1bc13d9f6c829fb0`, with default C# enrichment and
main-pass Rust exports. No parser output was changed to obtain a match.

| Public build | Reference movement rows | Rust movement rows | Unique identity matches | Missing / ambiguous |
| --- | ---: | ---: | ---: | ---: |
| 12.10 | 8,853 | 8,907 | 8,853 | 0 / 0 |
| 12.11 | 2,949 | 3,001 | 2,949 | 0 / 0 |
| 13.00 | 4,946 | 4,991 | 4,946 | 0 / 0 |

Across these matched rows, the maximum absolute position difference was
0.00046875000043655746 in the exported coordinate units; yaw and pitch were
equal, and maximum velocity differences were at most
0.000048828125045474735. These are observed differences, not a claim that all
fields are exact or semantically correct. The report applies no guessed
numeric pass threshold. Unmatched additional Rust rows and other field
semantics are outside this value comparison.

Packet, bunch and actor-open/close counts also matched on these fixtures.
The `(group, field)` inventory compares serialized C# property names with
Rust wire field paths; aliases and nested representations require
investigation before naming a difference a missing decoder or lost value.

## Export scopes differ

For reference revision `fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8`:

- Its [chunk dispatcher](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/Replay.Unreal/Chunks/ReplayChunkDispatcher.cs)
  skips checkpoint and container-event chunks. vrfkit's optional checkpoint
  tables and container event table therefore have no corresponding output in
  that export path.
- Its [export runner](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/CliReader/JsonExport/ReplayExportRunner.cs)
  opts into event enrichment and writes NDJSON. vrfkit writes typed/raw
  Parquet tables, with analytical views provided separately.
- The manifest writer explicitly states that only the latest decoded move
  per remote-character update is emitted, and undecoded export groups or
  decoded shells without payloads are omitted. The event and movement row
  counts therefore cannot be treated as equal-work totals.

Use these comparisons to locate coverage differences and investigate shared
values. A wall-time ratio between these default exports is not an
equivalent-work speed comparison. Recheck the reference revision and output
contract when repeating the exercise; the reference implementation is not a
ground-truth oracle.
