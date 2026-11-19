# Phase C gameplay author checks

`author_checks.json` binds these checks to the four relevant Rust source
hashes and the freshly built example binary. This is the author checkpoint;
independent acceptance and final platform CI remain separate required gates.

The original formatted worktree survived an execution-environment disconnect.
Before resuming, its five candidate files were compared with the isolated
`phase-c/gameplay-identity-unverified` backup at
`0e40d8c86623b4d1f0a78be9395fa213e54cc244`. Existing gameplay.rs and the API
document were byte-identical. The three new Rust files differed only by
Rust 1.86 formatting. The backup remains an immutable historical checkpoint;
the recovered original source was tested and committed here. The previously
interrupted compiler attempt is not counted as a completed build or test.

## Executed checks

- 15 new identity/lifecycle/movement unit tests passed.
- 12 existing native/phase gameplay integration tests passed.
- The core-only aggregate example built successfully.
- Core-only all-targets strict Clippy and strict rustdoc passed on Rust 1.86.
- Four implementation mutations each produced an actual targeted assertion
  failure, not a compilation failure: accepting a raw tail, admitting a
  same-packet future write, destroying dormant actors, and trusting a repeated
  open while the actor was active. Exact source bytes were restored, then all
  15 identity tests passed again.

The independent source review found the repeated-active-open case. The
implementation now retains conflicting lifecycle evidence rather than
assuming continuity; the regression test covers the same and another channel.

Local commands used one job, no debug information, and opt-level 0 to fit the
available environment. These are functional checks, with no performance
measurement or comparison:

```bash
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export CARGO_PROFILE_DEV_OPT_LEVEL=0 CARGO_PROFILE_TEST_OPT_LEVEL=0
cargo +1.86.0 test -p vrfkit --no-default-features --locked --lib gameplay::identity::tests
cargo +1.86.0 test -p vrfkit --no-default-features --locked --test gameplay
cargo +1.86.0 build -p vrfkit --no-default-features --locked --example gameplay_identity
cargo +1.86.0 clippy -p vrfkit --no-default-features --all-targets --locked -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo +1.86.0 doc -p vrfkit --no-default-features --no-deps --locked
```

The executed Cargo commands also supplied
`--config 'profile.dev.package."*".opt-level=0'` and, for test/all-targets
commands, `--config 'profile.test.package."*".opt-level=0'` explicitly.
No tracked Cargo profile or dependency changed.

## Fresh public example

The built `gameplay_identity` example read the pinned public 12.10 replay with
SHA-256 `a4ce1b72f9bdf99492162013c1c909e6994a0d22bef1899e687fde71fbc86606`.
It completed with one validated SpawnedCharacter declaration, 8,907 resolved
movement associations, zero unresolved/ambiguous/null associations, seven
explicitly skipped checkpoints and a `Passed` block verdict. The example
prints aggregates only, and the receipt includes its binary hash and stdout.

This demonstrates a working public Reader callback path, not an independent
proof of all identity values or complete game semantics. The six-replay
independent raw/export comparison, output preservation and final integration
CI must still complete before acceptance. Checkpoint identity, account joins,
native death/player timestamp joins, kill credit and full round outcomes
remain outside this optional main-stream API.

## Lifecycle follow-up from independent public-API tests

`lifecycle_followup.json` binds a later correction to its own source hashes.
Three public-API tests from commit
`57f992575989e03d94c33ad4c5a89772fcd08494` were preserved byte-for-byte in
`crates/vrfkit/tests/gameplay_lifecycle.rs`. Against author commit
`5acf6c0233678e2c4a68d72de1e05b2279b43b62`, two failed and one passed:
declarations from a retired PlayerState or character lifetime could wrongly
make a fresh relation ambiguous. The same-packet uncertainty test already
passed.

Candidate selection now excludes a declaration strictly earlier than the
current lifetime's prior end. Equal-packet declarations remain possible
conflicts because the separate lifecycle/field vectors do not establish
their order. A lone retired claim remains `Unresolved(PreviousLifetime)`;
historical queries and raw evidence are retained.

After the correction, all three preserved tests, all 15 identity unit tests
and all 12 prior gameplay tests passed. Core all-targets strict Clippy and
strict rustdoc also passed. An additional mutation changed the strict cutoff
to an inclusive cutoff; the same-packet ambiguity test failed by assertion
as required. Exact source was restored and the three public-API tests passed
again. The earlier aggregate example and four-mutation receipt remain
historical checks of their recorded hashes; final independent replay parity
and platform CI must use this corrected source.
