import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = (
    Path(__file__).resolve().parents[2]
    / "crates"
    / "vrf-export"
    / "tests"
    / "python_interop.py"
)


class ExactFixtureSelectionTests(unittest.TestCase):
    @staticmethod
    def run_script(temp: Path, configured: Path | None = None):
        env = os.environ.copy()
        env.update({"TEMP": str(temp), "TMP": str(temp), "TMPDIR": str(temp)})
        env.pop("VRFKIT_INTEROP_DIR", None)
        if configured is not None:
            env["VRFKIT_INTEROP_DIR"] = str(configured)
        return subprocess.run(
            [sys.executable, str(SCRIPT)],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            env=env,
            timeout=30,
        )

    def test_a_stale_temp_fixture_is_never_selected_implicitly(self):
        with tempfile.TemporaryDirectory() as temp_raw:
            temp = Path(temp_raw)
            stale = temp / "vrf_export_tests_stale" / "interop"
            stale.mkdir(parents=True)
            (stale / "fields_interop.parquet").write_bytes(b"stale")
            (stale / "movement_interop.parquet").write_bytes(b"stale")

            result = self.run_script(temp)

        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, output)
        self.assertIn("explicit interop directory required", output.lower())
        self.assertNotIn(str(stale), output)

    def test_environment_selects_one_exact_fixture_directory(self):
        with tempfile.TemporaryDirectory() as temp_raw:
            temp = Path(temp_raw)
            exact = temp / "selected" / "interop"
            exact.mkdir(parents=True)

            result = self.run_script(temp, exact)

        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, output)
        self.assertIn("interop dir:", output.lower())
        self.assertIn(str(Path("selected") / "interop").lower(), output.lower())
        self.assertIn("interop parquet files not found", output.lower())


if __name__ == "__main__":
    unittest.main()
