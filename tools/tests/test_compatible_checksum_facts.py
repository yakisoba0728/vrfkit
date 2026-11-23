"""Name-level facts that pin a property's C++ type, recomputed from its checksum.

A declaration's `compatible_checksum` hashes the property's lowercase name, C++
type and static index, chained from its containing property's checksum
(`check_checksum_types.chain_checksum`, pinned against an independent FCrc
implementation in test_check_checksum_types). A fact is right when its chain
reproduces the replay's checksum and a rival leaf type does not. The chains are
a parent's name and C++ type from the 13.06 executable's reflection; the
overlay comments cite them, and this file is where those citations can fail.

Each fact also names the `FieldType` `checksum_table.rs` gives that checksum,
when it has an entry, so a retyped donor whose table was not regenerated fails.
"""
import unittest

import support  # puts tools/ on sys.path
import extract_checksum_types as ect
import generate_scoped_types as gst
from check_checksum_types import (
    ALTERNATIVE_TYPES, chain_checksum, compatible_checksum, implied_parent)

#: (replay checksum, chain, rival leaf types that must NOT reproduce it,
#:  the FieldType checksum_table.rs must give it or None when it has no entry,
#:  where the fact is used)
FACTS = [
    (3336285386,
     [("Handle", "FForceModuleHandle"), ("HandleNumber", "uint32")],
     ["int32"], "FieldType::UInt32",
     "NetMulticast{Apply,Remove}ForceModule.HandleNumber"),
    (1457545067,
     [("ModulesCleanedUpByServer", "TArray"),
      ("ModulesCleanedUpByServer", "FForceModuleHandle"),
      ("HandleNumber", "uint32")],
     ["int32"], None,
     "NetMulticastEnforceEndOfLifeCleanup.HandleNumber"),
    (2340855891,
     [("EffectID", "FEffectID"), ("EffectID", "int64")],
     ["uint64"], "FieldType::Int64",
     "EffectManagerComponent:Multicast{Play,Update,Stop}ContinuousEffect.EffectID"),
    (2251343646,
     [("CurrentEffectID", "FEffectID"), ("EffectID", "int64")],
     ["uint64"], "FieldType::Int64",
     "ReplayEffectComponent:ReplayPlayContinuousEffectAtLocation.EffectID"),
    (1129645208,
     [("ServerActiveEffects", "TArray"), ("ServerActiveEffects", "FActiveEffectInfo"),
      ("EffectID", "FEffectID"), ("EffectID", "int64")],
     ["uint64"], "FieldType::Int64",
     "EffectManagerComponent.EffectID"),
    (3321413110,
     [("AuthBlindManagerState", "FBlindManagerState"), ("ActiveBlinds", "TArray"),
      ("ActiveBlinds", "FActiveBlind"), ("BlindEffectID", "FEffectID"),
      ("EffectID", "int64")],
     ["uint64"], None,
     "BlindManagerComponent ActiveBlinds[].EffectID (sink/blobs.rs)"),
    # Cypher's trapwire, before and after the 13.01 rename: the wire's
    # `PairedWire` parameter names its own class, so its checksum moves with
    # the class name -- the rename proven in the type system itself.
    (3902815170, [("Deployed", "bool")], ["uint8"], "FieldType::Bool",
     "GameObject_Gumshoe_{E,4}_TripWire(_SecondWire)_C.Deployed"),
    (3671888355, [("PairedWire", "AGameObject_Gumshoe_E_TripWire_C*")],
     ["AGameObject_Gumshoe_4_TripWire_C*"], None,
     "GameObject_Gumshoe_E_TripWire_C:SetEnemyInTrap.PairedWire (11.06-12.08)"),
    (3454621121, [("PairedWire", "AGameObject_Gumshoe_4_TripWire_C*")],
     ["AGameObject_Gumshoe_E_TripWire_C*"], None,
     "GameObject_Gumshoe_4_TripWire_C:SetEnemyInTrap.PairedWire (13.01-13.06)"),
    (2035145197, [("CreatedByCharacter", "AShooterCharacter*")], ["UShooterCharacter*"], None,
     "Ability_Gumshoe_{E_TripWire,4_TripWire,4_CageTrap,Q_CageTrap}_C.CreatedByCharacter"),
    # An FTransform reaches the wire as three members. `249` (the hardcoded
    # FName index of `Rotation`) is its FQuat, sent as X/Y/Z -- not a rotator.
    (747197698, [("Transform", "FTransform"), ("Rotation", "FQuat")],
     ["FRotator", "FVector"], "FieldType::VectorDouble",
     "MulticastPlay*Effect(FromClient).249, TransformTransitionContext.249"),
    (2235276067, [("Transform", "FTransform"), ("Translation", "FVector")],
     ["FQuat"], "FieldType::VectorDouble",
     "MulticastPlay*Effect(FromClient).Translation"),
    (2983776962, [("Transform", "FTransform"), ("Scale3D", "FVector")],
     ["FQuat"], "FieldType::VectorDouble",
     "MulticastPlay*Effect(FromClient).Scale3D"),
    (1874998526, [("SpawnTransform", "FTransform"), ("Rotation", "FQuat")],
     ["FRotator", "FVector"], "FieldType::VectorDouble",
     "AresGameStateBase:MulticastResetForRespawn.249"),
    (177696787, [("ValveSetTransform", "FTransform"), ("Rotation", "FQuat")],
     ["FRotator", "FVector"], None,
     "MulticastAddSmokeScreenPoint.249 / MulticastAddAnchor.249 (raw)"),
    # The effect-placement RPCs' `249` is a different property: a top-level
    # FRotator, typed RotationShort by name (no checksum-table entry).
    (2526428638, [("Rotation", "FRotator")], ["FQuat", "FVector"], None,
     "ClientPlayOneShotEffectAtLocation / ReplayPlay*AtLocation / ReplayRecord*.249"),
    (598402184, [("Location", "FVector")], ["FVector_NetQuantize"], "FieldType::VectorDouble",
     "the effect-placement RPCs' 248"),
]

