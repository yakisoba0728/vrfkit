# Final consolidation migration

This continuation updates the unpublished 0.6.0 integration candidate. Pin the
source revision reviewed in [PR #58](https://github.com/yakisoba0728/vrfkit/pull/58)
and read the [final acceptance receipt](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6932599335)
for the actual tested checkout and package identities. The earlier
[Phase C API/counter migration](PHASE_C_MIGRATION.md) still applies.

## Direct network callers

Two public enums add `PayloadOverrun`:

- `vrf_net::error::PartialSequenceKind`
- `vrf_net::pipeline::PartialPayloadReason`

Update downstream exhaustive matches. Existing variants and payloads remain
available. A direct `PartialBunchAccumulator::add_fragment` call whose byte
slice cannot hold its declared bit count now returns an explicit refusal
before sequence admission. The result reports `has_partial_error`, no completed
payload, and the retired prior assembly in `displaced`. Its `discarded_bits`
counts supplied bytes and prior buffered bits, never nonexistent claimed bytes.
Callers can inspect and preserve the displaced payload before continuing with
a fresh assembly.

The normal replication pipeline supplies complete declared byte windows. No
manifest counter is added. The CLI raw-partial reason mapping includes
`payload_overrun`; consumers that enumerate reason strings should accept it.
Direct packet-reader length arithmetic now uses checked wide operations;
ordinary frame packet limits and sentinel interpretation are unchanged.

## Checkpoint admission

Each checkpoint archive has a fixed 268,435,456-byte budget for cumulative
resolved UTF-8 GUID paths. Every declaration is charged, including repeated
GUIDs, literals, indices and legacy decimal paths. At the boundary the archive
is accepted; an excess declaration returns the existing
`SchemaError::ResourceLimit` with resource
`checkpoint resolved GUID path bytes` before that entry's indexed-path clone,
observer call or cache insertion.

This budget resets for each archive and is separate from the configurable
group/field-slot `SchemaLimits`. It does not establish a total RSS limit or a
live-stream GUID cap. Earlier observer callbacks are provisional when a later
entry fails. Do not commit a checkpoint as complete until the read succeeds.
See [schema allocation limits](SCHEMA_LIMITS.md).

## CLI publication

`export` refuses an output directory containing the source replay, even when
the replay is named like an export-owned table such as `fields.parquet`.
Canonical input and directory paths catch aliases; choose an output directory
that does not contain the replay. The existing foreign-entry refusal
retains priority, and rejection happens before staging or publication.

For `diag --json`, replacement uses the filesystem rename operation without
first deleting the prior report on Windows. A failed publication leaves the
old report in place. Crash durability and hostile concurrent filesystem
mutation are outside this in-process publication contract.

## Movement attribution and output

A malformed repeated shooter GUID invalidates the current identity until a
later valid GUID arrives. It no longer associates following components with a
stale shooter. Previously decoded rows survive, later valid identities recover,
and the original RPC remains available as raw data. No new wire type, field
meaning, schema column or required counter is inferred by this correction.

Ordinary replay equality, checkpoint separation, all nine native counters,
both FocusProjectiles counters, typed/raw agreement and CLI failure behavior
are verified by the independent tests linked from
[the consolidation status](FINAL_CONSOLIDATION_STATUS.md). That receipt states
the exact public input coverage and private-corpus limitation.
