# Independent Phase A native acceptance

The independent reviewer did not author the native correction. The
[review receipt](../evidence/phase-a-native-independent-review.json) records
the reviewed source, executed probes, mutation controls and limits. The
[output receipt](../evidence/phase-a-native-independent-outputs.json) records
an actual second read of all 78 exported Parquet files, their footers and the
strictly compared manifests. This was not another six-replay decode.

## Generated Reader cases

In a disposable, clean worktree of the reviewed source, run:

```sh
test ! -e crates/vrfkit/tests/phase_a_native_qa.rs
cp docs/verification/phase_a_native_reader.rs crates/vrfkit/tests/phase_a_native_qa.rs
trap 'rm -f crates/vrfkit/tests/phase_a_native_qa.rs' EXIT
cargo +1.86.0 test -p vrfkit --no-default-features --locked --test phase_a_native_qa
```

The helper has three tests containing 31 generated cases. It uses existing
transport/transform framing infrastructure but independently constructs the
native payloads and expected raw bytes, values and source identities. Cases
cover both fragmented and ordinary RPCs, all input/discrepancy body bit
residues, optional A4, nonzero unused high bits, invalid outer envelopes and
recovery of the following framed component. Supported forms explicitly
assert that duplicate raw RPC data is absent; unresolved forms assert that
the complete RPC remains available with its decoded movement prefix.

The helper is retained under `docs/verification` rather than added to the
normal Cargo suite. Its three executed tests therefore do not alter the
published workspace test count. The implementation PR already contains its
permanent regression and boundary tests.

The review receipt gives two exact source mutations: removing the A3 body
completion guard and relaxing the exact native envelope length. Each compiled
successfully and made the appropriate independent test fail at runtime. The
reviewer restored the original source byte for byte and reran all three tests
successfully. Apply mutations only in a disposable worktree and restore them
before running any acceptance build.

## Actual export comparison

Use the original pre-native accepted export root and the new Phase A export
root. Each must contain the six SHA-256-named replay directories created by
the common corpus audit, with an `export` directory inside each:

```sh
python docs/verification/phase_a_native_outputs.py \
  /private/before-audit /private/phase-a-audit . /private/independent-output-review.json
```

This script imports neither the production comparison helper nor its result
objects. It reads and hashes every Parquet byte, parses every footer, compares
the schema and row counts, and recursively compares JSON types and values.
Only the documented counter migration and a finite nonnegative elapsed time
can differ. Twenty-four in-memory negative controls must fail before the
report can pass. The script also hashes all 151 source files named by the
frozen native binary's build receipt. Paths and decoded replay values are
omitted from the report.
