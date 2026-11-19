# Final consolidation evidence

This index contains new work after recovered integration
`8581642283bd5a571967129d73897db8ee5408ba`. Earlier Phase A-D evidence stays
at its original paths with its original measurements and limitations. The
[fixed acceptance receipt](https://github.com/yakisoba0728/vrfkit/pull/58#issuecomment-6932599335)
records final branch/PR checkout, tree, CI and Windows package identities after
all gates. It is separate from these source-frozen intermediate receipts.

## Review map

| Area | Review and evidence | Scope |
| --- | --- | --- |
| Current state | [Consolidation status](../../FINAL_CONSOLIDATION_STATUS.md) | Findings, scope, acceptance and recovery |
| Compatibility | [Final migration](../../FINAL_CONSOLIDATION_MIGRATION.md) | Public enum cases, checkpoint budget and publication behavior |
| Core movement | [A audit](core/README.md) | Malformed repeated GUID failing-before/fixed-after regression |
| Architecture | [B audit](architecture/README.md) | Fixed cumulative checkpoint path admission and dependency boundaries |
| Reader and CLI | [D audit](reader/README.md) | Input protection, report replacement and compiled mutation/restoration |
| Network boundary | [Coordinator regressions](net/README.md) | Short buffers, preserved assemblies and checked wide packet arithmetic |
| Independent QA | [E evidence](independent_qa/README.md) | Public corpus, independent probes, raw/value preservation and compiled mutation |
| Performance | [C review](performance/README.md) | Retained arithmetic, harness controls, source review; zero fresh benchmark executions |
| CI and packaging | [F initial audit](release/README.md) | Baseline unit/doc checks; final hosted evidence is in the fixed receipt |
| Final artifact protocol | [F verifier](release-final/README.md) | Verified download capability and repeatable exact-source package/source checks |
| Local builds | [Executable provenance](builds/README.md) | Immutable source and executable hashes; failed build retained distinctly |
| PR history | [Original PR #58 body](pr58-before.md) | Preserved before the final review rewrite |

All author and independent local checks are complete. E's
[final functional receipt](independent_qa/final-acceptance.json) records no new
unresolved P0/P1 within its scope. Final actual-HEAD hosted CI and downloaded
Windows package acceptance are recorded in the fixed external receipt above;
intermediate runs are not credited as final acceptance.

## Evidence interpretation

Each receipt names actual source, commands, exit codes, input scope and limits.
Local isolated author commit IDs identify the source used in that checkout;
GitHub integration may have a different commit object with the same relevant
file fingerprints. The final receipt reconciles the complete published tree
and runtime/tool/workflow fingerprints. Source identity, hosted checkout
identity and executed coverage are separate facts.

[Artifact normalization](artifact-normalization.json) records the terminal
blank-line cleanup applied to six published logs; corresponding receipts keep
original and current published hashes. Every test outcome line is retained.
The recorded first baseline build ended with SIGKILL and is not credited as a
pass; the separate serialized build succeeded under unchanged release flags.

Large evidence JSON/logs are marked as generated in `.gitattributes` so a PR
reviewer can expand them deliberately. Narrative reviews and executable QA
helpers remain visible by default. No evidence is deleted by this display
setting. Original public inputs and detailed rows stay outside the repository;
private replay payloads, player identifiers and credentials are not published.