#: Blueprint properties typed by exact group, name and checksum
#: (tools/fixtures/scoped_type_evidence.json -> scoped_types.rs):
#: (replay checksum, property name, C++ type, rival types that must NOT
#:  reproduce it, the FieldType every scoped entry carrying the checksum must
#:  have, where).
#:
#: Each name and C++ type comes from the 13.06 Blueprint class definitions:
#: the FProperty class, and for an object reference its class, which fixes the
#: `A`/`U` prefix. A top-level Blueprint property's chain is one step.
SCOPED_FACTS = [
    (3110715024, "TrailPosition", "FVector", ["FVector3f", "FRotator"],
     "FieldType::VectorDouble", "Projectile_Hunter_{Q_RevealBolt,4_ExplosiveBolt}_C"),
    (1066899736, "IsPossessed", "bool", ["uint8"], "FieldType::Bool",
     "(Rift_)PossessableActorComponent_C"),
    (2181339745, "Possessed", "bool", ["uint8"], "FieldType::Bool",
     "Pawn_Gumshoe_E_PossessableCamera_C"),
    (2029268412, "IsDeployed", "bool", ["uint8"], "FieldType::Bool",
     "Pawn_Gumshoe_E_PossessableCamera_C"),
    (2740089937, "DeployedActor", "AActor*", ["UActor*", "APawn*"],
     "FieldType::ObjectNetGuid", "Ability_Killjoy_{E_Turret,Q_Alarmbot}_C"),
    (1908355023, "CurrentCharge", "double", ["float"], "FieldType::Double",
     "Comp_Equippable_Charged_C"),
    (1863385026, "CurrentLossStreak", "int32", ["uint32", "float"], "FieldType::Int32",
     "BombGameState_C"),
    (22256526, "LossStreakTeam", "UBaseTeamComponent*", ["ABaseTeamComponent*"],
     "FieldType::ObjectNetGuid", "BombGameState_C, Swiftplay_EoRCredits_GameState_C"),
    (2889152318, "ShouldOverrideMatchTimer", "bool", ["uint8"], "FieldType::Bool",
     "BombGameState_C, Swiftplay_EoRCredits_GameState_C"),
    # A TextProperty: typed with the full-tree reader, because the legacy
    # FText one keeps only string-table keys and refuses both forms it sends.
    (4004484071, "OverrideMatchTimerText", "FText", ["FString", "FName"],
     "FieldType::FTextTree", "BombGameState_C, Swiftplay_EoRCredits_GameState_C"),
    # A class reference is `UClass*`, not `TSubclassOf<>`; an object
    # reference's A/U prefix is its class's.
    (1807371052, "ActiveSlowTimeEffects", "bool", ["uint8"],
     "FieldType::Bool", "BombGameState_C, Swiftplay_EoRCredits_GameState_C"),
    (2379229353, "AssignedPlayspace", "UClass*", ["TSubclassOf<UPlayspace>", "TSubclassOf<APlayspace>"],
     "FieldType::ObjectNetGuid", "Actor_Sequoia_X_StandardArena_C"),
    (2511444522, "BlueTeamStartingAvgInventoryValue", "int32", ["uint32", "float"],
     "FieldType::Int32", "ThriftyCeremony_C"),
    (945042264, "Bubble_Destroy_Audio", "UAkAudioEvent*", ["AAkAudioEvent*"],
     "FieldType::ObjectNetGuid", "FXC_Finisher_Destructible_C, FXC_WaterBlaster_Finisher_Destructible_C"),
    (3040835906, "CharacterLocation", "FVector", ["FVector3f", "FRotator"],
     "FieldType::VectorDouble", "GameObject_Breach_E_SweetSpotFissure_C, GameObject_SoundSensor_SweetSpotFissure_C"),
    (4180852283, "Charge", "double", ["float"],
     "FieldType::Double", "GameObject_Breach_4_FusionBlast_C, Projectile_Breach_4_FusionBlast_C"),
    (261706910, "ChargeDistance", "double", ["float"],
     "FieldType::Double", "Projectile_Breach_4_FusionBlast_C"),
    (951985780, "CloserPlayer", "AShooterPlayerState*", ["UShooterPlayerState*"],
     "FieldType::ObjectNetGuid", "CloserCeremony_C"),
    (829915424, "ClutchPlayer", "AShooterPlayerState*", ["UShooterPlayerState*"],
     "FieldType::ObjectNetGuid", "ClutchCeremony_C"),
    (4217858995, "Collision Offset Position", "FVector", ["FVector3f", "FRotator"],
     "FieldType::VectorDouble", "FXC_Finisher_Destructible_C, FXC_WaterBlaster_Finisher_Destructible_C"),
    (3409090095, "DefaultToTargetViewModeTargeting", "bool", ["uint8"],
     "FieldType::Bool", "Ability_Mage_E_WorldSmoke_C, Ability_Wraith_4_Smoke_C"),
    (1997410783, "Destroyed", "bool", ["uint8"],
     "FieldType::Bool", "BP_Breakable_Simple_Rockman_C, BP_Breakable_Simple_Saltman_Helmet_C"),
    (3404560428, "Enable Rotate around Axis", "bool", ["uint8"],
     "FieldType::Bool", "FXC_Finisher_Destructible_C"),
    (1641873608, "EndingStrikeLocation", "FVector", ["FVector3f", "FRotator"],
     "FieldType::VectorDouble", "AIPawn_Cashew_E_SeekingTargetMissile_C"),
    (3355379499, "FlawlessTeam", "UBaseTeamComponent*", ["ABaseTeamComponent*"],
     "FieldType::ObjectNetGuid", "FlawlessCeremony_C"),
    (3878297219, "Float", "double", ["float"],
     "FieldType::Double", "FloatTransitionContext_C"),
    (422586278, "GameplayStartTime", "double", ["float"],
     "FieldType::Double", "Switch_BlackMarket_2_C"),
    (1717661931, "Has Succesfully Hit", "bool", ["uint8"],
     "FieldType::Bool", "Patch_Aggrobot_C_ExplodeyPatch_C, Patch_Cable_4_NetToss_C, Patch_Deadeye_E_Slow_Large_C, Patch_Pandemic_AcidMolotov_NewMolotov_C, Patch_Phoenix_MolotovFire_C, Patch_Sarge_Q_Molotov_Production_C"),
    (1537044997, "HasPlayed", "bool", ["uint8"],
     "FieldType::Bool", "Switch_BlackMarket_2_C"),
    (6924581, "Hidden in Game", "bool", ["uint8"],
     "FieldType::Bool", "FXC_Finisher_Destructible_C, FXC_Rogue_Finisher_Destructible_C, FXC_WaterBlaster_Finisher_Destructible_C"),
    (501730740, "Is State Concuss", "bool", ["uint8"],
     "FieldType::Bool", "Ability_Iris_Thumper_C"),
    (1939830999, "IsActivated", "bool", ["uint8"],
     "FieldType::Bool", "ActivatableActorComponent_C, ActivatableActor_Selectable_Component_C"),
    (445799890, "IsAlive_0", "bool", ["uint8"],
     "FieldType::Bool", "GameObject_Thorne_E_Wall_Segment_Fortifying_C"),
    (3166326863, "IsArmed", "bool", ["uint8"],
     "FieldType::Bool", "Pawn_Killjoy_Q_StealthAlarmbot_C"),
    (2671071791, "IsBurrowed", "bool", ["uint8"],
     "FieldType::Bool", "Pawn_Killjoy_Q_StealthAlarmbot_C"),
    (1136330846, "IsCurrentlyDoingInitialFlyOut", "bool", ["uint8"],
     "FieldType::Bool", "AIPawn_Cashew_E_SeekingTargetMissile_C"),
    (2206903017, "IsDisabled", "bool", ["uint8"],
     "FieldType::Bool", "Switch_BlackMarket_2_C"),
    (1443783453, "LastUsedTime", "double", ["float"],
     "FieldType::Double", "Switch_BlackMarket_2_C"),
    (3871083543, "LeverDown", "bool", ["uint8"],
     "FieldType::Bool", "Switch_BlackMarket_2_C"),
    (2321996340, "Projectile", "AActor*", ["UActor*"],
     "FieldType::ObjectNetGuid", "Ability_Sequoia_Q_FragileMissilePrototypeEquipped_C, Ability_Wraith_Q_NearsightMissile_C"),
    (1159482238, "RedTeamStartingAvgInventoryValue", "int32", ["uint32", "float"],
     "FieldType::Int32", "ThriftyCeremony_C"),
    (244340520, "RocketIndex", "int32", ["uint32", "float"],
     "FieldType::Int32", "AIPawn_Cashew_E_SeekingTargetMissile_C"),
    (2055764079, "Rotate Axis Location", "FVector", ["FVector3f", "FRotator"],
     "FieldType::VectorDouble", "FXC_Finisher_Destructible_C"),
    (3642914479, "Rotate Offset", "double", ["float"],
     "FieldType::Double", "FXC_Finisher_Destructible_C"),
    (6804471, "Rotate Speed", "double", ["float"],
     "FieldType::Double", "FXC_Finisher_Destructible_C"),
    (936511401, "SalvagingTeam", "UBaseTeamComponent*", ["ABaseTeamComponent*"],
     "FieldType::ObjectNetGuid", "ThriftyCeremony_C"),
    (1905883563, "Seeking Missile Actor", "AAIPawn_Cashew_E_SeekingTargetMissile_C*", ["UAIPawn_Cashew_E_SeekingTargetMissile_C*"],
     "FieldType::ObjectNetGuid", "GameObject_Cashew_E_MapMissileMarker_C, GameObject_Cashew_E_MapMissileMarker_SecondRocket_C"),
    (2924225553, "Target", "AActor*", ["UActor*"],
     "FieldType::ObjectNetGuid", "GameObject_Cable_4_RemovableNet_C, GameObject_Hunter_E_Drone_RevealDart_C, GameObject_RemovableObject_GumshoeTrackingDart_C"),
    (4028634209, "TargetEquippable", "AAresEquippable*", ["UAresEquippable*"],
     "FieldType::ObjectNetGuid", "EquipRequestTransitionContext_C"),
    (1655581842, "TeamAcingTeam", "UBaseTeamComponent*", ["ABaseTeamComponent*"],
     "FieldType::ObjectNetGuid", "TeamAceCeremony_C"),
    (470315408, "VFX Offset Position", "FVector", ["FVector3f", "FRotator"],
     "FieldType::VectorDouble", "FXC_Finisher_Destructible_C, FXC_WaterBlaster_Finisher_Destructible_C"),
    (1424672044, "VFX_Destroy", "UParticleSystem*", ["AParticleSystem*"],
     "FieldType::ObjectNetGuid", "Finisher_Ninja_Destructible_C"),
    (2949030953, "VFX_Duration_Ground", "UParticleSystem*", ["AParticleSystem*"],
     "FieldType::ObjectNetGuid", "Finisher_Ninja_Destructible_C"),
    (2234578776, "VFX_Duration_Water", "UParticleSystem*", ["AParticleSystem*"],
     "FieldType::ObjectNetGuid", "Finisher_Ninja_Destructible_C"),
    (2642729003, "VFX_Ground", "UParticleSystem*", ["AParticleSystem*"],
     "FieldType::ObjectNetGuid", "Finisher_Ninja_Destructible_C"),
    (1816143426, "VFX_Spawn", "UParticleSystem*", ["AParticleSystem*"],
     "FieldType::ObjectNetGuid", "Finisher_Ninja_Destructible_C"),
    (3224851082, "Victim FXC", "UClass*", ["TSubclassOf<UEffectContainer>", "TSubclassOf<AEffectContainer>"],
     "FieldType::ObjectNetGuid", "OnKillEffect_Base_C"),
    (3421571079, "VictimFXC_Planted", "UClass*", ["TSubclassOf<UEffectContainer>", "TSubclassOf<AEffectContainer>"],
     "FieldType::ObjectNetGuid", "OnKillEffect_Base_C"),
    (475466788, "WarningActor", "AGameObject_Global_LineMissile_TrajectoryWarning_C*", ["UGameObject_Global_LineMissile_TrajectoryWarning_C*"],
     "FieldType::ObjectNetGuid", "Ability_Sequoia_Q_FragileMissilePrototypeEquipped_C, Ability_Wraith_Q_NearsightMissile_C"),
    (2636440541, "bShouldDisplayCeremony", "bool", ["uint8"],
     "FieldType::Bool", "AceCeremony_C, CloserCeremony_C, ClutchCeremony_C, DefaultCeremony_C, FlawlessCeremony_C, TeamAceCeremony_C, ThriftyCeremony_C"),
]

