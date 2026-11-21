# Performance notes

The measurements behind the code comments that point here: each section names
the code it explains, the rule, its key number and how it was taken. They are
measurements on one build and one reference replay (02d4d478 unless named),
not guarantees, and nothing re-verifies them.

## Bit reader (vrf-bitio)

### Cold-path builders stay free functions

Code: `BitReader::need` (its `Eof` error build) and `load_u64_padded`, the tail
path of `BitReader::load_u64`, in `crates/vrf-bitio/src/lib.rs`.

A `#[cold]` out-of-line `Eof` builder taking `&self` made the reader
address-taken, so the optimiser kept it in memory across every read: ~2% on the
reference replay. Passing the fields by value only got back to parity, and the
optimiser already sinks the inline build into the unlikely branch.
`load_u64_padded` takes the slice, not `&BitReader`, for the same reason (the
two forms measured within noise; it cannot provoke the problem).

### load_u64: avoiding a memcpy

Code: `BitReader::load_u64` in `crates/vrf-bitio/src/lib.rs`, which loads 64
bits from absolute byte `byte`, zero-padding past the end.

The fast path is a fixed-size chunk, so the compiler sees one unaligned 8-byte
load. A runtime-length copy into a stack buffer compiled to a `callq memcpy`
plus zeroing and spilling on every bit read, which dominated the reader's
cost. Only the final seven bytes of `data` need padding, so that case is out of
line.

### read_int_packed: peeling the overflow check

Code: `BitReader::read_int_packed` in `crates/vrf-bitio/src/lib.rs`.

Only the fifth chunk, at shift 28, can overrun a `u32`, and `16u32 << 28` is
zero: folded in unchecked it returns `Ok(0)`, which ends the property loop
above. The check is peeled out of the loop, so the one-to-four byte path
(every value below 2^28) runs exactly the instructions it did before.

### copy_bits_to: the byte-aligned path

Code: `BitReader::copy_bits_to` in `crates/vrf-bitio/src/lib.rs`.

A byte-aligned `copy_from_slice` fast path measured neutral (1.224 s / 1.211 s
against 1.225 s / 1.211 s, two interleaved best-of-7 runs): payloads sit behind
variable-bit headers, so alignment is the exception, and one loop is kept.

### copy_bits_to: the tail write stays a memcpy

Code: the tail write in `BitReader::copy_bits_to`.

It lowers to a `callq memcpy` of 1..=8 bytes that nearly every call reaches. A
shift-and-peel loop removes the call (checked in the asm) but measured neutral
on `validate` and `export`, and a plain zip is turned back into the same memcpy
by LLVM's loop-idiom pass.

### BitError stays 32 bytes

A throwaway build whose error was a 1-byte enum, the best case possible,
bounded the gain at export -3.0% / validate -1.8%. A real shrink boxes the
payload (8 bytes, worth less) and costs either `alloc` in `vrf-bitio`'s
`no_std` build or the diagnostic fields that decide which blocks are recorded
as malformed.

## Frame walking (vrf-frame)

### Measured shape on real replays

Code: `crates/vrf-frame/src/sections.rs` and the header-flag table in
`crates/vrf-frame/src/lib.rs`.

`vrfkit diag` over 45 replays of 24 builds (two per build directory, plus the
12.10, 12.11 and 13.00 fixtures) walked 10,614,694 ReplayData and 860
checkpoint frames with **0** ExternalData blobs or bytes and **0**
GameSpecificFrameData bytes, and `inspect` shows header flags `0x0002`
(`HasStreamingFixes` only) on all 45. Both loops are frame overhead: the
reference replay's 226,190 frames carry 29 level names in all, so the name is
still read and validated rather than blind-skipped. `FrameSkips` moves if a
build starts sending either section; `crates/vrfkit/tests/frame_skips.rs`, not
the corpus guards, keeps every pass's tally wired.

## Schema hot path (vrf-schema)

### FxHash over SipHash

Code: `crates/vrf-schema/src/hash.rs`, the hasher `NetGuidCache`'s internal
maps use.

Those maps are keyed by replay data and probed on the hottest path: a group is
resolved for each of the reference replay's 608,080 content blocks, each
costing several `get_group_by_path` probes plus a `get_path_by_guid` per GUID
touched. SipHash-1-3's keying, setup and finalisation cost far more than a
`u32` probe; this hasher (rustc's `FxHasher`, reproduced rather than depended
on) is one rotate, one XOR and one multiply. vrfkit's sink maps and vrf-net's
per-bunch maps (`ChannelTable`, `partial_bunches`, `in_reliable_sequence`,
`fragments`, bounded by `MAX_ACTIVE_CHANNELS`) make the same trade: on vrf-net
it measured wall -0.3% to -2.7% over three replays, 11 interleaved rounds.

