import unittest

from svgo_py.centerline import CenterlineOptions, centerline_path_data


class CenterlineTests(unittest.TestCase):
    def test_simple_rectangle_centerline(self):
        d, stroke_width, _ctx = centerline_path_data(
            "M0 0L30 0L30 6L0 6Z",
            CenterlineOptions(scale=2, max_size=128, simplify=1, min_length=1, polyline=True),
        )
        self.assertTrue(d.startswith("M"))
        self.assertGreater(stroke_width, 0)


if __name__ == "__main__":
    unittest.main()
