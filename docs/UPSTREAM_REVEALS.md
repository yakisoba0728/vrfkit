# Reveal descriptors and player effect targets, 2026-09-24

The latest common all-build audit is [BUILD_VERIFICATION.md](BUILD_VERIFICATION.md).
This report retains its original implementation and sample scope.

Selective import from ValorantReplayParser
[`2b66c65a7b116154e18ebb84d9f6795f2b080233`](https://github.com/michel-giehl/ValorantReplayParser/commit/2b66c65a7b116154e18ebb84d9f6795f2b080233),
on top of vrfkit `7ae8efb9f8dfe328f9773e76b040cab7e6ffc518`.
The implementation is included in the commit introducing this report.
The previous upstream survey is [UPSTREAM_PARITY.md](UPSTREAM_PARITY.md).

## Imported behavior

- Imported the five reveal descriptors verbatim and registered them in the
  Hunter/BountyHunter catalogs. The upstream Sova projectile replaces the
  equivalent local recon-bolt descriptor; the drone and shock-bolt extensions
  remain. The generated overlay gains nine named entries and twelve explicit
  handle entries. The generator now resolves constants in their owning
  descriptor class, so the five independent `DescriptorPath` constants cannot
  overwrite each other or bind to an unrelated class.
- Fade's reveal projectile now decodes `ReplicatedMovement` with byte-component
  rotation. Sova's projectile already used that decoder. Device and pulse
  Owner/Instigator definitions make their provenance explicit; those values
  were already decoded through the engine reference fallback.
- `extract_player_effects.py` applies the upstream distinction between body
  identity and possession to blind updates and continuous-effect observations
  (including nearsight). A manifest `SpawnedCharacter` identity admits player
  targets; possession/ownership does not. Device observations remain available
  as source evidence but do not increment the player totals. The parser's
  existing manifest behavior already followed this rule and is now covered by
  the five upstream possession regression cases.

This is an observation view, not a port of the complete C# event enrichers.
There is no cast correlation, unique-hit deduplication, inferred explosion
time or effect-interval join. Reveal descriptors do not identify revealed
enemies or reveal duration. Original raw fields remain intact.

## Actual replay evidence

Twelve preserved replays were freshly exported with `--checkpoints`: six on
13.06 and one each on 12.10, 12.11, 13.00, 13.02, 13.04 and 13.05. Comparisons
use the matching `upstream-implementation-20260923/candidate` exports from
`4533ff77ad331a0899cfe3c8661bbc631e019a7a`. The existing 13.01 export is excluded
from these fresh-validation totals.

| Output change | 13.02 | 13.06 sample 03 | 13.06 sample 06 | Total |
|---|---:|---:|---:|---:|
| Fade movement rows: raw-only to typed JSON | 649 | 1,146 | 167 | 1,962 |

All 1,962 new values match an independent Python bit decoder, including full
payload consumption, location, rotation, velocities, flags and optional frame
fields. Choosing short-component rotation instead fails exact consumption on
832 of those payloads. All 1,559 existing Sova movement values also match that
independent decoder; all 1,090 Owner/Instigator values across the five exact
reveal group paths match independent packed-GUID decoding. No matching reveal
payloads occur in the checkpoint field tables in this sample.

Only the three `fields.parquet` files above change. Every row, raw byte and
column except the new Fade `value_str` values is identical. All other 153
Parquet files, including every checkpoint table, are byte-identical. The
checksum generator was run on all twelve fresh manifests: existing mappings
agree and the conflicting movement checksum remains excluded. Its 91 additional
donors from unrelated fields are outside this change and were not imported.

The effect tool observes 306 blind updates: 289 on confirmed player bodies
and 17 on other actors. Those 17 remain in the output. Their spawn classes
include Sova drones, Yoru decoys, Fade prowlers and other ability pawns.
These are replicated update counts, not unique flash hits. Effect paths are
reported without guessing whether every continuous effect is a debuff.

The framing guard passes 12/12 replays with 100% oracle rates and zero
malformed fields. The checkpoint decode guard reports zero overlay, struct,
array, truncated-RPC and movement failures: 7,483,677 decoded main rows and
590,387 decoded checkpoint fields. Existing partial/skipped data remains;
the framing totals still include 87,038,567 skipped bits.

## Reproduction

Inputs remain private under `$env:LOCALAPPDATA/vrfkit/baseline-corpora`.
Run artifacts, exact before/after audit script and its report are under
`$env:LOCALAPPDATA/vrfkit/upstream-reveals-20260924` (`audit.py`, `audit.json`).
The audit selects the five exact descriptor paths and only Owner, Instigator
and ReplicatedMovement, checks main and checkpoint tables, and compares all
Parquet columns before accepting a typed-only difference.

```powershell
cargo +1.86.0 build --release -p vrfkit --locked
& $exe export $replay --out $export --checkpoints
python -W error tools/extract_player_effects.py --export $export --out player-effects.json
python -W error tools/validate_corpus.py $exe $corpus --recursive
python -W error tools/check_decode_errors_corpus.py $exe $corpus --recursive --jobs 2 --checkpoints
python -W error tools/check_export_baseline.py --baseline $baseline --replay $replay --out $guardExport --checkpoints --require-input
```

Use the fresh executable, preserved corpus, matching replay/export and private
baseline paths for those variables. The 13.06 sample-03 export baseline changes
only `overlay_decoded_ok` (+1,146), `overlay_not_in_table` (-1,146), and the
`fields.parquet` byte size (+29,984) and hash, all explained by the Fade values.
Its updated private baseline is checked again without `--update`.

The MSRV 1.86 sweep covers formatting, clippy, workspace tests, all-target and
all-feature checks, rustdoc, the offset probe, Rust/Python Parquet interop and
all 27 advertised feature configurations. Generator, ASCII, baseline-schema
and full documentation checks are also run. The suites contain 702 Rust and
846 Python tests; the imported descriptor table is reproducible from source.
