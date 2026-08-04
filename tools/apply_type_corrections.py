"""Apply known type corrections to the generated overlay table.

These corrections represent empirically-verified differences between the C#
descriptor declarations and the actual wire format in build 13.01:
  - Time-related Float fields are actually Double (64 bits on wire)
  - Actor bookkeeping fields "215"/"216" in non-weapon groups are variable-width
    (3 bits in 13.01), not Int32

Run after extract_descriptors.py regenerates table.rs, and BEFORE cargo fmt.

That ordering is load-bearing and used to be silent. Two of the passes below
match one-line literals, which is the shape extract_descriptors.py emits but
not the shape rustfmt leaves behind, so running the corrector on an already
formatted table applies nothing. The old script printed "Applied 0" and
exited 0 for that case -- indistinguishable from "everything was already
correct".

So the script no longer trusts its own operation count. After writing, it
verifies the END STATE of every correction against the parsed table and fails
loudly if any is missing. That check is format-independent: if the
application patterns ever rot again, the verification still catches it.

Usage:
    python tools/apply_type_corrections.py            # apply, then verify
    python tools/apply_type_corrections.py --check    # verify only, no write
"""
import re
import sys
from collections import Counter
from pathlib import Path

TABLE_RS = Path(__file__).parent.parent / "crates" / "vrf-decode" / "src" / "table.rs"

#: (group_path substring, field_name, required FieldType substring).
#: One entry per correction the passes below make. Checked against the file
#: after writing; a miss is a hard failure.
EXPECTED = [
    ("TimedBomb.TimedBomb_C", "TimeRemainingToExplode", "Double"),
    ("TimedBomb.TimedBomb_C", "DefuseProgress", "Double"),
    ("Comp_Ability_CooldownComponent_C", "StartTimeStamp", "Double"),
    ("Comp_Ability_CooldownComponent_C", "CooldownSeconds", "Double"),
]
for _group in (
    "TimedBomb.TimedBomb_C",
    "EquippablePickupProjectile.EquippablePickupProjectile_C",
    "EquippableGroundPickup.EquippableGroundPickup_C",
    "OwnerExclusivePlayerInfo",
    "Projectile_Phoenix_Q_FlameWall_ThroughWall.Projectile_Phoenix_Q_FlameWall_ThroughWall_C",
):
    for _field in ("215", "216"):
        EXPECTED.append((_group, _field, "EnumRemainingBits"))
EXPECTED += [
    ("SmokeScreen", "ReplicatedMovement", "ByteComponents"),
    ("AresEquippableDataTracker", "OriginalBuyerTeam", "Raw"),
    ("MulticastNotifyDamage_Base", "EquippableUsed", "ObjectNetGuid"),
    ("MulticastNotifyDamage_Point", "EquippableUsed", "ObjectNetGuid"),
    ("MulticastNotifyDamage_Base", "DamageOrigin", "VectorNetQuantize { scale: 100 }"),
    ("MulticastNotifyDamage_Point", "DamageOrigin", "VectorNetQuantize { scale: 100 }"),
    ("MulticastNotifyDamage_Point", "DamageImpactLocation", "VectorNetQuantize { scale: 1 }"),
    ("MulticastNotifyDamage_Point", "DamageImpactBoneRelativeLocation",
     "VectorNetQuantize { scale: 1 }"),
    ("MulticastNotifyDamage_Point", "DamageDirection", "VectorNetQuantizeNormal"),
    ("MulticastNotifyDamage_Point", "DamageImpactNormal", "VectorNetQuantizeNormal"),
]

