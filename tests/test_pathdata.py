import unittest

from svgo_py.pathdata import PathData


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


if __name__ == "__main__":
    unittest.main()
