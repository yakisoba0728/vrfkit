# Movement decode provenance

`ReplayReader` can return each movement sample together with the wire and
transport context that produced it. The companion rows support precise
cross-parser comparisons and allow callers to retain all decoded moves while
also grouping them by their actual component stream.

The existing `MovementRecord` values and the default `movement.parquet`
schema remain unchanged. `packet_id` continues to identify the packet that
completed a bunch. Provenance is a separate borrowed slice in a Reader record
batch; pair the slices by row position, not by timestamp. Both slices have the
same length and are valid only during the callback.

```rust
use std::convert::Infallible;
use vrfkit::{ReplayEvent, ReplayReader};

ReplayReader::new().read_path("match.vrf", |event| {
    if let ReplayEvent::Records(batch) = event {
        for (movement, provenance) in batch.movement.iter()
            .zip(batch.movement_provenance)
        {
            if let Some(source) = provenance.source {
                println!("{} -> {}, update {}, move {}: {}",
                    source.initial_packet_id,
                    source.completion_packet_id,
                    provenance.update_index,
                    provenance.move_index,
                    movement.timestamp);
            }
        }
    }
    Ok::<(), Infallible>(())
})?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

This example prints replay-derived identifiers; keep such output private when
working with a private replay. The API does not publish anything automatically.

## Exact identity fields

| Field | Meaning |
| --- | --- |
| `initial_packet_id` | Packet ID from the authoritative reassembler's stored initial header; equals the current packet for an ordinary bunch |
| `completion_packet_id` | Packet that made the complete bunch available; equals the paired movement row's `packet_id` |
| `channel_index` | Channel containing the RPC |
| `actor_net_guid` | Actor identity of the content block |
| `object_net_guid` | Subobject identity; `None` means an actor block and `Some(0)` remains a distinct subobject value |
| `bunch_index` | Zero-based global ordinal of the completing bunch, including partial fragments in the count |
| `block_index` | Zero-based content-block ordinal within the whole, possibly reassembled bunch |
| `rpc_index` | Zero-based RPC callback ordinal in the completion packet, including other RPC names and channels |
| `rpc_handle` | Decoded RPC handle in that block's function table |
| `array_ordinal` | Zero-based occurrence of the RemoteCharacterUpdates array property in this RPC |
| `update_ordinal` | Zero-based arrival ordinal within that array |
| `update_index` | Sparse wire index, decoded as `encodedIndex - 1`; it need not equal the arrival ordinal |
| `component_ordinal` | Zero-based occurrence of ComponentDataStream in that update |
| `move_index` | Zero-based decoded move ordinal inside that component; the 3-bit marker wrapping after seven does not reset it |

The transport fields form `MovementSource`, held in
`MovementProvenance.source`. An absent source is explicit: low-level consumers
can decode an RPC without first framing a network block. The shared Reader
pipeline supplies the actual source for its movement rows. No packet-distance
heuristic or movement value is used to infer it. Packet and bunch ordinals are
local to one replication pass; checkpoint snapshots are separate streams.

Occurrence ordinals count encountered properties even when they emit no move.
A repeated sparse update index therefore retains distinct arrival ordinals.
Repeated arrays and repeated component properties are also kept separate.

## Lower-level consumers

`vrf_movement::decode_movement_rpc_with_context` calls
`FnMut(MovementMove, MovementContext)` for every successfully decoded move.
`MovementContext` supplies the five array/update/component/move fields above.
The existing `decode_movement_rpc` callback API delegates to the same decoder
and preserves its values, order and counters.

`vrf_net::pipeline::ReplicationSink::on_content_block_with_context` supplies
`ContentBlockContext` before a block's RPC callbacks. Its default delegates to
the original `on_content_block`, so existing sink implementations need no new
method. Overriding the new method allows consumers to retain the actual
initial/completion packet identities and bunch/block ordinals.

The internal sink collects companion records only when requested by a Reader.
The CLI's validate/export paths leave collection disabled, so they do not
allocate or retain a provenance vector per movement batch. Neither default
Parquet output nor JSONL converted from those tables gains these extra fields.

## Comparison limits

Provenance identifies emitted moves; it does not change the emission contract.
vrfkit emits each successfully decoded move. A reference implementation can
emit only the latest move of an update, replace repeated sparse array entries,
or replace an earlier component with a later empty component. Callers must
not assume that selecting the highest ordinal among emitted rows implements
those replacement rules: an empty or failed replacement may emit no row.

For strict comparisons, retain each actual array/update/component occurrence,
reject ambiguous cross-parser identities, and check all expected rows. Match
using wire identity before comparing character identity, movement timestamps,
position, velocity or rotation. A universal packet offset is not justified;
initial and completion packet IDs can have arbitrary distances.

A component can emit a valid prefix and fail on a later move. Its prefix keeps
its context, the existing error counters still increase, and the whole failed
RPC payload remains preserved as raw data. Callbacks remain provisional until
the Reader succeeds; successful traversal alone does not establish complete
semantic coverage. These fields do not add a reconstruction guarantee for
unknown movement flags, section tails or envelope trailers.

## Versioned diagnostic JSONL

The source example writes movement values and their companion context together:

```sh
cargo +1.86.0 run --release -p vrfkit --example movement_provenance --locked -- match.vrf > movement-provenance.jsonl
```

Keep this output local: it contains character and actor identities. The file
starts with a `movement_provenance_header` record declaring schema version 1
and `float_precision: "f32"`. Each `move` record contains the source fields,
occurrence ordinals, decoded character/timestamp/state/type, and eight numeric
position/angle/velocity fields. An actor block uses explicit JSON null for its
object GUID; a subobject GUID of zero remains the integer zero.

The original f32 values are expanded exactly to f64 before JSON formatting.
This allows readers to recover their stored value, including signed zero,
without assuming that a short decimal f32 rendering is an exact f64 value.
Non-finite values, absent transport context or mismatched companion lengths
fail the example. No movement values are used to infer an identity.

A terminal `movement_provenance_summary` is written only after Reader traversal
succeeds. It includes movement count, movement/RPC loss/partial counters, block
verdict and skipped checkpoints. Earlier rows remain provisional: a truncated
file without that summary is incomplete. A complete traversal can still report
quality failures; consumers must inspect the counters and verdict. This example
does not publish files atomically and does not decode checkpoint snapshots.

## Compatibility and verification

This is additive to the existing low-level callbacks and `MovementRecord`.
Consumers constructing movement records or reading the default Parquet schema
do not need to change. Reader batch fields are non-exhaustive; consumers can
opt into the companion slice and copy only the identities they need.

Focused regression tests cover repeated array properties, repeated sparse
indices, empty/orphaned/failed components, marker wrap through 20 moves,
failed prefixes, two movement RPCs in one packet, actor versus subobject-zero
identity, enabled/disabled collection parity, buffer reuse and packet-local
RPC ordinals while the decode pass carries cumulative statistics. A network test
reassembles three fragments with packet IDs 7, 19 and 41, then processes an
ordinary packet 43, establishing the actual initial/completion identities and
ensuring the previous fragment origin cannot leak into the next block.

```sh
cargo +1.86.0 test -p vrf-movement --locked
cargo +1.86.0 test -p vrf-net --locked
cargo +1.86.0 test -p vrfkit --no-default-features --locked
```

Real-replay and independent comparison results belong to the associated PR's
pinned validation receipt. The synthetic tests do not establish support for
previously unobserved game builds or reference emission contracts.

The [core validation receipt](evidence/movement-provenance-core.json) records
the initial implementation checkpoint. Independent Reader integration testing
subsequently found a packet-local RPC ordinal error caused by reusing a
cumulative counter. The [correction receipt](evidence/movement-provenance-rpc-index-fix.json)
preserves the failing reproduction, corrected source and new replay audit.
The initial measurements remain intact; they do not validate the later
corrected binary or the separate Reader hookup by themselves.