#: Entries the WIRE carries that the C# descriptors cannot declare, because the
#: group did not exist when the reference was pinned.
#:
#: This is a different kind of claim from every correction above. Those say
#: "the descriptor declares X and the wire disagrees". This says "the descriptor
#: is SILENT and here is the type anyway", so the bar is higher and only one
#: case clears it today.
#:
#: `/Script/ShooterGame.BaseTeamState` is new in build 13.02, which deleted
#: `BombGameState.TeamEconomy` and moved team economy into a separately
#: replicated actor. The property NAMES did not change, and the reference
#: declares their types at the pinned commit --
#: `GameState/AresTeamEconomy.cs:11-12`:
#:
#:     public sealed record AresTeamEconomyUpdate(
#:         int Index, uint? ReplicationId,
#:         int? LoadoutValue,            <- Int32
#:         int? AverageLoadoutValue);    <- Int32
#:
#: so this is a descriptor-sourced type for a relocated property, not a type
#: guessed from values. `OwnerExclusivePlayerInfo.{Start,End}OfRoundLoadoutValue`
#: are Int32 in the same reference, which corroborates the family.
#:
#: Corroborated on the wire, not decided by it: all 44+44 payloads are 32 bits,
#: read as little-endian i32 they run 4300/4150 at round 1 to 34300, and
#: AverageLoadoutValue is EXACTLY LoadoutValue/5 on every row (a five-player
#: team).
#:
#: DELIBERATELY NOT ADDED: `Wins`, `Points`, `InitialRole`, `TeamRole`,
#: `TeamPlayerStates`, `TeamComponent`, `TeamExclusiveTeamInfo`. The same
#: 13.02 group declares all of them and the reference declares NONE of them
#: under any group, so there is no source for their types. They keep their raw
#: bits and stay untyped. That line is what makes this addition defensible;
#: widening it by eye would undo the reason it is allowed at all.
#: `ChosenCeremonyForRound` is the second entry of this kind, and it rests on
#: wire evidence alone -- no descriptor names it, in either build. It is added
#: because that evidence is unusually complete and completely self-checking:
#:
#:   246 occurrences across 6 replays and BOTH builds (13.01 and 13.02)
#:   126 are 16 bits and read as IntPacked resolve, 126 of 126, to an actor
#:       whose class path ends in `Ceremony_C` -- Default, Clutch, Closer,
#:       Flawless, Ace, TeamAce
#:   120 are 8 bits of `00`, i.e. GUID 0, the null reference, written at round
#:       start and replaced by a real ceremony at round end
#:     0 odd GUIDs, and a dynamic NetGUID must be even
#:
#: An ObjectNetGuid reads an IntPacked, so it covers both widths without a
#: special case. A wrong type here cannot hide: every non-zero value has to
#: name a ceremony actor that the same export already lists in actors.parquet.
#:
#: This is a WEAKER justification than the BaseTeamState pair above, where the
#: reference declares the property's type and only its group moved. Recorded as
#: such deliberately -- see PROJECT_STATUS 31-C and 32.
#: The same field in the Swiftplay game state. 5 of the 215 corpus replays are
#: Swiftplay and carry no `BombGameState` at all; their 37 non-null values
#: resolve 37 of 37 to the same ceremony classes (Default, Closer, Clutch,
#: Flawless). A much smaller sample than the Bomb entry above -- said plainly
#: -- but leaving it out would mean one field decoding in one game mode and not
#: another for no reason anybody chose, which is the latent inconsistency
#: 25-G's fourth item closed elsewhere.
ADDITIONS = [
    ("/Game/GameModes/Bomb/BombGameState.BombGameState_C",
     "ChosenCeremonyForRound", "FieldType::ObjectNetGuid"),
    ("/Game/GameModes/_Development/Swiftplay_EndOfRoundCredits"
     "/Swiftplay_EoRCredits_GameState.Swiftplay_EoRCredits_GameState_C",
     "ChosenCeremonyForRound", "FieldType::ObjectNetGuid"),
    ("/Script/ShooterGame.BaseTeamState", "AverageLoadoutValue", "FieldType::Int32"),
    ("/Script/ShooterGame.BaseTeamState", "LoadoutValue", "FieldType::Int32"),
]
EXPECTED += [(g, f, t.split("::")[1]) for g, f, t in ADDITIONS]

GROUP_RE = re.compile(r'group_path: "([^"]+)"')
FIELD_RE = re.compile(r'field_name: "([^"]+)"')
TYPE_MARKER = "field_type:"


