# Phase A performance experiment: candidate withdrawn

The movement inlining candidate was implemented, built, measured, and
withdrawn. **No ordinary-replay performance optimization is accepted from this
experiment.** All output-equivalence checks passed, but the measurements did
not separate a reliable throughput or CPU benefit from the observed variation.
The measured bottleneck investigation remains useful and is retained in
[Phase A profiling](PHASE_A_PROFILING.md).

## Change and controlled builds

The complete runtime experiment was one attribute:
`#[inline(always)]` on `vrf_movement::moves::parse_single_move`. No validation,
decoding, output table, row, compression option, or resource limit changed.

Both executables were built sequentially from the **same checkout directory**
with Rust 1.86.0, the same lockfile, default features, and the repository's
release profile: optimization level 3, fat LTO, one codegen unit, debug level 1.
Each executable was copied to a separate frozen file before the next build.
No extra Rust flags were set. This avoids treating a changed source path or
different build profile as the inlining effect.

| Identity | Value |
| --- | --- |
| Local unannotated source | `f94d5cf1c38fdbfc97cecd4d7f290d2612ec70ac` |
| Local candidate source | `8a8810e3b7c676eae785b7d3066fabcc571da0b6` |
| Candidate tree | `078e1007c6c77d9c29a1657bdce87636f1b30982` |
| Unannotated executable SHA-256 | `e506eaaf0f443252a90ea7de22d3b316e5971575b32fcfab60632f6817eeb35a` |
| Inline executable SHA-256 | `e8fcd54da746dcf2bd0e4ed478a753b037ab628c135d5eecb7fac2eefcd6e04a` |
| Withdrawal commit | `325acda3092600aa3944c7d123f12a4fcdd6890b` |

The unannotated source matches all 151 files in the recovered native build
receipt. The earlier frozen native executable was not overwritten or timed as
one of these two variants. These local commit identifiers describe executed
source; the coordinator records any GitHub API publication mapping separately.
The local experiment branch preserves the candidate before withdrawal.

Fresh disassembly found two out-of-line calls to `parse_single_move` in the
unannotated executable and zero in the inline executable. Its standalone text
symbol also disappeared. The complete executable changed from 38,547,424 to
38,546,832 bytes. This establishes the intended code-generation change, not
an end-to-end speedup.

## Execution method

The existing `tools/benchmark_resources.py` ran complete exports on unchanged
public inputs. Each series used one warmup pair, then alternating variant order
for every measured pair. The first two comparisons and the single bounded
reverse-label confirmation used seven measured pairs each. The identical-binary
control used five pairs. Other agents held builds, tests, and exports during
each coordinated timing window. This is not an exclusive-host guarantee.

The inputs and output modes were:

| Public input | Bytes | SHA-256 | Mode |
| --- | ---: | --- | --- |
| 12.09 | 61,015,648 | `925fe2d1dfce6dff071df724bff03faa7220ffd209bdf795b76888b65157efff` | Main and checkpoints |
| 12.05 | 54,702,346 | `f5512818e79bd61d30ed8b7bbc34703e49f0e5a762acc96bf83fed08868c1b8b` | Main |

This is a warm-cache comparison. Input hashing and warmups precede measured
work; no cold-storage throughput claim follows. Wall time includes export I/O.
CPU utilization is `100 * total CPU / wall`, so multiple worker threads can
exceed 100%. Peak RSS is the parser process peak, including its threads.
Allocation counts, allocated bytes, and isolated I/O time were not measured.

## Fresh complete-export measurements

Each value below is the median of the seven measured observations for that
variant. These marginal medians are distinct from the paired-ratio statistics
in the next table; their ratios need not be equal.

| Input / mode | Variant | Wall s | CPU s | CPU use % | Peak RSS bytes | Input MiB/s | Parquet bytes | Total output bytes |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 12.09 / checkpoints | Unannotated | 2.028978 | 4.939242 | 236.368 | 196,775,936 | 28.679 | 43,984,122 | 44,676,171 |
| 12.09 / checkpoints | Inline | 1.942531 | 4.883212 | 249.369 | 197,349,376 | 29.955 | 43,984,122 | 44,676,171 |
| 12.05 / main | Unannotated | 1.485688 | 3.072077 | 204.154 | 133,054,464 | 35.114 | 31,815,958 | 32,463,843 |
| 12.05 / main | Inline | 1.358216 | 2.904734 | 204.140 | 132,976,640 | 38.409 | 31,815,958 | 32,463,843 |

### Paired effects and controls

Each ratio is computed within a measured pair before taking its median. The
reverse confirmation intentionally swaps the executable roles; its numbers
must not be read as `inline / unannotated`.

| Series | Ratio direction | Measured pairs | Paired wall ratio | Paired CPU ratio | Paired RSS ratio |
| --- | --- | ---: | ---: | ---: | ---: |
| 12.09 checkpoints | Inline / unannotated | 7 | 0.879335 | 0.983599 | 0.997995 |
| 12.05 main | Inline / unannotated | 7 | 0.972847 | 0.949747 | 1.015324 |
| Identical inline executable, 12.09 checkpoints | Same binary / same binary | 5 | 0.974209 | 1.039398 | 0.997303 |
| Reverse confirmation, 12.09 checkpoints | **Unannotated / inline** | 7 | 1.043695 | 0.983101 | 0.998694 |

