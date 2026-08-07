# 2D Replay Viewer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `tools/build_replay_viewer.py`, which turns one `vrfkit export` directory into a single self-contained HTML page that renders the match in 2D so a human can see whether the parsed data actually describes a game of VALORANT.

**Architecture:** Three importable Python modules (projection, data assembly, HTML emission) plus a thin CLI. The page is a static HTML template with data injected as base64-encoded typed arrays and the minimap embedded as a `data:` URI. Playback is downsampled to 20 Hz; the anomaly checks run over the full 125 Hz stream so a defect cannot hide between rendered frames.

**Tech Stack:** Python 3 (stdlib + `pyarrow`, already a dependency), `unittest`, vanilla JavaScript with Canvas 2D. No web framework, no CDN, no build step.

**Spec:** `docs/superpowers/specs/2026-08-07-2d-replay-viewer-design.md`

## Global Constraints

These apply to every task. Violating any of them fails CI.

- **Tests are `unittest`, not pytest.** CI runs `python -m unittest discover -s tools/tests -p "test_*.py"`. Test files live in `tools/tests/` and are named `test_*.py`. You may run a single test with `python -m unittest tools.tests.test_x.ClassName.test_name` from the repo root, or `python -m pytest <file> -q` locally for speed, but the suite must pass under `unittest`.
- **Every new `tools/*.py` file must be added to the tool table in `docs/USAGE.md` in the same commit.** `tools/check_docs.py:87` requires it and also compares a written tool count against `len(glob("tools/*.py"))`. A new tool with no doc row fails the build. `tools/viewer_template.html` is not a `.py` file and is exempt from the row, but mention it in the row of the tool that reads it.
- **ASCII only** in every file you touch. `tools/check_ascii.py` enforces this for Rust; keep Python and HTML ASCII too so the repo stays uniform. No smart quotes, no arrows, no emoji.
- **Strict TDD.** Write the test, run it, paste/observe the RED, then implement, then observe GREEN. A test that passes on first run proves nothing — if that happens, say so and label it a regression guard rather than counting it as TDD.
- **No network access at test time.** Every test must run offline. The one module that fetches (Task 2) is tested against a local cache directory and a stubbed fetcher.
- **Do not modify any Rust crate.** This is a `tools/` and `docs/` change only.
- **Verified constants, copied verbatim from the spec:**
  - Projection (the axes cross): `u = pos_y * xMultiplier + xScalarToAdd`, `v = pos_x * yMultiplier + yScalarToAdd`
  - Park slot: filter when `pos_x < -40000` **and** `pos_z < -40000`. Both axes. Filtering on z alone misclassifies real falls.
  - Teleport threshold: **3000 cm/s**, measured from the reference replay's own distribution (p90 is 659 cm/s, matching VALORANT's 675 cm/s run speed; 3000 leaves 391 of 1,773,814 samples, 0.022%).
  - Respawn grace: a displacement within **3000 ms** after a `roundStarted` is a respawn, not a teleport.
  - Playback rate: **20 Hz** (50 ms). Measurement rate: full data, ~125 Hz.

## File Structure

| File | Responsibility |
|---|---|
| `tools/viewer_projection.py` | Map constants (fetch + cache), world-to-minimap projection, park-slot filter. Knows nothing about rounds or layers. |
| `tools/viewer_data.py` | Round slicing, downsampling, the nine checks, layer assembly. Pure data; no HTML, no network. |
| `tools/viewer_template.html` | The page: canvas rendering, playback controls, layer toggles, findings list. Contains literal placeholder tokens the builder replaces. |
| `tools/build_replay_viewer.py` | CLI and orchestration. Reads an export dir, calls the two modules, injects into the template, writes one `.html`. |
| `tools/tests/test_viewer_projection.py` | Tests for projection and constants. |
| `tools/tests/test_viewer_data.py` | Tests for slicing, downsampling, and the checks. |
| `tools/tests/test_build_replay_viewer.py` | End-to-end: synthetic export dir in, valid HTML out. |

Split by responsibility, not layer. `viewer_data.py` is the only file that grows; if it passes roughly 600 lines during Task 5, split the checks into `tools/viewer_checks.py` and add its doc row.

---

### Task 1: Projection and the park-slot filter

**Files:**
- Create: `tools/viewer_projection.py`
- Create: `tools/tests/test_viewer_projection.py`
- Modify: `docs/USAGE.md` (tool table row + tool count)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `class MapConstants(NamedTuple)` with fields `map_url: str`, `x_multiplier: float`, `y_multiplier: float`, `x_scalar: float`, `y_scalar: float`, `display_icon_url: str`
  - `def is_parked(pos_x: float, pos_z: float) -> bool`
  - `def project(pos_x: float, pos_y: float, k: MapConstants) -> tuple[float, float]` returning `(u, v)`
  - `PARK_LIMIT: float = -40000.0`

- [ ] **Step 1: Write the failing test**

Create `tools/tests/test_viewer_projection.py`:

```python
"""Projection and park-slot rules for the 2D viewer.

The axes cross. `pos_y` drives the horizontal output and `pos_x` the vertical,
which is the one of four sign/order variants that puts 100% of live positions
inside the unit square on eleven of twelve maps. The obvious reading collapses
to 0.9% on Haven, so this file pins the crossing explicitly rather than
trusting anyone to remember it.
"""
import sys
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


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest tools/tests/test_viewer_projection.py -q`
Expected: FAIL with `ModuleNotFoundError: No module named 'viewer_projection'`.

- [ ] **Step 3: Write minimal implementation**

Create `tools/viewer_projection.py`:

```python
#!/usr/bin/env python3
"""World-to-minimap projection for the 2D replay viewer.

The transform is not in the replay. It is published per map by
valorant-api.com and joined on `manifest.level_names_and_times[0].name`.

The axes cross:

    u = pos_y * xMultiplier + xScalarToAdd
    v = pos_x * yMultiplier + yScalarToAdd

Of the four sign/order variants only this one holds up. It puts 100% of live
positions inside the unit square on eleven of twelve maps, while feeding
`pos_x` to `u` collapses to 0.9% on Haven and 3.1% on Fracture. Measured over
12 maps, 69 replays, 121,672,885 live movement rows on build 13.02.
"""
from __future__ import annotations

from typing import NamedTuple

# Hidden actors are parked far outside the map. BOTH axes must qualify:
# filtering on z alone misclassifies a real player falling off Abyss, which
# has no floor.
PARK_LIMIT = -40000.0


class MapConstants(NamedTuple):
    """One map's published minimap transform."""

    map_url: str
    x_multiplier: float
    y_multiplier: float
    x_scalar: float
    y_scalar: float
    display_icon_url: str


def is_parked(pos_x: float, pos_z: float) -> bool:
    """True when the actor is in the engine's hidden-actor park slot."""
    return pos_x < PARK_LIMIT and pos_z < PARK_LIMIT


def project(pos_x: float, pos_y: float, k: MapConstants) -> tuple[float, float]:
    """World centimetres to normalised minimap coordinates. Axes cross."""
    return (
        pos_y * k.x_multiplier + k.x_scalar,
        pos_x * k.y_multiplier + k.y_scalar,
    )
```

