# External Reader API verification

This is a standalone source consumer of the path-dependency example in
`docs/RUST_API.md`. It is outside the vrfkit workspace, declares Rust 1.86 and
edition 2024, disables vrfkit's default features, and does not add any other
Rust dependency. `publish = false` prevents accidental registry publication.

The actual run used verification source commit
`93e107a10788021b59fb30c5cb145960c3fd21a0`, tree
`e9c320bb96fa9e1d3c736913847b58353ad21735`, with a freshly generated external
Cargo lockfile. This is the local combined verification snapshot; publication
commits can have different metadata. The report retains the source tree and
API/manifest hashes rather than requiring that local commit to exist remotely.

## Assertions and result

The program uses the documented `ReplayReader::read_path` callback and
unknown-overlay field inspection. It reads one public build 13.00 fixture with
SHA-256 `fd49091dd43171bb060eb6bbae50ed6677aa1077344572c5bf65f0c6fe2b4c1a`.
The expected six checkpoint chunks come from the separately observed fixture
census. They must be reported as skipped, because this API does not decode
checkpoint snapshots.

The reader goes out of scope before the detached summary is inspected. The
consumer asserts that every callback row tally matches its summary counterpart,
that exactly one metadata callback ran, that six checkpoint chunks were skipped,
and that the existing ReplayData block verdict is `Passed`. Field callbacks
must be nonempty, preventing a vacuous zero-row pass.

Observed main-stream counts were 12,181 fields, 4,991 movement rows, 98 actor
rows, zero partial rows and six native container events. These counts also
matched the corresponding prior public Parquet tables in a separate Python QA
check. Python/PyArrow was used only to read that reference metadata; it is not
a dependency of the Rust consumer.

The program found 6,577 field rows without a typed overlay and zero reported
overlay decode errors. The block verdict does not establish full semantic
coverage. Output contains aggregate counts only, with no payloads, identifiers
or source replay paths; stderr was empty. The original fixture hash and the
source checkout were unchanged after the run.

Cargo metadata reports one workspace member: this consumer. The normal Cargo
tree contains 49 packages including the consumer, and the resolved graph
contains 69 packages. Neither contains an Arrow, Parquet or Zstd package. The
normal tree is retained without local paths in `normal-dependencies.txt`.

## Reproduction layout and commands

The retained Cargo manifest expects this relative layout under an experiment
directory:

| Relative path | Content |
| --- | --- |
| `worktrees/final-validation/` | Matching vrfkit source checkout |
| `evidence/reader-external-consumer/` | This consumer's source and lockfile |
| `evidence/corpus-links/13.00/fixture.vrf` | Read-only link to the matching public fixture |

The experiment directory can be anywhere: all manifest and command paths are
relative. Copy `Cargo.toml`, `Cargo.lock` and `src/main.rs` together, retaining
the `src/` directory. Run the commands from
`evidence/reader-external-consumer/`, not from the vrfkit checkout. The consumer's
own `[workspace]` declaration keeps it separate even if its new location happens
to be inside another Cargo workspace.

Do not try to fetch the historical local verification commit from GitHub.
Select the published integration revision that contains the Reader API and
compare its API/manifest hashes with `verification.json`. If those hashes or
the source tree differ, record the new revision and hashes: that is a new check,
not a byte-for-byte reproduction of this source snapshot. The fixture must
match the recorded SHA-256 before the expected count of six checkpoints is used.

Keep the retained `Cargo.lock` to reproduce the dependency selection. The
original `generate-lockfile` step is recorded for provenance, but existing
locked reproductions should start with `cargo +1.86.0 build --release --locked`.
The recorded binary command assumes Cargo's default `target/` directory; if
`CARGO_TARGET_DIR` is set in the reproduction environment, use that directory's
`release/vrfkit-reader-external-check` instead.
The exact successful command forms are retained in `commands.txt`; no registry
publication, repository mutation or timing benchmark is part of this check.

`verification.json` contains source, lockfile, executable and reference-table
hashes; the aggregate result; dependency/visibility checks; and the command
record. Raw Cargo logs and metadata in the scratch directory can include local
code paths and are not part of the sanitized report.
