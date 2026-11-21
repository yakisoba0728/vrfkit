# Changelog

## Unreleased: 0.6.0 candidate

The integration branch uses version 0.6.0 to prepare the breaking output and
source API migration. This is an engineering candidate, not a published
release. PR #58 remains draft, open and unmerged.

The final consolidation adds corrections
to the recovered Phase A-D candidate:

- Refuse export destinations containing the input replay before staging,
  including inputs whose names match generated output tables.
- Preserve the prior diagnostic JSON on a failed Windows replacement by
  removing the delete-before-rename step.
- Clear stale movement identity after a malformed repeated shooter GUID,
  retaining preceding valid rows and the complete raw RPC.
- Bound cumulative resolved checkpoint GUID paths to 256 MiB per archive
  before indexed-string copying, observer delivery and cache insertion.
- Reject short direct partial-bunch buffers explicitly and compute packet
  bit lengths with checked wide arithmetic.

The two public partial-error enums add `PayloadOverrun`; exhaustive matches
need a new case. See the [final migration](docs/FINAL_CONSOLIDATION_MIGRATION.md)
for error, checkpoint and publication behavior. No speculative wire-type
correction or new ordinary-replay performance claim is included.

- All ten production workspace crates and their lockfile entries use 0.6.0;
  the two unpublished development tools retain their independent metadata.
- The native movement migration replaces eight obsolete tail/trailer counters
  with nine component counters. Both main and checkpoint consumers must use
  the new names; historical values cannot be converted into the new counts.
- Reader and CLI file input, optional gameplay projections, dictionary-run
  construction and JSONL tooling from the earlier phases are included.
- The Phase C additions and migration below are included. Review
  [the API and counter migration](docs/PHASE_C_MIGRATION.md) before updating
  callers or strict export validators.
- Phase D measurements completed with strict output
  equality, but performance superiority remains inconclusive. The one bounded
  raw-byte reservation experiment was withdrawn; no Phase D runtime optimization
  is included.
- The Windows CLI remains the packaged distribution target. Rust consumers
  use a pinned source revision; Python tools remain source-only. No crates.io
  or PyPI package is prepared or published by this version change.

## Phase C engineering history (not separately released)

These entries describe integration-branch engineering. No tag,
release or registry publication is implied.

### Added

- Optional Reader checkpoint callbacks, separate snapshot state and summaries,
  and a shared Reader/CLI checkpoint decoder. Existing Reader calls continue
  to skip snapshots by default.
- Evidence-preserving main-stream body identity and movement association with
  explicit null, unresolved and ambiguous outcomes.
- Finite nonseekable `Read` input with a required encoded-byte cap through
  `ReplayReader::read_bounded`, plus aggregate-only examples.
- Exact 12.05 Clay FocusProjectiles child decoding with the raw parent retained.

### Compatibility and corrections

- The native movement-component counter migration and new required
  `route_children_focus_projectiles` / `focus_projectiles_empty_trailers`
  keys require updated downstream validators. The measured 12.05 fixture
  gains 53 main child rows. See [migration](docs/PHASE_C_MIGRATION.md).
- The new identity timeline rejects stale prior-lifetime declarations as
  current ambiguity candidates while retaining historical observations;
  same-packet ordering uncertainty remains explicit.
- Unmodified historical binaries are not valid baselines for the current
  strict output harness.

### Not measured or published

- No Phase C speed, CPU, peak-RSS, allocation or throughput improvement is
  claimed. Performance comparisons remain user-run.
- Main merges, version changes, tags, releases and registry uploads are
  excluded from this engineering phase.
