# Final CI and Windows package verification protocol

This directory is prepared before the final source freeze. It does not claim
that the final HEAD has passed CI. The completed exact-HEAD receipt belongs in
the [fixed acceptance comment](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6932599335),
after final push and PR checks finish, so evidence recording does not silently
replace the tested source with another commit.

## Actual artifact-byte capability probe

[intermediate-package-probe.json](intermediate-package-probe.json) records
only intermediate source `6e54158f0747ce55334f7138e7d5ce7ee8068bb1` and push
run [43979651544](https://github.com/yakisoba0728/vrfkit/actions/runs/43979651544).
Its Windows ZIP, packaged replay report and source snapshot were downloaded
through the authenticated GitHub artifact tool. The returned temporary file
URL was consumed locally, then omitted from persistent evidence.

The independently checked bytes match all three GitHub artifact digests,
the inner ZIP sidecar, build-info identity, AMD64 PE32+ image and executable
hash in the actual hosted replay report. The source archive matches all 676
included Git files and executable bits. The hosted package replay audit used
one public 12.10 replay with seven checkpoints and 236 typed comparisons.
These are preparation results, not final-source acceptance and not new unique
corpus coverage beyond that public input.

The [negative control](intermediate-negative-control.json) substitutes an
incorrect expected artifact digest and observes the verification assertion
fail before a receipt is written. Original downloaded bytes are unchanged.

## Repeatable independent checks

[verify_artifact_bytes.py](verify_artifact_bytes.py) accepts a final checkout
SHA, fetched GitHub artifact metadata and locally downloaded ZIP files. Run
it with ordinary Python, without optimization flags that disable assertions:

```bash
python docs/evidence/final_consolidation/release-final/verify_artifact_bytes.py \
  --repo . --sha FULL_ACTUAL_CHECKOUT_SHA \
  --artifacts /private/final-artifacts.json \
  --directory /private/final-artifacts \
  --output /private/final-package-verification.json
```

The metadata input is the artifact list returned for the selected run,
restricted to `windows-release`, `release-replay-verification` and optionally
`vrfkit-source`. The local files use those names followed by `.zip`. A PR
synthetic checkout differs from the run's integration head; pass
`--run-head-sha FULL_INTEGRATION_SHA` in that case.

The verifier checks the outer artifact sizes/digests, ZIP members and sidecar,
embedded source/target/executable identity, PE architecture, source notices,
hosted report identity and executable integrity, pinned replay hash, positive
checkpoint/value coverage, and source snapshot bytes/modes. It independently
reconstructs the runtime source fingerprint using the same documented file
set and ordering. The Windows checkout uses CRLF while Git blobs and the
Linux source archive use LF: the receipt records both fingerprints and which
matches the hosted report rather than claiming cross-platform byte equality.
It also verifies the harness and fixture-specification hashes.

It does not execute a Windows binary on Linux. Actual optimized build, tests,
ZIP extraction, executable launch and replay execution must be confirmed in
the hosted Windows job's complete log and successful step results.

## Exact final acceptance checklist

1. Freeze the intended integration commit, tree, workflows, lockfile and
   fixture/harness identities.
2. Inspect all final integration push jobs and final PR #58 jobs, including
   actual logs. Record each job ID, result and checkout from `git log` output.
   Completion wrappers have no checkout and must be marked as such.
3. Resolve the PR synthetic commit, verify its main/integration parents and
   compare its Git tree with the final integration tree.
4. Download final artifacts and run the byte verifier. Preserve compact
   results and public run/artifact links; never publish signed download URLs.
5. Read the actual hosted suite, feature, replay and packaging results.
   Separate unique replay coverage from repeated platform/CI executions.
6. Recheck main, Draft/unmerged/auto-merge state and unchanged tags/releases.
   Record the completed receipt in the existing fixed comment without a new
   source commit. No merge, tag, release or registry publication is authorized.