def _field_type_of(block: str) -> str | None:
    """The `field_type` value of one `OverlayEntry { ... }` block.

    Brace-counted rather than pattern-matched. Two things defeat a regex here,
    and both are live:

    * several field types contain their own braces
      (`VectorNetQuantize { scale: 100 }`), so a non-greedy match truncates;
    * the table exists in TWO layouts -- one entry per line as
      extract_descriptors.py emits it, and the rustfmt'd multi-line form that
      gets committed. A pattern anchored on a newline works on one and
      silently matches nothing on the other.

    That second case is not hypothetical: the first version of this function
    required a newline, so it reported all 25 corrections missing on a
    freshly generated table -- exactly when this script is supposed to run.

    The block has already had its opening `OverlayEntry {` consumed, so the
    first unmatched `}` is the one that closes the entry.
    """
    start = block.find(TYPE_MARKER)
    if start == -1:
        return None
    start += len(TYPE_MARKER)
    depth = 0
    for i, ch in enumerate(block[start:], start):
        if ch == "{":
            depth += 1
        elif ch == "}":
            if depth == 0:
                return " ".join(block[start:i].rstrip().rstrip(",").split())
            depth -= 1
    return None


def parse_entries(content: str):
    """Yield (group_path, field_name, field_type) for every OverlayEntry.

    Split per `OverlayEntry {` block first so each lookup is scoped to one
    entry and cannot bleed into its neighbour.
    """
    for block in content.split("    OverlayEntry {")[1:]:
        group = GROUP_RE.search(block)
        field = FIELD_RE.search(block)
        ftype = _field_type_of(block)
        if group and field and ftype:
            yield group.group(1), field.group(1), ftype


def apply_additions(content: str) -> tuple[str, int]:
    """Insert every entry in `ADDITIONS` that is not already present.

    Insertion, not replacement, so it cannot reuse the `.replace()` shape the
    corrections above use -- there is nothing to replace.

    The slice is sorted by `(group_path, field_name)` and
    `tests::overlay::table_is_sorted` enforces it, so a new entry goes at its
    sorted position rather than at the end. We find the first existing entry
    that sorts AFTER ours and splice in before it; if no such entry exists we
    fail loudly rather than append, because appending past the final block
    would write into the `];` that closes the slice.
    """
    added = 0
    for group, field, ftype in ADDITIONS:
        blocks = content.split("    OverlayEntry {")
        keys = []
        for block in blocks[1:]:
            g, f = GROUP_RE.search(block), FIELD_RE.search(block)
            keys.append((g.group(1), f.group(1)) if g and f else ("", ""))
        if (group, field) in keys:
            continue

        target = next((i for i, k in enumerate(keys) if k > (group, field)), None)
        if target is None:
            raise SystemExit(
                f"{TABLE_RS}: {group}/{field} sorts after every existing entry; "
                f"refusing to append past the end of the slice."
            )
        entry = (
            "    OverlayEntry {\n"
            f'        group_path: "{group}",\n'
            f'        field_name: "{field}",\n'
            f"        field_type: {ftype},\n"
            "    },\n"
        )
        # blocks[target + 1] is the block for `keys[target]`; put the new entry
        # in front of the marker that introduces it.
        head = "    OverlayEntry {".join(blocks[: target + 1])
        tail = "    OverlayEntry {" + "    OverlayEntry {".join(blocks[target + 1:])
        content = head + entry + tail
        added += 1
    return content, added


def verify(content: str) -> list[str]:
    """Return one line per correction that is NOT present in `content`."""
    entries = list(parse_entries(content))
    problems = []
    for group_part, field, required in EXPECTED:
        hits = [ft for g, f, ft in entries if group_part in g and f == field]
        if not hits:
            problems.append(f"{field} in *{group_part}*: entry not found at all")
        elif not any(required in ft for ft in hits):
            problems.append(
                f"{field} in *{group_part}*: expected {required}, found {hits}"
            )
    return problems


HEADER_COUNTS_RE = re.compile(
    r"^// Raw/Custom: \d+, Skip: \d+, Typed: \d+\.$", re.MULTILINE
)

#: The slice is declared with an explicit length, so an addition that does not
#: update it does not compile. That is the good failure -- it is caught by
#: `cargo build` rather than by anyone noticing -- but the script should not
#: hand over a file that cannot build.
TABLE_LEN_RE = re.compile(r"(pub static OVERLAY_TABLE: \[OverlayEntry; )(\d+)(\])")


