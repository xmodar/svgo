import math
import unittest

from svgo_py import circle_to_path, multiply_matrices, rect_to_path, rotate_2d, transform_2d, translate_2d


class GeometryTests(unittest.TestCase):
    def test_transform_helpers_use_svg_affine_convention(self):
        matrix = multiply_matrices(translate_2d(10, 5), rotate_2d(math.pi / 2))
        x, y = transform_2d(matrix, 2, 0)
        self.assertAlmostEqual(x, 10)
        self.assertAlmostEqual(y, 7)

    def test_circle_to_path_uses_cubics(self):
        path = circle_to_path(10, 10, 5, decimals=3, minify=True)
        self.assertTrue(path.startswith("M10 5C"))
        self.assertEqual(path.count("C"), 4)
        self.assertTrue(path.endswith("Z"))

    def test_rounded_rect_to_path_uses_cubic_corners(self):
        path = rect_to_path(0, 0, 20, 10, rx=2, ry=3, decimals=3, minify=True)
        self.assertIn("C", path)
        self.assertTrue(path.endswith("Z"))


if __name__ == "__main__":
    unittest.main()
