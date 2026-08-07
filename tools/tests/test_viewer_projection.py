"""Projection and park-slot rules for the 2D viewer.

The axes cross. `pos_y` drives the horizontal output and `pos_x` the vertical,
which is the one of four sign/order variants that puts 100% of live positions
inside the unit square on eleven of twelve maps. The obvious reading collapses
to 0.9% on Haven, so this file pins the crossing explicitly rather than
trusting anyone to remember it.
"""
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import viewer_projection as vp  # noqa: E402

# Ascent's published constants, used only as a realistic shape.
ASCENT = vp.MapConstants(
    map_url="/Game/Maps/Ascent/Ascent",
    x_multiplier=7.2e-05,
    y_multiplier=-7.2e-05,
    x_scalar=0.500202,
    y_scalar=0.510265,
    display_icon_url="https://example.invalid/ascent.png",
)


class ProjectionTests(unittest.TestCase):
    def test_pos_y_drives_the_horizontal_axis(self):
        """Feeding pos_x to u is the variant that collapses on Haven."""
        u_a, _ = vp.project(0.0, 10000.0, ASCENT)
        u_b, _ = vp.project(10000.0, 0.0, ASCENT)
        self.assertNotAlmostEqual(u_a, u_b)
        self.assertAlmostEqual(u_a, 10000.0 * ASCENT.x_multiplier + ASCENT.x_scalar)

    def test_pos_x_drives_the_vertical_axis(self):
        _, v = vp.project(10000.0, 0.0, ASCENT)
        self.assertAlmostEqual(v, 10000.0 * ASCENT.y_multiplier + ASCENT.y_scalar)


class ParkSlotTests(unittest.TestCase):
    def test_a_parked_actor_needs_both_axes_to_qualify(self):
        self.assertTrue(vp.is_parked(-50000.0, -49900.0))

    def test_a_deep_fall_is_not_a_parked_actor(self):
        """z alone would misclassify this: a real player falling off Abyss."""
        self.assertFalse(vp.is_parked(1234.0, -49900.0))

    def test_a_far_x_alone_is_not_a_parked_actor(self):
        self.assertFalse(vp.is_parked(-50000.0, 120.0))


class ConstantsTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.cache = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.payload = json.dumps({"data": [{
            "mapUrl": "/Game/Maps/Ascent/Ascent",
            "xMultiplier": 7.2e-05, "yMultiplier": -7.2e-05,
            "xScalarToAdd": 0.500202, "yScalarToAdd": 0.510265,
            "displayIcon": "https://example.invalid/ascent.png",
        }]}).encode()

    def test_a_fetched_map_is_cached_and_the_second_call_does_not_fetch(self):
        calls = []

        def fetch(url):
            calls.append(url)
            return self.payload

        first = vp.load_constants("/Game/Maps/Ascent/Ascent", self.cache, fetch=fetch)
        second = vp.load_constants("/Game/Maps/Ascent/Ascent", self.cache, fetch=fetch)
        self.assertEqual(first, second)
        self.assertEqual(len(calls), 1, "the cache did not prevent a second fetch")

    def test_an_unavailable_transform_fails_the_build(self):
        """No constants means no projection. Drawing on a blank square at a
        guessed scale would be a plausible wrong picture, which is worse than
        no picture -- the same rule the decoder follows."""
        def fetch(url):
            raise OSError("network down")

        with self.assertRaises(vp.ConstantsUnavailable):
            vp.load_constants("/Game/Maps/Ascent/Ascent", self.cache, fetch=fetch)

    def test_a_map_absent_from_the_published_list_fails_by_name(self):
        def fetch(url):
            return self.payload

        with self.assertRaises(vp.ConstantsUnavailable) as caught:
            vp.load_constants("/Game/Maps/Nowhere/Nowhere", self.cache, fetch=fetch)
        self.assertIn("Nowhere", str(caught.exception))

    def test_malformed_json_does_not_persist_a_cache_and_raises_unavailable(self):
        """Bad JSON should not be written to disk, and should raise
        ConstantsUnavailable instead of a raw JSONDecodeError."""
        def fetch(url):
            return b"not json"

        with self.assertRaises(vp.ConstantsUnavailable):
            vp.load_constants("/Game/Maps/Ascent/Ascent", self.cache, fetch=fetch)
        # A subsequent call with good fetch should succeed, proving the poison
        # was not persisted.
        def good_fetch(url):
            return self.payload
        result = vp.load_constants("/Game/Maps/Ascent/Ascent", self.cache,
                                    fetch=good_fetch)
        self.assertEqual(result.map_url, "/Game/Maps/Ascent/Ascent")

    def test_missing_required_key_raises_unavailable_not_key_error(self):
        """An entry that matches mapUrl but is missing yScalarToAdd should
        raise ConstantsUnavailable, not a raw KeyError."""
        bad_payload = json.dumps({"data": [{
            "mapUrl": "/Game/Maps/Ascent/Ascent",
            "xMultiplier": 7.2e-05, "yMultiplier": -7.2e-05,
            "xScalarToAdd": 0.500202,
            # missing yScalarToAdd
            "displayIcon": "https://example.invalid/ascent.png",
        }]}).encode()

        def fetch(url):
            return bad_payload

        with self.assertRaises(vp.ConstantsUnavailable):
            vp.load_constants("/Game/Maps/Ascent/Ascent", self.cache, fetch=fetch)

    def test_a_poisoned_cache_is_retried_on_next_call(self):
        """If the cache file is unparseable, a subsequent call with a working
        fetch should clear the poison and succeed."""
        # Write bad JSON to the cache file manually.
        self.cache.mkdir(parents=True, exist_ok=True)
        cached = self.cache / "maps.json"
        cached.write_bytes(b"not json")

        # Now call with a good fetch. It should treat the bad cache as a miss
        # and re-fetch.
        def good_fetch(url):
            return self.payload

        result = vp.load_constants("/Game/Maps/Ascent/Ascent", self.cache,
                                    fetch=good_fetch)
        self.assertEqual(result.map_url, "/Game/Maps/Ascent/Ascent")

    def test_a_wrong_shaped_cache_entry_recovers_via_fetch(self):
        """A cache file that parses as valid JSON but holds non-dict entries
        under "data" (e.g. {"data": [1, 2, 3]}) must not escape the
        cache-read branch as a raw AttributeError: `entry.get("mapUrl")`
        raises AttributeError when `entry` is an int, not a dict, and
        `cached.is_file()` short-circuits the fetch path's own guard below
        it, so an uncaught AttributeError here would become permanently
        fatal rather than a recoverable cache miss. This is the poisoned-
        cache bug this branch spent two fix rounds closing (see
        `test_a_poisoned_cache_is_retried_on_next_call` and
        `test_wrong_shaped_json_raises_unavailable_not_attribute_error`
        above and below) -- narrowing the except tuple on the cache-read
        branch back to (json.JSONDecodeError, KeyError, ValueError) drops
        AttributeError and reintroduces it silently."""
        self.cache.mkdir(parents=True, exist_ok=True)
        cached = self.cache / "maps.json"
        cached.write_text(json.dumps({"data": [1, 2, 3]}), encoding="utf-8")

        calls = []

        def good_fetch(url):
            calls.append(url)
            return self.payload

        result = vp.load_constants("/Game/Maps/Ascent/Ascent", self.cache,
                                    fetch=good_fetch)
        self.assertEqual(result.map_url, "/Game/Maps/Ascent/Ascent")
        self.assertEqual(len(calls), 1,
                         "recovery must go through fetch, not some other path")


class MinimapTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.cache = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.icon_bytes = b"\x89PNG\r\n\x1a\n" + b"\x00" * 100  # Fake PNG header
        self.k = vp.MapConstants(
            map_url="/Game/Maps/Ascent/Ascent",
            x_multiplier=7.2e-05,
            y_multiplier=-7.2e-05,
            x_scalar=0.500202,
            y_scalar=0.510265,
            display_icon_url="https://example.invalid/ascent.png",
        )

    def test_the_minimap_bytes_come_back_unchanged(self):
        def fetch(url):
            return self.icon_bytes

        result = vp.load_minimap_png(self.k, self.cache, fetch=fetch)
        self.assertEqual(result, self.icon_bytes)

    def test_the_second_call_does_not_re_fetch(self):
        calls = []

        def fetch(url):
            calls.append(url)
            return self.icon_bytes

        first = vp.load_minimap_png(self.k, self.cache, fetch=fetch)
        second = vp.load_minimap_png(self.k, self.cache, fetch=fetch)
        self.assertEqual(first, second)
        self.assertEqual(len(calls), 1, "the cache did not prevent a second fetch")

    def test_a_failing_fetch_raises_unavailable(self):
        def fetch(url):
            raise OSError("network down")

        with self.assertRaises(vp.ConstantsUnavailable):
            vp.load_minimap_png(self.k, self.cache, fetch=fetch)

    def test_a_failing_fetch_does_not_persist_a_cache_file(self):
        """If fetch fails, no .png file should exist on disk."""
        def fetch(url):
            raise OSError("network down")

        with self.assertRaises(vp.ConstantsUnavailable):
            vp.load_minimap_png(self.k, self.cache, fetch=fetch)
        # Verify no cache file was created.
        name = self.k.map_url.strip("/").replace("/", "_") + ".png"
        cached = self.cache / name
        self.assertFalse(cached.exists(),
                        f"cache file {cached} should not exist after failed fetch")


class ConstantsCacheFileTests(unittest.TestCase):
    """Tests that verify cache files are not persisted on validation failure."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.cache = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.payload = json.dumps({"data": [{
            "mapUrl": "/Game/Maps/Ascent/Ascent",
            "xMultiplier": 7.2e-05, "yMultiplier": -7.2e-05,
            "xScalarToAdd": 0.500202, "yScalarToAdd": 0.510265,
            "displayIcon": "https://example.invalid/ascent.png",
        }]}).encode()

    def test_malformed_json_does_not_persist_cache_file(self):
        """Fetching malformed JSON must not leave a cache file on disk."""
        def fetch(url):
            return b"not json"

        with self.assertRaises(vp.ConstantsUnavailable):
            vp.load_constants("/Game/Maps/Ascent/Ascent", self.cache, fetch=fetch)
        # Verify no cache file was created.
        cached = self.cache / "maps.json"
        self.assertFalse(cached.exists(),
                        f"cache file {cached} should not exist after malformed JSON")

    def test_missing_required_key_does_not_persist_cache_file(self):
        """Entry matching mapUrl but missing yScalarToAdd must not be persisted."""
        bad_payload = json.dumps({"data": [{
            "mapUrl": "/Game/Maps/Ascent/Ascent",
            "xMultiplier": 7.2e-05, "yMultiplier": -7.2e-05,
            "xScalarToAdd": 0.500202,
            # missing yScalarToAdd
            "displayIcon": "https://example.invalid/ascent.png",
        }]}).encode()

        def fetch(url):
            return bad_payload

        with self.assertRaises(vp.ConstantsUnavailable):
            vp.load_constants("/Game/Maps/Ascent/Ascent", self.cache, fetch=fetch)
        # Verify no cache file was created.
        cached = self.cache / "maps.json"
        self.assertFalse(cached.exists(),
                        f"cache file {cached} should not exist after missing key")

    def test_wrong_shaped_json_raises_unavailable_not_attribute_error(self):
        """Cache with valid JSON but wrong shape (bare []) should raise
        ConstantsUnavailable, not AttributeError."""
        # Valid JSON but wrong shape: a bare list instead of dict with "data" key.
        bad_payload = json.dumps([]).encode()

        def fetch(url):
            return bad_payload

        with self.assertRaises(vp.ConstantsUnavailable):
            vp.load_constants("/Game/Maps/Ascent/Ascent", self.cache, fetch=fetch)


if __name__ == "__main__":
    unittest.main()
