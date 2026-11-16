# Reconstructing the measured schema sources

These artifacts make the measured source snapshots reconstructible from the
public GitHub history. **The historical local commit IDs in the reports are not
required.** Only public commit `67c0a2d94fbcbeeefa56761f1bc13d9f6c829fb0`, the
retained text patches, and their verified Git tree hashes are needed.

The main [benchmark report](../../SCHEMA_REDECLARATION_BENCHMARK.md) explains
the workload and limits. This directory retains evidence, source patches and
measurement code; it contains no replay data or executable binaries.

## Exact source snapshots

[reconstruction.json](reconstruction.json) records each patch's SHA-256,
expected parent tree, expected result tree, changed paths, and exact harness
hashes. Apply the patches in order:

| Patch | Snapshot reconstructed | Expected Git tree |
| --- | --- | --- |
| Public base, no patch | Original integration baseline | `446af6223790182df4d41b83dcf70cdf34547044` |
| [01-resource-budget.patch](source-patches/01-resource-budget.patch) | Full-export baseline, historically `90aa753` | `f9ccceb32c751d2f73510c6dbfc2caadb351f67c` |
| [02-generation-doc.patch](source-patches/02-generation-doc.patch) | Micro baseline, historically `470372e` | `bfd41f27059ab4bf7247dc4d990e126c808c215b` |
| [03-redeclaration-reuse.patch](source-patches/03-redeclaration-reuse.patch) | Measured candidate, historically `dc31cd5` | `51b4380303460c0173f2b94dd794fd9502503caf` |

The second patch changes only a Rust documentation comment. The third includes
the registration example and strengthened model/resource checks as well as the
two production shortcuts. None of these patches contains the later benchmark
documentation, which did not exist when the executables were built.

Verification in a fresh detached public-base worktree applied each patch using
`git apply --check --index` and `git apply --index`, then compared `git write-tree`
with the recorded result. All three trees matched. The
[verification receipt](reconstruction-verification.json) also records checks
against the measured candidate source, example and lockfile hashes. No timing
was rerun for this verification.

## Prepare a new experiment directory

The commands below use a POSIX shell, Git and Python. Clone the integration
revision containing this report, or check out the published PR revision that
contains these artifacts:

```sh
git clone --branch integration/vrfkit-overhaul https://github.com/yakisoba0728/vrfkit.git vrfkit
cd vrfkit
VRFKIT_REPO="$(pwd)"
VRFKIT_REPRO="$(mktemp -d)/schema-experiment"
python3 docs/benchmarks/schema-redeclaration/reconstruct_sources.py \
  --repo "$VRFKIT_REPO" --out "$VRFKIT_REPRO"
```

For a shallow clone that lacks the public base, fetch it before reconstruction:

```sh
git fetch origin 67c0a2d94fbcbeeefa56761f1bc13d9f6c829fb0
```

The [reconstructor](reconstruct_sources.py) verifies the archived patch/harness
hashes, creates three new detached worktrees, applies the required patch prefix,
and checks every intermediate and final tree. It creates local commits with
fresh metadata so subsequent runner reports identify the reconstructed HEADs.
Those commit IDs need not match the historical IDs; their source trees must
match the table above. Nothing is pushed, and existing worktrees are not edited.
The output directory must be new and outside the clone.

The worktrees are:

- `worktrees/schema-budget`: full-export baseline, first patch only.
- `worktrees/schema-reuse-baseline`: micro baseline, first two patches.
- `worktrees/schema-reuse`: measured candidate, all three patches.

After verifying the source trees, the reconstructor copies the candidate's
registration example into the micro baseline. That measurement-only file was
also untracked in the original baseline. Its exact SHA-256 is recorded
separately; all other baseline sources retain the verified tree contents.

## Exact measurement code and path relocation

The six files under `recorded-harness/` preserve the **exact original bytes**
of the three orchestration scripts and the resource tool's three modules.
Their hashes are listed in `reconstruction.json`. In particular,
`recorded_micro.py` matches `runner_sha256` in the original micro report, and
`recorded_benchmark_resources.py` matches the original resource-tool hash.

