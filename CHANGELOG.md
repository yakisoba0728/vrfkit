# Changelog

## 0.6.0

0.6.0 changes what `vrfkit export` writes to the manifest and summary, so it
is a minor version bump. It also turns the `vrfkit` crate into a library with
a public Rust reader API. Read the two migration guides before updating a
consumer or a strict output validator:

- [Output counters and the Reader API](docs/PHASE_C_MIGRATION.md)
- [Error, network and publication changes](docs/FINAL_CONSOLIDATION_MIGRATION.md)

The Windows CLI is still the only packaged distribution. Rust consumers pin a
source revision such as the `v0.6.0` tag; the Python tools are source-only.
Nothing is published to crates.io or PyPI. No speed, CPU, memory or
throughput improvement is claimed for this release.

### Breaking changes

Export output (manifest keys and summary lines):

- Native movement components: the eight movement tail/trailer counters are
  removed and nine `movement_component_*` counters are added, in the main
  and checkpoint `sink` blocks. The summary's tail/trailer lines are replaced
  by `Movement components:`, `Component envelopes:` and `Component headers:`
  (and their `Checkpoint` twins). Old values cannot be converted into the new
  counts. See
  [native movement components](docs/NATIVE_MOVEMENT_COMPONENTS.md#counter-and-source-api-migration).
- New required `sink` keys, main and checkpoint, present at zero:
  `route_children_focus_projectiles`, `focus_projectiles_empty_trailers` and
  `focus_projectiles_declaration_refusals`. The `Route children:` line gains
  a final `N focus projectiles` item, and new lines
  `FocusProjectiles trailers: N empty deltas` and
  `FocusProjectiles refused: N declaration mismatches` (plus `Checkpoint`
  twins) are printed.
- New manifest `net` key `partial_payload_overrun`; the `Partial causes:` and
  `Checkpoint causes:` lines gain an `N payload overrun` item after
  `unaligned`. The raw-partial reason strings include `payload_overrun`.
- A container or frame failure inside a checkpoint is reported as
  `checkpoint <id>: <error>` (`checkpoint #<index>: <error>` before the id is
  read). Main-stream failures keep their bare text.
- Strict baseline validators require the new keys; an archived baseline from
  an earlier executable fails the current harness.

Rust API:

- `NetStats`, `Preamble`, `CheckpointTables` and `CheckpointIdentity` are
  `#[non_exhaustive]`. Start `NetStats` from `default()` and assign fields;
  build `CheckpointIdentity` with `CheckpointIdentity::new`.
- `vrf_net::error::PartialSequenceKind` and
  `vrf_net::pipeline::PartialPayloadReason` add `PayloadOverrun`;
  `NetStats` adds `partial_payload_overrun`.
- `RpcDecodeResult` drops its eight tail/trailer fields for nine component
  fields, and `MovementError` adds six variants.
- The new `vrfkit` library types `ReplayError`, `ReadOptions`, `SchemaLimits`
  and `ValidationVerdict` are `#[non_exhaustive]`. Code written against a
  pre-release source revision of this API must match `ReplayError` with a
  wildcard, handle `ReplayError::UnsupportedBranch { branch, message }`
  (previously `Usage`, same text) and
  `ReplayError::Checkpoint { checkpoint_index, checkpoint_id, error }`, and
  build options with `default()` plus `with_checkpoints`,
  `with_schema_limits`, `with_max_groups` and `with_max_field_slots`
  instead of struct literals.

### Added

- `vrfkit` library: `ReplayReader` streams borrowed main-stream records and
  native Event payloads through callbacks without writing Parquet. With
  `default-features = false` it has no Arrow, Parquet or Zstd dependency. See
  [Rust API](docs/RUST_API.md).
- Optional checkpoint callbacks (`ReadOptions::default().with_checkpoints(true)`)
  with separate snapshot state and summaries; existing calls still skip
  snapshots.
- `ReplayReader::read_bounded` for finite nonseekable `Read` input with a
  required encoded-byte cap.
- Configurable schema budgets (`SchemaLimits`) that refuse excess group and
  field-slot declarations before allocating; see
  [schema limits](docs/SCHEMA_LIMITS.md).
- Optional gameplay projections (`native_event`, `round_phase_event`) and
  `MainIdentityTimeline` for body identity and movement association, with
  explicit null, unresolved and ambiguous outcomes; see
  [Gameplay API](docs/GAMEPLAY_API.md).
- Movement provenance through the Reader, and
  `tools/compare_movement_provenance.py` for comparing it with the reference
  parser; see [movement comparison](docs/MOVEMENT_COMPARISON.md).
- `validate` and `export` read replay chunks from the file instead of holding
  the whole replay; `export --checkpoints` uses its own checkpoint cursor. See
  [CLI input](docs/CLI_INPUT_STREAMING.md).
- The 12.05 `Clay_PC_C.FocusProjectiles` array route: exact child rows before
  the unchanged raw parent, on that build only; see
  [the route](docs/PHASE_C_FOCUS_PROJECTILES.md).
- `tools/export_jsonl.py`: selected Parquet columns to schema-described JSONL;
  see [JSONL](docs/JSONL.md).
- `tools/benchmark_resources.py`: paired export comparisons with CPU, RSS and
  output-equality gates; see [resource benchmarks](docs/RESOURCE_BENCHMARKS.md).

### Fixed

- `export` refuses an output directory that contains the input replay, also
  when the replay is named like an export table, before staging.
- A failed Windows replacement of the `diag --json` report keeps the previous
  report; the delete-before-rename step is gone.
- A malformed repeated shooter GUID clears the current movement identity;
  earlier valid rows and the complete raw RPC are kept.
- Resolved checkpoint GUID paths are bounded to 256 MiB per archive before
  indexed-string copying, observer delivery and cache insertion.
- Short direct partial-bunch buffers are refused explicitly, and packet bit
  lengths use checked wide arithmetic.
- The native movement decoder validates component envelopes and keeps
  unsupported component bodies as raw RPCs instead of reading them as
  tail/trailer bits.
- 12.05 `FocusProjectiles` parents refused for a parent-handle or
  member-declaration mismatch are counted
  (`focus_projectiles_declaration_refusals`) instead of leaving no trace.
- `MainIdentityTimeline` reports a body whose only declaration sits on an
  ended PlayerState lifetime as `IdentityUnresolved::Lifecycle`, not
  `NoReference`.
- `MovementProvenance::source` is `None` for a negative packet ID instead of
  a wrapped `u32` packet number.
- Partial payload overruns are classified (`partial_payload_overrun`) instead
  of counting as unclassified.

### Tools and CI

- `compare_movement_provenance.py` exits 2 with an `error` report for a
  truncated or corrupt gzip input, and replaces any previous report with an
  `error` report before reading input, so a failed run never leaves an
  earlier `pass` behind.
- `validate_type_evidence.py --output` removes the previous report before
  validating and publishes the new one atomically. It refuses an `--output`
  that is the specification, the tool itself, a `.parquet` file or a
  `manifest.json`.
- `package_release.py` refuses a tag that does not name the Cargo workspace
  version, and a new `version` command prints that version. Untagged CI runs
  package as `v<version>+ci.<commit>`.
- CI runs Linux MSRV tests and public-replay validation through a reusable
  `ci.yml`, which tagged releases also call. The branch-only workflows are
  removed.
- Tests that could not fail were made able to fail (reader assertions,
  movement-comparison gates, benchmark counter equality, movement accounting
  in the corpus guards, the JSONL batch-column check).

## 0.5.1

- VALORANT 13.10 replays are supported: its payload transform is
  registered, and the CombatReport regional arrays and damage life-change
  members follow the handles 13.10 declares.
- The structured-array routes admitted on 13.01--13.06 are admitted on 13.10
  too, so its ActiveBlinds, KillData assist and other route children are
  emitted, and the kill extractors accept the build.
- CombatReport `DamageIcon` references and the 13.10 `TemporaryDeath`
  life-change array are typed.
