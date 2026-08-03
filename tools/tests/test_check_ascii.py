import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPOSITORY_ROOT / "tools" / "check_ascii.py"
NESTED_WORKING_DIRECTORY = REPOSITORY_ROOT / "crates" / "vrfkit"


class CheckAsciiTests(unittest.TestCase):
    def run_checker(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(SCRIPT), *arguments],
            cwd=NESTED_WORKING_DIRECTORY,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_default_check_uses_full_repository_from_nested_directory(self):
        """Run from a nested crate, the checker must still scan the whole repo.

        The count is derived from `git ls-files`, not written here. It used to
        be the literal 61, which meant the test failed the moment a Rust file
        was added -- and it did, silently, for the four files two sessions
        added, because nothing ran this suite until it was put in QUICK START.
        A test that has to be edited whenever the codebase grows is a test that
        will be edited without being read.
        """
        tracked = subprocess.run(
            ["git", "-C", str(REPOSITORY_ROOT), "ls-files", "*.rs"],
            capture_output=True, text=True, check=True,
        ).stdout.split()
        self.assertGreater(len(tracked), 0, "no tracked Rust files found")

        result = self.run_checker("--check")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            result.stdout,
            f"OK: {len(tracked)} tracked Rust file(s), ASCII only\n",
        )
        self.assertEqual(result.stderr, "")

    def test_explicit_temporary_fixture_is_detected_from_nested_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory) / "planted.rs"
            fixture.write_bytes(b"// planted: \xc3\xa9\n")

            result = self.run_checker("--check", "--path", str(fixture))

        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertEqual(
            result.stderr,
            f"{fixture.as_posix()}:1:13: non-ASCII byte 0xC3\n"
            f"{fixture.as_posix()}:1:14: non-ASCII byte 0xA9\n"
            "FAILED: 1 line(s), 2 byte(s)\n",
        )

    def test_default_check_detects_tracked_fixture_outside_nested_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            repository = Path(directory)
            nested_directory = repository / "nested"
            nested_directory.mkdir()
            (repository / "tools").mkdir()
            copied_script = repository / "tools" / "check_ascii.py"
            shutil.copyfile(SCRIPT, copied_script)
            (nested_directory / "local.rs").write_bytes(b"// ASCII\n")
            (repository / "planted.rs").write_bytes(b"// planted: \xc3\xa9\n")
            subprocess.run(
                ["git", "init", "--quiet"], cwd=repository, check=True
            )
            subprocess.run(
                [
                    "git",
                    "-c",
                    "core.autocrlf=false",
                    "add",
                    "--",
                    "nested/local.rs",
                    "planted.rs",
                ],
                cwd=repository,
                check=True,
            )

            result = subprocess.run(
                [sys.executable, str(copied_script), "--check"],
                cwd=nested_directory,
                capture_output=True,
                text=True,
                check=False,
            )

        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertEqual(
            result.stderr,
            "planted.rs:1:13: non-ASCII byte 0xC3\n"
            "planted.rs:1:14: non-ASCII byte 0xA9\n"
            "FAILED: 1 line(s), 2 byte(s)\n",
        )


if __name__ == "__main__":
    unittest.main()