**What is given up.** It is not HashDoS-resistant: a crafted replay could make
probes collide. The maps are bounded by the replay's own range-checked counts
(`MAX_GUID_ENTRIES`, `MAX_GROUPS`, `MAX_ACTIVE_CHANNELS`) and the input is a
local file the operator chose. **If this ever moves behind a network boundary,
revert the vrf-schema, vrf-net and vrfkit maps to `std`'s default hasher.**

### Path alias enumeration

Code: `for_each_replay_path_key` in `crates/vrf-schema/src/path.rs`.

The sink calls it once per content block (608,080 times per replay). A visitor
handing out `&str` makes the common no-alias case allocation-free; returning a
`Vec<String>` cost two heap allocations per call. An alias still costs one
`String`, because it is new text.

### Registering a new export group

Code: `NetGuidCache::index_group` in `crates/vrf-schema/src/cache.rs`.

A new group registers only itself; a changed identity or a reused index still
rebuilds. Merging fields under the same canonical path and index leaves those
lookup keys intact.
Rebuilding every group on every add cost a checkpoint of n groups n(n+1)/2
registrations: 3.6-4.5 million per export where 14-17 thousand are needed.
`export --checkpoints` on three 84-89 MB 13.05/13.06 replays, 7 alternating
A/B pairs each: 0.76-1.09 s faster (paired medians), Parquet byte-identical,
peak commit unchanged.

### Reusing an unchanged export-group declaration

`declare_export_group` first checks identity and the aggregate resource budget.
An unchanged canonical path and index can then retain its populated slots and
grow only when needed, without constructing a temporary empty field vector.
`add_export_group` still merges supplied descriptors, but skips rebuilding
indexes when the identity is unchanged. Both paths preserve the generation
bump and resource accounting.

The 2026-11-16 experiment measured a large
reduction in work for repeated declarations. Its six public fixtures contained
**zero repeated declarations** in their main streams and 76 checkpoint streams.
Full-export timings changed direction on a follow-up run, and an identical-binary
control also varied substantially. This change therefore has a measured benefit
for repeated-declaration API and wire workloads; **ordinary replay throughput
and peak-RSS benefits have not been established**. The linked report retains
every series, including the initial slower results.

## Replication pipeline (vrf-net)

### Measured rates, reference replay 02d4d478

Code: the module docs of `crates/vrf-net/src/pipeline/` (`mod.rs`,
`framing.rs`, `spawn.rs`) and of `crates/vrfkit/src/sink/mod.rs`.

530,401 packets and bunches, 608,080 content blocks, 2,028 actor opens and
232 distinct channel indices (`tools/baselines/export_02d4d478.json`
counters; the channel peak is `MAX_ACTIVE_CHANNELS`'s doc).

### Allocation strategy

Code: the module doc of `crates/vrf-net/src/pipeline/mod.rs`.

The reader allocates nothing per packet, bunch or content block; the channel
table grows once per distinct channel index (232 on the reference replay),
never per bunch.

### Packet processing is interleaved

Code: `ReplicationReader::process_packet` in
`crates/vrf-net/src/pipeline/mod.rs`.

Bunches are walked inside the packet reader's callback; destructuring `self`
hands it `&mut` to the fields it needs while `packet_reader` stays borrowed.
Parsing every header first and walking copies cost one zero-filled `Vec` per
bunch plus one per packet -- about 1.06 million allocate/free pairs and two
passes over ~108 MB of payload on the reference replay.

### Decode stays sequential within a replay

Only the payload transform is per-block pure (`(bits, seed)`), and it was 3.4%
of an instrumented export. Decode is not: channel opens register paths in the
`NetGuidCache`, `on_actor_open` writes the archetypes `on_content_block` reads,
and the resolved group sets `function_count`, hence the handle read width. A
block decoded against a stale cache misframes from its first field. Parallelise
across replays (`VRFKIT_JOBS`), not within one.

## Export sink (vrfkit)

### Name interning

Code: `crates/vrfkit/src/sink/intern.rs` and the name columns of `FieldRecord`
in `crates/vrf-export/src/record.rs`.

The reference replay emits 1,296,660 field rows but names only **475**
distinct group paths and 4,557 distinct field names. With `String` columns a
row cost up to three heap allocations; interning makes it a refcount and lets
buffered rows share one copy. The Parquet bytes do not change.

### Group-path resolution memo

Code: the memo in `crates/vrfkit/src/sink/paths.rs`.

Counted on 02d4d478 `validate` with stderr-only counters on
`BlockPathMemo::get`: 608,071 probes, 448,937 hits (73.8%), and 6,634
whole-memo clears -- the GUID stamp moved on 3,516, the resolution stamp on
3,097, the schema stamp on 155 -- with 6 entries held at the end. The table is
discarded about every 92 blocks, so it never grows although `actor_net_guid`
is part of the key; which stamp to narrow first is the open question.

### RPC parameter walking

Code: `crates/vrfkit/src/sink/rpc.rs`.

Walking ClassNetCache RPC payloads into one row per named parameter produces
580,027 of the reference replay's 1,296,660 field rows (rows under a
`_ClassNetCache` group with a function-qualified `Function.Param` name).

