"""Apply known type corrections to the generated overlay table.

These corrections represent empirically-verified differences between the C#
descriptor declarations and the actual wire format in build 13.01:
  - Time-related Float fields are actually Double (64 bits on wire)
  - Actor bookkeeping fields "215"/"216" in non-weapon groups are variable-width
    (3 bits in 13.01), not Int32

Run after extract_descriptors.py regenerates table.rs.
"""
import sys
from pathlib import Path

TABLE_RS = Path(__file__).parent.parent / "crates" / "vrf-decode" / "src" / "table.rs"


def main():
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

    # Fix: FName -> Raw for DamagedBone in MulticastNotifyDamage_Point.
    # Wire sends 9 bits consistently (176 occurrences in 02d4d478); this is
    # too short for the standard IntPacked FName reader (minimum ~8-16 bits for
    # index alone). C# uses a custom decoder path for this field. Mark as Raw
    # to preserve the bits without decode errors. Value is always bone index 0.
    blocks = content.split("    OverlayEntry {")
    for i, block in enumerate(blocks):
        if i == 0:
            continue
        if "MulticastNotifyDamage_Point" not in block:
            continue
        if 'field_name: "DamagedBone"' not in block:
            continue
        if "FieldType::FName" in block:
            blocks[i] = block.replace("FieldType::FName", "FieldType::Raw")
            count += 1
    content = "    OverlayEntry {".join(blocks)

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

    TABLE_RS.write_text(content, encoding="utf-8")
    print(f"Applied {count} type corrections to {TABLE_RS}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
