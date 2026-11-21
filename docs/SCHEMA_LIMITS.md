# Schema allocation limits

Schema declarations can reserve field slots without transmitting field values.
Before aggregate limits were applied, a 215-byte synthetic live export table
declaring 16 empty groups with 65,536 slots each was accepted and reserved
33,554,432 bytes of field-slot storage on x86_64. Repeating these declarations
with distinct identities could keep growing the cache. A per-group count limit
alone did not bound that amplification.

`NetGuidCache` now applies these default resource limits to both live schema
exports and checkpoint schema tables:

| Resource | Default maximum |
| --- | ---: |
| Registered groups per cache | 4,096 |
| Declared field slots across those groups, including holes | 1,048,576 |

These are configurable resource policies, not assertions about the game's wire
format. The existing per-group wire checks remain separate. One million slots
represent 32 MiB of logical field-slot storage on the measured x86_64 target;
this is **not a total memory limit**. Strings, hash maps, temporary incoming
groups, allocator overhead and spare vector capacity need additional memory.
GUID mappings and other parser components have their own resource behavior.

## Admission and error behavior

Wire readers call `declare_export_group`, which checks aggregate usage before
allocating the field-slot vector. The existing `add_export_group` API enforces
the same admission policy, but its caller has already allocated the supplied
group. Prefer the declaration API for untrusted declared counts.

An exact re-declaration costs no additional retained slots. Growing a group
charges only the growth; declaring fewer slots keeps the old slots. Replacing
an index with a new group releases the old group's charge. Alias and index
changes retain their existing merge behavior. `clear` resets usage and keeps
the configured limits. Mutable access to a group's public fields invalidates
the cached count, so the next admission recomputes it; wire field updates do
not take that path.

A refusal returns `SchemaError::ResourceLimit` through the existing frame or
checkpoint error path. It never reports a successful parse with a silently
dropped declaration. Previously accepted groups remain intact. Checkpoint group
observers still see a declaration before cache storage, including one whose
admission is refused; no field rows for that refused group are decoded.
The CLI follows its existing export-failure behavior. Original replay files
remain read-only. Valid exports within the limits retain their schemas and
raw data preservation behavior.

## Library configuration

Applications with trusted inputs can choose explicit budgets:

```rust
use vrf_schema::{NetGuidCache, SchemaLimits};

fn main() -> Result<(), vrf_schema::SchemaError> {
    let mut cache = NetGuidCache::with_limits(
        SchemaLimits::default()
            .with_max_groups(8_192)
            .with_max_field_slots(2_097_152),
    );
    cache.declare_export_group("Example".into(), 1, 16)?;
    Ok(())
}
```

Zero is permitted: a zero group budget rejects every group, and a zero slot
budget permits only empty groups subject to the group limit. Raising a budget
raises the allocation an input can request. Direct mutation through the public
mutable group accessor is a trusted caller operation; admission limits cannot
prevent that caller from allocating outside the declaration APIs.

`SchemaError` is `#[non_exhaustive]`: downstream matches need a wildcard arm
beside the new `ResourceLimit` variant. No exported column or valid-value decoding changes.

The high-level `vrfkit::ReplayReader` accepts the same `SchemaLimits` through
`read_with_schema_limits(data, limits, callback)` and
`read_path_with_schema_limits(path, limits, callback)`. Limits apply to the
entire main stream, including declarations in later ReplayData chunks, and
reset on the next call. Existing `read` and `read_path` calls use defaults.
See the [Reader configuration example](RUST_API.md#resource-limits).

`FrameError::Schema` now retains `SchemaError` as its typed source instead of
converting it to a `String`. Existing displayed messages are unchanged, and
`ReadError::Replay` exposes the cause through the standard error source chain.
Downstream code constructing or destructuring `FrameError::Schema(String)`
must handle the typed payload; use `to_string()` for logging. Checkpoint error
types already retain the schema source. The high-level Reader skips
checkpoints by default; `ReadOptions` opts into separate snapshot callbacks.
Its path methods buffer container chunks rather than the complete input file.

## Checkpoint GUID path expansion

Each checkpoint permits at most **256 MiB of cumulative resolved UTF-8 GUID
path bytes**. A short indexed reference can name a long literal, so the
encoded archive size and GUID count alone do not bound expanded strings.
The parser charges every declaration, including literal, indexed, legacy
decimal and repeated-GUID entries, before cloning an indexed path, invoking
that entry's observer or inserting it into the cache. The exact boundary is
accepted; the next excess byte returns `SchemaError::ResourceLimit` for
`checkpoint resolved GUID path bytes`.

Earlier observer calls remain provisional. A refused GUID entry is not
delivered or stored. Previously delivered declarations remain available;
truncated flags and invalid path indices retain their original error priority.
The budget starts afresh for each checkpoint archive. It is a separate fixed
checkpoint policy, not a field of the two-member `SchemaLimits` configuration
and not a total process-RSS cap or a bound on live-stream GUID paths.

The limit covers both cache strings and later owned summary-row expansion.
Merely sharing cached strings would move the same amplification into those
detached rows. This change rejects pathological expanded inputs explicitly;
it does not silently truncate names or discard raw indexed identities.

## Observed public-fixture margin

The following figures were measured from checkpoint declaration Parquet files
produced by the baseline parser at
`67c0a2d94fbcbeeefa56761f1bc13d9f6c829fb0`. They cover one public fixture per
listed build, 76 checkpoints in total, not the full private archive.

| Build | Checkpoints | Largest group count | Largest slot total | Largest single group |
| --- | ---: | ---: | ---: | ---: |
| 12.05 | 19 | 505 | 10,031 | 539 |
| 12.08 | 17 | 500 | 9,780 | 542 |
| 12.09 | 21 | 542 | 10,851 | 555 |
| 12.10 | 7 | 176 | 3,867 | 604 |
| 12.11 | 6 | 183 | 4,088 | 636 |
| 13.00 | 6 | 177 | 3,882 | 650 |

The method groups `checkpoint_export_groups.parquet` by `checkpoint_index`,
counts rows, and sums `declared_slots` within each checkpoint. The largest
observed checkpoint uses less than 14% of the group budget and less than 1.1%
of the slot budget. These observations do not establish
the maxima for untested builds or transient live-stream states.

Regression tests use small explicit budgets for growth, replacement, aliases,
cache resets, mutation, truncation precedence and checkpoint observations.
The default group-limit regression uses empty groups, keeping test allocations
small even when the limit is deliberately disabled during verification.
