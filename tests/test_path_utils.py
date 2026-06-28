import unittest

from svgo.path_utils import (
    detect_polyline_accuracy,
    filled_loops,
    normalize_color,
    polygon_area,
    polygon_centroid,
    polyline_lengths,
    polyline_subpaths,
    radial_centerline_candidate,
    remove_collinear_points,
    round_to,
    serialize_polyline_subpaths,
    simplify_radial_distance,
    simplify_rdp,
    stitch_subpaths,
    turn_stats,
)


class PathUtilsTests(unittest.TestCase):
    def test_color_and_path_loop_helpers(self):
        self.assertEqual(normalize_color("143861"), "#143861")
        loops = filled_loops("M0 0H10V10H0Z M2 2H8V8H2Z")
        self.assertEqual(len(loops), 2)
        self.assertEqual(polygon_area(loops[0]), 100)
        self.assertEqual(polygon_centroid(loops[1]), (5, 5))

    def test_polyline_metrics_and_stitching(self):
        paths = polyline_subpaths("M0 0L5 0 M5.5 0L10 0", close_paths=False)
        stitched = stitch_subpaths(paths, max_gap=1)
        self.assertEqual(len(stitched), 1)
        self.assertEqual(polyline_lengths(stitched), [10])
        self.assertEqual(turn_stats("M0 0L5 0L5 5")[2], 1)
        self.assertEqual(serialize_polyline_subpaths(stitched, decimals=1), "M0 0 L5 0 L5.5 0 L10 0")

    def test_simplification_helpers(self):
        points = [(0, 0), (1, 0), (2, 0), (2, 2), (4, 2)]
        self.assertEqual(remove_collinear_points(points), [(0, 0), (2, 0), (2, 2), (4, 2)])
        self.assertEqual(simplify_radial_distance([(0, 0), (0.2, 0), (2, 0), (2.1, 0), (4, 0)], "1"), [(0, 0), (2, 0), (4, 0)])
        self.assertEqual(simplify_rdp([(0, 0), (1, 0.1), (2, 0), (2, 3)], "0.5"), [(0, 0), (2, 0), (2, 3)])
        self.assertGreaterEqual(detect_polyline_accuracy([(0, 0), (0.1, 0), (1, 0)]), 1)
        self.assertEqual(round_to(1.234, 1), 1.2)

    def test_radial_centerline_candidate_for_two_loop_outline(self):
        result = radial_centerline_candidate("M0 0H20V20H0Z M5 5H15V15H5Z", samples=48, simplify=1, decimals=2)
        self.assertIsNotNone(result)
        assert result is not None
        d, stroke_width = result
        self.assertTrue(d.startswith("M"))
        self.assertTrue(any(command in d for command in "CcSs"))
        self.assertGreater(stroke_width, 0)


if __name__ == "__main__":
    unittest.main()
