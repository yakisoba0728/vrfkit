# Complete movement-update comparison

`tools/compare_movement_provenance.py` checks every latest-move update in two
compatible exports. It uses the actual initial-packet and update context
provided by the Rust Reader. The existing sampled
[`compare_with_csharp.py`](REFERENCE_COMPARISON.md) remains a separate report;
its historical results are not rewritten by this tool.

## Reproduce

Use the same unchanged replay for both parsers. The current comparison contract
is based on reference revision
[`fbf24b6`](https://github.com/michel-giehl/ValorantReplayParser/tree/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8).
Export its default `movement.ndjson`, then generate the Rust Reader companion
stream from the source checkout containing the provenance API:

```sh
cargo +1.86.0 run --release -p vrfkit --example movement_provenance --locked -- match.vrf > movement-provenance.jsonl
python tools/compare_movement_provenance.py reference/movement.ndjson movement-provenance.jsonl --out movement-comparison.json
```

Either input may be gzip-compressed with a `.gz` suffix. An ordinary
`movement.parquet` export lacks the required transport/update identities and
cannot substitute for the companion stream. Keep raw diagnostic files local;
the aggregate JSON report contains counts, input digests and numeric error
maxima, without row identities or source filenames.

Exit 0 means all gates below passed. Exit 1 means a complete comparison found
missing/extra/ambiguous updates or value/count disagreement. Exit 2 means the
input contract, quality, configured resource limits or I/O prevented a complete
comparison. The report distinguishes `pass`, `fail` and `error`; a count-only
match or empty comparison cannot pass. Report publication is atomic, and an
output path resolving to an input or implementation file is refused.

## Identity before values

The join key is **initial packet, actor GUID, object GUID, channel and sparse
wire update index**. The reference emits this source identity through its
[movement enricher](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/Replay.Valorant.Events/Movement/ValorantMovementEventEnricher.cs)
and [JSON writer](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/CliReader/JsonExport/ReplayEventJsonWriter.cs).
For an actor content block, the reference uses the actor GUID as object GUID;
the Reader's explicit `None` is normalized accordingly. `Some(0)` is a real
subobject identity and remains zero. A missing nullable member is an error.

No position, angle, velocity, movement timestamp, character GUID or approximate
display time participates in candidate selection. No fixed fragment-packet
offset is permitted. The Reader supplies the actual initial and completion
packets from authoritative reassembly, so fragmented packets are handled under
the same key as ordinary packets.

The Rust stream groups moves by its actual packet/RPC/array/update-arrival/
component occurrence. Move indices must start at zero and be contiguous;
transport source and character identity must remain stable within a component.
Groups cannot recur out of emission order. A distinct component occurrence
under the same shared join key is ambiguous, even if its values are identical.

Exactly one reference row and one Rust component must exist for each shared
key. Multiple RPCs or replacing array/update/component properties can violate
that condition. They are reported as ambiguous or extra/missing, never resolved
by picking the nearest value or highest timestamp. Later empty replacements
cannot justify choosing an earlier decoded prefix as a final state. Such
reference emission contracts need additional evidence before support expands.

## What passes

The reference emits one latest move per eligible component; Rust emits every
decoded move. For every uniquely matched update, the tool requires:

- Reference `move_index` equals the final observed Rust move index, and its
  represented move count equals the complete contiguous Rust component count.
- Character GUID, movement timestamp, movement state and move type match
  exactly. Reference mode flags must equal its movement state and its error
  sentinel must be explicitly false.
- All three position axes, both angles and all three velocity axes match
  **bit-for-bit after rounding the reference value to IEEE binary32**. This
  includes the sign of zero. NaN, infinity, booleans and out-of-range values
  cannot pass numeric validation.
- All reference updates are matched, no Rust component is left over, no
  shared identity is duplicated, and the sum of represented moves equals the
  total Rust movement row count. At least one update must be compared.

The report separately retains the maximum absolute difference between the
original reference numeric value and the stored Rust f32 value. A nonzero
difference can therefore accompany successful quantized parity; this is not
a claim of original f64 equality. No arbitrary epsilon is used.

The terminal Reader summary must be present, final, and consistent with the
observed movement count. Movement RPC failures, lost content blocks, rejected
partials and unfinished partials must all be zero, and the structural block
verdict must be `Passed`. A successful summary is necessary, but its absence
must never be inferred as zero errors.

## Scope and resource bounds

The supported comparison contract is deliberately narrower than the parser:
successful variant-1 movement components with at most 16 moves, matching the
pinned reference's emission capacity. A variant-0/null-velocity row, unsupported
sentinel, larger component or malformed input produces an explicit error. This
limit does not restrict the Rust parser. Checkpoints remain outside the Reader
stream and their skipped count is reported.

Earlier Rust moves are fully counted and their source/ordinal sequence is
validated, but the reference does not emit their numeric values. Only the
latest movement values of each component are compared. Fields the Rust movement
record does not expose, broader gameplay semantics and whole-file preservation
are outside this gate. Neither parser is assumed to be ground truth.

Inputs are read line-by-line, including compressed inputs. Defaults are
10,000,000 movement rows per input, 1 MiB per JSONL line and a 2 GiB SQLite
database limit. Indexed temporary storage uses a 16 MiB SQLite cache; these
settings do not bound total process RSS or every possible filesystem allocation.
Use `--max-rows`, `--max-line-bytes`, `--max-db-bytes` and `--work-dir` to choose
limits and a private working location. Database exhaustion fails closed. The
connection closes before temporary cleanup, including on Windows.

Input digests cover the complete decoded JSONL byte streams, including line
endings. For gzip input this is the logical uncompressed content, not the gzip
container. File identity, size and modification time are checked across each
read; retain the original replay/parser hashes separately when making a
reproducible acceptance claim.

## Executed public-input verification

The source example at `44fcb31d36eb5bb2638ad630e44954d16c024235` generated
the companion streams below. Its executable SHA-256 was
`39ab667a79fda997eeaeb4df4e67e618744f5c3099b6e32cb3416d355f1b3d61`.
The complete pinned reference exports were reused without another reference
decode. The strict comparator and a separate independent audit both retain
their own source hashes and complete input digests.

| Public build | Unique latest updates compared | Rust individual moves | Missing / extra / duplicate keys | Latest-value mismatches |
| --- | ---: | ---: | --- | ---: |
| 12.09 | 2,616,931 | 2,634,029 | 0 / 0 / 0 | 0 |
| 12.10 | 8,853 | 8,907 | 0 / 0 / 0 | 0 |
| 12.11 | 2,949 | 3,001 | 0 / 0 / 0 | 0 |
| 13.00 | 4,946 | 4,991 | 0 / 0 / 0 | 0 |

The 12.09 input is the
[public full match at a pinned revision](https://media.githubusercontent.com/media/Matthias1590/unsupported-replays/c5ba8419b96afa8e88e7afb3e594cf4cce9c75fe/replays/12.09/sample-1.replay),
61,015,648 bytes, SHA-256
`925fe2d1dfce6dff071df724bff03faa7220ffd209bdf795b76888b65157efff`.
The original bytes were unchanged after the new Reader traversal. The three
small inputs are the same pinned public fixtures required by normal CI.

On the full match, actual reassembly provenance identifies **6,330 fragmented
move rows**. The original strict 50,000-row sample remains an immutable
measurement; the new format resolves the origin convention without applying
a fixed offset. The full maximum original position delta is
`0.0018750000017462298`, velocity delta `0.00009765625009094947`, and both angle
deltas are zero. All eight fields match after f32 quantization.

An independent implementation loaded both complete exports into bounded NumPy
arrays, sorted only the five independent wire keys, checked uniqueness and
complete key equality, and compared every latest scalar and f32 bit pattern.
It imports neither this comparator nor SQLite, and uses Python's C float array
for its separate f32 conversion path. Both complete sorted record arrays have
SHA-256 `b3b7ff2709a935e0eea89e0c82886d3cb558d1d1164d4ae19ed49ef01f98a476`.
It reproduces all counts, logical digests and numerical deltas.

The independent pass used 413,864 KiB peak RSS; this is the QA helper's resource
use, not a replay parser benchmark. Earlier moves' values are still outside
reference coverage. The successful whole-update comparison does not expand the
variant, checkpoint or absent-field scope stated above.
