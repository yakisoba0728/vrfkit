# Phase C release and compatibility checklist

This is release preparation, not a release approval or a final acceptance
receipt. It does not change package versions, tags, releases or registry state.
The minimum next version requested for the changed native-counter contract is
0.6.0. Select 0.6.0 or a later version only as part of a separately authorized
release. Keep `main` and final Draft PR #58 unmerged throughout Phase C.

## 1. Freeze an accepted functional candidate

- Record the exact integration commit, tree, runtime-source fingerprint and
  source-corresponding independent QA receipt in the Phase C results ledger.
- Distinguish task PR heads, synthetic PR merge checkouts and the actual final
  integration push. A successful older checkout is not final-head validation.
- Require Windows/Linux, Rust 1.86/MSRV, stable, rustfmt, strict Clippy,
  strict rustdoc, feature matrix, Python, Rust/PyArrow interoperability and
  packaged-CLI checks. Obtain suite counts from actual executing logs; do not
  infer success from authored test counts or a job that skipped later steps.
- Require fresh public replay/checkpoint comparisons and mutation controls
  against the accepted source. Preserve aggregate hashes and exclusions;
  do not publish full replay, Parquet, raw probe or player-identifying data.
- Keep this checklist incomplete until the current final source passes.
  Individual author smoke checks and historical Phase B audits are not a
  substitute for final independent acceptance.

## 2. Review migration and output contracts

| Area | Required review before release |
| --- | --- |
| Native counters | Retain the nine current native counters and document removed/renamed legacy counters using the Phase A migration record. Validate both main and checkpoint summary/manifest contracts. |
| Reader checkpoints (#71) | Default methods continue to skip snapshots. Opt-in `ReadOptions` delivers explicitly scoped checkpoint metadata, declarations, rows, blocks and summaries. Packet/GUID namespaces and parser state remain independent. |
| Reader errors | Document file-order structural error priority, original consumer-error preservation and provisional callbacks. A checkpoint summary is not whole-replay success; the CLI's parallel-pass priority is a distinct contract. |
| Gameplay identity (#70) | Main-stream opt-in only. Preserve explicit null, unresolved and ambiguous states, packet boundaries and lifecycle evidence. Do not imply account resolution, player kill credit, checkpoint identity or complete round outcomes. |
| FocusProjectiles (#72) | Admit the evidenced 12.05 declarations only. Preserve original raw parents/rows. Explain the expected 53 additive main child rows, three empty trailers and two new counter keys; require exact fresh comparison, not only aggregate totals. |
| Bounded Read input (#73) | Require an explicit encoded-input byte cap. State that the entire encoded input is buffered, with callbacks only after EOF. The limit is not a total-memory limit. Excess input consumes one probe byte; no automatic spool or temporary file is created. |

Compile consumer examples with Rust 1.86 and the documented feature settings.
Retain the existing default Reader and CLI examples alongside opt-in examples.
Check downstream matches against public non-exhaustive enums and the new
`ReadError::InputLimit` case. Preserve unknown raw data instead of inventing
new field types to make a migration appear complete.

Current implementation and migration references:

- [Rust Reader API](RUST_API.md)
- [Gameplay API](GAMEPLAY_API.md)
- [Phase A results and native-counter migration](PHASE_A_RESULTS.md)
- [CLI input contract](CLI_INPUT_STREAMING.md)
- [Phase C execution ledger](PHASE_C_STATUS.md)

The task PRs above are candidates until their acceptance gates and final
integration evidence are recorded. Their presence in this checklist is not
a claim that they have already merged.

## 3. Authorized release actions - not performed in Phase C

After separate user approval, select a version of at least 0.6.0 and update
all applicable workspace/package metadata, lockfile metadata, version strings,
installation examples and release notes consistently. Run the existing package
and interoperability checks against that exact source. Review supported builds,
unresolved semantic scopes, raw-data guarantees and breaking counter migration.

Only after those checks and explicit publication approval may the maintainer
merge the final PR into main, create a tag, publish a GitHub Release, or upload
a crates.io/PyPI package. Building or retaining a CI test package is not public
release approval. No such main merge, tag or publication is authorized here.

## 4. User-run performance handoff

Update [the benchmark handoff](PHASE_B_BENCHMARK_HANDOFF.md) to the actually
accepted final candidate and retain the existing tool commands and inputs.
Until that SHA is recorded, do not label a task branch as the final benchmark
candidate. Static tool validation and functional output checks are permitted;
benchmark commands, repeated A/B runs and new timing/CPU/RSS improvement claims
remain excluded. Functional test durations are not performance measurements.