def resync_table_len(content: str) -> str:
    """Rewrite the declared `OVERLAY_TABLE` length to the entries present."""
    n = sum(1 for _ in parse_entries(content))
    new_content, hits = TABLE_LEN_RE.subn(
        lambda m: f"{m.group(1)}{n}{m.group(3)}", content, count=1
    )
    if hits != 1:
        raise SystemExit(
            f"{TABLE_RS}: expected exactly one OVERLAY_TABLE length declaration, "
            f"found {hits}."
        )
    return new_content


def rewrite_header_counts(content: str) -> tuple[str, str]:
    """Recount the type buckets and rewrite the generated header line.

    extract_descriptors.py writes that line from the descriptors it read, and
    then this script changes some of those types -- so between the two the
    header is a statement about a table that no longer exists. It said
    "Raw/Custom: 164 ... Typed: 864" while the file held 157 and 871: the seven
    corrections that turn a Raw into a real type.

    Nothing reads the header, which is exactly why it went unnoticed and why it
    is worth fixing -- a comment on a generated file that quietly disagrees with
    the file is how a reader learns not to trust the comments.

    Counted from the parsed entries rather than by substring, so a type whose
    name contains another's (`FieldType::RawPayload` would contain `Raw`)
    cannot miscount.
    """
    buckets = Counter()
    for _group, _field, ftype in parse_entries(content):
        if ftype == "FieldType::Raw":
            buckets["raw"] += 1
        elif ftype == "FieldType::Skip":
            buckets["skip"] += 1
        else:
            buckets["typed"] += 1
    line = (f"// Raw/Custom: {buckets['raw']}, Skip: {buckets['skip']}, "
            f"Typed: {buckets['typed']}.")
    new_content, n = HEADER_COUNTS_RE.subn(lambda _m: line, content, count=1)
    if n != 1:
        raise SystemExit(
            f"{TABLE_RS}: expected exactly one generated bucket-count header "
            f"line, found {n}. Regenerate with extract_descriptors.py first."
        )
    return new_content, line


