"""Equippable class path -> display name and category.

GENERATED FILE -- DO NOT EDIT BY HAND.
Regenerate with: python tools/extract_equippables.py
Source: src/Replay.Valorant/Combat/ValorantEquippableResolver.cs

Keys cover the three path shapes that appear in replay data, mirroring
the C# CreateDefinitions(): the full 'Package.Class_C' path, the package
path alone, and the 'Default__Class_C' archetype form.
"""

# fmt: off

EQUIPPABLE_DEFINITIONS = [
    ('/Game/Characters/_Core/Equippable_Unarmed.Equippable_Unarmed_C', 'Unarmed', 'unarmed'),
    ('/Game/Equippables/Melee/Ability_Melee_Base.Ability_Melee_Base_C', 'Melee', 'melee'),
    ('/Game/Equippables/Bomb/BombEquippable.BombEquippable_C', 'Spike', 'bomb'),
    ('/Game/Equippables/Guns/Sidearms/BasePistol/BasePistol.BasePistol_C', 'Classic', 'sidearm'),
    ('/Game/Equippables/Guns/Sidearms/Slim/SawedOffShotgun.SawedOffShotgun_C', 'Shorty', 'sidearm'),
    ('/Game/Equippables/Guns/Sidearms/AutoPistol/AutomaticPistol.AutomaticPistol_C', 'Frenzy', 'sidearm'),
    ('/Game/Equippables/Guns/Sidearms/Luger/LugerPistol.LugerPistol_C', 'Ghost', 'sidearm'),
    ('/Game/Equippables/Guns/Sidearms/Compact/CompactPistol.CompactPistol_C', 'Compact Pistol', 'sidearm'),
    ('/Game/Equippables/Guns/Sidearms/Revolver/RevolverPistol.RevolverPistol_C', 'Sheriff', 'sidearm'),
    ('/Game/Equippables/Guns/SubMachineGuns/Vector/Vector.Vector_C', 'Stinger', 'smg'),
    ('/Game/Equippables/Guns/SubMachineGuns/MP5/SubMachineGun_MP5.SubMachineGun_MP5_C', 'Spectre', 'smg'),
    ('/Game/Equippables/Guns/Shotguns/PumpShotgun/PumpShotgun.PumpShotgun_C', 'Bucky', 'shotgun'),
    ('/Game/Equippables/Guns/Shotguns/AutoShotgun/AutomaticShotgun.AutomaticShotgun_C', 'Judge', 'shotgun'),
    ('/Game/Equippables/Guns/Rifles/Burst/AssaultRifle_Burst.AssaultRifle_Burst_C', 'Bulldog', 'rifle'),
    ('/Game/Equippables/Guns/SniperRifles/Dmr/DMR.DMR_C', 'Guardian', 'rifle'),
    ('/Game/Equippables/Guns/Rifles/Carbine/AssaultRifle_ACR.AssaultRifle_ACR_C', 'Phantom', 'rifle'),
    ('/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK.AssaultRifle_AK_C', 'Vandal', 'rifle'),
    ('/Game/Equippables/Guns/SniperRifles/Leversniper/LeverSniperRifle.LeverSniperRifle_C', 'Marshal', 'sniper_rifle'),
    ('/Game/Equippables/Guns/SniperRifles/Boltsniper/BoltSniper.BoltSniper_C', 'Operator', 'sniper_rifle'),
    ('/Game/Equippables/Guns/SniperRifles/Doublesniper/DS_Gun.DS_Gun_C', 'Outlaw', 'sniper_rifle'),
    ('/Game/Equippables/Guns/HvyMachineGuns/LMG/LightMachineGun.LightMachineGun_C', 'Ares', 'machine_gun'),
    ('/Game/Equippables/Guns/HvyMachineGuns/HMG/HeavyMachineGun.HeavyMachineGun_C', 'Odin', 'machine_gun'),
    ('/Game/Characters/Deadeye/S0/Ability_Q/Gun/Gun_Deadeye_Q_Pistol.Gun_Deadeye_Q_Pistol_C', 'Headhunter', 'ability'),
    ('/Game/Characters/Deadeye/S0/Ability_X/Gun_Giantslayer/Gun_Deadeye_X_Giantslayer_Prototype_FIreRatePrototype.Gun_Deadeye_X_Giantslayer_Prototype_FireRatePrototype_C', 'Tour de Force', 'ability'),
]


def _build_lookup():
    """class path (all three shapes) -> (name, category, canonical path)."""
    out = {}
    for class_path, name, category in EQUIPPABLE_DEFINITIONS:
        value = (name, category, class_path)
        out[class_path] = value
        if '.' in class_path:
            package, _, class_name = class_path.rpartition('.')
            out[package] = value
            out['Default__' + class_name] = value
    return out


EQUIPPABLE_BY_PATH = _build_lookup()

# fmt: on
