# Overhaul execution ledger

The current continuation is tracked in [Phase C status](PHASE_C_STATUS.md)
and [Phase C results](PHASE_C_RESULTS.md). [Phase B status](PHASE_B_STATUS.md)
and [Phase B results](PHASE_B_RESULTS.md) retain their completed phase's scope.
The user has assigned final
quantitative performance comparison to themselves; functional verification
and integration gates remain required. [Phase A status](PHASE_A_STATUS.md)
and the historical checkpoint below retain their original evidence and scope.

Updated for the 2026-11-16 session. The detailed
[verified results](OVERHAUL_RESULTS_2026_11_16.md) and
[sanitized evidence](evidence/overhaul_2026_11_16.json) record the accepted
runtime checkpoint and distinguish actual execution from source correspondence.
The documentation PR and subsequent Draft PR retain their own later CI records.

## Boundaries and baseline

Work targets `integration/vrfkit-overhaul` through separate reviewed PRs.
Do not push or merge to `main`, publish releases/packages, mutate the private
archive or edit the external reference repository when resuming this work.
Original replays remain read-only. Private bytes, catalog rows, player
identifiers and credentials must not enter public logs or artifacts.

- Unchanged main: `9ece4ab84dee62cbf8bf34b7996d2efb0593f9c9`.
- Measured starting integration: `67c0a2d94fbcbeeefa56761f1bc13d9f6c829fb0`.
- Accepted runtime integration: `ed7dc30978261f27d3999b194344bcf55287860d`.
- Read `CONTRIBUTING.md` and `CLAUDE.md` before implementation changes.
- Rust 1.86, Python, local execution, isolated worktrees and independent workers
  were available and used. Earlier no-toolchain/no-worker claims are obsolete.
  GitHub Actions executes CI; it does not continue AI workers after the session.

## Execution state

