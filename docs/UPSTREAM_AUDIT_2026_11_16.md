# Upstream audit - 2026-11-16

This inventory compares the previously audited ValorantReplayParser revision
`400f7295964a7f6486378eac6decda3cefe42acb` with its latest default-branch revision
`fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8`, checked through the GitHub API and a
fresh Git fetch on 2026-11-16. The default branch is `main`. The range contains
four commits (three implementation commits and one merge), changing 16 final
paths. No commit or final changed path in this range was excluded from the
inventory. The local comparison baseline is vrfkit
`94fcf11376829199ae648edda360b593840fd6a7`, after PR #45.

Source-code evidence and local raw-window measurements are distinguished below.
A C# descriptor declaring a name and type alone is not replay evidence. The
checkpoint-enabled before/after results are recorded separately below.

## Behavior decisions

| Upstream behavior | Current vrfkit state | Decision |
|---|---|---|
| Global 13.10 payload transform | PR #45 registered the same addend `0x2c249eb3`, positive offset `0x33`, tail XOR `0xb3`, and equivalent 64/32/8-bit operations. All 11 upstream fixture outputs, shared input bytes, actor GUID 2 and seed rule match `recovered_1310_vectors.rs`. | Already implemented. Record the independent upstream native capture provenance rather than porting another transform. |
| 13.10 CombatReport nested handle changes | `COMBAT_ROUNDS_SCHEMA_1310` selects dealt regional handle 45, received handle 62 and received regional handle 81. The bundle now resolves shifted leaf names through the replay's declarations and normalizes regional containers to `Regions`. | Already implemented and expanded in PR #45. Do not apply another fixed-handle rename. |
| New CombatInteraction `DamageIcon` | Upstream adds an ObjectNetGuid after `DamageType` at dealt handle 36 and received handle 72. At the comparison baseline these leaves are raw. The local corpus supplies 84,867 exact 16-bit packed references with nonzero values, resolving to texture paths. | Ported as an exact CombatReport group/name overlay pin through `apply_type_corrections.py`. The bundle recognizes the declared `DamageIcon` name at either handle. Raw windows remain present. |
| New damage-response `DamageIcon` | Upstream adds ObjectNetGuid at damage response handle 9, with later fields shifted by one. The local wire declares the inserted member as `DamageIconOverride`, and its observed raw windows are all zero. Named-parameter resolution already avoids shifting declared names; PR #45 supplies damage life-change members 11-14. | No icon typing port for this RPC member. All-zero windows do not establish the claimed reference type or its meaning. Preserve the original name and raw bytes. The residual unnamed-handle fallback risk remains separate. |
| Temporary-death response layout | C# selects the DamageIcon-aware response layout with offset 2 for Base and Point temporary-death RPCs. Local inspection covers 23 calls and 601 named parameter rows. Five 13.10 Point array parents contain three elements each, yielding 60 independently checked member leaves at handles 13-16. | Ported only for exact global 13.10 `MulticastReceivePlayerTemporaryDeathEvent_Point.LifeChangeEvents`. Base, Downed, older builds, future builds and other parameter names stay raw. The two observed 13.08 array parents remain raw. |
| CombatReport timestamps, respawn/death envelope | C# changes internal fixed offsets for combat index 96 -> 98, resurrector 98 -> 100 and subsequent fields. vrfkit names ordinary leaves from replay declarations. PR #45's bundle correction resolves recognized shifted envelope fields from those declarations. | No corresponding fixed-offset code to port for named leaves. New enriched timestamp models would be a separate feature and need measured semantics. |
| C# constructor/versioned-definition plumbing | Five parameter constructors pass `hasDamageIcon` to shared definitions; descriptor catalog selects those definitions at 13.10. | Architecture-specific. The behavioral parts are inventoried above. |
| China build support | No China file or behavior changes in this four-commit range. The earlier China 13.09 change remains outside vrfkit main's supported exact global branch strings. | No new China port. Previous native-vector and replay requirements remain unchanged. |

## Transform evidence

Upstream commit `093fe9b` says its 13.10 fixtures were captured by emulating the
arbitrary-length native routine at RVA `0x432eb50` on private buffers. The existing
vrfkit record independently captured only the 1/7/8/31/32-bit cases through the
integer reader at RVA `0x432e600`; its longer expected outputs were reconstructed
from the disassembled operations. The upstream fixture now supplies a separate
published native-capture claim for the longer cases, with exactly matching
outputs at 63/64/65/287/288 bits as well. This is source attribution to upstream;
this audit did not execute the game binary or independently repeat that capture.

