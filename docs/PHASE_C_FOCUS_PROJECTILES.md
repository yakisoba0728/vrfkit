# 0.6.0: the 12.05 FocusProjectiles array route

0.6.0 decodes Clay's `FocusProjectiles` array into child rows on build 12.05
only. This page states what the route accepts, what it emits, which counters
report it, and the evidence behind it.

## Evidence and support boundary

The `Clay_PC_C.FocusProjectiles` route has bounded public-input evidence.
The route is enabled only for the exact branch string
`++Ares-Core+release-12.05` and group
`/Game/Characters/Clay/Clay_PC.Clay_PC_C`. Both declarations must be present:

| Role | Handle | Declared name | Compatible checksum |
| --- | ---: | --- | ---: |
| Parent array | 45 | `FocusProjectiles` | 957945936 |
| Object reference member | 46 | `FocusProjectiles` | 831657485 |

The independent check re-read an earlier export of one public 12.05 replay;
it does not claim a new full-corpus audit.

That input contains 102 main parents: 53 populated deltas and 49 empty
deltas. Three empty deltas carry one extra zero byte. The 53 member windows
are 16 or 24 bits and independently decode to NetGUIDs matching actor-open
records for the expected projectile archetypes. The final GUID path map
does not resolve these references, so no final-path lookup or gameplay
credit is inferred. Eighteen checkpoint parents are ordinary empty deltas;
no populated checkpoint child is observed in this fixture.

Other builds remain unsupported by this route, and on 12.05 the route
applies only to the Clay group above.

## Decoder and output contract

The observed grammar is a capacity followed by changed indices, each with
exactly one member at encoded handle 47. The member is one fully consumed
IntPacked NetGUID. Each element and the array require their own explicit zero
terminators. Indices must be distinct and below capacity. The existing array
capacity limit still applies.

A dedicated preflight admits one exactly eight-bit zero suffix only for an
empty delta. It rejects nonzero or truncated trailers, populated-array
suffixes, unexpected or duplicate members, duplicate or out-of-range indices,
zero/invalid member widths, overflowing GUIDs, and incomplete terminators.
The generic array grammar is unchanged. The exact existing array walker then
supplies the raw child windows. The full array must pass before any child is
emitted. Rejected payloads retain their parent and increment the existing
array-error counter. A parent that reaches the route by group, name and
checksum but whose parent handle is not 45, or whose handle-46 member name or
checksum differs, stays raw, emits no child and increments
`focus_projectiles_declaration_refusals`.

Each accepted member adds a row immediately **before** its original parent,
following the existing array-output order:

- `field_name`: `FocusProjectiles[index].FocusProjectiles`, using the wire index.
- `handle`: 46; `compatible_checksum`: null, as for other additive children.
- `raw_bits` and `bit_count`: the exact member window.
- `value_i64`: the decoded NetGUID; other typed columns are null.
- Packet, time, channel, actor, object and group: the parent's exact context.

The original parent row, raw bytes, checksum and existing values remain
unchanged. An empty delta adds no child. A zero-width field retains the
existing absent-raw representation and does not invent an empty array.

## Counter and baseline migration

Three additive sink counters are present in both main and checkpoint
manifest blocks, including zero values. The fixture column records the
evidence run, which predates the refusal counter:

| Counter | Meaning | Expected main / checkpoint on the evidence fixture |
| --- | --- | ---: |
| `route_children_focus_projectiles` | Accepted additive child rows | 53 / 0 |
| `focus_projectiles_empty_trailers` | Accepted empty deltas with one final zero byte | 3 / 0 |
| `focus_projectiles_declaration_refusals` | Routed parents refused for a parent-handle or member-declaration mismatch | not recorded |

The `Route children` summary gains a final `focus projectiles` value.
`FocusProjectiles trailers: N empty deltas` and
`FocusProjectiles refused: N declaration mismatches`, with their checkpoint
counterparts, are separate summary lines. Current summary parsers require these counts, and the
export-baseline checker reconciles them against the manifest. Missing lines,
missing manifest keys and mismatches remain failures.

The expected 12.05 change is 53 additional main field rows, and increments of
53 to `fields_emitted`, `array_elements_decoded` and `array_fields_emitted`.
The native movement counter contract is unchanged. A comparison must remove
only the independently verified new child rows and compare every original
row, column and order; changed Parquet bytes alone are not a regression or
an equivalence proof. Other stream data and previous typed values must still
match.

Historical private baselines are left byte-for-byte unchanged. Their
legacy-only schema excludes these new counters because the historical run
never measured them. The exclusion does not apply to current baselines and
does not convert missing counts to zero. Existing obsolete native-counter
baselines still require genuine remeasurement before current comparison.

## Local verification

When the route was added:

```sh
cargo +1.86.0 test -p vrfkit --no-default-features --locked --lib sink::
cargo +1.86.0 clippy -p vrfkit --no-default-features --all-targets --locked -- -D warnings
python -W error -m unittest discover -b -s tools/tests -p 'test_*.py'
```

The core sink suite passed 102 tests; the Python suite passed 1,181 tests.
Four new Rust tests exercise exact values/order/raw preservation, empty
trailers, exact declaration/build gates, and malformed/all-truncated windows.
Three new Python tests require the new current counters, reject manifest
disagreement or absence, and preserve the legacy/current schema separation.
The existing all-build route matrix also covers this ninth route.
Strict core Clippy, rustfmt, ASCII and the unchanged historical-baseline
schema checks passed.

Mutation control: removing only the member-checksum gate caused
`focus_projectiles_requires_both_exact_declarations_and_the_observed_build`
to fail on checksum 831657484: two rows were emitted where only the raw
parent was allowed (exit 101). Restoring the exact original source made the
test pass (exit 0). No mutant is retained.

The local checks above do not substitute for a public replay export, an
exact fresh output comparison, full-feature builds or Windows/Linux CI. No
performance measurements were run.