#: The C++ leaf type each checksum-table FieldType above stands for.
FIELD_TYPE_OF = {
    "int64": "FieldType::Int64",
    "uint32": "FieldType::UInt32",
    "int32": "FieldType::Int32",
    "double": "FieldType::Double",
    "FText": "FieldType::FTextTree",
    "bool": "FieldType::Bool",
    # Three doubles on this wire (UE5 large world coordinates); FQuat sends
    # X/Y/Z only, W implied.
    "FQuat": "FieldType::VectorDouble",
    "FVector": "FieldType::VectorDouble",
}


def field_type_of(cpp_type: str) -> str:
    """An object reference of any class is a NetGUID on the wire."""
    if cpp_type.endswith("*"):
        return "FieldType::ObjectNetGuid"
    return FIELD_TYPE_OF[cpp_type]


class FormulaTests(unittest.TestCase):
    def test_a_known_blueprint_field_reproduces(self):
        """A fact with no struct nesting, as an anchor for the formula itself."""
        self.assertEqual(chain_checksum([("Deployed", "bool")]), 3902815170)

    def test_every_leaf_depends_on_its_parents(self):
        leaf_only = chain_checksum([("EffectID", "int64")])
        self.assertNotEqual(leaf_only, 2340855891)


class FactTests(unittest.TestCase):
    def test_every_chain_reproduces_its_replay_checksum(self):
        for checksum, chain, _rivals, _table, where in FACTS:
            with self.subTest(where=where):
                self.assertEqual(chain_checksum(chain), checksum)

    def test_no_rival_leaf_type_reproduces_it(self):
        for checksum, chain, rivals, _table, where in FACTS:
            for rival in rivals:
                with self.subTest(where=where, rival=rival):
                    self.assertNotEqual(
                        chain_checksum(chain[:-1] + [(chain[-1][0], rival)]),
                        checksum)

    def test_the_checksum_table_agrees_with_the_proven_type(self):
        committed = ect.load_committed()
        for checksum, chain, _rivals, table_type, where in FACTS:
            with self.subTest(where=where):
                if table_type is None:
                    continue
                self.assertEqual(FIELD_TYPE_OF[chain[-1][1]], table_type)
                self.assertEqual(committed.get(checksum), table_type)


