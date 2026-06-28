import contextlib
import base64
import io
import json
import tempfile
import unittest
from pathlib import Path

from svgo.cli import main


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

    def test_version_option(self):
        code, stdout, stderr = self.run_cli(["--version"])
        self.assertEqual(code, 0, stderr)
        self.assertEqual(stdout.strip(), "svgo 0.4.0")

        code, stdout, stderr = self.run_cli(["-v"])
        self.assertEqual(code, 0, stderr)
        self.assertEqual(stdout.strip(), "svgo 0.4.0")

    def test_top_level_help_options(self):
        for flag in ("--help", "-h", "help"):
            with self.subTest(flag=flag):
                code, stdout, stderr = self.run_cli([flag])
                self.assertEqual(code, 0, stderr)
                self.assertIn("Usage:", stdout)
                self.assertIn("svgo <command> [options]", stdout)
                self.assertIn("svgo <command> --help", stdout)

    def test_subcommand_help_options(self):
        for argv, expected in (
            (["path", "--help"], "Edit raw SVG path data"),
            (["p", "-h"], "svgo path --path <D>"),
            (["opt", "--help"], "Optimize an SVG document"),
            (["trace", "--help"], "Trace a non-interlaced"),
            (["trace2", "--help"], "VTracer-compatible"),
            (["center", "--help"], "centerlines"),
            (["info", "--help"], "structured SVG metadata"),
            (["validate", "--help"], "Validate SVG XML"),
            (["v", "--help"], "Alias for `svgo validate`"),
            (["measure", "--help"], "Measure path or SVG geometry"),
            (["sanitize", "--help"], "Remove active or unsafe"),
            (["viewbox", "--help"], "Edit root SVG viewBox"),
            (["convert", "--help"], "Convert and normalize"),
            (["plugins", "--help"], "List built-in optimizer plugins"),
            (["recipe", "--help"], "declarative JSON recipes"),
        ):
            with self.subTest(argv=argv):
                code, stdout, stderr = self.run_cli(argv)
                self.assertEqual(code, 0, stderr)
                self.assertIn("Usage:", stdout)
                self.assertIn(expected, stdout)

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

    def test_trace2_alias_uses_vtracer_defaults(self):
        png = base64.b64decode(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ"
            "AAAADUlEQVR42mNgYGD4DwABBAEAghF2NwAAAABJRU5ErkJggg=="
        )
        with tempfile.TemporaryDirectory() as tmp:
            image = Path(tmp) / "icon.png"
            image.write_bytes(png)
            code, stdout, stderr = self.run_cli(["t2", "--input", str(image), "--filter-speckle", "1"])

        self.assertEqual(code, 0, stderr)
        self.assertIn("<svg", stdout)
        self.assertIn("<path", stdout)

    def test_trace_components_json(self):
        png = base64.b64decode(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ"
            "AAAADUlEQVR42mNgYGD4DwABBAEAghF2NwAAAABJRU5ErkJggg=="
        )
        with tempfile.TemporaryDirectory() as tmp:
            image = Path(tmp) / "icon.png"
            image.write_bytes(png)
            code, stdout, stderr = self.run_cli(["trace", "--input", str(image), "--components-json", "--min-area", "1"])

        self.assertEqual(code, 0, stderr)
        info = json.loads(stdout)
        self.assertEqual(info["viewBox"], "0 0 1 1")
        self.assertEqual(len(info["components"]), 1)
        self.assertIn("d", info["components"][0])

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

    def test_sanitize_alias_removes_scripts(self):
        with tempfile.TemporaryDirectory() as tmp:
            svg = Path(tmp) / "unsafe.svg"
            svg.write_text('<svg xmlns="http://www.w3.org/2000/svg" onclick="x()"><script>x()</script><path d="M0 0H1"/></svg>', encoding="utf-8")
            code, stdout, stderr = self.run_cli(["s", "--input", str(svg)])
        self.assertEqual(code, 0, stderr)
        self.assertNotIn("script", stdout)
        self.assertNotIn("onclick", stdout)
        self.assertIn("<path", stdout)

    def test_removed_long_compatibility_names(self):
        code, _stdout, stderr = self.run_cli(["optimize", "--help"])
        self.assertNotEqual(code, 0)
        self.assertIn("invalid choice", stderr)

    def test_recipe_init_outputs_json_template(self):
        code, stdout, stderr = self.run_cli(["recipe", "init", "--kind", "cleanup"])
        self.assertEqual(code, 0, stderr)
        template = json.loads(stdout)
        self.assertEqual(template["name"], "svg-cleanup")
        self.assertGreaterEqual(len(template["steps"]), 3)

    def test_recipe_run_cleanup_pipeline(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "icon.svg"
            recipe = root / "cleanup.svgo.json"
            output = root / "out.svg"
            report = root / "report.json"
            source.write_text(
                '<svg xmlns="http://www.w3.org/2000/svg"><rect x="1" y="2" width="3" height="4"/></svg>',
                encoding="utf-8",
            )
            recipe.write_text(
                json.dumps(
                    {
                        "steps": [
                            {"command": "sanitize"},
                            {"command": "convert", "shapesToPaths": True, "precision": 2},
                            {"command": "viewbox", "fitContent": True, "padding": 0, "removeDimensions": True, "precision": 2},
                            {"command": "validate", "strict": True},
                            {"command": "opt", "multipass": True, "precision": 2},
                        ]
                    }
                ),
                encoding="utf-8",
            )
            code, stdout, stderr = self.run_cli(
                ["recipe", "run", "--recipe", str(recipe), "--input", str(source), "--output", str(output), "--report", str(report)]
            )
            self.assertEqual(code, 0, stderr)
            self.assertIn(str(output), stdout)
            result = output.read_text(encoding="utf-8")
            self.assertIn("viewBox", result)
            self.assertIn("<path", result)
            details = json.loads(report.read_text(encoding="utf-8"))
            self.assertEqual(details[0]["output"], str(output))
            self.assertEqual(details[0]["steps"][3]["command"], "validate")
            self.assertTrue(details[0]["steps"][3]["valid"])


if __name__ == "__main__":
    unittest.main()
