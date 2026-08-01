import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPO = Path(__file__).resolve().parents[2]
GENERATOR = REPO / "tools" / "extract_descriptors.py"

ENTRY_RE = re.compile(
    r'OverlayEntry \{ group_path: "([^"]+)", field_name: "([^"]+)", '
    r'field_type: ([^}]+(?:\{[^}]+\})?) \},'
)
HANDLE_ENTRY_RE = re.compile(
    r'OverlayHandleEntry \{ group_path: "([^"]+)", handle: (\d+), '
    r'field_name: "([^"]+)" \},'
)
RUNTIME_AGENT_CACHE_FACTORY = r'''
internal static class AgentClassNetCacheDescriptors
{
    private const string KillFunctionName = "MulticastNotifyKilledEnemy";
    public static IReadOnlyList<ClassNetCacheDescriptor> Create(
        IEnumerable<ExportGroupDescriptor> agentDescriptors)
    {
        return agentDescriptors
            .Select(agent => new ClassNetCacheDescriptor(
                agent.Path + "_ClassNetCache", [CreateKillRpc()]))
            .ToArray();
    }
    private static RpcDescriptor CreateKillRpc() => new RpcDescriptor
    {
        Name = KillFunctionName,
    };
}
'''


class ExtractDescriptorsTests(unittest.TestCase):
    def run_generator_process(
        self, sources: dict[str, str]
    ) -> tuple[subprocess.CompletedProcess[str], str | None]:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source_root = root / "Replay.Valorant"
            source_root.mkdir()
            for relative_path, source in sources.items():
                path = source_root / relative_path
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(source, encoding="utf-8")

            output = root / "table.rs"
            result = subprocess.run(
                [sys.executable, str(GENERATOR), str(source_root), str(output)],
                check=False,
                capture_output=True,
                text=True,
                encoding="utf-8",
            )
            output_text = output.read_text(encoding="utf-8") if output.exists() else None
            return result, output_text

    def run_generator(self, sources: dict[str, str]) -> str:
        result, output = self.run_generator_process(sources)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIsNotNone(output)
        return output or ""

    def run_generator_expecting_failure(self, sources: dict[str, str]) -> str:
        result, _ = self.run_generator_process(sources)
        self.assertNotEqual(result.returncode, 0, "generator unexpectedly succeeded")
        return result.stderr

    def runtime_cache_entries(self, output: str) -> set[tuple[str, str, str]]:
        return {
            (group, field, field_type.strip())
            for group, field, field_type in ENTRY_RE.findall(output)
            if group.endswith("_ClassNetCache")
        }

    def test_add_raw_wrapper_calls_emit_all_eleven_raw_fields(self):
        output = self.run_generator(
            {
                "DamageParameters.cs": r'''
public abstract class DamageParameters<T> : ExportGroupDescriptor<T>
{
    protected void AddSharedFields()
    {
        AddRaw(9, x => x.LifeChangeEvents, "LifeChangeEvents");
        AddRaw(11, x => x.LifeResult, "LifeResult");
    }

    protected void AddRaw(uint handle, Expression<Func<T, ValorantRawPayload?>> property, string typeName) =>
        AddPropertyHandle(handle, property, ExportCategory.Gunplay).Decode(RawPayload(typeName));
}
''',
                "MulticastNotifyDamageBaseParameters.cs": r'''
public sealed class MulticastNotifyDamageBaseParameters : DamageParameters<MulticastNotifyDamageBaseParameters>
{
    public override string Path => "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Base";
    protected override void Configure()
    {
        AddSharedFields();
        AddRaw(35, x => x.DeathMontageEffectOverride, "DeathMontageEffectOverride");
        AddRaw(36, x => x.DeathMontageEffectOverrideContext, "DeathMontageEffectOverrideContext");
    }
}
''',
                "MulticastNotifyDamagePointParameters.cs": r'''
public sealed class MulticastNotifyDamagePointParameters : DamageParameters<MulticastNotifyDamagePointParameters>
{
    public override string Path => "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Point";
    protected override void Configure()
    {
        AddSharedFields();
        AddRaw(34, x => x.AssistsList, "AssistsList");
        AddRaw(35, x => x.AssistType, "AssistType");
        AddRaw(38, x => x.AssistTag, "AssistTag");
        AddRaw(43, x => x.DeathMontageEffectOverride, "DeathMontageEffectOverride");
        AddRaw(44, x => x.DeathMontageEffectOverrideContext, "DeathMontageEffectOverrideContext");
    }
}
''',
            }
        )

        entries = {
            (group, field, field_type.strip())
            for group, field, field_type in ENTRY_RE.findall(output)
        }
        self.assertEqual(
            entries,
            {
                (
                    "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Base",
                    "LifeChangeEvents",
                    "FieldType::Raw",
                ),
                (
                    "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Base",
                    "LifeResult",
                    "FieldType::Raw",
                ),
                (
                    "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Base",
                    "DeathMontageEffectOverride",
                    "FieldType::Raw",
                ),
                (
                    "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Base",
                    "DeathMontageEffectOverrideContext",
                    "FieldType::Raw",
                ),
                (
                    "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Point",
                    "LifeChangeEvents",
                    "FieldType::Raw",
                ),
                (
                    "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Point",
                    "LifeResult",
                    "FieldType::Raw",
                ),
                (
                    "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Point",
                    "AssistsList",
                    "FieldType::Raw",
                ),
                (
                    "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Point",
                    "AssistType",
                    "FieldType::Raw",
                ),
                (
                    "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Point",
                    "AssistTag",
                    "FieldType::Raw",
                ),
                (
                    "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Point",
                    "DeathMontageEffectOverride",
                    "FieldType::Raw",
                ),
                (
                    "/Script/ShooterGame.DamageableComponent:MulticastNotifyDamage_Point",
                    "DeathMontageEffectOverrideContext",
                    "FieldType::Raw",
                ),
            },
        )

    def test_called_class_net_cache_helpers_emit_two_skip_entries(self):
        output = self.run_generator(
            {
                "BombGameStateClassNetCacheDescriptor.cs": r'''
public sealed class BombGameStateClassNetCacheDescriptor
    : ClassNetCacheDescriptor<BombGameStateClassNetCacheDescriptor>
{
    public override string Path => "/Game/GameModes/Bomb/BombGameState.BombGameState_C_ClassNetCache";

    protected override void Configure()
    {
        AddTemporaryDeathBase();
        AddTemporaryDeathPoint();
    }

    private void AddTemporaryDeathBase()
    {
        AddFunction<MulticastReceivePlayerTemporaryDeathEventBaseParameters>(
            "MulticastReceivePlayerTemporaryDeathEvent_Base", "/base", ExportCategory.GameState);
    }

    private void AddTemporaryDeathPoint()
    {
        AddFunction<MulticastReceivePlayerTemporaryDeathEventPointParameters>(
            "MulticastReceivePlayerTemporaryDeathEvent_Point", "/point", ExportCategory.GameState);
    }

    private void UnusedHelper()
    {
        AddFunction("MustNotBeExtracted", "/unused", ExportCategory.GameState);
    }
}
'''
            }
        )

        entries = {
            (group, field, field_type.strip())
            for group, field, field_type in ENTRY_RE.findall(output)
        }
        self.assertEqual(
            entries,
            {
                (
                    "/Game/GameModes/Bomb/BombGameState.BombGameState_C_ClassNetCache",
                    "MulticastReceivePlayerTemporaryDeathEvent_Base",
                    "FieldType::Skip",
                ),
                (
                    "/Game/GameModes/Bomb/BombGameState.BombGameState_C_ClassNetCache",
                    "MulticastReceivePlayerTemporaryDeathEvent_Point",
                    "FieldType::Skip",
                ),
            },
        )

    def test_runtime_agent_cache_factory_emits_each_agent_cache(self):
        output = self.run_generator(
            {
                "GenericAgentDescriptor.cs": r'''
public abstract class GenericAgentDescriptor : ExportGroupDescriptor<GenericAgentDescriptor>
{
    public override ExportCategory Categories => ExportCategory.Agent;
    protected override void Configure() { AddProperty(x => x.Owner).ObjectNetGuid(); }
}
''',
                "AlphaAgentDescriptor.cs": r'''
public sealed class AlphaAgentDescriptor : GenericAgentDescriptor
{
    public override string Path => "/Game/Characters/Alpha/Alpha_PC.Alpha_PC_C";
}
''',
                "BetaAgentDescriptor.cs": r'''
public sealed class BetaAgentDescriptor : GenericAgentDescriptor
{
    public override string Path => "/Game/Characters/Beta/Beta_PC.Beta_PC_C";
}
''',
                "AgentClassNetCacheDescriptors.cs": r'''
internal static class AgentClassNetCacheDescriptors
{
    private const string KillFunctionName = "MulticastNotifyKilledEnemy";
    public static IReadOnlyList<ClassNetCacheDescriptor> Create(IEnumerable<ExportGroupDescriptor> agentDescriptors)
    {
        return agentDescriptors
            .Select(agent => new ClassNetCacheDescriptor(
                agent.Path + "_ClassNetCache", [CreateKillRpc()]))
            .ToArray();
    }
    private static RpcDescriptor CreateKillRpc() => new RpcDescriptor
    {
        Name = KillFunctionName,
    };
}
''',
            }
        )

        entries = {
            (group, field, field_type.strip())
            for group, field, field_type in ENTRY_RE.findall(output)
            if group.endswith("_ClassNetCache")
        }
        self.assertEqual(
            entries,
            {
                (
                    "/Game/Characters/Alpha/Alpha_PC.Alpha_PC_C_ClassNetCache",
                    "MulticastNotifyKilledEnemy",
                    "FieldType::Skip",
                ),
                (
                    "/Game/Characters/Beta/Beta_PC.Beta_PC_C_ClassNetCache",
                    "MulticastNotifyKilledEnemy",
                    "FieldType::Skip",
                ),
            },
        )

    def test_runtime_agent_cache_respects_explicit_non_agent_category_override(self):
        output = self.run_generator(
            {
                "GenericAgentDescriptor.cs": r'''
public abstract class GenericAgentDescriptor : ExportGroupDescriptor<GenericAgentDescriptor>
{
    public override ExportCategory Categories => ExportCategory.Agent;
    protected override void Configure() { AddProperty(x => x.Owner).ObjectNetGuid(); }
}
''',
                "AlphaAgentDescriptor.cs": r'''
public sealed class AlphaAgentDescriptor : GenericAgentDescriptor
{
    public override string Path => "/Game/Characters/Alpha/Alpha_PC.Alpha_PC_C";
}
''',
                "HunterDronePawnDescriptor.cs": r'''
public sealed class HunterDronePawnDescriptor : GenericAgentDescriptor
{
    public override string Path =>
        "/Game/Characters/Hunter/Drone/Pawn_Hunter_Drone.Pawn_Hunter_Drone_C";
    public override ExportCategory Categories => ExportCategory.Ability;
}
''',
                "AgentClassNetCacheDescriptors.cs": r'''
internal static class AgentClassNetCacheDescriptors
{
    private const string KillFunctionName = "MulticastNotifyKilledEnemy";
    public static IReadOnlyList<ClassNetCacheDescriptor> Create(
        IEnumerable<ExportGroupDescriptor> agentDescriptors)
    {
        return agentDescriptors
            .Select(agent => new ClassNetCacheDescriptor(
                agent.Path + "_ClassNetCache", [CreateKillRpc()]))
            .ToArray();
    }
    private static RpcDescriptor CreateKillRpc() => new RpcDescriptor
    {
        Name = KillFunctionName,
    };
}
''',
            }
        )

        self.assertEqual(
            {
                (group, field, field_type.strip())
                for group, field, field_type in ENTRY_RE.findall(output)
                if group.endswith("_ClassNetCache")
            },
            {
                (
                    "/Game/Characters/Alpha/Alpha_PC.Alpha_PC_C_ClassNetCache",
                    "MulticastNotifyKilledEnemy",
                    "FieldType::Skip",
                ),
            },
        )

    def test_fully_qualified_ability_override_suppresses_runtime_agent_cache(self):
        output = self.run_generator(
            {
                "GenericAgentDescriptor.cs": r'''
public abstract class GenericAgentDescriptor : ExportGroupDescriptor<GenericAgentDescriptor>
{
    public override ExportCategory Categories => ExportCategory.Agent;
    protected override void Configure() { AddProperty(x => x.Owner).ObjectNetGuid(); }
}
''',
                "OrdinaryAgentDescriptor.cs": r'''
public sealed class OrdinaryAgentDescriptor : GenericAgentDescriptor
{
    public override string Path => "/Game/Agents/Ordinary.Ordinary_C";
}
''',
                "QualifiedAbilityDescriptor.cs": r'''
public sealed class QualifiedAbilityDescriptor : GenericAgentDescriptor
{
    public override string Path => "/Game/Abilities/Qualified.Qualified_C";
    private string Display => $"{Format("/*")}";
    public override Replay./* namespace trivia */Models.Descriptors.ExportCategory Categories =>
        global::Replay.Models.Descriptors.ExportCategory /* value trivia */ . Ability;
}
''',
                "AgentClassNetCacheDescriptors.cs": RUNTIME_AGENT_CACHE_FACTORY,
            }
        )

        self.assertEqual(
            self.runtime_cache_entries(output),
            {
                (
                    "/Game/Agents/Ordinary.Ordinary_C_ClassNetCache",
                    "MulticastNotifyKilledEnemy",
                    "FieldType::Skip",
                ),
            },
        )

    def test_alias_category_override_fails_loudly(self):
        error = self.run_generator_expecting_failure(
            {
                "GenericAgentDescriptor.cs": r'''
public abstract class GenericAgentDescriptor : ExportGroupDescriptor<GenericAgentDescriptor>
{
    public override ExportCategory Categories => ExportCategory.Agent;
}
''',
                "AliasAbilityDescriptor.cs": r'''
using EC = Replay.Models.Descriptors.ExportCategory;

public sealed class AliasAbilityDescriptor : GenericAgentDescriptor
{
    public override string Path => "/Game/Abilities/Alias.Alias_C";
    public override EC Categories => EC.Ability;
}
''',
                "AgentClassNetCacheDescriptors.cs": RUNTIME_AGENT_CACHE_FACTORY,
            }
        )

        self.assertIn("AliasAbilityDescriptor", error)
        self.assertIn("unsupported ExportCategory override", error)
        self.assertIn("EC", error)

    def test_escaped_alias_and_property_category_override_fails_loudly(self):
        error = self.run_generator_expecting_failure(
            {
                "GenericAgentDescriptor.cs": r'''
public abstract class GenericAgentDescriptor : ExportGroupDescriptor<GenericAgentDescriptor>
{
    public override ExportCategory Categories => ExportCategory.Agent;
}
''',
                "EscapedAliasAbilityDescriptor.cs": r'''
using @EC = Replay.Models.Descriptors.ExportCategory;

public sealed class EscapedAliasAbilityDescriptor : GenericAgentDescriptor
{
    public override string Path => "/Game/Abilities/EscapedAlias.EscapedAlias_C";
    public override @EC @Categories => @EC.Ability;
}
''',
                "AgentClassNetCacheDescriptors.cs": RUNTIME_AGENT_CACHE_FACTORY,
            }
        )

        self.assertIn("EscapedAliasAbilityDescriptor", error)
        self.assertIn("unsupported ExportCategory override", error)
        self.assertIn("@EC", error)

    def test_raw_string_category_decoys_preserve_inherited_agent_cache(self):
        variants = (
            (
                "plain",
                r'''
    private const string Decoy = """"
three quotes remain content: """
public override ExportCategory Categories => ExportCategory.Ability;
"""";
''',
                "/Game/RawStrings/Plain.Plain_C",
                "/Game/RawStrings/Plain.Plain_C_ClassNetCache",
            ),
            (
                "interpolated",
                r'''
    private string Decoy => $""""
{Format("\"\"\"")}
public override ExportCategory Categories => ExportCategory.Ability;
"""";
''',
                "/Game/RawStrings/Interpolated.Interpolated_C",
                "/Game/RawStrings/Interpolated.Interpolated_C_ClassNetCache",
            ),
            (
                "double-dollar-four-quotes",
                r'''
    private string Decoy => $$""""
{ "literal": "single braces" }
{{Format("""inner raw""")}}
public override ExportCategory Categories => ExportCategory.Ability;
"""";
''',
                "/Game/RawStrings/DoubleDollar.DoubleDollar_C",
                "/Game/RawStrings/DoubleDollar.DoubleDollar_C_ClassNetCache",
            ),
        )

        for label, declaration, path, expected_group in variants:
            with self.subTest(label=label):
                class_name = f"{label.replace('-', '').title()}AgentDescriptor"
                output = self.run_generator(
                    {
                        "GenericAgentDescriptor.cs": r'''
public abstract class GenericAgentDescriptor : ExportGroupDescriptor<GenericAgentDescriptor>
{
    public override ExportCategory Categories => ExportCategory.Agent;
}
''',
                        f"{class_name}.cs": f'''
public sealed class {class_name} : GenericAgentDescriptor
{{
    public override string Path => "{path}";
{declaration}
}}
''',
                        "AgentClassNetCacheDescriptors.cs": RUNTIME_AGENT_CACHE_FACTORY,
                    }
                )

                self.assertEqual(
                    self.runtime_cache_entries(output),
                    {
                        (
                            expected_group,
                            "MulticastNotifyKilledEnemy",
                            "FieldType::Skip",
                        ),
                    },
                )

    def test_nested_category_override_does_not_change_outer_category(self):
        output = self.run_generator(
            {
                "GenericAgentDescriptor.cs": r'''
public abstract class GenericAgentDescriptor : ExportGroupDescriptor<GenericAgentDescriptor>
{
    public override ExportCategory Categories => ExportCategory.Agent;
}
''',
                "OuterAgentDescriptor.cs": r'''
public sealed class OuterAgentDescriptor : GenericAgentDescriptor
{
    public override string Path => "/Game/Agents/Outer.Outer_C";

    public sealed class NestedAbilityDescriptor : GenericAgentDescriptor
    {
        public override string Path => "/Game/Abilities/Nested.Nested_C";
        public override ExportCategory Categories => ExportCategory.Ability;
    }
}
''',
                "OuterAbilityDescriptor.cs": r'''
public sealed class OuterAbilityDescriptor : GenericAgentDescriptor
{
    public sealed class NestedAgentDescriptor : GenericAgentDescriptor
    {
        public override ExportCategory Categories => ExportCategory.Agent;
    }

    public override string Path => "/Game/Abilities/Outer.Outer_C";
    public override ExportCategory Categories => ExportCategory.Ability;
}
''',
                "AgentClassNetCacheDescriptors.cs": RUNTIME_AGENT_CACHE_FACTORY,
            }
        )

        self.assertEqual(
            self.runtime_cache_entries(output),
            {
                (
                    "/Game/Agents/Outer.Outer_C_ClassNetCache",
                    "MulticastNotifyKilledEnemy",
                    "FieldType::Skip",
                ),
            },
        )

    def test_nested_path_does_not_supply_outer_path(self):
        output = self.run_generator(
            {
                "GenericAgentDescriptor.cs": r'''
public abstract class GenericAgentDescriptor : ExportGroupDescriptor<GenericAgentDescriptor>
{
    public override ExportCategory Categories => ExportCategory.Agent;
}
''',
                "GenericAbilityDescriptor.cs": r'''
public abstract class GenericAbilityDescriptor : ExportGroupDescriptor<GenericAbilityDescriptor>
{
    public override ExportCategory Categories => ExportCategory.Ability;
}
''',
                "OuterAgentDescriptor.cs": r'''
public sealed class OuterAgentDescriptor : GenericAgentDescriptor
{
    public sealed class NestedAbilityDescriptor : GenericAbilityDescriptor
    {
        public override string Path => "/Game/Abilities/Nested.Nested_C";
        protected override void Configure()
        {
            AddProperty(x => x.NestedSecret).ObjectNetGuid();
        }
    }

    public override string Path => "/Game/Agents/Outer.Outer_C";
    protected override void Configure()
    {
        AddProperty(x => x.OuterOwner).ObjectNetGuid();
    }
}
''',
                "AgentClassNetCacheDescriptors.cs": RUNTIME_AGENT_CACHE_FACTORY,
            }
        )

        descriptor_entries = {
            (group, field, field_type.strip())
            for group, field, field_type in ENTRY_RE.findall(output)
            if not group.endswith("_ClassNetCache")
        }
        self.assertEqual(
            descriptor_entries,
            {
                (
                    "/Game/Abilities/Nested.Nested_C",
                    "NestedSecret",
                    "FieldType::ObjectNetGuid",
                ),
                (
                    "/Game/Agents/Outer.Outer_C",
                    "OuterOwner",
                    "FieldType::ObjectNetGuid",
                ),
            },
        )
        self.assertEqual(
            self.runtime_cache_entries(output),
            {
                (
                    "/Game/Agents/Outer.Outer_C_ClassNetCache",
                    "MulticastNotifyKilledEnemy",
                    "FieldType::Skip",
                ),
            },
        )

    def test_nested_configure_does_not_supply_outer_fields(self):
        output = self.run_generator(
            {
                "OuterDescriptor.cs": r'''
public sealed class OuterDescriptor : ExportGroupDescriptor<OuterDescriptor>
{
    public override string Path => "/Game/Outer.Outer_C";

    public sealed class NestedDescriptor : ExportGroupDescriptor<NestedDescriptor>
    {
        public override string Path => "/Game/Nested.Nested_C";
        protected override void Configure()
        {
            AddProperty(x => x.NestedSecret).ObjectNetGuid();
        }
    }

    protected override void Configure()
    {
        AddProperty(x => x.OuterOwner).ObjectNetGuid();
    }
}
''',
            }
        )

        descriptor_entries = {
            (group, field, field_type.strip())
            for group, field, field_type in ENTRY_RE.findall(output)
        }
        self.assertEqual(
            descriptor_entries,
            {
                (
                    "/Game/Nested.Nested_C",
                    "NestedSecret",
                    "FieldType::ObjectNetGuid",
                ),
                (
                    "/Game/Outer.Outer_C",
                    "OuterOwner",
                    "FieldType::ObjectNetGuid",
                ),
            },
        )

    def test_expression_bodied_configure_stops_before_nested_type(self):
        output = self.run_generator(
            {
                "OuterDescriptor.cs": r'''
public sealed class OuterDescriptor : ExportGroupDescriptor<OuterDescriptor>
{
    public override string Path => "/Game/Outer.Outer_C";
    protected override void Configure() =>
        AddProperty(x => x.OuterOwner).ObjectNetGuid();

    public sealed class NestedDescriptor : ExportGroupDescriptor<NestedDescriptor>
    {
        public override string Path => "/Game/Nested.Nested_C";
        protected override void Configure()
        {
            AddProperty(x => x.NestedSecret).ObjectNetGuid();
        }
    }
}
''',
            }
        )

        descriptor_entries = {
            (group, field, field_type.strip())
            for group, field, field_type in ENTRY_RE.findall(output)
        }
        self.assertEqual(
            descriptor_entries,
            {
                (
                    "/Game/Nested.Nested_C",
                    "NestedSecret",
                    "FieldType::ObjectNetGuid",
                ),
                (
                    "/Game/Outer.Outer_C",
                    "OuterOwner",
                    "FieldType::ObjectNetGuid",
                ),
            },
        )

    def test_category_like_comments_preserve_real_and_inherited_categories(self):
        output = self.run_generator(
            {
                "GenericAgentDescriptor.cs": r'''
public abstract class GenericAgentDescriptor : ExportGroupDescriptor<GenericAgentDescriptor>
{
    public override ExportCategory Categories => ExportCategory.Agent;
    protected override void Configure() { AddProperty(x => x.Owner).ObjectNetGuid(); }
}
''',
                "CommentedInheritedAgentDescriptor.cs": r'''
public sealed class CommentedInheritedAgentDescriptor : GenericAgentDescriptor
{
    public override string Path => "/Game/Comments//Inherited/*literal*/.Inherited_C";
    private const string Regular =
        "escaped \" // public override ExportCategory Categories => ExportCategory.Ability;";
    private const string Verbatim =
        @"escaped "" /* public override ExportCategory Categories => ExportCategory.Ability;";
    private string Interpolated =>
        $"// public override ExportCategory Categories => ExportCategory.Ability;";
    private string InterpolatedVerbatim =>
        $@"/* public override ExportCategory Categories => ExportCategory.Ability;";
    private string VerbatimInterpolated =>
        @$"// public override ExportCategory Categories => ExportCategory.Ability;";
    private const char Slash = '/';
    private const char Quote = '\'';
    // public override ExportCategory Categories => ExportCategory.Ability;
    /*
    public override ExportCategory Categories
    {
        get => ExportCategory.Ability;
    }
    */
}
''',
                "CommentedAbilityDescriptor.cs": r'''
public sealed class CommentedAbilityDescriptor : GenericAgentDescriptor
{
    public override string Path => "/Game/Comments/Ability.Ability_C";
    /* public override ExportCategory Categories => ExportCategory.Agent; */
    // } This comment does not close the class.
    public override ExportCategory Categories => ExportCategory.Ability;
}
''',
                "AgentClassNetCacheDescriptors.cs": RUNTIME_AGENT_CACHE_FACTORY,
            }
        )

        self.assertEqual(
            self.runtime_cache_entries(output),
            {
                (
                    "/Game/Comments//Inherited/*literal*/.Inherited_C_ClassNetCache",
                    "MulticastNotifyKilledEnemy",
                    "FieldType::Skip",
                ),
            },
        )

    def test_agent_flags_and_all_retain_runtime_agent_caches(self):
        output = self.run_generator(
            {
                "GenericAbilityDescriptor.cs": r'''
public abstract class GenericAbilityDescriptor : ExportGroupDescriptor<GenericAbilityDescriptor>
{
    public override ExportCategory Categories => ExportCategory.Ability;
    protected override void Configure() { AddProperty(x => x.Owner).ObjectNetGuid(); }
}
''',
                "AgentAbilityDescriptor.cs": r'''
public sealed class AgentAbilityDescriptor : GenericAbilityDescriptor
{
    public override string Path => "/Game/Flags/AgentAbility.AgentAbility_C";
    public override Replay.Models.Descriptors.ExportCategory Categories =>
        Replay.Models.Descriptors.ExportCategory.Agent |
        Replay.Models.Descriptors.ExportCategory.Ability;
}
''',
                "AllCategoriesDescriptor.cs": r'''
public sealed class AllCategoriesDescriptor : GenericAbilityDescriptor
{
    public override string Path => "/Game/Flags/All.All_C";
    public override Replay.Models.Descriptors.ExportCategory Categories =>
        Replay.Models.Descriptors.ExportCategory.All;
}
''',
                "AgentClassNetCacheDescriptors.cs": RUNTIME_AGENT_CACHE_FACTORY,
            }
        )

        self.assertEqual(
            self.runtime_cache_entries(output),
            {
                (
                    "/Game/Flags/AgentAbility.AgentAbility_C_ClassNetCache",
                    "MulticastNotifyKilledEnemy",
                    "FieldType::Skip",
                ),
                (
                    "/Game/Flags/All.All_C_ClassNetCache",
                    "MulticastNotifyKilledEnemy",
                    "FieldType::Skip",
                ),
            },
        )

    def test_unknown_category_override_fails_loudly(self):
        error = self.run_generator_expecting_failure(
            {
                "UnknownCategoryDescriptor.cs": r'''
public sealed class UnknownCategoryDescriptor : ExportGroupDescriptor<UnknownCategoryDescriptor>
{
    public override string Path => "/test/unknown";
    public override ExportCategory Categories => ExportCategory.Telepathy;
    protected override void Configure() { AddProperty(x => x.Owner).ObjectNetGuid(); }
}
'''
            }
        )

        self.assertIn("UnknownCategoryDescriptor", error)
        self.assertIn("unknown ExportCategory Telepathy", error)

    def test_malformed_category_override_fails_loudly(self):
        error = self.run_generator_expecting_failure(
            {
                "MalformedCategoryDescriptor.cs": r'''
public sealed class MalformedCategoryDescriptor : ExportGroupDescriptor<MalformedCategoryDescriptor>
{
    public override string Path => "/test/malformed";
    public override ExportCategory Categories
    {
        get => ExportCategory.Ability;
    }
    protected override void Configure() { AddProperty(x => x.Owner).ObjectNetGuid(); }
}
'''
            }
        )

        self.assertIn("MalformedCategoryDescriptor", error)
        self.assertIn("unsupported ExportCategory override", error)

    def test_runtime_cache_factory_missing_method_fails_loudly(self):
        error = self.run_generator_expecting_failure(
            {
                "AgentClassNetCacheDescriptors.cs": r'''
internal static class AgentClassNetCacheDescriptors
{
    public static IReadOnlyList<ClassNetCacheDescriptor> Create(
        IEnumerable<ExportGroupDescriptor> agentDescriptors)
    {
        return agentDescriptors
            .Select(agent => new ClassNetCacheDescriptor(
                agent.Path + "_ClassNetCache", [CreateKillRpc()]))
            .ToArray();
    }
}
'''
            }
        )

        self.assertIn("runtime ClassNetCache factory CreateKillRpc", error)
        self.assertIn("method body", error)

    def test_runtime_cache_factory_name_is_bounded_to_its_method(self):
        error = self.run_generator_expecting_failure(
            {
                "AgentClassNetCacheDescriptors.cs": r'''
internal static class AgentClassNetCacheDescriptors
{
    public static IReadOnlyList<ClassNetCacheDescriptor> Create(
        IEnumerable<ExportGroupDescriptor> agentDescriptors)
    {
        return agentDescriptors
            .Select(agent => new ClassNetCacheDescriptor(
                agent.Path + "_ClassNetCache", [CreateKillRpc()]))
            .ToArray();
    }

    private static RpcDescriptor CreateKillRpc()
    {
        return new RpcDescriptor
        {
            FunctionExportPath = "/Script/ShooterGame.Agent:MulticastNotifyKilledEnemy",
        };
    }

    private static RpcDescriptor UnrelatedRpc() => new RpcDescriptor
    {
        Name = "MustNotBeCaptured",
    };
}
'''
            }
        )

        self.assertIn("runtime ClassNetCache factory CreateKillRpc", error)
        self.assertIn("RpcDescriptor.Name", error)

    def test_commented_runtime_cache_factory_is_ignored(self):
        output = self.run_generator(
            {
                "GenericAgentDescriptor.cs": r'''
public abstract class GenericAgentDescriptor : ExportGroupDescriptor<GenericAgentDescriptor>
{
    public override ExportCategory Categories => ExportCategory.Agent;
}
''',
                "LiveAgentDescriptor.cs": r'''
public sealed class LiveAgentDescriptor : GenericAgentDescriptor
{
    public override string Path => "/Game/Agents/Live.Live_C";
}
''',
                "AgentClassNetCacheDescriptors.cs": RUNTIME_AGENT_CACHE_FACTORY
                + r'''
/*
private static RpcDescriptor CreateBogusRpc() => new RpcDescriptor
{
    Name = "MustNotBeCaptured",
};

private static void CreateBogusCaches(
    IEnumerable<ExportGroupDescriptor> agentDescriptors)
{
    _ = agentDescriptors.Select(agent => new ClassNetCacheDescriptor(
        agent.Path + "_Bogus_ClassNetCache", [CreateBogusRpc()]));
}
*/
''',
            }
        )

        self.assertEqual(
            self.runtime_cache_entries(output),
            {
                (
                    "/Game/Agents/Live.Live_C_ClassNetCache",
                    "MulticastNotifyKilledEnemy",
                    "FieldType::Skip",
                ),
            },
        )

    def test_runtime_cache_factory_syntax_inside_string_is_ignored(self):
        output = self.run_generator(
            {
                "GenericAgentDescriptor.cs": r'''
public abstract class GenericAgentDescriptor : ExportGroupDescriptor<GenericAgentDescriptor>
{
    public override ExportCategory Categories => ExportCategory.Agent;
}
''',
                "LiveAgentDescriptor.cs": r'''
public sealed class LiveAgentDescriptor : GenericAgentDescriptor
{
    public override string Path => "/Game/Agents/Live.Live_C";
}
''',
                "AgentClassNetCacheDescriptors.cs": r'''
internal static class AgentClassNetCacheDescriptors
{
    private const string KillFunctionName = "MulticastNotifyKilledEnemy";
    public static IReadOnlyList<ClassNetCacheDescriptor> Create(
        IEnumerable<ExportGroupDescriptor> agentDescriptors)
    {
        return agentDescriptors
            .Select(agent => new ClassNetCacheDescriptor(
                agent.Path + "_ClassNetCache", [CreateKillRpc()]))
            .ToArray();
    }
    private static RpcDescriptor CreateKillRpc() => new RpcDescriptor
    {
        Name = KillFunctionName,
    };

    private static string BuildDiagnostic(GenericAgentDescriptor agent)
    {
        return agent.Path + "_Bogus_ClassNetCache\", [CreateBogusRpc()]";
    }
    private static RpcDescriptor CreateBogusRpc() => new RpcDescriptor
    {
        Name = "MustNotBeCaptured",
    };
}
''',
            }
        )

        runtime_entries = {
            (group, field, field_type.strip())
            for group, field, field_type in ENTRY_RE.findall(output)
            if field in {"MulticastNotifyKilledEnemy", "MustNotBeCaptured"}
        }
        self.assertEqual(
            runtime_entries,
            {
                (
                    "/Game/Agents/Live.Live_C_ClassNetCache",
                    "MulticastNotifyKilledEnemy",
                    "FieldType::Skip",
                ),
            },
        )

    def test_literal_property_handles_emit_handle_metadata(self):
        output = self.run_generator(
            {
                "ReplayParameters.cs": r'''
public sealed class ReplayParameters : ExportGroupDescriptor<ReplayParameters>
{
    public override string Path => "/Script/ShooterGame.ReplayEffectComponent:ReplayPlayContinuousEffectAtLocation";
    protected override void Configure()
    {
        AddPropertyHandle(26, x => x.Location, ExportCategory.Effects).FVector();
        AddPropertyHandle(27, x => x.Rotation, ExportCategory.Effects).FRotatorShort();
        AddProperty(x => x.StartMovementTime, ExportCategory.Effects).Float();
    }
}
'''
            }
        )

        self.assertEqual(
            {
                (group, int(handle), field)
                for group, handle, field in HANDLE_ENTRY_RE.findall(output)
            },
            {
                (
                    "/Script/ShooterGame.ReplayEffectComponent:ReplayPlayContinuousEffectAtLocation",
                    26,
                    "Location",
                ),
                (
                    "/Script/ShooterGame.ReplayEffectComponent:ReplayPlayContinuousEffectAtLocation",
                    27,
                    "Rotation",
                ),
            },
        )

    def test_decoder_arguments_do_not_emit_handle_metadata(self):
        output = self.run_generator(
            {
                "BoundedPayloadDescriptor.cs": r'''
public sealed class BoundedPayloadDescriptor : ExportGroupDescriptor<BoundedPayloadDescriptor>
{
    public override string Path => "/Game/Effects/Bounded.Bounded_C";
    protected override void Configure()
    {
        AddProperty(x => x.Payload).Decode(SomeDecoder.WithBounds(9, 10));
    }
}
'''
            }
        )

        self.assertEqual(HANDLE_ENTRY_RE.findall(output), [])
        self.assertIn(
            (
                "/Game/Effects/Bounded.Bounded_C",
                "Payload",
                "FieldType::Raw",
            ),
            {
                (group, field, field_type.strip())
                for group, field, field_type in ENTRY_RE.findall(output)
            },
        )


if __name__ == "__main__":
    unittest.main()
