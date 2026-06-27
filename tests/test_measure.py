import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

from svgo import path_bbox, path_length, path_metrics, point_at_length, svg_metrics
from svgo.cli import main


class MeasureTests(unittest.TestCase):
    def test_path_metrics_for_closed_square(self):
        metrics = path_metrics("M0 0H10V10H0Z", decimals=3)
        self.assertEqual(metrics["length"], 40)
        self.assertEqual(metrics["bbox"]["width"], 10)
        self.assertEqual(metrics["bbox"]["height"], 10)
        self.assertEqual(path_bbox("M0 0H10V10H0Z", decimals=3)["cx"], 5)
        self.assertAlmostEqual(path_length("M0 0H10"), 10)

    def test_point_at_length(self):
        point = point_at_length("M0 0H10V10", 15)
        self.assertAlmostEqual(point["x"], 10)
        self.assertAlmostEqual(point["y"], 5)

    def test_svg_metrics_include_shapes_and_group_transforms(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg"><g transform="translate(5 2)"><rect width="10" height="4"/></g></svg>'
        metrics = svg_metrics(svg, decimals=3)
        self.assertEqual(metrics["path_count"], 1)
        self.assertEqual(metrics["bbox"]["x"], 5)
        self.assertEqual(metrics["bbox"]["y"], 2)
        self.assertEqual(metrics["bbox"]["width"], 10)
        self.assertEqual(metrics["bbox"]["height"], 4)

    def test_measure_cli_outputs_json(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "icon.svg"
            source.write_text('<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0H10"/></svg>', encoding="utf-8")
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                code = main(["m", "--input", str(source), "--decimals", "2", "--compact"])
        self.assertEqual(code, 0)
        metrics = json.loads(stdout.getvalue())
        self.assertEqual(metrics["path_count"], 1)
        self.assertEqual(metrics["length"], 10)

    def test_measure_cli_point_uses_transformed_svg_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "icon.svg"
            source.write_text('<svg xmlns="http://www.w3.org/2000/svg"><g transform="translate(5 2)"><path d="M0 0H10"/></g></svg>', encoding="utf-8")
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                code = main(["m", "--input", str(source), "--at", "5", "--decimals", "2", "--compact"])
        self.assertEqual(code, 0)
        metrics = json.loads(stdout.getvalue())
        self.assertEqual(metrics["point"], {"x": 10, "y": 2})


if __name__ == "__main__":
    unittest.main()
