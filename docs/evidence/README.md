# Overhaul evidence and reproduction

The [results report](../OVERHAUL_RESULTS_2026_11_16.md) explains the scope and
limitations of these records. [The aggregate JSON](overhaul_2026_11_16.json)
links public input pins, successful CI runs, measured source/binary hashes,
output equivalence and unresolved work. Historical measurements retain their
original source revisions; a later merge commit does not replace that provenance.

## Combined runtime source

`overhaul-runtime-source.json` records correspondence between the public
integration source and the locally measured combined candidate. Its source
fingerprint uses exactly the algorithm in `tools/verify_build_corpus.py`:
Python `sorted(Path(...).rglob("*.rs"))` under `crates`, followed by
`sorted(Path(...).rglob("Cargo.toml"))` under `crates`, followed by root
`Cargo.lock` and `Cargo.toml`. Sort the `Path` objects, whose comparison uses
path components; sorting the rendered POSIX strings can give a different order.
Each entry is its repository-relative path,
a space and its SHA-256. Join entries with newlines, without a final newline,
then SHA-256 the UTF-8 bytes. The recorded per-file hashes allow an independent
comparison without access to the local candidate commit object.

Run this from the matching repository root to reproduce the fingerprint:

```python
from pathlib import Path
import hashlib

root = Path.cwd()
paths = (sorted((root / "crates").rglob("*.rs"))
         + sorted((root / "crates").rglob("Cargo.toml"))
         + [root / "Cargo.lock", root / "Cargo.toml"])
lines = [f"{p.relative_to(root).as_posix()} "
         f"{hashlib.sha256(p.read_bytes()).hexdigest()}" for p in paths]
print(hashlib.sha256("\n".join(lines).encode("utf-8")).hexdigest())
```

The fingerprint is a source identity check, not an additional execution of a
replay or a promise of reproducible binary bytes across environments. Actual
candidate executions and exact-head hosted CI remain separately recorded.

`pr56-actions-observed-test-counts.json` distinguishes the PR head, the
synthetic merge actually checked out by Actions, and the later integration
merge. The two merge trees are identical. It records counts extracted from
completed Windows/Linux Rust and Python job logs, together with job URLs and
decoded-log hashes. Raw Actions logs are not copied into this directory.
`postmerge-actions-43783444550.json` records the separate automatic push run:
all 11 jobs passed, and the inspected checkout log names the actual integration
merge. It adds post-integration execution evidence without changing earlier
measured provenance.

## External Rust source consumer

`reader-external-consumer/` preserves the exact source, lockfile and sanitized
receipt used by the standalone consumer check. Its manifest deliberately has
its own `[workspace]`. It is an archived external experiment, not another
member of the vrfkit workspace. Copy it into the layout below before building;
do not run its path dependency directly from this documentation directory.

| Path beneath a new experiment directory | Contents |
| --- | --- |
| `worktrees/final-validation/` | Public vrfkit source matching the recorded runtime fingerprint |
| `evidence/reader-external-consumer/` | A copy of this retained consumer directory |
| `evidence/corpus-links/13.00/fixture.vrf` | A read-only link to the pinned public 13.00 fixture |

Use the integrated runtime revision identified in
`overhaul-runtime-source.json`. The original local candidate commit need not
exist in the public repository. Documentation and commit metadata can differ;
verify the recorded parser files and lockfile before reproducing. Obtain the
fixture from the immutable public URL in `overhaul_2026_11_16.json` and verify
its SHA-256 before linking it. Do not rename or modify the original.

From `evidence/reader-external-consumer/`, with Rust 1.86 available:

```bash
cargo +1.86.0 fmt --check
cargo +1.86.0 build --release --locked
cargo +1.86.0 tree --locked --edges normal --prefix none --format '{p}'
./target/release/vrfkit-reader-external-check ../corpus-links/13.00/fixture.vrf 6
```

Keep the retained external `Cargo.lock`. `commands.txt` also records the
original lockfile-generation step, which is not needed for a locked repeat.
The receipt distinguishes the external dependency selection from the parser
workspace's lockfile. No registry publication or performance claim follows
from this check.

## Linux feature and documentation checks

`final-linux-feature-matrix.json` records the clean candidate source, toolchain,
27 configurations, core-only runtime tests, strict documentation commands and
crate-root unsafe-code policy checks. It contains aggregate results and log
digests, not the private local logs. Its recorded commands can be run from a
matching source checkout with a new target directory. Set
`RUSTDOCFLAGS="-D warnings"` for the two strict documentation checks.

`performance-publish-independent-review.json` records the independent
comparison of every performance publication file with the reviewed original,
and all 149 runtime-fingerprint inputs with the combined candidate. The
published PR tree was also checked against the complete local Git tree before
its ref moved. This review did not rerun timing measurements.

## Performance experiments

The [schema benchmark report](../SCHEMA_REDECLARATION_BENCHMARK.md) and
[source reconstruction guide](../benchmarks/schema-redeclaration/README.md)
retain actual measurement JSON, noisy/negative results, same-binary controls,
declaration census, exact patches and original harness bytes. Reconstruct
those historical source trees for measurement replication; do not silently
substitute a later integration binary.

No raw replay, player payload, private catalog row, credential or private
export is included in this evidence directory. Hashes identify retained local
evidence but do not make omitted raw logs publicly downloadable. Public CI
links and the reproduction commands provide independent checks within the
documented input scope.