### What the whole sink costs

Code: the module doc of `crates/vrfkit/src/sink/mod.rs`.

`vrfkit validate` runs the whole sink and writes no file, so it times the sink
alone. Five interleaved runs on 02d4d478 against the binary before the memo
and the name pool: median 1.395 s -> 1.062 s, and peak working set 64.5 MB ->
65.0 MB (the two together hold under a megabyte).

### RecordBuffers are lent, not owned

Code: `RecordBuffers` in `crates/vrfkit/src/sink/mod.rs`.

The sink is rebuilt for each of a replay's ~530 k packets, so a `Vec` its
constructor allocated cost ~290 ms of a 1.79 s export, more than the whole
movement decoder. The buffers are empty at the end of every packet, so lending
them keeps one allocation for the run.

### Driver: prefetch, batch reuse and the checkpoint thread

Code: `crates/vrfkit/src/driver/`.

The driver decompresses the next ReplayData chunk while the current one is
walked, reuses the writer batches, and runs the checkpoint pass on its own
thread. Interleaved A/B on 02d4d478, 7 rounds, medians: export -184 ms,
`--checkpoints` -499 ms, `validate` -149 ms, at +7 to +16 MiB peak working set
(one chunk held ahead, plus the concurrent checkpoint pass).

## Columnar output (vrf-export)

### Sparse nullable columns vs Arrow Union

A `fields` row carries at most one typed value, so it has four nullable value
columns rather than an Arrow DenseUnion: a mostly-null column compresses to
nearly nothing (the validity bitmap is run-length encoded) while a Union stores
type-id and offset arrays; DuckDB, pandas and PyArrow all handle nullable
primitives, while Union support varies and can disable predicate pushdown; and
`WHERE value_i64 IS NOT NULL` needs no type dispatch.

### Dictionary encoding is chosen per column

Code: `Table::DICTIONARY_COLUMNS` and `TableWriter::writer_properties` in
`crates/vrf-export/src/writer.rs`; each table's list, with its listed and
unlisted ratio ranges, is in `crates/vrf-export/src/tables/`.

parquet-rs dictionary-encodes every non-boolean column unless told otherwise,
so the writer turns it off everywhere and back on per list. **The rule:** every
string column keeps its dictionary, as the docs promise; any other column gets
one only where it was smaller than PLAIN summed over 45 replays (two per build
folder 11.06-13.06 plus the three public fixtures, `--checkpoints`), measured
per column chunk from an all-dictionary and a no-dictionary export. The
roundtrip tests check each file's footer against its list and reject a list
entry naming no column or a BOOLEAN one. Per-column ratios are not kept; the
table comments give the ranges.

Over those 45 replays the 13 tables went from 2,402,583,395 to 1,649,194,482
bytes (-31.4%), and on 20 other replays from 1,086,341,147 to 750,058,550
(-31.0%), with values unchanged (row multisets in pyarrow and DuckDB, and
`Table.equals` with metadata on the reference replay). `checkpoint_fields`
also keeps dictionaries on `raw_bits` and `value_f64`: checkpoints restate the
same payloads (7,084 distinct in 343,683 rows on the reference replay). Wall
time did not move beyond noise; process CPU fell 15-23%.

Left alone:

- Twelve string columns measure larger as dictionaries (4,375,159 bytes over
  the 45 replays, 3,228,193 of it `checkpoint_guid_entries.literal_path` at
  1.19x); writing them PLAIN means narrowing the documented promise first.
- The three checkpoint declaration tables were measured while every 8,192-row
  batch closed its own row group (10 groups for the reference replay's 74,270
  GUID entries); one group is written now, so re-measure before relying on
  their lists. `partials` has no rows to measure.
- Only dictionary on or off was measured, not BYTE_STREAM_SPLIT for floats or
  DELTA_BINARY_PACKED for `time_ms`/`packet_id`.

### raw_bits: SmallVec, and the rejected arena

Code: `FieldRecord::raw_bits` in `crates/vrf-export/src/record.rs`.

Inlined as `SmallVec<[u8; 16]>`: most field payloads are <=16 bytes, so the
inline array spares a heap allocation on most of the reference export's rows,
and larger payloads spill transparently. Interning is the wrong shape for
payload bytes (the pool would approach one entry per row), and an arena would
have to travel with the rows to the writer thread; bounding the writer's
buffer (`writer::MAX_BUFFERED_ROWS`) had already cut the live payload vectors
from ~390,000 to ~90,000.