| Track | State at this checkpoint | Durable evidence / next step |
| --- | --- | --- |
| Bit-reader width/scale errors | Merged, independently verified | [PR 49](https://github.com/yakisoba0728/vrfkit/pull/49), [successful CI](https://github.com/yakisoba0728/vrfkit/actions/runs/43776108241) |
| Paired CPU/RSS/full-export harness | Merged, independently verified | [PR 50](https://github.com/yakisoba0728/vrfkit/pull/50), [successful CI](https://github.com/yakisoba0728/vrfkit/actions/runs/43777023619) |
| Aggregate schema allocation budget | Merged, independently verified | [PR 51](https://github.com/yakisoba0728/vrfkit/pull/51), [successful CI](https://github.com/yakisoba0728/vrfkit/actions/runs/43778079065); six public replays, 78 unchanged Parquet files |
| Shared Rust Reader API | Merged, independently verified | [PR 52](https://github.com/yakisoba0728/vrfkit/pull/52), [successful CI](https://github.com/yakisoba0728/vrfkit/actions/runs/43778665193); 18 API and 39 CLI table files unchanged |
| Streaming JSONL conversion | Merged, independently verified | [PR 53](https://github.com/yakisoba0728/vrfkit/pull/53), [successful CI](https://github.com/yakisoba0728/vrfkit/actions/runs/43779583452); 39 tables / 138,727 rows / 298,133 binary bytes round-trip |
| Reference comparison corrections | Merged, independently verified | [PR 54](https://github.com/yakisoba0728/vrfkit/pull/54), [successful CI](https://github.com/yakisoba0728/vrfkit/actions/runs/43780363565); 31 focused tests; final reports unchanged on all three cached public export pairs |
| Permanent required Linux CI | Merged, independently verified | [PR 55](https://github.com/yakisoba0728/vrfkit/pull/55), [successful CI](https://github.com/yakisoba0728/vrfkit/actions/runs/43781227633); ten fault-injection cases and four gate states passed |
| Repeated-schema reuse | Merged, independently verified | [PR 56](https://github.com/yakisoba0728/vrfkit/pull/56), [successful CI](https://github.com/yakisoba0728/vrfkit/actions/runs/43782959411); synthetic benefit, no ordinary-replay throughput/RSS claim |
| Latest private archive inventory | Metadata verification complete | 1,011 assets, 50,938,107,071 bytes, 24 builds; zero private binaries decoded |
| Combined candidate replay audit | Passed; integrated source correspondence verified | `93e107a`: 774 Rust tests, 76 checkpoints, 78 unchanged Parquet files; 22,561 unresolved RPCs preserved, zero lost blocks |
| Integrated runtime acceptance | Complete at `ed7dc309` | All 149 runtime-fingerprint files match audited candidate; final runtime PR passed 11/11 required jobs; [source receipt](evidence/overhaul-runtime-source.json) |
| Post-merge regression CI | Passed on actual `ed7dc309` checkout | [Automatic push run 43783444550](https://github.com/yakisoba0728/vrfkit/actions/runs/43783444550), 11/11 jobs; [receipt](evidence/postmerge-actions-43783444550.json) |
| Independent Linux matrix and external consumer | Passed | 27 feature configurations, 135 core-only tests, strict docs; standalone Rust 1.86 consumer with Arrow/Parquet/Zstd absent |
| Additional 12.09 reference comparison | Completed with unresolved differences | First 50,000 rows: 49,980 exact matches, 20 missing, zero ambiguous; whole-export movement totals differ by 17,098 |
| Documentation and Draft PR to main | Publication follows this runtime checkpoint | Preserve the docs PR's own exact-head CI and the Draft PR's completion record; leave main unmerged |
| Public release / registry publication | Deferred | Separate release decision and exact-version validation required |

Baseline counts were 743 non-Windows / 744 Windows Rust tests and 1,089 Python
tests. The Reader checkpoint has 774 / 775 Rust tests. The combined JSONL and
comparator Python run passed 1,143 tests. Hosted checks and independent local
executions remain separately attributed; source identity does not create an
extra execution or change the measured candidate's original commit.
The [hosted log receipt](evidence/pr56-actions-observed-test-counts.json)
confirms 775 Windows / 774 Linux Rust tests and 1,143 Python tests in each
of four Python jobs. Actions tested synthetic merge `22cd027c`, whose tree
exactly equals integrated `ed7dc309`; it did not execute that later commit object.

## Resume without duplicating work

1. Fetch integration and inspect existing PRs, exact heads and CI conclusions.
   This ledger is a publication checkpoint, not a substitute for current remote
   state. Reuse existing branches and PRs.
2. PRs 49-56 are complete. Locate the final documentation PR and Draft PR by
   branch before creating anything new; their bodies/checks record publication
   after this committed runtime checkpoint. Confirm all required checks pass.
3. Preserve accepted fingerprint
   `92328ce1ab9a4236c38ae07c49ed816d259e0107cac4470295e05de48fbff93d`.
   The integrated runtime already matched every file from audited `93e107a`.
   If a new runtime change alters it, rebuild and rerun affected tests and the
   common public main/checkpoint audit in new private output paths. Compare
   Parquet hashes with `67c0a2d` and explain every difference.
4. Continue an open backlog item only with its required evidence. Do not repeat
   completed noisy timings or claim the historical private audit was rerun.
   Preserve original measurement commits, input populations and failures.
5. For new PRs, require MSRV/platform/feature/docs/Clippy/Python gates and an
   independent review, then merge only into integration. Update evidence and
   migration documentation with the actual tested head.
6. Keep the final Draft PR to main unmerged. No release or registry deployment
   follows automatically from CI success or this ledger.

## Historical research and validation backlog

This table preserves the backlog at that historical runtime checkpoint. Reader
file input, optional checkpoint callbacks and explicitly bounded nonseekable
input were subsequently completed within the documented Phase B/C scope;
the current contracts and remaining limits are in [Phase C results](PHASE_C_RESULTS.md).

| Priority | Work | Evidence required before adoption |
| --- | --- | --- |
| High | Fresh private-corpus binary audit | Authorized binary acquisition, source hash verification, private execution, aggregate-only publication |
| High | Older-build damage handles and version maps | Exact group/field/checksum identity, complete payload consumption and independent values; no name-only guess |
| High | 12.09 movement attribution differences | Trace 20 missing exact identities and whole-export row-count differences on the pinned public full match; preserve strict joins until the cause is demonstrated |
| Medium | Reader input streaming and checkpoint callbacks | Memory/ownership design, partial-publication contract, ordering and replay parity |
| Medium | Event enrichment and gameplay semantics | Observed wire evidence, explicit unknown/partial states and independent comparisons |
| Medium | Equivalent-scope competitor benchmark | Same inputs, work contract, environment and required output; repeated wall/CPU/RSS measurements |
| Medium | China 13.05/13.09 support | Fixture-backed wire identity and independent validation; current 27-global-branch inventory unchanged |
| Medium | Repository About wording | Align losslessness language with measured preservation after main review; no metadata change claimed |
| Medium | Distribution and formal release | Version/migration decision, package metadata, platform tests and verified release assets |

The default reference CLI and Rust exporter have different output scopes, so
an unqualified timing ratio is not accepted evidence. The RPC experiment was
not adopted. Repeated-schema reuse has a synthetic benefit; ordinary public
streams showed zero identical redeclarations and noisy timings. Preserve these
negative results instead of restarting an unproven optimization without new
inputs or evidence.

## Access and evidence limits

The private archive was inventoried read-only at catalog commit
`b1fff6bc26aa040f1948931fadb9ebf29c961ec9`. Metadata access did not provide a
usable private release-binary download path. No private replay was downloaded
or decoded, and Actions does not inherit interactive connector access. The
historical 1,018-replay report in `docs/BUILD_VERIFICATION.md` is not a fresh
audit of this overhaul or the current 1,011-item archive.

Source/toolchain access was recovered and actual local execution completed.
The unchanged reference parser built under requested .NET 10.0.301 after serial
restore/build with node reuse disabled; its precise parallel failure cause
remains unknown. Detailed paths, replay values and raw logs stay private. The
committed evidence JSON contains reviewed aggregates/provenance; a full private
corpus download is not a prerequisite for normal PR CI.