class ScopedFactTests(unittest.TestCase):
    """The scoped Blueprint entries against the type their checksum proves."""

    def test_every_blueprint_property_reproduces_its_replay_checksum(self):
        for checksum, name, cpp_type, _rivals, _field_type, where in SCOPED_FACTS:
            with self.subTest(where=where, name=name):
                self.assertEqual(chain_checksum([(name, cpp_type)]), checksum)

    def test_no_rival_type_reproduces_it(self):
        for checksum, name, _cpp_type, rivals, _field_type, where in SCOPED_FACTS:
            for rival in rivals:
                with self.subTest(where=where, name=name, rival=rival):
                    self.assertNotEqual(chain_checksum([(name, rival)]), checksum)

    def test_every_scoped_entry_with_the_checksum_has_the_proven_type(self):
        """Each fact must be used, by entries of its own name and its type.

        Read from the reviewed fixture; `generate_scoped_types.py --check`
        (in the sweep) holds scoped_types.rs to the same entries.
        """
        entries = gst.load(gst.EVIDENCE)
        for checksum, name, cpp_type, _rivals, field_type, where in SCOPED_FACTS:
            with self.subTest(where=where, name=name):
                self.assertEqual(field_type_of(cpp_type), field_type)
                carriers = [e for e in entries if e["checksum"] == checksum]
                self.assertTrue(carriers, "no scoped entry uses this fact")
                for entry in carriers:
                    self.assertEqual(entry["field"], name)
                    self.assertEqual(gst.TYPES[entry["type"]], (field_type,))


