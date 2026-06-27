import unittest

from svgo.pathdata import PathData, parse_path, path_to_absolute, path_to_cubics, transform_path


class PathDataTests(unittest.TestCase):
    def test_ordered_matrix_and_optimize(self):
        path = PathData.parse("M10 10h5v5z")
        path.apply_operation("matrix(-1,0,0,1,30,0)")
        path.apply_operation("optimize:safe")
        out = path.to_string(decimals=2, minify=True)
        self.assertIn("20", out)
        self.assertIn("15", out)
        self.assertTrue(out[0] in {"M", "m"})

    def test_relative_serialization(self):
        path = PathData.parse("M0 0L10 0L10 10")
        path.apply_operation("relative")
        self.assertIn("l", path.to_string(minify=True))

    def test_compact_arc_flags_parse(self):
        path = PathData.parse("M0 0A10 10 0 0110 0")
        self.assertIn("A", path.to_string())

    def test_reverse_closed_path(self):
        path = PathData.parse("M0 0L10 0L10 10Z")
        path.apply_operation("reverse")
        out = path.to_string(minify=True)
        self.assertTrue(out.startswith("M"))
        self.assertIn("Z", out)

    def test_parse_path_exports_absolute_commands(self):
        commands = parse_path("m1 1l2 3")
        self.assertEqual(commands[0]["command"], "M")
        self.assertEqual(commands[1]["args"], [3.0, 4.0])

    def test_path_to_absolute_and_transform_path(self):
        self.assertEqual(path_to_absolute("m1 1l2 3", minify=True), "M1 1L3 4")
        self.assertEqual(transform_path("M0 0L1 1", (1, 0, 0, 1, 2, 3), minify=True), "M2 3L3 4")

    def test_path_to_cubics_converts_lines_and_quadratics(self):
        out = path_to_cubics("M0 0L3 0Q6 0 6 3Z", decimals=3, minify=True)
        self.assertEqual(out.count("C"), 3)
        self.assertTrue(out.endswith("Z"))


if __name__ == "__main__":
    unittest.main()