The identical-binary control itself showed an apparent 2.58% wall reduction
and 3.94% CPU increase. Its individual wall ratios ranged from 0.886704 to
1.062176, and CPU ratios from 0.886007 to 1.075661. The second input's small
paired wall reduction overlaps that control variation.

The reverse confirmation retains a weak wall direction favoring inline, but
its CPU direction reverses: expressed as inline/unannotated, the paired CPU
ratio is about 1.0172. Only four of its seven pairs favor inline in wall time,
and only three favor inline in CPU. The original 12.09 series favored inline
in five of seven wall pairs and four of seven CPU pairs; 12.05 favored it in
four of seven wall pairs and six of seven CPU pairs. No outlier was removed.

For completeness, the control and reverse marginal medians were:

| Series / role | Wall s | CPU s | CPU use % | Peak RSS bytes | Input MiB/s |
| --- | ---: | ---: | ---: | ---: | ---: |
| Control / first label, inline | 1.670501 | 4.136004 | 250.101 | 195,891,200 | 34.833 |
| Control / second label, same inline | 1.624042 | 4.298956 | 258.801 | 195,514,368 | 35.830 |
| Reverse / baseline label, inline | 1.575242 | 4.102078 | 252.814 | 194,510,848 | 36.940 |
| Reverse / candidate label, unannotated | 1.737708 | 4.227251 | 251.502 | 195,178,496 | 33.486 |

These observations do not prove that inlining is harmful or universally
neutral. They leave its real-input benefit unresolved. The attribute is
therefore removed from the proposed integrated source. There is no accepted
speedup, CPU saving, RSS improvement, or competitor-performance claim from this
experiment. Earlier favorable measurements remain historical evidence in the
profiling report and are not substituted for this decision.

## Correctness and scope

All **60 complete exports** passed the harness's process, counter, and
preservation gates. Every run matched the exact Parquet bytes and all normalized
manifest values. This covers 668 individual Parquet-file fingerprints across
the runs: 13 files per checkpoint export and six per main-only export. All
three 12.09 series also have the same full output fingerprint. No input,
executable, or harness hash changed during a series.

Fresh Rust 1.86 optimized movement tests passed **22/22**, with zero failed or
ignored tests. Movement all-target/all-feature Clippy with warnings denied and
workspace rustfmt passed. These focused Linux checks do not claim a fresh full
workspace or Windows execution of the withdrawn candidate. The integration's
ordinary CI and the independent native/Reader QA retain their own scope.

The candidate modifies no protocol rule, and the final proposed source has
**zero runtime diff** from the unannotated native baseline. Public replay
originals and the old native binary are unchanged. No private archive input,
reference-parser execution, release, package publication, or main merge was
part of this experiment.

## Reproduction and retained evidence

- [Build, source, assembly, and focused-test receipt](evidence/phase-a-inline-build.json)
- [Exact candidate patch](evidence/phase-a-inline-candidate.patch)
- [Decision and aggregate provenance](evidence/phase-a-inline-decision.json)
- [12.09 checkpoints, original order](evidence/phase-a-inline-1209-checkpoints.json)
- [12.05 main](evidence/phase-a-inline-1205-main.json)
- [Identical-executable control](evidence/phase-a-inline-same-binary.json)
- [12.09 checkpoints, reversed labels](evidence/phase-a-inline-1209-reversed.json)

Pin the preserved experiment source in an isolated checkout. Obtain the patch
from the accepted report revision, reverse it for the unannotated build, then
apply it for the candidate build in the same directory. Freeze both outputs:

```bash
git show <report-revision>:docs/evidence/phase-a-inline-candidate.patch > /private/inline.patch
git switch --detach <experiment-revision>
git apply -R /private/inline.patch
cargo +1.86.0 build --release -p vrfkit --locked
cp target/release/vrfkit /private/vrfkit-unannotated
git apply /private/inline.patch
cargo +1.86.0 build --release -p vrfkit --locked
cp target/release/vrfkit /private/vrfkit-inline
```

Use new work and report paths for every series. The exact command shape used
for the primary series was:

```bash
python tools/benchmark_resources.py \
  --baseline-exe /private/vrfkit-unannotated \
  --candidate-exe /private/vrfkit-inline \
  --replay /public/12.09/sample-1.replay \
  --work-dir /private/new-1209-work \
  --output /private/new-1209-report.json \
  --warmups 1 --repeats 7 --checkpoints
```

For 12.05, change the input and omit `--checkpoints`. For the control, supply
the exact same inline path in both executable arguments and use five measured
pairs. For the reverse confirmation, exchange the two executable arguments,
keep 12.09 and checkpoints, and retain seven measured pairs. Input and output
checks remain enabled in every command.

Further optimization research is deferred. A more stable measurement setting
or better real-input attribution is needed before accepting this candidate or
changing the measured Parquet hot paths. This record does not start Phase B.