INTERACTABLE_USER = "/Script/ShooterGame.InteractableUserComponent"

#: Struct members with no known parent name: CRC-32 runs backwards, so each
#: (name, checksum, C++ type) implies one parent (`implied_parent`), and
#: differently named members typed right imply the same one. Each set is
#: (members as (name, checksum, C++ type, rivals, FieldType), group, where).
#: The parent of the first set is `InteractableUserComponent`'s unnamed use
#: state; the second, its FNetworkedMovementTimestamp member.
SIBLING_FACTS = [
    ([("ChangeId", 3631242266, "int32", ["uint32", "float"], "FieldType::Int32"),
      ("CurrentUsingStartTime", 2319860043, "float", ["int32", "double"], "FieldType::Float"),
      ("CurrentUsingComponent", 2976205367, "UUsableComponent*", ["AUsableComponent*"],
       "FieldType::ObjectNetGuid")],
     INTERACTABLE_USER, "InteractableUserComponent use state"),
    ([("NetTimestamp", 1526373506, "float", ["int32", "double"], "FieldType::Float"),
      ("RespawnNumber", 1804051860, "int32", ["uint32", "float"], "FieldType::Int32"),
      ("Character", 1771532671, "TWeakObjectPtr<AShooterCharacter>", ["AShooterCharacter*"],
       "FieldType::ObjectNetGuid")],
     INTERACTABLE_USER, "InteractableUserComponent timestamp"),
]

