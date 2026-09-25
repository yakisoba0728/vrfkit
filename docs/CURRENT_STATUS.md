# Current status

The latest common verification run is dated 2026-09-25 and covers all 24
supported builds. See [BUILD_VERIFICATION.md](BUILD_VERIFICATION.md) for
per-build replay counts, the shared acceptance criteria, and resolved findings.
All 986 unique replays now pass: the ActiveBlinds empty-delta and null-actor
fixes eliminate the earlier 81-file failures and recover 522 typed children.
The [README support table](../README.md#supported-valorant-builds) reports
strictly clean replays against all replays checked, using that same report.
Current workspace test counts are also maintained in the README.

The 2026-09-24 [build support update](LEGACY_BUILD_SUPPORT.md) recovered
11.06--12.09 transforms and validated all 48 available samples. The
2026-09-23 [upstream parity update](UPSTREAM_PARITY.md) added 13.06 and scoped
ability decoding. The new common audit includes these changes; the older
physical field inventory below retains its own date and parser revision.
Start with [DATA.md](DATA.md) for the schema and [USAGE.md](USAGE.md) for commands.

## Historical physical field inventory, 2026-09-09

The accepted 2026-09-09 inventory covered 714 replays and reported:

| Table | Physical rows | Rows with a typed value |
|---|---:|---:|
| Main `fields` | 1,020,577,224 | 726,993,845 |
| `checkpoint_fields` | 285,420,158 | 220,648,387 |
| Combined | 1,305,997,382 | 947,642,232 |

The combined typed-value presence is 72.5608%. This counts physical rows with
at least one non-null typed value column. It is not semantic coverage, gameplay
accuracy, or a percentage of known field meanings. Earlier percentages in
phase documents are dated measurements of earlier parser states and remain
historical.

That untyped-row inventory includes named properties and whole RPC payloads
without an accepted type, anonymous or checksum-unresolved fields, structured
or nested raw parents whose synthesized children may be only partly
understood, and decoded movement rows whose input bytes are not duplicated in
`raw_bits`. These categories need separate evidence and denominators; they
must not be collapsed into one
semantic backlog percentage.

Across the two field tables, the inventory contains 36,839 table/group/field/
checksum/build catalog keys. It records 187,106,485 untyped rows with preserved
raw payloads, 3,075,937 zero-bit markers, and zero preserved rows with a wrong
raw length. The 168,172,728 nonempty untyped rows without duplicated `raw_bits`
are all on the decoded movement route. These are transport and catalog counts,
not counts of distinct gameplay facts or meanings.

The measured starting point for follow-up work is
[CURRENT_RAW_BACKLOG.md](CURRENT_RAW_BACKLOG.md). Its ranked populations are an
investigation inventory, not a completed semantic partition or a claim that
each catalog key is one distinct field meaning.

The subsequent [GAS and PatchVolume wire investigation](GAS_AND_PATCHVOLUME_INVESTIGATION.md)
recovers numeric FastArray structure from all 2,882,152 AbilitiesAndBuffs inner
windows in those exports. The public
`tools/extract_fastarray_observations.py` command writes a separate GAS
observation stream containing numeric headers, item IDs, and raw property
boundaries while preserving the original bits. It does not add typed values to
`fields.parquet` or `checkpoint_fields.parquet`, so the accepted Parquet counts
and typed-presence ratio above remain unchanged.

The investigation also records private structural evidence that the same
generic walk fully consumes 26,303 selected PatchVolume windows. PatchVolume
does not yet have a public extraction route or an established item/property
schema. In both populations, numeric boundaries do not establish gameplay
meanings.

## Completed evidence phases

The main completed phases are recorded in
[TRANSPORT_PRESERVATION.md](TRANSPORT_PRESERVATION.md),
[PARTIAL_HEADER_CORRECTION.md](PARTIAL_HEADER_CORRECTION.md),
[CHECKPOINT_SCHEMA_PRESERVATION.md](CHECKPOINT_SCHEMA_PRESERVATION.md),
[CHECKPOINT_PATH_RESOLUTION.md](CHECKPOINT_PATH_RESOLUTION.md),
[STRUCTURED_ARRAY_EXPANSION.md](STRUCTURED_ARRAY_EXPANSION.md),
[NESTED_ARRAY_REFERENCES.md](NESTED_ARRAY_REFERENCES.md),
[TEXT_HISTORY_EXPANSION.md](TEXT_HISTORY_EXPANSION.md),
[REFERENCE_VALUE_EXPANSION.md](REFERENCE_VALUE_EXPANSION.md),
[TARGETING_AND_HEAL_VALUES.md](TARGETING_AND_HEAL_VALUES.md),
[HEALING_OBSERVATIONS.md](HEALING_OBSERVATIONS.md),
[KILL_OBSERVATIONS.md](KILL_OBSERVATIONS.md),
[KILL_LEDGER.md](KILL_LEDGER.md),
[SECTION_OBSERVATIONS.md](SECTION_OBSERVATIONS.md),
[SECTION_TIMELINE.md](SECTION_TIMELINE.md), and
[SECTION_PACKET_TIMELINE.md](SECTION_PACKET_TIMELINE.md). Each document states
its own evidence boundary; derived section and kill views do not prove game HP,
healing attribution, damage attribution, causality, or player credit.

The parser acceptance commit for the physical field counts was `fc50bfe`.
Later derived-view commits did not change those recorded counts. They remain
a dated physical-row inventory, separate from the current build verification.
