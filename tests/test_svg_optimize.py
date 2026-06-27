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

    def test_single_pass_keeps_explicit_conversion_even_when_longer(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg"><circle cx="5" cy="5" r="2"/></svg>'
        out = optimize_svg(svg, OptimizeOptions(preset="none", plugins=[PluginSpec("convertShapeToPath")], float_precision=2))
        self.assertIn("<path", out)
        self.assertNotIn("<circle", out)

    def test_pysvgo_plugin_name_compatibility(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script><defs><path d="M0 0"/><path id="keep" d="M1 1"/></defs></svg>'
        out = optimize_svg(svg, OptimizeOptions(preset="none", plugins=[PluginSpec("removeScriptElement"), PluginSpec("removeUselessDefs")]))
        self.assertNotIn("<script", out)
        self.assertNotIn('d="M0 0"', out)
        self.assertIn('id="keep"', out)


if __name__ == "__main__":
    unittest.main()
