import unittest

from svgo.centerline import CenterlineOptions, centerline_path_data


class CenterlineTests(unittest.TestCase):
    def test_simple_rectangle_centerline(self):
        d, stroke_width, _ctx = centerline_path_data(
            "M0 0L30 0L30 6L0 6Z",
            CenterlineOptions(scale=2, max_size=128, simplify=1, min_length=1, polyline=True),
        )
        self.assertTrue(d.startswith("M"))
        self.assertGreater(stroke_width, 0)

    def test_ring_centerline_keeps_closed_cycle_in_all_mode(self):
        d, stroke_width, _ctx = centerline_path_data(
            "M0 0H30V20H0Z M5 5H25V15H5Z",
            CenterlineOptions(mode="all", scale=2, max_size=128, simplify=1, min_length=1, polyline=True),
        )
        self.assertTrue(d.startswith("M"))
        self.assertGreaterEqual(d.count("L"), 4)
        self.assertGreater(stroke_width, 0)

    def test_bridge_gap_connects_nearby_skeleton_components(self):
        d, _stroke_width, _ctx = centerline_path_data(
            "M0 0H20V6H0Z M26 0H46V6H26Z",
            CenterlineOptions(mode="all", scale=2, max_size=128, simplify=1, min_length=1, polyline=True, bridge_gap=16),
        )
        self.assertEqual(d.count("M"), 1)


if __name__ == "__main__":
    unittest.main()
