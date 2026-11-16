# Interpreting reference-parser comparisons

`tools/compare_with_csharp.py` compares exports of the **same input replay**.
It reports differences in metadata, field/RPC coverage and sampled movement
values. A successful exit means the comparison observed overlapping field
coverage; it does not mean every compared value matched or either parser is
complete. Keep the report and both exports private when the input is private.

```bash
python tools/compare_with_csharp.py /private/csharp-export /private/vrfkit-export
```

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