#: A member of the first set that is an enum: no plain spelling reproduces it
#: under the parent its siblings imply, so EnumByte is the only reader left.
ENUM_SIBLING = ("UseFinishType", 985368745, "FieldType::EnumByte")


class SiblingFactTests(unittest.TestCase):
    """Members of one struct, each typed right, imply one parent checksum."""

    def test_members_imply_one_parent(self):
        for members, _group, where in SIBLING_FACTS:
            with self.subTest(where=where):
                parents = {implied_parent(c, n, t) for n, c, t, _r, _f in members}
                self.assertEqual(len(parents), 1, parents)

    def test_no_rival_implies_that_parent(self):
        for members, _group, where in SIBLING_FACTS:
            (name0, c0, t0, _r0, _f0) = members[0]
            parent = implied_parent(c0, name0, t0)
            for name, checksum, _cpp, rivals, _ft in members:
                for rival in rivals:
                    with self.subTest(where=where, name=name, rival=rival):
                        self.assertNotEqual(implied_parent(checksum, name, rival), parent)

    def test_the_overlay_table_types_each_member_as_proven(self):
        table = ect.load_overlay_table()
        for members, group, where in SIBLING_FACTS:
            for name, _checksum, _cpp, _rivals, field_type in members:
                with self.subTest(where=where, name=name):
                    self.assertEqual(table.get((group, name)), field_type)

    def test_the_weak_character_pointer_is_the_force_modules_too(self):
        """The same spelling reproduces NetMulticastApplyForceModule's
        Character under its named `TimeStamp` chain: one struct type."""
        self.assertEqual(
            chain_checksum([("TimeStamp", "FNetworkedMovementTimestamp"),
                            ("Character", "TWeakObjectPtr<AShooterCharacter>")]),
            1346692128)

    def test_use_finish_type_is_an_enum(self):
        name, checksum, field_type = ENUM_SIBLING
        members, group, _where = SIBLING_FACTS[0]
        parent = implied_parent(members[0][1], members[0][0], members[0][2])
        for spelling in ALTERNATIVE_TYPES:
            with self.subTest(spelling=spelling):
                self.assertNotEqual(compatible_checksum(name, spelling, 0, parent), checksum)
        self.assertEqual(ect.load_overlay_table().get((group, name)), field_type)
