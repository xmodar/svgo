import unittest

from svgo_py import convert_shapes_svg, flatten_svg, get_svg_info, to_plain_svg, validate_svg


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


if __name__ == "__main__":
    unittest.main()
