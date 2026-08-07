"""Guards for the shared `.vrf` corpus discovery.

The defect: `validate_corpus.py` walked a corpus with `root.rglob("*.vrf")`
and `check_decode_errors_corpus.py` walked the same corpus with
`args.corpus.glob("*.vrf")`. Pointed at the same directory these produced 153
files versus 126 -- the 27-file gap lived in a `Demos/old` subdirectory, and
those 27 replays had their framing checked but never their overlay or
struct-blob decoders, with nothing printed to say the two tools disagreed.

`corpus_scan.discover()` is the single place both tools now ask "what is the
corpus", so they cannot drift apart again. The default is non-recursive (a
subdirectory is not guaranteed to hold more of the same corpus -- it could be
`Demos/old`, the live client's own archive of replays it is rotating out,
which may span builds); `--recursive` opts in on both tools at once. Whichever
mode runs, `discover()` reports how many files a non-recursive scan left out,
so the scope is legible from the printed output alone, not just from reading
which glob call a tool happens to make.
"""
import sys
import tempfile
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import corpus_scan  # noqa: E402


class DiscoverTests(unittest.TestCase):
    def _make_corpus(self, directory: Path) -> None:
        (directory / "a.vrf").write_bytes(b"")
        (directory / "b.vrf").write_bytes(b"")
        nested = directory / "old"
        nested.mkdir()
        (nested / "c.vrf").write_bytes(b"")
        (nested / "d.vrf").write_bytes(b"")
        (nested / "e.vrf").write_bytes(b"")
        # A non-.vrf file must never be picked up by either mode.
        (directory / "notes.txt").write_bytes(b"")

    def test_non_recursive_scans_top_level_only(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            self._make_corpus(root)
            scan = corpus_scan.discover(root, recursive=False)
            self.assertEqual([p.name for p in scan.files], ["a.vrf", "b.vrf"])

    def test_non_recursive_reports_the_excluded_count(self):
        """The 27-file gap from the audit: this is the number that must print."""
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            self._make_corpus(root)
            scan = corpus_scan.discover(root, recursive=False)
            self.assertEqual(scan.excluded, 3)

    def test_recursive_scans_everything_and_excludes_nothing(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            self._make_corpus(root)
            scan = corpus_scan.discover(root, recursive=True)
            self.assertEqual(len(scan.files), 5)
            self.assertEqual(scan.excluded, 0)

    def test_recursive_flag_is_recorded_on_the_scan(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            self._make_corpus(root)
            self.assertFalse(corpus_scan.discover(root, recursive=False).recursive)
            self.assertTrue(corpus_scan.discover(root, recursive=True).recursive)

    def test_a_flat_corpus_excludes_nothing_either_way(self):
        """No subdirectory at all: both modes must agree, and say so."""
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "a.vrf").write_bytes(b"")
            (root / "b.vrf").write_bytes(b"")
            flat = corpus_scan.discover(root, recursive=False)
            deep = corpus_scan.discover(root, recursive=True)
            self.assertEqual(flat.excluded, 0)
            self.assertEqual([p.name for p in flat.files], [p.name for p in deep.files])


class ScopeLineTests(unittest.TestCase):
    """The line a reader sees is the only thing that has to prove the scope."""

    def test_scope_line_names_file_count_and_root(self):
        scan = corpus_scan.CorpusScan(
            files=[Path("a.vrf"), Path("b.vrf")], scanned_root=Path("/corpus"),
            recursive=False, excluded=3)
        line = corpus_scan.scope_line(scan)
        self.assertIn("2", line)
        self.assertIn("corpus", line)

    def test_scope_line_states_the_excluded_count_even_when_nonzero(self):
        scan = corpus_scan.CorpusScan(
            files=[], scanned_root=Path("/corpus"), recursive=False, excluded=27)
        self.assertIn("27", corpus_scan.scope_line(scan))

    def test_scope_line_states_zero_excluded_explicitly(self):
        """A silent line here is how the original defect happened -- print 0."""
        scan = corpus_scan.CorpusScan(
            files=[], scanned_root=Path("/corpus"), recursive=False, excluded=0)
        self.assertIn("0", corpus_scan.scope_line(scan))

    def test_scope_line_names_recursive_mode(self):
        scan = corpus_scan.CorpusScan(
            files=[], scanned_root=Path("/corpus"), recursive=True, excluded=0)
        self.assertIn("recursive", corpus_scan.scope_line(scan))


if __name__ == "__main__":
    unittest.main()