The original orchestration scripts retain their historical workspace `ROOT`
assignment. The reconstructor leaves those archived originals untouched. It
copies the resource modules, with their original module filenames, into
`worktrees/benchmark/tools` without changing their bytes. In **copies** of the
three orchestration scripts under `evidence/`, it changes exactly one line:
the `ROOT = Path(...)` assignment now names the new experiment directory.

`evidence/reproduction-receipt.json` records both the archived and restored
SHA-256 values. A relocated script consequently has a different hash from the
historical original. New micro reports record that relocated script's actual
hash; they must not claim to match the original runner hash. The checked
relocation changes no workload, iteration order, metric or acceptance rule.

Reconstruction prepares sources and scripts only. It does not build, download
fixtures or execute any measurement.

## Build the measured targets

Install Rust 1.86.0 and use the recorded Cargo lockfile. Build each target into
its own worktree's target directory so baseline and candidate binaries cannot
overwrite one another:

```sh
for variant in schema-reuse-baseline schema-reuse; do
  CARGO_TARGET_DIR="$VRFKIT_REPRO/worktrees/$variant/target" \
    cargo +1.86.0 build --release --locked \
    --manifest-path "$VRFKIT_REPRO/worktrees/$variant/Cargo.toml" \
    -p vrf-schema --example registration_bench
done

for variant in schema-budget schema-reuse; do
  CARGO_TARGET_DIR="$VRFKIT_REPRO/worktrees/$variant/target" \
    cargo +1.86.0 build --release --locked \
    --manifest-path "$VRFKIT_REPRO/worktrees/$variant/Cargo.toml" -p vrfkit
done
```

The profile settings are stored in the reconstructed workspace. Source tree,
example and lockfile hashes are exact inputs. Executable hashes identify the
binaries actually measured in the original environment; a different machine
or absolute build path, especially with debug information, need not reproduce
the same executable bytes or timing.

## Optional new measurements

This step is separate from source reconstruction. Do not run compilation or
other heavy work concurrently with a comparison. Preserve the new reports
alongside the historical evidence rather than replacing it.

The focused measurement needs no replay data:

```sh
python3 "$VRFKIT_REPRO/evidence/schema-reuse-micro.py"
```

The full-export scripts expect six local fixture links named
`evidence/corpus-links/BUILD/fixture.vrf` under the new experiment directory,
for builds 12.05, 12.08, 12.09, 12.10, 12.11 and 13.00. Their expected input
SHA-256 values are in the census and each full-export report. Create links to
the matching public fixtures without renaming or changing the originals. The
fixture downloader and access instructions are outside this source archive;
no private corpus access is required.

Once the linked inputs match those hashes:

```sh
python3 "$VRFKIT_REPRO/evidence/schema-reuse-replays.py"
python3 "$VRFKIT_REPRO/evidence/schema-reuse-confirm.py"
```

The first script uses one warmup pair and five measured pairs per fixture. The
second uses two warmup pairs and seven measured pairs on 12.05, then repeats
that plan with the same baseline executable in both positions. All reports
retain their samples and provenance. Their results need not match the original
host's measurements; the identical-binary control exists to expose that limit.

To reproduce the schema census, copy the retained diagnostic into the
reconstructed micro-baseline worktree and build that example:

```sh
mkdir -p "$VRFKIT_REPRO/worktrees/schema-reuse-baseline/crates/vrfkit/examples"
cp "$VRFKIT_REPO/docs/benchmarks/schema-redeclaration/schema_declaration_counts.rs" \
  "$VRFKIT_REPRO/worktrees/schema-reuse-baseline/crates/vrfkit/examples/"
CARGO_TARGET_DIR="$VRFKIT_REPRO/worktrees/schema-reuse-baseline/target" \
  cargo +1.86.0 build --release --locked --no-default-features \
  --manifest-path "$VRFKIT_REPRO/worktrees/schema-reuse-baseline/Cargo.toml" \
  -p vrfkit --example schema_declaration_counts
"$VRFKIT_REPRO/worktrees/schema-reuse-baseline/target/release/examples/schema_declaration_counts" PUBLIC_FIXTURE
```

This prints aggregate schema counts. It skips packet decoding and is not a
substitute for complete-export equivalence or independent semantic validation.