The exact shared test input is
`BFDF6F9EA1F27BA00000C66EAFAF2E0000339C0DD34B0C45C48063038003562A43C0C949`.
The seed is `bit_count ^ 2`. The compared bit counts are
`0, 1, 7, 8, 31, 32, 63, 64, 65, 287, 288`.

Sources:
[13.10 transform](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/Replay.Encoding/PayloadEncryption/VersionedTransforms/ValorantSeededTransform13_10.cs),
[native fixture](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/tests/Replay.Encoding.Tests/PayloadEncryption/ValorantSeededTransformTests.cs).

## DamageIcon and damage layouts

The new CombatReport model exposes `DamageIcon` separately from `DamageType`;
it does not establish that the icon reference is a weapon, a damage source, or
player credit. `CombatReportHandles.Release1310` records received interactions
62, combat index 98 and resurrector 100. `RegionsOf(26)` is 45 and `RegionsOf(62)`
is 81. The source's public recorded fixtures independently expect DamageType
4815, DamageIcon 4831, region 0, Hits 1 and Damage 26 for both dealt and received
interactions.

`DamageParameters.LayoutHandle` adds one at the inserted handle boundary, after
applying the older pre-12.10 KillCreditPlayerState rule. Its shared fields put
DamageIcon at 9 + offset and LifeChangeEvents at 10 + offset. Its nested
life-change constructor receives offset + 1. Temporary-death Base and Point use
offset 2. These are C# handle assignments, not proof that vrfkit's unknown handle
fallback is safe on every legacy or 13.10 replay.

The upstream recorded Point damage fixture from replay
`df095e3a-4bcf-49e8-a91b-00835f61e0dd`, packet 16157, channel 62, expects DamageIcon
4195, ChangedComponent 2741, LifeResult 0 and DeltaLife -120. This public fixture
can support a cross-implementation regression with attribution; do not publish
additional bytes from local private replays just to create a fixture.

