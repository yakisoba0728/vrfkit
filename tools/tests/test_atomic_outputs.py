"""Output publishers must keep the previous complete file on write failure."""

from __future__ import annotations

import contextlib
import io
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import atomic_io  # noqa: E402
import bench_export  # noqa: E402
import check_metrics_baseline  # noqa: E402
import compare_with_csharp  # noqa: E402
import extract_equippables  # noqa: E402
import extract_sboxes  # noqa: E402


class AtomicOutputTests(unittest.TestCase):
    OLD = "previous complete output\n"

    def assert_preserved_when_replace_fails(
        self, output: Path, operation, *, previous: str | None = None
    ) -> None:
        previous = self.OLD if previous is None else previous
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(previous, encoding="utf-8")
        with mock.patch.object(
            atomic_io.os, "replace", side_effect=OSError("simulated replace failure")
        ):
            with self.assertRaisesRegex(OSError, "simulated replace failure"):
                operation()
        self.assertEqual(output.read_text(encoding="utf-8"), previous)

    def test_benchmark_baseline_update_preserves_previous_file(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            executable = root / "vrfkit"
            replay = root / "match.vrf"
            executable.write_bytes(b"exe")
            replay.write_bytes(b"replay")
            baseline = root / "bench.json"

            def update():
                argv = sys.argv
                sys.argv = [
                    "bench_export.py",
                    "--exe", str(executable),
                    "--replay", str(replay),
                    "--baseline", str(baseline),
                    "--repeats", "1",
                    "--update",
                ]
                try:
                    with mock.patch.object(bench_export, "time_export", return_value=[1.0]):
                        bench_export.main()
                finally:
                    sys.argv = argv

            self.assert_preserved_when_replace_fails(
                baseline, update, previous='{"sentinel": "previous"}\n'
            )

    def test_metrics_baseline_update_preserves_previous_file(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            executable = root / "vrfkit"
            compute_metrics = root / "compute_metrics.py"
            replay = root / "match.vrf"
            for path in (executable, compute_metrics, replay):
                path.write_bytes(b"fixture")
            baseline = root / "metrics.json"
            healthy = {
                "rounds_rpc": 1,
                "rounds_objective": 1,
                "team_score": {"Blue": 1},
                "players": 1,
                "kills": 0,
                "damage_dealt": 0,
            }

            def update():
                argv = sys.argv
                sys.argv = [
                    "check_metrics_baseline.py",
                    "--exe", str(executable),
                    "--baseline", str(baseline),
                    "--jobs", "1",
                    "--update",
                ]
                try:
                    with mock.patch.object(
                        check_metrics_baseline, "COMPUTE_METRICS", compute_metrics
                    ), mock.patch.object(
                        check_metrics_baseline, "REPLAYS", {"13.01": str(replay)}
                    ), mock.patch.object(
                        check_metrics_baseline,
                        "run_one",
                        return_value=("13.01", healthy, ""),
                    ):
                        check_metrics_baseline.main()
                finally:
                    sys.argv = argv

            self.assert_preserved_when_replace_fails(
                baseline, update, previous='{"metrics": {"old": {}}}\n'
            )

    def test_sbox_generator_preserves_previous_file(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "helpers.cs"
            output = root / "sboxes.rs"
            hex_table = bytes(range(256)).hex()
            source.write_text(
                "\n".join(
                    f'{name} = Convert.FromHexString("{hex_table}");'
                    for name in extract_sboxes.TABLES
                ),
                encoding="utf-8",
            )

            self.assert_preserved_when_replace_fails(
                output,
                lambda: extract_sboxes.main(
                    ["extract_sboxes.py", str(source), str(output)]
                ),
            )

    def test_equippable_generator_preserves_previous_file(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            resolver = root / extract_equippables.RESOLVER_RELPATH
            resolver.parent.mkdir(parents=True)
            resolver.write_text(
                'Define("/Game/Vandal.Vandal_C", "Vandal", '
                "ValorantEquippableCategory.Rifle)\n",
                encoding="utf-8",
            )
            output = root / "equippable_table.py"

            def generate():
                argv = sys.argv
                sys.argv = [
                    "extract_equippables.py", "--csharp-root", str(root)
                ]
                try:
                    with mock.patch.object(extract_equippables, "OUTPUT_PATH", output):
                        extract_equippables.main()
                finally:
                    sys.argv = argv

            self.assert_preserved_when_replace_fails(output, generate)

    def test_comparison_report_preserves_previous_file(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            csharp = root / "csharp"
            vrfkit = root / "vrfkit"
            csharp.mkdir()
            vrfkit.mkdir()
            (csharp / "manifest.json").write_text("{}", encoding="utf-8")
            (vrfkit / "manifest.json").write_text("{}", encoding="utf-8")
            report = vrfkit / "comparison_report.txt"

            def generate():
                argv = sys.argv
                sys.argv = ["compare_with_csharp.py", str(csharp), str(vrfkit)]
                try:
                    with contextlib.redirect_stdout(io.StringIO()), mock.patch.object(
                        compare_with_csharp, "compare_totals", return_value="totals"
                    ), mock.patch.object(
                        compare_with_csharp, "compare_group_paths", return_value="groups"
                    ), mock.patch.object(
                        compare_with_csharp,
                        "compare_group_field_coverage",
                        return_value=("coverage", []),
                    ), mock.patch.object(
                        compare_with_csharp, "compare_rpc_names", return_value="rpcs"
                    ), mock.patch.object(
                        compare_with_csharp, "compare_movement", return_value="movement"
                    ), mock.patch.object(
                        compare_with_csharp, "compare_raw_blobs", return_value="raw"
                    ):
                        compare_with_csharp.main()
                finally:
                    sys.argv = argv

            self.assert_preserved_when_replace_fails(report, generate)


if __name__ == "__main__":
    unittest.main()
