import unittest

from svgo_py import convert_shapes_svg, flatten_svg, get_svg_info, inline_styles_svg, sanitize_svg, to_plain_svg, validate_svg


class InspectSvgTests(unittest.TestCase):
    def test_validate_svg_accepts_structural_warnings_by_default(self):
        result = validate_svg('<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0"/></svg>')
        self.assertTrue(result["valid"])
        self.assertTrue(result["issues"])

    def test_validate_svg_rejects_invalid_xml(self):
        result = validate_svg("<svg><path></svg>")
        self.assertFalse(result["valid"])
        self.assertIn("XML parse error", result["error"])

    def test_get_svg_info_counts_shapes_and_fonts(self):
        info = get_svg_info(
            '<svg width="10" height="20"><text style="font-family:Roboto, sans-serif">Hi</text><rect width="1" height="2"/></svg>'
        )
        self.assertEqual(info["width"], "10")
        self.assertEqual(info["height"], "20")
        self.assertEqual(info["shapes"], 1)
        self.assertIn("Roboto", info["fonts"])

    def test_to_plain_removes_editor_data(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg" data-name="x"><metadata>m</metadata><rect width="1" height="1"/></svg>'
        out = to_plain_svg(svg)
        self.assertNotIn("metadata", out)
        self.assertNotIn("data-name", out)

    def test_convert_and_flatten_svg(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg"><g><rect width="10" height="10" transform="translate(2 3)"/></g></svg>'
        converted = convert_shapes_svg(svg)
        self.assertIn("<path", converted)
        flattened = flatten_svg(svg, precision=2)
        self.assertIn("<path", flattened)
        self.assertNotIn("transform=", flattened)

    def test_flatten_svg_bakes_group_transform(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg"><g transform="translate(5 2)"><path d="M0 0H10"/></g></svg>'
        flattened = flatten_svg(svg, precision=2, shapes_to_paths=False, flatten_groups=True)
        self.assertIn('d="M5 2L15 2"', flattened)
        self.assertNotIn("transform=", flattened)

    def test_validate_svg_flags_active_content(self):
        result = validate_svg('<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"><script>alert(1)</script></svg>')
        self.assertFalse(result["valid"])
        reasons = " ".join(issue["reason"] for issue in result["issues"])
        self.assertIn("<script>", reasons)
        self.assertIn("Event handler", reasons)

    def test_sanitize_svg_removes_active_content(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"><a href="javascript:alert(1)"><script>x()</script><path d="M0 0H1"/></a></svg>'
        out = sanitize_svg(svg)
        self.assertNotIn("script", out)
        self.assertNotIn("onload", out)
        self.assertNotIn("javascript:", out)
        self.assertIn("<path", out)

    def test_inline_styles_svg_moves_simple_rules_to_attrs(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg"><style>.hot{fill:red;stroke:#000}</style><path class="hot" d="M0 0H1"/></svg>'
        out = inline_styles_svg(svg)
        self.assertNotIn("<style", out)
        self.assertIn('fill="red"', out)
        self.assertIn('stroke="#000"', out)

    def test_inline_styles_svg_preserves_later_rule_order(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg"><style>.hot{fill:red}.hot{fill:blue}</style><path class="hot" d="M0 0H1"/></svg>'
        out = inline_styles_svg(svg)
        self.assertIn('fill="blue"', out)


if __name__ == "__main__":
    unittest.main()
