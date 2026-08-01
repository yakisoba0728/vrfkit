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


if __name__ == "__main__":
    unittest.main()
