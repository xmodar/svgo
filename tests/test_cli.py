import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

from svgo_py.cli import main


class CliTests(unittest.TestCase):
    def run_cli(self, argv):
        stdout = io.StringIO()
        stderr = io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            try:
                code = main(argv)
            except SystemExit as exc:
                code = int(exc.code or 0)
        return code, stdout.getvalue(), stderr.getvalue()

    def test_path_alias(self):
        code, stdout, stderr = self.run_cli(["p", "--path", "M10 10h5v5z", "--op", "optimize:safe", "--minify"])
        self.assertEqual(code, 0, stderr)
        self.assertIn("M10", stdout)

    def test_opt_alias(self):
        with tempfile.TemporaryDirectory() as tmp:
            svg = Path(tmp) / "icon.svg"
            svg.write_text('<svg xmlns="http://www.w3.org/2000/svg"><rect width="10" height="10"/></svg>', encoding="utf-8")
            code, stdout, stderr = self.run_cli(["o", "--input", str(svg), "--svgo-precision", "2"])
        self.assertEqual(code, 0, stderr)
        self.assertIn("<path", stdout)

    def test_center_alias(self):
        code, stdout, stderr = self.run_cli(
            ["c", "--path", "M0 0L30 0L30 6L0 6Z", "--emit", "d", "--scale", "2", "--max-size", "128", "--simplify", "1", "--min-length", "1", "--polyline"]
        )
        self.assertEqual(code, 0, stderr)
        self.assertTrue(stdout.startswith("M"))

    def test_plugins_alias(self):
        code, stdout, stderr = self.run_cli(["l"])
        self.assertEqual(code, 0, stderr)
        self.assertIn("convertPathData", stdout)

    def test_info_alias_outputs_json(self):
        with tempfile.TemporaryDirectory() as tmp:
            svg = Path(tmp) / "icon.svg"
            svg.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="10" height="20"><rect width="1" height="2"/></svg>', encoding="utf-8")
            code, stdout, stderr = self.run_cli(["i", "--input", str(svg), "--compact"])
        self.assertEqual(code, 0, stderr)
        info = json.loads(stdout)
        self.assertEqual(info["width"], "10")
        self.assertEqual(info["shapes"], 1)

    def test_validate_alias_reports_strict_warning_as_invalid(self):
        with tempfile.TemporaryDirectory() as tmp:
            svg = Path(tmp) / "icon.svg"
            svg.write_text('<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0"/></svg>', encoding="utf-8")
            code, stdout, stderr = self.run_cli(["v", "--input", str(svg), "--strict"])
        self.assertEqual(code, 1)
        self.assertIn("warning:", stdout)
        self.assertEqual(stderr, "")

    def test_convert_alias_defaults_to_shapes_to_paths(self):
        with tempfile.TemporaryDirectory() as tmp:
            svg = Path(tmp) / "icon.svg"
            svg.write_text('<svg xmlns="http://www.w3.org/2000/svg"><circle cx="5" cy="5" r="2"/></svg>', encoding="utf-8")
            code, stdout, stderr = self.run_cli(["x", "--input", str(svg), "--precision", "2"])
        self.assertEqual(code, 0, stderr)
        self.assertIn("<path", stdout)
        self.assertNotIn("<circle", stdout)

    def test_removed_long_compatibility_names(self):
        code, _stdout, stderr = self.run_cli(["optimize", "--help"])
        self.assertNotEqual(code, 0)
        self.assertIn("invalid choice", stderr)


if __name__ == "__main__":
    unittest.main()
