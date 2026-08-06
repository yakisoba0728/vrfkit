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
