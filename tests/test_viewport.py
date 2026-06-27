import contextlib
import io
import tempfile
import unittest
from pathlib import Path

from svgo import fit_viewbox_svg, resize_svg, set_viewbox_svg
from svgo.cli import main


class ViewportTests(unittest.TestCase):
    def test_set_viewbox_svg(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><path d="M0 0H1"/></svg>'
        out = set_viewbox_svg(svg, "0 0 20 20", remove_dimensions=True)
        self.assertIn('viewBox="0 0 20 20"', out)
        self.assertNotIn('width="10"', out)
        self.assertNotIn('height="10"', out)

    def test_fit_viewbox_svg_uses_measured_bounds(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg"><g transform="translate(5 2)"><rect width="10" height="4"/></g></svg>'
        out = fit_viewbox_svg(svg, padding=1, precision=2)
        self.assertIn('viewBox="4 1 12 6"', out)

    def test_resize_svg_sets_dimensions_and_infers_viewbox(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="20"><path d="M0 0H1"/></svg>'
        out = resize_svg(svg, width=40, height="80px")
        self.assertIn('viewBox="0 0 10 20"', out)
        self.assertIn('width="40"', out)
        self.assertIn('height="80px"', out)

    def test_viewbox_cli_alias(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "icon.svg"
            source.write_text('<svg xmlns="http://www.w3.org/2000/svg"><path d="M2 3H6V7H2Z"/></svg>', encoding="utf-8")
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                code = main(["b", "--input", str(source), "--fit-content", "--precision", "1"])
        self.assertEqual(code, 0)
        self.assertIn('viewBox="2 3 4 4"', stdout.getvalue())


if __name__ == "__main__":
    unittest.main()
