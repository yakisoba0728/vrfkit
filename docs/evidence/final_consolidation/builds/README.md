# Local executable provenance

`baseline-build.json` records the clean recovered baseline, full runtime/tool/
workflow fingerprints, frozen executable identity and unchanged release profile.
The original jobs=2 build ended with a rustc SIGKILL and exit 101. It is not a
passing build. A single serialized jobs=1 retry with other Cargo builds held
passed without changing release flags. Resource contention is suspected;
there is no kernel OOM diagnosis in this evidence.

The frozen baseline executable was copied to a separate read-only path before
any target reuse. Its SHA-256 is
`01e828332fe3e904e7051e5b2826addf0f4e5029ddb5e8e775240242ccadeb8c`.
It is the executable used for E's five fresh public baseline validations and
checkpoint exports. Baseline Python ran all 1,181 tests with warnings as errors.
These results are distinct from final candidate qualification.

## Integrated candidate

`candidate-build-original.json` preserves the exact original receipt hash
referenced by independent QA. `candidate-build.json` adds published-log
mappings and normalized artifact paths. Both record the clean runtime source
`1d2d8f6fe68f3ad1f196bca5adac24b0d3fbee6c`, tree
`833dae7add06f21589ffdae9680d5df12a5f89fe`. All 12 workspace packages were
cleaned from the shared release target before compiling the candidate. Cargo
then emitted 11 current workspace artifacts, each with `fresh: false` and the
candidate worktree source path; external dependencies could remain cached.
The frozen executable is 38,832,064 bytes, SHA-256
`c3a3010083b81332e03d90b888d38dfe069dfb659f3c866637cfed0051d6d417`.
The default release profile, Rust 1.86 and Cargo.lock are unchanged.

The actual artifact count is recorded in the JSON; source, command, profile,
complete file fingerprints and raw/published log hashes are preserved. Later
documentation/evidence commits must retain every runtime/tool/workflow hash.
`integrated-local-tests.json` records the complete regular Rust workspace run:
885 passed, zero failed/ignored, plus rustfmt. The fresh Python 3.12 suite
passes all 1,181 tests with warnings as errors and no skips. These runs use
unmodified regular test profiles and exclude temporary independent adapters.
Both full logs are preserved with original and published hashes. Final
published-tree reconciliation is recorded separately. A matching version
string alone is insufficient.