Sources:
[CombatReport layout and DamageIcon](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/Replay.Valorant/GameState/CombatRoundReports.cs),
[damage response layout](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/src/Replay.Valorant/Combat/DamageParameters.cs),
[recorded damage regression and release boundaries](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/tests/Replay.Valorant.Tests/Combat/DamageRpcLayoutTests.cs),
[recorded dealt/received and character-envelope regressions](https://github.com/michel-giehl/ValorantReplayParser/blob/fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8/tests/Replay.Valorant.Tests/GameState/CombatRoundReportsDecoderTests.cs).

## Local implementation and measured limits

The pre-change inspection reads 29 local exports: 27 global 13.10 replays,
one 13.09 and one 13.08. Main and checkpoint observations are kept separate
when determining whether a route was observed; an absent route does not pass
by contributing a zero error count.

### CombatReport icons

Across the main and checkpoint populations, 84,867 CombatReport `DamageIcon`
windows consume exactly as 16-bit packed NetGUIDs. Every decoded value is
nonzero and resolves through the export's GUID table to a texture path. This
establishes the encoded reference type and the observed destination category,
not that the reference identifies a weapon, player or source of damage.

The implementation adds one exact identity to `ADDITIONS` in
`tools/apply_type_corrections.py`:
`/Game/GameModes/Bomb/Bomb_CombatReportComponent.Bomb_CombatReportComponent_C`
plus the declared name `DamageIcon`, typed `ObjectNetGuid`. Both handles use
the replay's declared name; an unrelated group with the same name is excluded.
The maintained overlay adds one entry. Regression coverage checks both handles,
a foreign group and a truncated packed reference. The bundle's name map adds
`DamageIcon`; its regression exercises dealt and received interactions through
the actual converter.

The similarly positioned RPC member is declared `DamageIconOverride` on the
wire. Its observed raw windows are all zero. That population cannot establish
that it carries packed object references, so it receives no new overlay entry,
renaming or invented value.

### Temporary-death Point arrays

The main-stream temporary-death inspection covers 23 calls and 601 named
parameter rows. Five 13.10 Point `LifeChangeEvents` parents each contain three
elements. An independent raw-parent walk checks the four members of all fifteen
elements: handles 13, 14, 15 and 16 map to `ChangedComponent`, `LifeResult`,
`DeltaLife` and `bAliveAfterChange`, yielding 60 member leaves. The reference,
float and boolean readers must consume each declared window exactly.

`LIFE_CHANGE_TEMPORARY_DEATH_POINT_SCHEMA_1310` supplies that layout.
`life_change_schema_for_param` selects it only for exact global 13.10,
`MulticastReceivePlayerTemporaryDeathEvent_Point` and `LifeChangeEvents`.
The ordinary packet and checkpoint passes both use that branch selection.
The existing additive emitter keeps the raw parent, emits raw child windows,
and records malformed array walks or typed child failures through the existing
array counters. No raw parent is replaced by its interpreted children.

Synthetic regressions exercise the real RPC parameter walker: positive typed
values and raw-parent preservation; refusal on 13.08, 13.09, a future build,
Base, Downed and a different parameter; and truncated or wrong-width payloads
that must preserve raw input while reporting failure. Two observed 13.08 array
parents remain raw. No new route is admitted merely because its C# descriptor
resembles Point's. Checkpoint support follows the same code path; it is not
replay evidence for a temporary-death event absent from checkpoint snapshots.

### Verification results

The final build passes the strict checkpoint-enabled gate on all **29/29**
local replays (27 on 13.10, one on 13.09 and one on 13.08), and on the three
pinned public 12.10/12.11/13.00 replays. The local run scores
**13,579,265/13,579,265** main blocks, walks **396** checkpoint chunks, and
has zero array errors, truncations or leaf decode errors. The executable is
unchanged at completion. The [machine-readable record](../tools/fixtures/upstream_audit_2026_11_16.json)
pins the Rust source digest, executable SHA-256, replay hashes and counts.

An independent Python parent-array walk matches **3,372,901** emitted leaf
paths, widths and raw windows and **889,409** typed values. This includes
all **84,867** newly typed icons and **60** new temporary-death values, in
addition to the previous 804,482 comparisons. The five temporary-death parents
are observed on two Bomb and three Swiftplay calls. Six negative controls
(wrong typed value, omitted child and wrong child bytes for each new data
population) are all detected, and empty input is rejected.

A separate streaming before/after comparison checks every column, row order,
raw window and floating-point bit of **31,837,660 existing field rows**.
Only the new icon `value_i64` cells and the 60 added temporary-death children
differ. The other **323 Parquet files** are byte-identical, including all
13 tables for each older replay. Manifest JSON differs only in elapsed time
and the expected aggregate increments: 60 emitted fields, 60 array fields
and 15 array elements. Typed-value, raw-byte and signed-zero mutation controls
confirm that unrelated changes fail this comparison.

CombatReport icon values are populated on flattened child rows; the new
temporary-death values also come from an additive child pass. These changes are
separate from the top-level overlay's `Decoded OK`/raw buckets. Do not infer
typed-child counts from a change, or absence of change, in those buckets.

The legacy unnamed damage-parameter fallback remains unverified for the
pre-12.10 shift recorded in the previous audit. A named field resolving by its
own descriptor does not prove that an unnamed field's fixed handle is correct.
This change neither relaxes that boundary nor claims to resolve that risk.
The private 11.06-12.09 corpus (48 replays) has since been measured: no damage
or temporary-death parameter arrived unnamed there, so the fallback is
unexercised rather than verified
([measurement](UPSTREAM_AUDIT_2026_11.md#legacy-corpus-measurement)).

## Complete commit ledger (4)

| Commit | Author date UTC | Subject | Classification |
|---|---|---|---|
| `093fe9b6bb2db2018b4e1edfa10cb4a0d4e6cac9` | 2026-11-11 09:51:56 | feat: 13.10 transform | Already implemented; native fixture provenance can be strengthened. |
| `d883009ca08dbc9cc014e63b285fdf5d821cbf3d` | 2026-11-11 10:01:36 | feat: decode 13.10 combat reports | Array shifts already implemented; measured CombatReport DamageIcon typing added. |
| `e97409a4445bd98a727820527e622b09bf156223` | 2026-11-11 10:51:41 | fix: select the 13.10 damage response layout | Damage array shift already implemented; measured 13.10 temporary-death Point expansion added, other routes and DamageIconOverride remain raw. |
| `fbf24b62c5a8b8b7a64df71b4cbaaf0a651ce5b8` | 2026-11-11 11:28:21 | Merge pull request #24 from michel-giehl/release-13.10 | Merge of the above behavior; no additional final changed path. |

## Complete final-state changed-path inventory (16)

| Status | Path | Disposition |
|---|---|---|
| M | `CHANGELOG.md` | Documentation of the three behaviors above. |
| M | `src/Replay.Encoding/PayloadEncryption/PayloadTransformRegistry.cs` | Already registered in Rust. |
| A | `src/Replay.Encoding/PayloadEncryption/VersionedTransforms/ValorantSeededTransform13_10.cs` | Equivalent transform already present. |
| M | `src/Replay.Valorant/Combat/DamageParameters.cs` | Named field paths and damage array shift already present; DamageIconOverride stays raw; older unnamed fallback limit retained. |
| M | `src/Replay.Valorant/Combat/DamageableComponentClassNetCacheDescriptor.cs` | C# versioned binding selection; Rust exact 13.10 gate already present for the expanded damage arrays. |
| M | `src/Replay.Valorant/Combat/MulticastNotifyDamageBaseParameters.cs` | Constructor plumbing for shared DamageIcon layout. |
| M | `src/Replay.Valorant/Combat/MulticastNotifyDamagePointParameters.cs` | Constructor plumbing for shared DamageIcon layout. |
| M | `src/Replay.Valorant/Combat/PointDamageParameters.cs` | Constructor plumbing for shared DamageIcon layout. |
| M | `src/Replay.Valorant/GameState/BombCombatReportComponentDescriptor.cs` | Measured DamageIcon pin added; versioned array selection already present. |
| M | `src/Replay.Valorant/GameState/BombGameStateClassNetCacheDescriptor.cs` | C# version selection is architecture-specific; only measured global 13.10 Point array expansion added in Rust. |
| M | `src/Replay.Valorant/GameState/CombatRoundReports.cs` | Existing array-shift parity and measured DamageIcon pin; C# timestamp envelope model is separate. |
| M | `src/Replay.Valorant/GameState/MulticastReceivePlayerTemporaryDeathEventBaseParameters.cs` | Constructor plumbing; Base remains raw without equivalent array evidence. |
| M | `src/Replay.Valorant/GameState/MulticastReceivePlayerTemporaryDeathEventPointParameters.cs` | Constructor plumbing; measured Point array behavior implemented on exact 13.10 only. |
| M | `tests/Replay.Encoding.Tests/PayloadEncryption/ValorantSeededTransformTests.cs` | All 11 new vector outputs already match; published native provenance recorded above. |
| A | `tests/Replay.Valorant.Tests/Combat/DamageRpcLayoutTests.cs` | Public exact-byte damage example and version-boundary evidence. |
| M | `tests/Replay.Valorant.Tests/GameState/CombatRoundReportsDecoderTests.cs` | Public dealt/received, envelope and negative old-schema examples. |

## Reproduction and validation scope

The strict run uses the same command and acceptance rule as the common build
audit, with new output paths for the same 29 SHA-256-identified inputs:

```powershell
cargo +1.86.0 build --release -p vrfkit --locked
python tools/verify_build_corpus.py --exe target/release/vrfkit.exe --corpus '<replay-root>' --work-dir '<new-audit-root>' --output '<new-report.json>' --jobs 6
```

The independent array and streaming preservation checkers, their source
checksums, negative controls and detailed reports are retained with the private
corpus. Their aggregate results and input hashes are in the public record
linked above; no additional private replay bytes are published here.

Rust 1.86 all-feature workspace tests pass (744 on Windows), as do format,
all-target/all-feature Clippy with warnings denied, strict rustdoc, core-only
CLI/container tests, all 27 feature cases and Rust/Python Parquet interop.
Generator, ASCII, baseline and fast documentation checks pass. The full local
Python guard executes the 1,081-test inventory but cannot create symlinks under
this Windows account (`WinError 1314`); the complete suite is also exercised
by the PR's Windows/Ubuntu Python CI matrix.

The previous audit's legacy unnamed damage-handle question remains open in
the sense that matters: the fallback has no unnamed input to verify on the
48-replay legacy corpus (0 rows), so its correctness is untested, not proven.
Unobserved Base/Downed arrays, zero-only overrides and absent checkpoint
temporary-death calls are not counted as verified behavior.