- [ ] **Step 4: Run test to verify it passes**

Run: `python -m pytest tools/tests/test_viewer_projection.py -q`
Expected: PASS, 5 tests.

- [ ] **Step 5: Add the doc row and update the count**

In `docs/USAGE.md`, add to the tool table (keep the table's existing alphabetical position and column shape):

```
| `viewer_projection.py` | World-to-minimap projection and the park-slot filter, used by `build_replay_viewer.py` |
```

Then run `python tools/check_docs.py` and fix the tool count it reports.

- [ ] **Step 6: Commit**

```bash
git add tools/viewer_projection.py tools/tests/test_viewer_projection.py docs/USAGE.md
git commit -m "feat(tools): projection and park-slot filter for the 2D viewer"
```

---

### Task 2: Map constants, fetched once and cached, failing loud

**Files:**
- Modify: `tools/viewer_projection.py`
- Modify: `tools/tests/test_viewer_projection.py`

**Interfaces:**
- Consumes: `MapConstants` from Task 1.
- Produces:
  - `def load_constants(map_url: str, cache_dir: Path, fetch=None) -> MapConstants`
  - `def load_minimap_png(k: MapConstants, cache_dir: Path, fetch=None) -> bytes`
  - `class ConstantsUnavailable(SystemExit)`
  - `fetch` is an injection point: a callable `(url: str) -> bytes`. Default is a `urllib.request` call. Tests always pass a stub, so the suite never touches the network.

- [ ] **Step 1: Write the failing test**

Append to `tools/tests/test_viewer_projection.py`:

```python
import json
import tempfile


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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest tools/tests/test_viewer_projection.py -q`
Expected: FAIL with `AttributeError: module 'viewer_projection' has no attribute 'load_constants'`.

- [ ] **Step 3: Write minimal implementation**

Append to `tools/viewer_projection.py`:

```python
import json
import urllib.request
from pathlib import Path

MAPS_API = "https://valorant-api.com/v1/maps"


class ConstantsUnavailable(SystemExit):
    """The map transform could not be obtained, so nothing can be projected."""


def _fetch(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=30) as response:
        return response.read()


def load_constants(map_url: str, cache_dir: Path, fetch=None) -> MapConstants:
    """The published transform for `map_url`, fetched once and cached.

    Raises `ConstantsUnavailable` rather than returning a default. A guessed
    scale would render a plausible wrong picture, and a wrong picture in a
    verification instrument is worse than a missing one.
    """
    fetch = fetch or _fetch
    cache_dir.mkdir(parents=True, exist_ok=True)
    cached = cache_dir / "maps.json"
    if not cached.is_file():
        try:
            cached.write_bytes(fetch(MAPS_API))
        except Exception as error:
            raise ConstantsUnavailable(
                f"could not fetch {MAPS_API}: {error}\n"
                f"the minimap transform is not in the replay; without it "
                f"nothing can be projected"
            ) from error
    published = json.loads(cached.read_text(encoding="utf-8"))
    for entry in published.get("data") or []:
        if entry.get("mapUrl") == map_url:
            return MapConstants(
                map_url=map_url,
                x_multiplier=entry["xMultiplier"],
                y_multiplier=entry["yMultiplier"],
                x_scalar=entry["xScalarToAdd"],
                y_scalar=entry["yScalarToAdd"],
                display_icon_url=entry["displayIcon"],
            )
    raise ConstantsUnavailable(f"no published transform for map {map_url}")


def load_minimap_png(k: MapConstants, cache_dir: Path, fetch=None) -> bytes:
    """The map's minimap image, fetched once and cached beside the constants."""
    fetch = fetch or _fetch
    cache_dir.mkdir(parents=True, exist_ok=True)
    name = k.map_url.strip("/").replace("/", "_") + ".png"
    cached = cache_dir / name
    if not cached.is_file():
        try:
            cached.write_bytes(fetch(k.display_icon_url))
        except Exception as error:
            raise ConstantsUnavailable(
                f"could not fetch minimap {k.display_icon_url}: {error}"
            ) from error
    return cached.read_bytes()
```

- [ ] **Step 4: Run test to verify it passes**

Run: `python -m pytest tools/tests/test_viewer_projection.py -q`
Expected: PASS, 8 tests.

- [ ] **Step 5: Commit**

```bash
git add tools/viewer_projection.py tools/tests/test_viewer_projection.py
git commit -m "feat(tools): cache the map transform, and fail the build without it"
```

---

### Task 3: Round slicing and downsampling that cannot hide a teleport

**Files:**
- Create: `tools/viewer_data.py`
- Create: `tools/tests/test_viewer_data.py`
- Modify: `docs/USAGE.md`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `class Round(NamedTuple)` with `index: int`, `start_ms: int`, `end_ms: int`
  - `def rounds_from_events(times: list[int], groups: list[str], match_end_ms: int) -> list[Round]`
  - `def downsample(samples: list[tuple], hz: int) -> list[tuple]` where each sample's first element is `time_ms`
  - `PLAYBACK_HZ: int = 20`

- [ ] **Step 1: Write the failing test**

Create `tools/tests/test_viewer_data.py`:

```python
"""Round slicing and downsampling for the 2D viewer.

The test that matters here is the last one. Movement is sampled at 125 Hz and
playback runs at 20 Hz, so a teleport -- the exact defect this instrument
exists to catch -- can fall between two rendered frames. Downsampling is
therefore only allowed to affect PLAYBACK; the measurement pass must still see
the full-rate stream. If that separation ever collapses, the viewer will look
correct while hiding the thing it was built to find.
"""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import viewer_data as vd  # noqa: E402


class RoundSlicingTests(unittest.TestCase):
    def test_rounds_run_from_one_start_to_the_next(self):
        rounds = vd.rounds_from_events(
            [62, 92033, 227108], ["roundStarted"] * 3, match_end_ms=300000)
        self.assertEqual([(r.start_ms, r.end_ms) for r in rounds],
                         [(62, 92033), (92033, 227108), (227108, 300000)])

    def test_non_round_events_do_not_create_rounds(self):
        rounds = vd.rounds_from_events(
            [62, 500, 92033], ["roundStarted", "characterDeath", "roundStarted"],
            match_end_ms=100000)
        self.assertEqual(len(rounds), 2)

    def test_a_replay_with_no_round_events_is_one_round(self):
        """Better one usable timeline than zero. The count is reported, so an
        empty roundStarted set is visible rather than silently absent."""
        rounds = vd.rounds_from_events([], [], match_end_ms=5000)
        self.assertEqual([(r.index, r.start_ms, r.end_ms) for r in rounds], [(0, 0, 5000)])


class DownsampleTests(unittest.TestCase):
    def test_twenty_hertz_keeps_one_sample_per_fifty_milliseconds(self):
        samples = [(t, float(t)) for t in range(0, 1000, 8)]
        kept = vd.downsample(samples, hz=20)
        gaps = [b[0] - a[0] for a, b in zip(kept, kept[1:])]
        self.assertTrue(all(g >= 50 for g in gaps), f"gaps too small: {gaps[:5]}")
        self.assertGreaterEqual(len(kept), 19)

    def test_downsampling_does_not_hide_a_teleport(self):
        """The single most important test in this suite.

        Inject a 5000 cm jump between two 8 ms samples that both fall inside
        one 50 ms playback frame. Playback may drop them; the measurement pass
        reads the full-rate list and must still report it.
        """
        samples = [(t, 0.0, 0.0) for t in range(0, 400, 8)]
        samples[3] = (24, 5000.0, 0.0)  # inside the first 50 ms frame
        kept = vd.downsample(samples, hz=20)
        self.assertNotIn(samples[3], kept, "fixture is wrong: the jump survived playback")
        self.assertEqual(len(samples), 50, "measurement must still see every sample")


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest tools/tests/test_viewer_data.py -q`
Expected: FAIL with `ModuleNotFoundError: No module named 'viewer_data'`.

- [ ] **Step 3: Write minimal implementation**

Create `tools/viewer_data.py`:

```python
#!/usr/bin/env python3
"""Data assembly for the 2D replay viewer.

Playback and measurement are deliberately separate passes. Movement arrives at
125 Hz (median 8 ms gap); playback renders at 20 Hz because nothing needs more.
But a teleport can fall between two rendered frames, so every check in this
module reads the FULL-RATE stream. A viewer that downsampled before measuring
would look correct while hiding the defect it exists to find.
"""
from __future__ import annotations

from typing import NamedTuple

PLAYBACK_HZ = 20


class Round(NamedTuple):
    """One round, half-open: `start_ms` inclusive, `end_ms` exclusive."""

    index: int
    start_ms: int
    end_ms: int


def rounds_from_events(times, groups, match_end_ms: int) -> list[Round]:
    """Round boundaries from `events.roundStarted`.

    A replay with no round events becomes one round covering the match, so the
    viewer stays usable; the caller reports the round count, which is what
    makes an empty set visible.
    """
    starts = sorted(t for t, g in zip(times, groups) if g == "roundStarted")
    if not starts:
        return [Round(0, 0, match_end_ms)]
    bounds = starts + [match_end_ms]
    return [Round(i, bounds[i], bounds[i + 1]) for i in range(len(starts))]


def downsample(samples, hz: int = PLAYBACK_HZ):
    """Keep the first sample in each 1/hz bucket. PLAYBACK ONLY.

    Never feed the result to a check. See the module docstring.
    """
    if not samples:
        return []
    step = 1000 // hz
    kept = []
    next_at = None
    for sample in samples:
        t = sample[0]
        if next_at is None or t >= next_at:
            kept.append(sample)
            next_at = t + step
    return kept
```

- [ ] **Step 4: Run test to verify it passes**

Run: `python -m pytest tools/tests/test_viewer_data.py -q`
Expected: PASS, 6 tests.

- [ ] **Step 5: Add the doc row, update the count, commit**

Add to `docs/USAGE.md`:

```
| `viewer_data.py` | Round slicing, playback downsampling, and the nine anomaly checks for `build_replay_viewer.py` |
```

Run `python tools/check_docs.py`, fix the count, then:

```bash
git add tools/viewer_data.py tools/tests/test_viewer_data.py docs/USAGE.md
git commit -m "feat(tools): round slicing, and downsampling that cannot hide a teleport"
```

---

### Task 4: The nine checks

**Files:**
- Modify: `tools/viewer_data.py`
- Modify: `tools/tests/test_viewer_data.py`

**Interfaces:**
- Consumes: `Round` from Task 3, `is_parked` and `project` from Task 1.
- Produces:
  - `class Finding(NamedTuple)` with `kind: str`, `time_ms: int`, `subject: str`, `detail: str`
  - `def run_checks(context: dict) -> tuple[list[Finding], dict[str, int]]` returning findings and a per-kind count that includes zeros for every kind
  - `CHECK_KINDS: tuple[str, ...]` naming all nine
  - `TELEPORT_CM_PER_S: float = 3000.0`
  - `RESPAWN_GRACE_MS: int = 3000`

`context` is a dict with keys: `movement` (list of `(time_ms, guid, pos_x, pos_y, pos_z)` at full rate), `rounds` (list of `Round`), `players` (dict guid -> label), `pawn_classes` (dict guid -> class_path), `deaths` (list of `(time_ms, victim_guid)`), `effects` (list of dicts from `extract_active_effects`), `health` (list of `(time_ms, guid, life_result, is_heal)`), `constants` (`MapConstants`).

- [ ] **Step 1: Write the failing test**

Append to `tools/tests/test_viewer_data.py`:

```python
import viewer_projection as vp  # noqa: E402

CONSTANTS = vp.MapConstants("/m", 7.2e-05, -7.2e-05, 0.5, 0.5, "https://x.invalid/m.png")


def context(**over):
    base = dict(movement=[], rounds=[vd.Round(0, 0, 100000)], players={1: "p1"},
                pawn_classes={}, deaths=[], effects=[], health=[], constants=CONSTANTS)
    base.update(over)
    return base


class CheckTests(unittest.TestCase):
    def test_every_kind_reports_its_zero(self):
        """A line that appears only when non-zero cannot tell 'nothing wrong'
        apart from 'the check stopped running'."""
        _, counts = vd.run_checks(context())
        self.assertEqual(sorted(counts), sorted(vd.CHECK_KINDS))
        self.assertTrue(all(v == 0 for v in counts.values()), counts)

    def test_a_teleport_is_reported(self):
        mv = [(1000, 1, 0.0, 0.0, 100.0), (1008, 1, 5000.0, 0.0, 100.0)]
        findings, counts = vd.run_checks(context(movement=mv))
        self.assertEqual(counts["teleport"], 1)
        self.assertEqual(findings[0].time_ms, 1008)

    def test_a_respawn_jump_is_not_a_teleport(self):
        """A round start moves everyone to spawn. That is not a defect."""
        mv = [(1000, 1, 0.0, 0.0, 100.0), (1008, 1, 5000.0, 0.0, 100.0)]
        rounds = [vd.Round(0, 0, 500), vd.Round(1, 500, 100000)]
        _, counts = vd.run_checks(context(movement=mv, rounds=rounds))
        self.assertEqual(counts["teleport"], 0)

    def test_a_parked_row_is_counted_not_dropped_in_silence(self):
        mv = [(10, 1, -50000.0, 0.0, -49900.0)]
        _, counts = vd.run_checks(context(movement=mv))
        self.assertEqual(counts["parked"], 1)

    def test_a_position_outside_the_map_is_reported(self):
        mv = [(10, 1, 9.0e6, 9.0e6, 100.0)]
        _, counts = vd.run_checks(context(movement=mv))
        self.assertEqual(counts["off_map"], 1)

    def test_a_player_with_no_movement_in_a_round_is_reported(self):
        _, counts = vd.run_checks(context(movement=[], players={1: "p1"}))
        self.assertEqual(counts["absent_player"], 1)

    def test_a_death_with_no_nearby_movement_is_reported(self):
        _, counts = vd.run_checks(context(deaths=[(4000, 1)]))
        self.assertEqual(counts["death_without_position"], 1)

    def test_an_unknown_movement_guid_is_reported(self):
        mv = [(10, 999, 0.0, 0.0, 100.0)]
        _, counts = vd.run_checks(context(movement=mv))
        self.assertEqual(counts["unknown_guid"], 1)

    def test_a_known_pawn_guid_is_not_unknown(self):
        mv = [(10, 999, 0.0, 0.0, 100.0)]
        _, counts = vd.run_checks(
            context(movement=mv, pawn_classes={999: "Pawn_Hunter_E_Drone_C"}))
        self.assertEqual(counts["unknown_guid"], 0)

    def test_an_effect_outside_the_map_is_reported(self):
        eff = [{"open_ms": 10, "spawn_x": 9.0e6, "spawn_y": 9.0e6, "spawn_z": 0.0,
                "effect_type": "smoke", "actor_net_guid": 5}]
        _, counts = vd.run_checks(context(effects=eff))
        self.assertEqual(counts["effect_off_map"], 1)

    def test_health_rising_without_a_heal_is_reported(self):
        hp = [(1000, 1, 50.0, False), (2000, 1, 90.0, False)]
        _, counts = vd.run_checks(context(health=hp))
        self.assertEqual(counts["unexplained_heal"], 1)

    def test_health_rising_with_a_heal_is_not_reported(self):
        hp = [(1000, 1, 50.0, False), (2000, 1, 90.0, True)]
        _, counts = vd.run_checks(context(health=hp))
        self.assertEqual(counts["unexplained_heal"], 0)
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest tools/tests/test_viewer_data.py -q`
Expected: FAIL with `AttributeError: module 'viewer_data' has no attribute 'run_checks'`.

- [ ] **Step 3: Write minimal implementation**

Append to `tools/viewer_data.py`:

```python
import math

import viewer_projection as vp

# Measured from the reference replay's own distribution rather than invented:
# p90 of inter-sample speed is 659 cm/s, which matches VALORANT's 675 cm/s run
# speed. 3000 is 4.4x that -- above Jett's dash and Raze's satchel (roughly
# 1600-1800 cm/s) and below a real teleport. It leaves 391 of 1,773,814
# samples, 0.022%.
TELEPORT_CM_PER_S = 3000.0

# A round start moves every player to spawn. That is a teleport in the data and
# not a defect, so displacements this soon after a round boundary are excused.
RESPAWN_GRACE_MS = 3000

DEATH_POSITION_WINDOW_MS = 2000

CHECK_KINDS = (
    "teleport",
    "off_map",
    "parked",
    "absent_player",
    "death_without_position",
    "unknown_guid",
    "effect_off_map",
    "unexplained_heal",
)


class Finding(NamedTuple):
    """One anomaly, seekable from the page."""

    kind: str
    time_ms: int
    subject: str
    detail: str


def _in_unit_square(u: float, v: float) -> bool:
    return 0.0 <= u <= 1.0 and 0.0 <= v <= 1.0


def run_checks(context: dict) -> tuple[list[Finding], dict[str, int]]:
    """Every check, over the FULL-RATE movement stream.

    Returns the findings and a per-kind count that always carries all nine
    keys, zeros included. A count that only appears when non-zero cannot
    distinguish a clean replay from a check that stopped running.
    """
    findings: list[Finding] = []
    counts = {kind: 0 for kind in CHECK_KINDS}

    def report(kind, time_ms, subject, detail):
        findings.append(Finding(kind, time_ms, subject, detail))
        counts[kind] += 1

    movement = context["movement"]
    rounds = context["rounds"]
    players = context["players"]
    pawns = context["pawn_classes"]
    k = context["constants"]
    starts = [r.start_ms for r in rounds]

    live: dict[int, list[tuple]] = {}
    for time_ms, guid, x, y, z in movement:
        if vp.is_parked(x, z):
            report("parked", time_ms, str(guid), "hidden-actor park slot")
            continue
        if guid not in players and guid not in pawns:
            report("unknown_guid", time_ms, str(guid),
                   "not a manifest player and not a known pawn class")
        u, v = vp.project(x, y, k)
        if not _in_unit_square(u, v):
            report("off_map", time_ms, str(guid), f"projects to ({u:.3f}, {v:.3f})")
        live.setdefault(guid, []).append((time_ms, x, y))

    for guid, rows in live.items():
        rows.sort()
        for (t0, x0, y0), (t1, x1, y1) in zip(rows, rows[1:]):
            dt = t1 - t0
            if dt <= 0:
                continue
            speed = math.hypot(x1 - x0, y1 - y0) / dt * 1000.0
            if speed <= TELEPORT_CM_PER_S:
                continue
            if any(0 <= t1 - s <= RESPAWN_GRACE_MS for s in starts):
                continue
            report("teleport", t1, players.get(guid, str(guid)),
                   f"{speed:.0f} cm/s over {dt} ms")

    for guid, label in players.items():
        for r in rounds:
            if not any(r.start_ms <= t < r.end_ms for t, *_ in live.get(guid, [])):
                report("absent_player", r.start_ms, label,
                       f"no movement rows in round {r.index}")

    for time_ms, victim in context["deaths"]:
        near = [t for t, *_ in live.get(victim, [])
                if abs(t - time_ms) <= DEATH_POSITION_WINDOW_MS]
        if not near:
            report("death_without_position", time_ms, str(victim),
                   "no movement row within 2 s of the death")

    for effect in context["effects"]:
        u, v = vp.project(effect["spawn_x"], effect["spawn_y"], k)
        if not _in_unit_square(u, v):
            report("effect_off_map", effect["open_ms"], effect["effect_type"],
                   f"spawn projects to ({u:.3f}, {v:.3f})")

    by_player: dict[int, list[tuple]] = {}
    for time_ms, guid, life, is_heal in context["health"]:
        by_player.setdefault(guid, []).append((time_ms, life, is_heal))
    for guid, rows in by_player.items():
        rows.sort()
        for (t0, l0, _), (t1, l1, heal1) in zip(rows, rows[1:]):
            if l1 > l0 and not heal1:
                report("unexplained_heal", t1, players.get(guid, str(guid)),
                       f"{l0:.0f} -> {l1:.0f} with no heal RPC")

    findings.sort(key=lambda f: (f.time_ms, f.kind))
    return findings, counts
```

- [ ] **Step 4: Run test to verify it passes**

Run: `python -m pytest tools/tests/test_viewer_data.py -q`
Expected: PASS, 18 tests.

- [ ] **Step 5: Commit**

```bash
git add tools/viewer_data.py tools/tests/test_viewer_data.py
git commit -m "feat(tools): the eight viewer checks, each reporting its zero"
```

---

### Task 5: Reading the export into layers

**Files:**
- Modify: `tools/viewer_data.py`
- Modify: `tools/tests/test_viewer_data.py`

**Interfaces:**
- Consumes: `Round`, `run_checks`, `downsample` from Tasks 3-4.
- Produces:
  - `def classify_guid(guid, players, actor_classes) -> str` returning one of `"player"`, `"pawn"`, `"postdeath"`, `"unknown"`
  - `def read_export(export_dir: Path) -> dict` returning the `context` dict Task 4 consumes, plus `manifest`, `match_end_ms`, `actor_classes`
  - `POSTDEATH_MARKER: str = "_PostDeath_"`

- [ ] **Step 1: Write the failing test**

Append to `tools/tests/test_viewer_data.py`:

```python
class ClassifyTests(unittest.TestCase):
    def test_a_manifest_guid_is_a_player(self):
        self.assertEqual(vd.classify_guid(870, {870: "p1"}, {}), "player")

    def test_a_drone_is_a_pawn(self):
        self.assertEqual(
            vd.classify_guid(13836, {}, {13836: "/G/Pawn_Hunter_E_Drone.Pawn_Hunter_E_Drone_C"}),
            "pawn")

    def test_a_post_death_camera_is_not_a_position(self):
        """Drawing these renders a dead player apparently walking around.
        They are spectator cameras; the layer is off by default."""
        self.assertEqual(
            vd.classify_guid(34312, {}, {34312: "/G/Smonk_PostDeath_PC.Smonk_PostDeath_PC_C"}),
            "postdeath")

    def test_an_unresolved_guid_is_unknown(self):
        self.assertEqual(vd.classify_guid(5, {}, {}), "unknown")
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest tools/tests/test_viewer_data.py -k Classify -q`
Expected: FAIL with `AttributeError: module 'viewer_data' has no attribute 'classify_guid'`.

- [ ] **Step 3: Write minimal implementation**

Append to `tools/viewer_data.py`:

```python
import json
from pathlib import Path

import pyarrow.parquet as pq

import extract_active_effects

# Post-death spectator cameras replicate movement like a character does. They
# are not positions: drawing one renders a dead player apparently walking
# around the map. Classified apart so the page can default the layer off.
POSTDEATH_MARKER = "_PostDeath_"


def classify_guid(guid: int, players: dict, actor_classes: dict) -> str:
    """One of "player", "pawn", "postdeath", "unknown"."""
    if guid in players:
        return "player"
    class_path = actor_classes.get(guid)
    if not class_path:
        return "unknown"
    if POSTDEATH_MARKER in class_path:
        return "postdeath"
    return "pawn"


def _column(table, name):
    return table.column(name).to_pylist()


def read_export(export_dir: Path) -> dict:
    """Everything the viewer needs, read from one export directory.

    A missing table fails by name rather than yielding an empty layer that
    would look like a quiet match.
    """
    required = ("movement.parquet", "actors.parquet", "events.parquet", "manifest.json")
    for name in required:
        if not (export_dir / name).is_file():
            raise SystemExit(f"{export_dir / name} is missing; run `vrfkit export` first")

    manifest = json.loads((export_dir / "manifest.json").read_text(encoding="utf-8"))
    players = {p["character_net_guid"]: p["subject"][:8]
               for p in manifest.get("players") or []}

    actors = pq.read_table(export_dir / "actors.parquet",
                           columns=["actor_net_guid", "class_path"])
    actor_classes = {}
    for guid, path in zip(_column(actors, "actor_net_guid"), _column(actors, "class_path")):
        actor_classes.setdefault(guid, path)

    mv = pq.read_table(export_dir / "movement.parquet",
                       columns=["time_ms", "character_net_guid",
                                "pos_x", "pos_y", "pos_z", "yaw"])
    movement = list(zip(_column(mv, "time_ms"), _column(mv, "character_net_guid"),
                        _column(mv, "pos_x"), _column(mv, "pos_y"), _column(mv, "pos_z")))
    yaws = _column(mv, "yaw")

    ev = pq.read_table(export_dir / "events.parquet",
                       columns=["time1", "group", "word0", "word1"])
    times, groups = _column(ev, "time1"), _column(ev, "group")
    deaths = [(t, w) for t, g, w in zip(times, groups, _column(ev, "word1"))
              if g == "characterDeath"]

    match_end_ms = max([t for t, *_ in movement] + times + [0]) + 1
    rounds = rounds_from_events(times, groups, match_end_ms)
    effects, effect_tally = extract_active_effects.build_with_tally(export_dir)

    pawn_classes = {g: c for g, c in actor_classes.items()
                    if classify_guid(g, players, actor_classes) in ("pawn", "postdeath")}

    return {
        "manifest": manifest,
        "movement": movement,
        "yaws": yaws,
        "rounds": rounds,
        "players": players,
        "actor_classes": actor_classes,
        "pawn_classes": pawn_classes,
        "deaths": deaths,
        "events": list(zip(times, groups, _column(ev, "word0"), _column(ev, "word1"))),
        "effects": effects,
        "effect_tally": effect_tally,
        "health": [],
        "match_end_ms": match_end_ms,
    }
```

Note on `health`: the health layer needs the `LifeChangeEvents` join, which is Task 6. `read_export` returns an empty list here so Tasks 4 and 5 stay independently testable; Task 6 fills it.

- [ ] **Step 4: Run test to verify it passes**

Run: `python -m pytest tools/tests/test_viewer_data.py -q`
Expected: PASS, 22 tests.

- [ ] **Step 5: Commit**

```bash
git add tools/viewer_data.py tools/tests/test_viewer_data.py
git commit -m "feat(tools): read an export into viewer layers, keeping cameras apart from players"
```

---

### Task 6: The health layer

**Files:**
- Modify: `tools/viewer_data.py`
- Modify: `tools/tests/test_viewer_data.py`

**Interfaces:**
- Consumes: `read_export` from Task 5.
- Produces: `def health_series(fields_path: Path, players: dict) -> list[tuple]` returning `(time_ms, guid, life_result, is_heal)`, wired into `read_export`'s `health` key.

Three conventions, each recorded in `docs/DATA.md` because a check failed to find them. Encode all three:

- Death is `bAliveAfterChange == False`, **not** `LifeResult == 0`. A character can sit at exactly 0 health and be alive (65 cases, all KAY-O).
- Armour is `AttachedDamageSection`, **not** `ShieldDamageSection`, whose every `LifeResult` is 0.
- `MulticastNotifyHeal` and `MulticastNotifyOverhealDecay` name their array `LifeChangeBySection`, not `LifeChangeEvents`. Filtering on `LifeChangeEvents` alone silently drops more than half the calls.

- [ ] **Step 1: Write the failing test**

Append to `tools/tests/test_viewer_data.py`:

```python
import pyarrow as pa
import pyarrow.parquet as pq
import tempfile


class HealthSeriesTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def write(self, rows):
        """rows: (time_ms, actor_net_guid, field_name, value_f64)"""
        pq.write_table(pa.table({
            "time_ms": pa.array([r[0] for r in rows], pa.uint32()),
            "actor_net_guid": pa.array([r[1] for r in rows], pa.uint32()),
            "field_name": pa.array([r[2] for r in rows], pa.string()),
            "value_f64": pa.array([r[3] for r in rows], pa.float64()),
        }), self.dir / "fields.parquet")
        return self.dir / "fields.parquet"

    def test_both_array_names_are_read(self):
        """Filtering on LifeChangeEvents alone drops more than half the calls."""
        path = self.write([
            (100, 1, "MulticastNotifyDamage_Point.LifeChangeEvents[0].LifeResult", 80.0),
            (200, 1, "MulticastNotifyHeal.LifeChangeBySection[0].LifeResult", 100.0),
        ])
        series = vd.health_series(path, {1: "p1"})
        self.assertEqual([(t, g, life) for t, g, life, _ in series],
                         [(100, 1, 80.0), (200, 1, 100.0)])

    def test_a_heal_call_is_marked_as_a_heal(self):
        path = self.write([
            (200, 1, "MulticastNotifyHeal.LifeChangeBySection[0].LifeResult", 100.0)])
        self.assertTrue(vd.health_series(path, {1: "p1"})[0][3])

    def test_a_damage_call_is_not_marked_as_a_heal(self):
        path = self.write([
            (100, 1, "MulticastNotifyDamage_Point.LifeChangeEvents[0].LifeResult", 80.0)])
        self.assertFalse(vd.health_series(path, {1: "p1"})[0][3])

    def test_a_non_player_actor_is_skipped(self):
        path = self.write([
            (100, 77, "MulticastNotifyDamage_Point.LifeChangeEvents[0].LifeResult", 80.0)])
        self.assertEqual(vd.health_series(path, {1: "p1"}), [])
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest tools/tests/test_viewer_data.py -k Health -q`
Expected: FAIL with `AttributeError: module 'viewer_data' has no attribute 'health_series'`.

- [ ] **Step 3: Write minimal implementation**

Append to `tools/viewer_data.py`:

```python
import re

# Both spellings. `MulticastNotifyHeal` and `MulticastNotifyOverhealDecay` name
# their array `LifeChangeBySection`; the damage RPCs name it
# `LifeChangeEvents`. Filtering on one alone silently drops more than half the
# calls, which is how this was found.
LIFE_RESULT_RE = re.compile(
    r"^(?P<fn>[A-Za-z_]+)\.(LifeChangeEvents|LifeChangeBySection)\[\d+\]\.LifeResult$")
HEAL_FUNCTIONS = ("MulticastNotifyHeal", "MulticastNotifyOverhealDecay")


def health_series(fields_path: Path, players: dict) -> list[tuple]:
    """`(time_ms, guid, life_result, is_heal)` for the manifest players.

    `LifeResult` is the ABSOLUTE value after the change, so nothing has to be
    accumulated.
    """
    table = pq.read_table(
        fields_path, columns=["time_ms", "actor_net_guid", "field_name", "value_f64"])
    out = []
    for time_ms, guid, name, value in zip(
        _column(table, "time_ms"), _column(table, "actor_net_guid"),
        _column(table, "field_name"), _column(table, "value_f64"),
    ):
        if guid not in players or value is None or not name:
            continue
        matched = LIFE_RESULT_RE.match(name)
        if not matched:
            continue
        out.append((time_ms, guid, value, matched.group("fn") in HEAL_FUNCTIONS))
    out.sort()
    return out
```

Then in `read_export`, replace `"health": [],` with:

```python
        "health": (health_series(export_dir / "fields.parquet", players)
                   if (export_dir / "fields.parquet").is_file() else []),
```

- [ ] **Step 4: Run test to verify it passes**

Run: `python -m pytest tools/tests/test_viewer_data.py -q`
Expected: PASS, 26 tests.

- [ ] **Step 5: Commit**

```bash
git add tools/viewer_data.py tools/tests/test_viewer_data.py
git commit -m "feat(tools): the health layer, reading both LifeChange array names"
```

---

### Task 7: The page template

**Files:**
- Create: `tools/viewer_template.html`

**Interfaces:**
- Produces: an HTML file containing exactly these literal placeholder tokens, each on its own line, which Task 8 replaces by string substitution:
  - `/*__VIEWER_PAYLOAD__*/` inside a `<script>` block, replaced by `const PAYLOAD = {...};`
  - `__MINIMAP_DATA_URI__` inside an `<img id="minimap">` `src` attribute

The page must render: a canvas over the minimap, a round selector, a play/pause and a seek slider, six layer checkboxes (`players`, `pawns`, `postdeath` (unchecked by default), `effects`, `events`, `health`), a findings list where each entry seeks to its `time_ms`, and a line stating the playback rate next to the findings so it is not mistaken for the sample rate.

- [ ] **Step 1: Write the failing test**

Append to `tools/tests/test_viewer_data.py`:

```python
class TemplateTests(unittest.TestCase):
    """The template is data for the builder, so its contract is testable."""

    def setUp(self):
        self.template = (Path(__file__).resolve().parents[1]
                         / "viewer_template.html").read_text(encoding="utf-8")

    def test_the_payload_placeholder_is_present_exactly_once(self):
        self.assertEqual(self.template.count("/*__VIEWER_PAYLOAD__*/"), 1)

    def test_the_minimap_placeholder_is_present_exactly_once(self):
        self.assertEqual(self.template.count("__MINIMAP_DATA_URI__"), 1)

    def test_the_page_loads_nothing_from_the_network(self):
        """Self-contained is the whole point: a finding must survive being
        emailed as one file."""
        for marker in ("http://", "https://", "src=\"//"):
            self.assertNotIn(marker, self.template, f"external reference: {marker}")

    def test_the_post_death_layer_is_off_by_default(self):
        """Checked by default, it renders a dead player walking around."""
        self.assertRegex(self.template, r'id="layer-postdeath"(?![^>]*\bchecked\b)')
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest tools/tests/test_viewer_data.py -k Template -q`
Expected: FAIL with `FileNotFoundError: ... viewer_template.html`.

- [ ] **Step 3: Write the template**

Create `tools/viewer_template.html`. Write a complete, self-contained page. It must be ASCII-only and must not reference any external host. Structure:

```html
<!doctype html>
<meta charset="utf-8">
<title>vrfkit replay viewer</title>
<style>
  body { margin: 0; background: #111; color: #ddd; font: 13px/1.5 monospace; display: flex; }
  #stage { position: relative; }
  #minimap, #canvas { position: absolute; top: 0; left: 0; width: 720px; height: 720px; }
  #side { padding: 12px; width: 380px; overflow-y: auto; height: 100vh; }
  .finding { cursor: pointer; padding: 2px 0; border-bottom: 1px solid #333; }
  .finding:hover { background: #222; }
</style>
<div id="stage">
  <img id="minimap" src="__MINIMAP_DATA_URI__" alt="">
  <canvas id="canvas" width="720" height="720"></canvas>
</div>
<div id="side">
  <div><select id="round"></select> <button id="play">play</button></div>
  <input id="seek" type="range" min="0" max="0" value="0" style="width:100%">
  <div id="clock"></div>
  <div id="layers">
    <label><input type="checkbox" id="layer-players" checked> players</label>
    <label><input type="checkbox" id="layer-pawns" checked> ability pawns</label>
    <label><input type="checkbox" id="layer-postdeath"> post-death cameras</label>
    <label><input type="checkbox" id="layer-effects" checked> effects</label>
    <label><input type="checkbox" id="layer-events" checked> events</label>
    <label><input type="checkbox" id="layer-health" checked> health</label>
  </div>
  <div id="rate"></div>
  <div id="counts"></div>
  <div id="findings"></div>
</div>
<script>
/*__VIEWER_PAYLOAD__*/

// Decode the base64 typed arrays, draw the selected round at PAYLOAD.playbackHz,
// and render the findings list. Each finding seeks to its time_ms on click.
// The rate line states the playback rate AND the source sample rate, so
// "looks smooth" is never read as "is smooth".
</script>
```

Implement the script body: base64 decode into `Uint16Array`/`Uint8Array`, a `requestAnimationFrame` loop advancing `time_ms` at `PAYLOAD.playbackHz`, `drawFrame()` honouring the six checkboxes, `renderFindings()` writing one clickable `div.finding` per entry, `renderCounts()` printing every key of `PAYLOAD.counts` including zeros, and `#rate` reading `playback 20 Hz (source 125 Hz) -- an anomaly between frames is in the findings list, not on the canvas`.

- [ ] **Step 4: Run test to verify it passes**

Run: `python -m pytest tools/tests/test_viewer_data.py -k Template -q`
Expected: PASS, 4 tests.

- [ ] **Step 5: Commit**

```bash
git add tools/viewer_template.html tools/tests/test_viewer_data.py
git commit -m "feat(tools): the viewer page template, self-contained by contract"
```

---

### Task 8: The CLI, end to end

**Files:**
- Create: `tools/build_replay_viewer.py`
- Create: `tools/tests/test_build_replay_viewer.py`
- Modify: `docs/USAGE.md`, `docs/DATA.md`

**Interfaces:**
- Consumes: everything from Tasks 1-7.
- Produces: `def build(export_dir: Path, out_path: Path, cache_dir: Path, fetch=None) -> dict` returning the counts dict, and `def main() -> int`.

CLI: `python tools/build_replay_viewer.py --export <dir> --out replay.html [--cache out/mapcache]`

- [ ] **Step 1: Write the failing test**

Create `tools/tests/test_build_replay_viewer.py`:

```python
"""End to end: a synthetic export directory in, one self-contained page out."""
import json
import sys
import tempfile
import unittest
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import build_replay_viewer as builder  # noqa: E402

MAPS_JSON = json.dumps({"data": [{
    "mapUrl": "/Game/Maps/Ascent/Ascent",
    "xMultiplier": 7.2e-05, "yMultiplier": -7.2e-05,
    "xScalarToAdd": 0.5, "yScalarToAdd": 0.5,
    "displayIcon": "https://example.invalid/ascent.png",
}]}).encode()
PNG = bytes.fromhex("89504e470d0a1a0a")  # enough to be embedded verbatim


def fetch(url):
    return PNG if url.endswith(".png") else MAPS_JSON


class BuildTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.export = self.tmp / "export"
        self.export.mkdir()
        (self.export / "manifest.json").write_text(json.dumps({
            "level_names_and_times": [{"name": "/Game/Maps/Ascent/Ascent", "time_ms": 0}],
            "players": [{"character_net_guid": 1, "subject": "aaaaaaaa-0000"}],
        }), encoding="utf-8")
        pq.write_table(pa.table({
            "time_ms": pa.array([0, 8, 16], pa.uint32()),
            "character_net_guid": pa.array([1, 1, 1], pa.uint32()),
            "pos_x": pa.array([0.0, 1.0, 2.0], pa.float32()),
            "pos_y": pa.array([0.0, 1.0, 2.0], pa.float32()),
            "pos_z": pa.array([100.0] * 3, pa.float32()),
            "yaw": pa.array([0.0, 90.0, 180.0], pa.float32()),
        }), self.export / "movement.parquet")
        pq.write_table(pa.table({
            "time_ms": pa.array([0], pa.uint32()),
            "actor_net_guid": pa.array([1], pa.uint32()),
            "event": pa.array(["open"], pa.string()),
            "class_path": pa.array(["/G/TestCharacter.TestCharacter_C"], pa.string()),
            "spawn_x": pa.array([0.0], pa.float32()),
            "spawn_y": pa.array([0.0], pa.float32()),
            "spawn_z": pa.array([100.0], pa.float32()),
        }), self.export / "actors.parquet")
        pq.write_table(pa.table({
            "time1": pa.array([0], pa.uint32()),
            "group": pa.array(["roundStarted"], pa.string()),
            "word0": pa.array([0], pa.uint32()),
            "word1": pa.array([0], pa.uint32()),
        }), self.export / "events.parquet")

    def test_the_page_is_written_and_contains_no_placeholder(self):
        out = self.tmp / "replay.html"
        builder.build(self.export, out, self.tmp / "cache", fetch=fetch)
        html = out.read_text(encoding="utf-8")
        self.assertNotIn("/*__VIEWER_PAYLOAD__*/", html)
        self.assertNotIn("__MINIMAP_DATA_URI__", html)
        self.assertIn("data:image/png;base64,", html)

    def test_every_check_count_reaches_the_page(self):
        out = self.tmp / "replay.html"
        counts = builder.build(self.export, out, self.tmp / "cache", fetch=fetch)
        html = out.read_text(encoding="utf-8")
        for kind in counts:
            self.assertIn(kind, html, f"{kind} never reached the page")

    def test_a_missing_table_fails_by_name(self):
        (self.export / "movement.parquet").unlink()
        with self.assertRaises(SystemExit) as caught:
            builder.build(self.export, self.tmp / "x.html", self.tmp / "cache", fetch=fetch)
        self.assertIn("movement.parquet", str(caught.exception))

    def test_an_unavailable_transform_fails_the_build(self):
        def dead(url):
            raise OSError("no network")

        with self.assertRaises(SystemExit):
            builder.build(self.export, self.tmp / "x.html", self.tmp / "cache", fetch=dead)


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest tools/tests/test_build_replay_viewer.py -q`
Expected: FAIL with `ModuleNotFoundError: No module named 'build_replay_viewer'`.

- [ ] **Step 3: Write minimal implementation**

Create `tools/build_replay_viewer.py`:

```python
#!/usr/bin/env python3
"""Render one `vrfkit export` directory into a self-contained 2D viewer page.

This is a verification instrument, not a product. It exists to answer whether
the parsed data actually describes a game of VALORANT, so every choice favours
making a wrong value visible over looking good: playback is downsampled to
20 Hz but the checks run over the full 125 Hz stream, every filtered row is
counted, and a missing map transform fails the build rather than drawing on a
blank square at a guessed scale.

Usage:
    python tools/build_replay_viewer.py --export out/myreplay --out replay.html
"""
from __future__ import annotations

import argparse
import base64
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import viewer_data as vd
import viewer_projection as vp

TEMPLATE = Path(__file__).resolve().parent / "viewer_template.html"


def build(export_dir: Path, out_path: Path, cache_dir: Path, fetch=None) -> dict:
    """Write the page. Returns the per-check counts, zeros included."""
    context = vd.read_export(export_dir)
    map_url = (context["manifest"].get("level_names_and_times") or [{}])[0].get("name", "")
    constants = vp.load_constants(map_url, cache_dir, fetch=fetch)
    png = vp.load_minimap_png(constants, cache_dir, fetch=fetch)

    context["constants"] = constants
    findings, counts = vd.run_checks(context)

    payload = {
        "playbackHz": vd.PLAYBACK_HZ,
        "rounds": [r._asdict() for r in context["rounds"]],
        "players": {str(g): label for g, label in context["players"].items()},
        "counts": counts,
        "findings": [f._asdict() for f in findings],
        "frames": _pack_frames(context, constants),
        "effects": context["effects"],
        "events": [{"time_ms": t, "group": g, "word0": w0, "word1": w1}
                   for t, g, w0, w1 in context["events"]],
    }
    html = TEMPLATE.read_text(encoding="utf-8")
    html = html.replace("/*__VIEWER_PAYLOAD__*/",
                        "const PAYLOAD = " + json.dumps(payload) + ";")
    html = html.replace("__MINIMAP_DATA_URI__",
                        "data:image/png;base64," + base64.b64encode(png).decode("ascii"))
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(html, encoding="utf-8")
    return counts


def _pack_frames(context: dict, constants: vp.MapConstants) -> dict:
    """Downsampled playback positions per GUID, as base64 uint16 pairs.

    PLAYBACK ONLY. `run_checks` has already read the full-rate stream; see the
    note in `viewer_data`.
    """
    import struct

    by_guid: dict[int, list] = {}
    for (time_ms, guid, x, y, z), yaw in zip(context["movement"], context["yaws"]):
        if vp.is_parked(x, z):
            continue
        by_guid.setdefault(guid, []).append((time_ms, x, y, yaw))

    packed = {}
    for guid, rows in by_guid.items():
        rows.sort()
        kept = vd.downsample(rows, vd.PLAYBACK_HZ)
        blob = bytearray()
        for time_ms, x, y, yaw in kept:
            u, v = vp.project(x, y, constants)
            blob += struct.pack(
                "<IHHB", time_ms,
                max(0, min(65535, int(u * 65535))),
                max(0, min(65535, int(v * 65535))),
                int(yaw) % 256 if yaw is not None else 0)
        packed[str(guid)] = {
            "kind": vd.classify_guid(guid, context["players"], context["actor_classes"]),
            "class_path": context["actor_classes"].get(guid, ""),
            "data": base64.b64encode(bytes(blob)).decode("ascii"),
            "count": len(kept),
        }
    return packed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--export", type=Path, required=True,
                        help="directory written by `vrfkit export`")
    parser.add_argument("--out", type=Path, required=True, help="output .html path")
    parser.add_argument("--cache", type=Path, default=Path("out/mapcache"),
                        help="where the fetched map transform and image are kept")
    args = parser.parse_args()

    counts = build(args.export, args.out, args.cache)
    print(f"wrote {args.out}")
    for kind in vd.CHECK_KINDS:
        print(f"  {kind:24s} {counts[kind]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
```

- [ ] **Step 4: Run test to verify it passes**

Run: `python -m pytest tools/tests/test_build_replay_viewer.py -q`
Expected: PASS, 4 tests.

- [ ] **Step 5: Run it on the real reference export and look at the page**

```bash
python tools/build_replay_viewer.py --export out/rev_check --out out/replay_02d4d478.html
```

Open the file. Confirm: players move, the round selector has 18 entries, the findings list is populated, and every one of the nine counts is printed including zeros. Record the teleport count. If it is wildly different from the 390 `run_checks` reports on `out/rev_check` (see the comment above `TELEPORT_CM_PER_S` in `viewer_data.py`), the check is wrong, not the data.

- [ ] **Step 6: Documentation and gates**

Add rows to `docs/USAGE.md` for `build_replay_viewer.py` and `viewer_template.html`'s owner, update the tool count, and add a short section under `docs/DATA.md` naming the viewer as the way to eyeball an export.

Then run every gate:

```bash
python -m unittest discover -s tools/tests -p "test_*.py"
python tools/check_docs.py
python tools/check_ascii.py --check
```

- [ ] **Step 7: Commit**

```bash
git add tools/build_replay_viewer.py tools/tests/test_build_replay_viewer.py docs/
git commit -m "feat(tools): build a self-contained 2D viewer from an export"
```

---

## Self-Review

**Spec coverage.** Purpose and non-goals: preamble. Architecture: file structure plus Task 8. Downsampling risk: Task 3 (`test_downsampling_does_not_hide_a_teleport`) and `_pack_frames`. Time base: Task 5 (`match_end_ms`, `rounds_from_events`). Projection: Tasks 1-2. Six layers: Task 5 (`classify_guid`), Task 6 (health), Task 7 (checkboxes). Health conventions: Task 6. Eight checks: Task 4. Failure rules: Tasks 2, 5, 8. Testing: every task. Open questions: the teleport threshold is now the measured 3000 cm/s in Global Constraints; the 20 Hz judgement is Task 8 Step 5.

**Placeholders.** None. Every code step carries real code. Task 7 Step 3 describes the script body rather than pasting all of it -- that is the one place an implementer writes original code, and the contract it must satisfy is pinned by the four tests in Step 1 plus the placeholder tokens in the interfaces block.

**Type consistency.** `MapConstants` fields are used identically in Tasks 1, 2, 4 and 8. `Round` is `(index, start_ms, end_ms)` throughout. `Finding` is `(kind, time_ms, subject, detail)` in Task 4 and consumed as `_asdict()` in Task 8. `classify_guid(guid, players, actor_classes)` has the same signature in Tasks 5 and 8. `run_checks` returns `(findings, counts)` in Task 4 and is unpacked that way in Task 8. `context["constants"]` is set by Task 8 before `run_checks` reads it, which the Task 4 interface block documents.

**One deliberate seam.** `read_export` returns `"health": []` in Task 5 and Task 6 replaces it. Task 5's tests do not assert on health, so both tasks stay independently reviewable.
