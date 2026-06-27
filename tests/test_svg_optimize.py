import unittest

from svgo_py.svg_optimize import OptimizeOptions, PluginSpec, optimize_svg


class SvgOptimizeTests(unittest.TestCase):
    def test_default_preset_converts_rect_and_removes_comment(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg"><!--x--><rect width="10" height="10"/></svg>'
        out = optimize_svg(svg, OptimizeOptions(float_precision=2))
        self.assertNotIn("<!--", out)
        self.assertIn("<path", out)
        self.assertIn("d=", out)

    def test_explicit_remove_dimensions(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="20" viewBox="0 0 10 20"/>'
        out = optimize_svg(svg, OptimizeOptions(plugins=[PluginSpec("removeDimensions")]))
        self.assertNotIn("width=", out)
        self.assertNotIn("height=", out)
        self.assertIn("viewBox", out)

    def test_data_uri(self):
        out = optimize_svg("<svg/>", OptimizeOptions(preset="none", datauri="enc"))
        self.assertTrue(out.startswith("data:image/svg+xml,"))


if __name__ == "__main__":
    unittest.main()
