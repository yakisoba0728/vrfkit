# Architecture review and checkpoint path-expansion admission

Implementation source: `f849f62ed10518905dce845743e07daa2d55a8b3`.
The [machine-readable receipt](audit.json) names source and dependency hashes,
commands, actual results and unexecuted gates. This is the author's bounded
schema/architecture record; independent final-source QA remains a separate gate.

## Reproduced resource defect

Checkpoint GUID entries can refer to preceding literal paths. The old reader
cloned a whole resolved `String` for every short index, and the detached Reader
summary/CLI GUID rows cloned the cache paths again. The existing per-string,
entry-count and decoded-archive bounds did not limit that expansion.

The standalone [public-API regression](checkpoint_path_budget_probe.rs) supplies
one 262,144-byte literal and 1,024 references under distinct GUIDs. Its encoded
archive is **268,195 bytes**. The baseline accepted all 1,025 entries and
retained **268,697,600 path bytes** in the cache. The expected-refusal probe
therefore exited 1. These are exact byte counts, not measured RSS, a speed ratio
or an induced out-of-memory crash.

The [before log](probe-before.log) preserves that actual semantic failure.
An earlier probe import named a private module and did not compile. Its
[separate log](probe-initial-compile-error.log) is retained as a harness mistake;
it is not counted as a product failure or successful test. The corrected probe
uses the exported public function.

## Fix and contract

Each checkpoint table read has a **256 MiB cumulative resolved UTF-8 GUID-path
byte budget**. Every declaration is charged: literal paths, resolved indices,
legacy decimal paths, and repeated GUID replacements. This measures declared
path work; it is deliberately not a live-cache-only count.

Indexed paths stay borrowed through wire validation. After the flags byte has
been read, admission checks the resolved length before making an owned copy,
calling that entry's observer or inserting it into the cache. A refusal uses
the existing `SchemaError::ResourceLimit` with resource
`checkpoint resolved GUID path bytes`, requested bytes and the 268,435,456-byte
limit. Earlier accepted entries remain intact and callbacks remain provisional.
The budget resets for each table read.

This is a new resource-policy refusal, not a new claim about wire-format
validity. Archives above the policy were previously accepted and now fail
explicitly. Literal decoding can allocate up to its existing FString bound
before admission. Map overhead, literal-table copies, decoded archives, later
frame declarations and caller-retained output remain additional costs; the
budget is **not a total process-memory guarantee**. The public parser still
requires a fresh checkpoint cache. The existing two-field `SchemaLimits` does
not configure this independent checkpoint policy.

Public signatures, enum variants, raw index/flag observations, cache generation
rules and successful within-budget values are unchanged. `Cow` separates path
resolution from ownership until admission; this is a narrow refactor tied to
the defect, not an unmeasured throughput optimization.

Internal `Arc<str>` sharing was considered and not implemented: it would change
the general cache while leaving the owned detached summary amplification in
place. No parser rewrite or speculative type interpretation was introduced.

## Executed author checks

- Rust 1.86 schema suite: **84 passed** (68 unit, 15 resource integration,
  one Rustdoc test), zero failed/ignored. [Log](schema-tests.log).
- Seven new small tests cover literal/reference/replacement charging, refusal
  before callback/cache mutation, the exact boundary, per-read reset, decoded
  UTF-8 lengths, legacy mode, empty paths, truncation/index-error precedence
  and preservation of the original earlier sink error.
- Package rustfmt and Git whitespace checks passed.
- The actual locked core dependency tree contains no Arrow, Parquet or Zstd.
  [Dependency tree](core-dependencies.log).

The final public-limit positive/negative probe, real replay parity, combined
Clippy/Rustdoc and Windows checks are delegated to independent QA/coordinator.
The author's post-fix 256 MiB probe was held while the coordinator serialized
memory-heavy compilation. It is not counted as executed here. Final receipts
must close those gates before acceptance.

## Architecture review scope

The reviewed manifests and source hashes are listed in the receipt. All ten
production crates retain `#![forbid(unsafe_code)]`. The dependency layering
keeps primitive bit reading below schema/frame/network/decode, and keeps Arrow
writers behind export features while the record types remain available to the
core Reader. No dependency or Cargo.lock change is needed for this fix.

Reader and CLI share `Pass`, the schema cache, sink and checkpoint decoder.
Their separate outer orchestration has deliberate contracts: Reader callbacks
follow file order and retain consumer-error identity, while CLI checkpoints
run in a separate pass with main-error priority. Combining those adapters into
one larger function would risk changing those contracts without removing a
demonstrated defect, so it was not done.

Writer queues, row batches and final GUID rows were traced to distinguish
row-count bounds from byte bounds. Queue depth is not a guarantee for arbitrary
large records. Cache generation/unchanged-mapping behavior, resource admission
and leaf-index rebuilding retain their existing tests and implementation.
This is a bounded architectural review, not a proof of every line of source.

Two direct lower-level API risks identified by role A remain separately
tracked: a caller-supplied fragment bit count exceeding its byte slice can
panic in the partial accumulator, and an extreme direct packet larger than
268,435,455 bytes can overflow the signed bit-size expression. Ordinary
DemoFrame packets are bounded to 2,048 bytes, and the pipeline passes matched
fragment slices/counts. Neither limitation is claimed fixed by this schema
commit; the coordinator must record their final disposition.

## Reproduction

The probe intentionally retains about 256 MiB of decoded path data. Use a
separate process outside memory-heavy build windows. The same retained external
Cargo lockfile was used for the before probe; it pins thiserror 2.0.21, while
the unchanged workspace lockfile pins 2.0.19. This is a public-source semantic
proof, not a same-binary performance comparison. The regular schema suite uses
the repository's unchanged lockfile.

```bash
python docs/evidence/final_consolidation/architecture/run_probe.py --repo .
```

To replay the failing-before case, provide a separate checkout of baseline
`6e54158f0747ce55334f7138e7d5ce7ee8068bb1`. The runner does not modify that
checkout. The expected outcome is exit 1 on the baseline and exit 0 after the
fix. Full final public-input compatibility belongs to the independent receipt.
