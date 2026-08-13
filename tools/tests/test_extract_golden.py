import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPO = Path(__file__).resolve().parents[2]
GENERATOR = REPO / "tools" / "extract_golden.py"
BRANCHES = [
    "++Ares-Core+release-12.10",
    "++Ares-Core+release-12.11",
    "++Ares-Core+release-13.00",
    "++Ares-Core+release-13.01",
    "++Ares-Core+release-13.02",
]
BOUNDARIES = [0, 1, 7, 8, 31, 32, 63, 64, 65, 287, 288]


def fixture(cases: list[tuple[str, int]]) -> str:
    rows = []
    for branch, bits in cases:
        hex_text = "00" * ((bits + 7) // 8)
        rows.append(f'new("{branch}", {bits}, "{hex_text}")')
    return (
        'private const string PayloadHex = "' + ("00" * 36) + '";\n'
        "private const uint ActorNetGuid = 2;\n"
        "var cases = new[] {\n    " + ",\n    ".join(rows) + "\n};\n"
    )


class GoldenOracleContractTests(unittest.TestCase):
    def run_generator(self, cases, previous_output=""):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "fixture.cs"
            output = root / "golden_vectors.rs"
            source.write_text(fixture(cases), encoding="utf-8")
            if previous_output:
                output.write_text(previous_output, encoding="utf-8")
            result = subprocess.run(
                [sys.executable, str(GENERATOR), str(source), str(output)],
                capture_output=True,
                text=True,
                encoding="utf-8",
                check=False,
            )
            content = output.read_text(encoding="utf-8") if output.exists() else None
            return result, content

    @staticmethod
    def valid_cases():
        return [(branch, bits) for branch in BRANCHES for bits in BOUNDARIES]

    def test_exact_five_build_by_eleven_boundary_oracle_succeeds(self):
        result, output = self.run_generator(self.valid_cases())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("55 vectors across 5 builds", output)

    def test_non_55_oracle_is_rejected_without_replacing_previous_output(self):
        result, output = self.run_generator(
            self.valid_cases()[:-1], previous_output="previous oracle\n"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(output, "previous oracle\n")
        self.assertIn("55", result.stderr)

    def test_each_promised_build_must_have_the_full_boundary_set(self):
        cases = self.valid_cases()
        cases[-1] = (BRANCHES[0], cases[-1][1])
        result, _ = self.run_generator(cases)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(BRANCHES[-1], result.stderr)

    def test_duplicate_boundary_cannot_replace_a_missing_boundary(self):
        cases = self.valid_cases()
        cases[-1] = (BRANCHES[-1], 287)
        result, _ = self.run_generator(cases)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("288", result.stderr)


if __name__ == "__main__":
    unittest.main()
