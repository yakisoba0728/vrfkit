# Changelog

## Unreleased

### Breaking changes

Rust API: `vrf_net::stats::NetStats::rpc_payloads_lost` returns
`Option<u64>`, `None` when more unresolved RPC payloads were preserved than
RPC streams failed. It used to saturate to 0 there. `validate` and the export
summary print that case as `?` (`RPC payload lost:`, `Content failures:` and
`Checkpoint loss:`), and `lost_content_blocks` (manifest
`content_blocks_lost`) adds the excess as loss instead of netting it to 0.
Output on a replay whose counts are consistent is unchanged.

## 0.7.0

0.7.0 is a minor bump for one source-breaking Rust API change. `vrfkit export`
writes the same Parquet tables, manifest and summary counters as 0.6.0.

### Breaking changes

Rust API (source-breaking for downstream exhaustive matches; CLI arguments
and output schemas are unchanged):

- `vrf_bitio::BitError` and `vrf_net::stats::SkipReason` are
  `#[non_exhaustive]`; a `match` outside their crate needs a wildcard arm.
  Inside the workspace that arm is loud, not silent: a `BitError` with no
  case in `DecodeErrorKind::from_bit_error` is counted as the new
  `DecodeErrorKind::Unclassified`, and a `SkipReason` the `validate` skip
  breakdown does not know is tallied on a new `Unclassified:` row, printed
  with zeros like the others. The decode error report's kind column widens
  to fit `Unclassified`.

### Fixed

- `-V` and `--version` are also recognised after a subcommand, wherever
  `-h`/`--help` already were (`vrfkit diag replay.vrf -V`). A valued
  option's value is still a value: `--out -V` names a directory, and a
  replay literally named `-V` is reached as `./-V`.
- The corrupt-archive tests assert the property that matters, a clean
  `ContainerError::OodleDecompression` and a CLI exit of 1 without a panic
  report, rather than the `decoder panicked: ` wording, so an oozextract
  release that returns `Err` instead of panicking on the same stream does not
  fail them. The panic boundary is tested on its own against a closure that
  always panics.
- The committed 13.01 export and checkpoint baselines
  (`tools/baselines/*_02d4d478.json`) were remeasured with the 0.6.0 decoder,
  so `check_export_baseline.py` compares current output again. Every Parquet
  file and every carried-over counter is unchanged; the tail/trailer keys are
  replaced by the native component counters, and the FocusProjectiles counters
  are pinned as measured zeros. The schema validator no longer accepts the
  archived key set.
- `check_export_baseline.py` and `check_baseline_schemas.py` also refuse a
  baseline whose checkpoint counters alone carry obsolete tail/trailer keys.

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
  `FocusProjectiles refused: N parent updates with a mismatched declaration`
  (plus `Checkpoint` twins) are printed.
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
- `ContainerError`, `FrameError` and `SchemaError` are `#[non_exhaustive]`;
  exhaustive matches need a wildcard. `EventChunk`, `ReplayInfo`,
  `ReplayHeader`, `ReplayVersion`, `OverlayErrorRow`, `DecodeErrorKind`,
  `DiagnosticEvent`, `BunchFlagSnapshot` and `ContentBlockHeaderSnapshot` are
  `#[non_exhaustive]` read-only results; build an `EventChunk` with
  `vrf_container::parse_event_chunk`.
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
- `vrfkit --help`/`-h` (also after any subcommand) and `vrfkit --version`/`-V`,
  both exit 0. `vrfkit <subcommand> --help` used to read `--help` as the
  replay path.
- I/O errors name the path: `I/O error: <path>: <OS message>` for the input
  replay (CLI and `ReplayReader::read_path*`), the `diag --json` output and
  export staging/table files, and `<path>: is a directory, not a replay file`
  for a directory given as the replay. For `ReplayReader::read_path*` the
  `io::ErrorKind` of `NotFound` and `PermissionDenied` is kept, but the OS
  code is on the error's `source()` (the outer `raw_os_error()` is `None`),
  and a directory reports `IsADirectory` (on Windows it was
  `PermissionDenied`).
- `validate` prints `=== Stream failures (0 shown) ===` and
  `=== Diagnostic Events (0 total) ===` when those sections are empty, so
  `--diagnostics` with nothing retained is visible.

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
- Corrupt compressed data is reported as an error instead of a panic. The
  Oodle decoder (oozextract) panics on some malformed streams; a single
  flipped byte in a compressed chunk made `validate`, `diag`, `export` and
  the Reader exit 101. The panic is now caught and returned as
  `ContainerError::OodleDecompression("decoder panicked: ...")`
  (`ReadError::Replay`, wrapped in `ReplayError::Checkpoint` inside a
  snapshot); the CLI exits 1 without a panic report.
- `export --out .` (or `..`, or a root) is refused with the actual rule: the
  destination must end in a directory name, because the export is staged in a
  sibling named after it. It was described as "not a filesystem root".

### Tools and CI

- `compare_movement_provenance.py` exits 2 with an `error` report for a
  truncated or corrupt gzip input, and replaces any previous report with an
  `error` report before reading input, so a failed run never leaves an
  earlier `pass` behind.
- `validate_type_evidence.py` reports carry `report` and `status`
  (`pass`/`fail`). `--output` holds `incomplete` or `error` after a run that
  did not finish, and an existing `--output` that is not one of its reports
  is refused (exit 2) instead of replaced, as is the specification, the tool
  itself, a `.parquet` file or a `manifest.json`. A missing export root or an
  unreadable or malformed specification is one `error:` line and exit 2
  instead of a traceback, and still leaves an `error` report at `--output`.
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