def main():
    check_only = "--check" in sys.argv[1:]
    content = TABLE_RS.read_text(encoding="utf-8")
    count = 0

    # Fix: Float -> Double for time-related fields (verified: 64-bit on wire)
    float_to_double = [
        ('TimedBomb.TimedBomb_C", field_name: "TimeRemainingToExplode", field_type: FieldType::Float',
         'TimedBomb.TimedBomb_C", field_name: "TimeRemainingToExplode", field_type: FieldType::Double'),
        ('TimedBomb.TimedBomb_C", field_name: "DefuseProgress", field_type: FieldType::Float',
         'TimedBomb.TimedBomb_C", field_name: "DefuseProgress", field_type: FieldType::Double'),
        ('Comp_Ability_CooldownComponent_C", field_name: "StartTimeStamp", field_type: FieldType::Float',
         'Comp_Ability_CooldownComponent_C", field_name: "StartTimeStamp", field_type: FieldType::Double'),
        ('Comp_Ability_CooldownComponent_C", field_name: "CooldownSeconds", field_type: FieldType::Float',
         'Comp_Ability_CooldownComponent_C", field_name: "CooldownSeconds", field_type: FieldType::Double'),
    ]
    for old, new in float_to_double:
        if old in content:
            content = content.replace(old, new)
            count += 1

    # Fix: Int32 -> EnumRemainingBits for "215"/"216" actor bookkeeping in
    # non-weapon groups. Weapons use Raw (correct). These groups use Int32 in
    # the C# descriptor but the wire sends 3 bits.
    groups_215_216 = [
        "TimedBomb.TimedBomb_C",
        "EquippablePickupProjectile.EquippablePickupProjectile_C",
        "EquippableGroundPickup.EquippableGroundPickup_C",
        "OwnerExclusivePlayerInfo",
        "Projectile_Phoenix_Q_FlameWall_ThroughWall.Projectile_Phoenix_Q_FlameWall_ThroughWall_C",
    ]
    for g in groups_215_216:
        for field in ["215", "216"]:
            old = f'{g}", field_name: "{field}", field_type: FieldType::Int32'
            new = f'{g}", field_name: "{field}", field_type: FieldType::EnumRemainingBits'
            if old in content:
                content = content.replace(old, new)
                count += 1

    # Fix: rotation quantization for the Astra smoke-screen projectiles.
    #
    # `ProjectileSmokeScreenDescriptor.cs` calls `.ReplicatedMovement()` with no
    # argument, which defaults to ShortComponents (16 bits per rotation axis).
    # Every other projectile descriptor in the same codebase passes
    # ByteComponents explicitly -- FlameWall, MageWall, NeonTunnel and
    # EquippablePickup -- so the bare call reads as an oversight rather than a
    # deliberate difference.
    #
    # The wire agrees with the other projectiles: on release-13.01 these payloads
    # arrive at 113-124 bits and a ShortComponents read runs off the end (137 EOF
    # failures on one replay, every one of them from this single group).
    #
    # Entries span several lines, so rewrite per `OverlayEntry { .. }` block
    # rather than per line: a line-wise match cannot see the group path and the
    # field type at the same time.
    blocks = content.split("    OverlayEntry {")
    for i, block in enumerate(blocks):
        if i == 0:
            continue
        if "SmokeScreen" not in block or 'field_name: "ReplicatedMovement"' not in block:
            continue
        if "RotatorQuantization::ShortComponents" in block:
            blocks[i] = block.replace(
                "RotatorQuantization::ShortComponents",
                "RotatorQuantization::ByteComponents",
            )
            count += 1
    content = "    OverlayEntry {".join(blocks)

    # REMOVED: FName -> Raw for DamagedBone in MulticastNotifyDamage_Point.
    #
    # This pass existed because 177 of 581 payloads arrive at 9 bits, which
    # the FName decoder could not read. Its comment claimed the value "is
    # always bone index 0" -- generalised from those 177 and wrong: the
    # reference has 22 distinct bone names (Head 69, Spine4 51, L_Shoulder
    # 46, ...) and forcing Raw made us ship mojibake for all 581.
    #
    # The real cause was decode_fname ignoring the isHardcoded bit. Fixed in
    # vrf-decode, so the field decodes as the FName the C# descriptor
    # declares and needs no correction here.

    # Fix: EnumByte -> Raw for AresEquippableDataTracker.OriginalBuyerTeam.
    # C# declares this as EnumByte (single byte), but on wire it arrives as
    # 97-105 bits consistently (248 occurrences in 02d4d478). This is likely
    # a serialized FastArray entry or struct, not a bare enum. Mark Raw.
    blocks = content.split("    OverlayEntry {")
    for i, block in enumerate(blocks):
        if i == 0:
            continue
        if "AresEquippableDataTracker" not in block:
            continue
        if 'field_name: "OriginalBuyerTeam"' not in block:
            continue
        if "FieldType::EnumByte" in block:
            blocks[i] = block.replace("FieldType::EnumByte", "FieldType::Raw")
            count += 1
    content = "    OverlayEntry {".join(blocks)

    # Fix: Raw -> ObjectNetGuid for EquippableUsed on both damage RPCs.
    #
    # Not a wire/declaration mismatch like the others above -- the declaration
    # is simply invisible to the extractor. DamageParameters.cs:51 attaches a
    # custom decoder:
    #
    #   AddPropertyHandle(7, x => x.EquippableUsed, ...)
    #       .Decode(ValorantPayloadDecoders.Equippable)
    #
    # and that decoder (ValorantPayloadDecoders.cs:158) is exactly
    #
    #   var netGuid = archive.ReadIntPacked();
    #
    # which is what FieldType::ObjectNetGuid already implements. Because
    # extract_descriptors.py cannot see through .Decode(...), the field lands
    # here as Raw, and every consumer has to guess the encoding.
    #
    # Verified on 02d4d478 across all 632 occurrences: read as IntPacked the
    # values are 116 distinct and 100% even -- the engine requires dynamic
    # NetGUIDs to be even (IsDynamic => (Value & 1) == 0) -- and 114 of 115
    # resolve to a weapon class path in actors.parquet. The bits are 8, 16 or
    # 24 wide depending on the value, so any fixed-width read is wrong by
    # construction.
    blocks = content.split("    OverlayEntry {")
    for i, block in enumerate(blocks):
        if i == 0:
            continue
        if "DamageableComponent:MulticastNotifyDamage_" not in block:
            continue
        if 'field_name: "EquippableUsed"' not in block:
            continue
        if "FieldType::Raw" in block:
            blocks[i] = block.replace("FieldType::Raw", "FieldType::ObjectNetGuid")
            count += 1
    content = "    OverlayEntry {".join(blocks)

    # Fix: Raw -> quantized vectors for the damage geometry fields.
    #
    # Same invisibility problem as EquippableUsed above: these are attached
    # with .Decode(ValorantPayloadDecoders.VectorNetQuantize*(...)), so the
    # extractor sees a custom decoder and emits Raw, even though vrf-decode
    # already implements the exact quantization.
    #
    # Scales come from the C# call sites, not from guesswork:
    #   DamageParameters.cs:50                    VectorNetQuantize100
    #   MulticastNotifyDamagePointParameters.cs:40 VectorNetQuantizeNormal
    #   MulticastNotifyDamagePointParameters.cs:42 VectorNetQuantize
    #   MulticastNotifyDamagePointParameters.cs:44 VectorNetQuantizeNormal
    #   MulticastNotifyDamagePointParameters.cs:46 VectorNetQuantize
    #
    # Confirmed by the reference bundle's own output: DamageImpactLocation is
    # integral (scale 1), DamageOrigin carries two decimals (scale 100), and
    # DamageDirection / DamageImpactNormal are unit vectors.
    damage_vectors = {
        "DamageOrigin": "FieldType::VectorNetQuantize { scale: 100 }",
        "DamageImpactLocation": "FieldType::VectorNetQuantize { scale: 1 }",
        "DamageImpactBoneRelativeLocation": "FieldType::VectorNetQuantize { scale: 1 }",
        "DamageDirection": "FieldType::VectorNetQuantizeNormal",
        "DamageImpactNormal": "FieldType::VectorNetQuantizeNormal",
    }
    blocks = content.split("    OverlayEntry {")
    for i, block in enumerate(blocks):
        if i == 0:
            continue
        if "DamageableComponent:MulticastNotifyDamage_" not in block:
            continue
        for field, new_type in damage_vectors.items():
            if f'field_name: "{field}"' not in block:
                continue
            if "FieldType::Raw" in block:
                blocks[i] = block.replace("FieldType::Raw", new_type)
                count += 1
            break
    content = "    OverlayEntry {".join(blocks)

    # Additions last, so the bucket recount below sees them.
    content, n_added = apply_additions(content)
    count += n_added
    content = resync_table_len(content)

    content, header_line = rewrite_header_counts(content)

    if not check_only:
        TABLE_RS.write_text(content, encoding="utf-8")

    # The operation count is a diagnostic, not the verdict. 0 is correct when
    # the table was already corrected and wrong when the patterns are dead;
    # only the end state distinguishes them.
    problems = verify(content)
    if problems:
        print(f"FAILED: {len(problems)} of {len(EXPECTED)} corrections are "
              f"missing from {TABLE_RS}", file=sys.stderr)
        for line in problems:
            print(f"  {line}", file=sys.stderr)
        print("If table.rs was regenerated, run extract_descriptors.py, then "
              "THIS script, and only then cargo fmt.", file=sys.stderr)
        return 1

    stale = HEADER_COUNTS_RE.search(TABLE_RS.read_text(encoding="utf-8"))
    if check_only and stale and stale.group(0) != header_line:
        print(f"FAILED: the generated header disagrees with the table.\n"
              f"  file says {stale.group(0)}\n"
              f"  counted   {header_line}", file=sys.stderr)
        return 1

    verb = "verified" if check_only else "applied"
    print(f"{verb}: {count} replacement(s) made, "
          f"all {len(EXPECTED)} corrections present in {TABLE_RS}; "
          f"{header_line.lstrip('/ ').rstrip('.')}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
