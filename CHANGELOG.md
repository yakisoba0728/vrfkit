# Changelog

## Unreleased: 0.6.0 candidate

The integration branch uses version 0.6.0 to prepare the breaking output and
source API migration. This is an engineering candidate, not a published
release. PR #58 remains draft, open and unmerged. The
[Phase D release checklist](docs/PHASE_D_RELEASE_CHECKLIST.md) records the
required final-source, package and approval gates.

- All ten production workspace crates and their lockfile entries use 0.6.0;
  the two unpublished development tools retain their independent metadata.
- The native movement migration replaces eight obsolete tail/trailer counters
  with nine component counters. Both main and checkpoint consumers must use
  the new names; historical values cannot be converted into the new counts.
- Reader and CLI file input, optional gameplay projections, dictionary-run
  construction and JSONL tooling from the earlier phases are included. Their
  scope and existing evidence remain in [Phase A](docs/PHASE_A_RESULTS.md) and
  [Phase B](docs/PHASE_B_RESULTS.md).
- The Phase C additions and migration below are included. Review
  [the API and counter migration](docs/PHASE_C_MIGRATION.md) before updating
  callers or strict export validators.
- Quantitative Phase D performance qualification is pending its own final
  receipt. The version change establishes no speed, CPU or RSS improvement.
- The Windows CLI remains the packaged distribution target. Rust consumers
  use a pinned source revision; Python tools remain source-only. No crates.io
  or PyPI package is prepared or published by this version change.

## Phase C engineering history (not separately released)

These entries describe integration-branch engineering. Final functional
acceptance is tracked in [Phase C results](docs/PHASE_C_RESULTS.md); no tag,
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
  strict output harness. The [user-run handoff](docs/PHASE_B_BENCHMARK_HANDOFF.md)
  supplies an explicit output-compatible comparison source.

### Not measured or published

- No Phase C speed, CPU, peak-RSS, allocation or throughput improvement is
  claimed. Performance comparisons remain user-run.
- Main merges, version changes, tags, releases and registry uploads are
  excluded from this engineering phase.

Historical evidence remains in [Phase B status](docs/PHASE_B_STATUS.md) and
[the earlier engineering report](docs/OVERHAUL_RESULTS_2026_11_16.md).
