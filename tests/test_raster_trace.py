import unittest
from xml.etree import ElementTree as ET

from svgo.raster_trace import Image, RasterTraceError, TraceOptions, trace_image, trace_image_components


class RasterTraceTests(unittest.TestCase):
    def test_trace_simple_alpha_mask(self):
        image = Image(
            width=2,
            height=2,
            pixels=[
                (0, 0, 0, 255),
                (0, 0, 0, 255),
                (0, 0, 0, 255),
                (0, 0, 0, 255),
            ],
        )
        out = trace_image(image, TraceOptions(mode="alpha", min_area=1))
        self.assertIn("<svg", out)
        self.assertIn("<path", out)
        self.assertIn("#000000", out)

    def test_curve_mode_exact_keeps_pixel_trace(self):
        image = Image(
            width=1,
            height=1,
            pixels=[(10, 20, 30, 255)],
        )
        out = trace_image(image, TraceOptions(mode="exact", curve_mode="exact", min_area=1))
        self.assertIn("#001818", out)
        root = ET.fromstring(out)
        path = root.find("{http://www.w3.org/2000/svg}path")
        self.assertIsNotNone(path)
        d = path.attrib["d"]
        for point in ("0 0", "1 0", "1 1", "0 1"):
            self.assertIn(point, d)
        self.assertNotIn("C", d)
        self.assertNotIn("Q", d)

    def test_rejects_non_pixel_curve_modes(self):
        image = Image(
            width=1,
            height=1,
            pixels=[(0, 0, 0, 255)],
        )
        with self.assertRaises(RasterTraceError):
            trace_image(image, TraceOptions(curve_mode="spline", min_area=1))

    def test_trace_components_keeps_disconnected_masks_separate(self):
        image = Image(
            width=3,
            height=1,
            pixels=[
                (20, 56, 97, 255),
                (255, 255, 255, 0),
                (20, 56, 97, 255),
            ],
        )
        out = trace_image_components(image, TraceOptions(mode="alpha", min_area=1))
        self.assertEqual(out["viewBox"], "0 0 3 1")
        self.assertEqual(len(out["components"]), 2)
        self.assertTrue(all(component["color"] == "#183060" for component in out["components"]))
        self.assertNotEqual(out["components"][0]["d"], out["components"][1]["d"])

    def test_trace_components_can_snap_to_fixed_palette(self):
        image = Image(
            width=2,
            height=1,
            pixels=[
                (18, 55, 99, 255),
                (0, 184, 148, 255),
            ],
        )
        out = trace_image_components(
            image,
            TraceOptions(mode="palette", palette=("#143861", "#00b795"), min_area=1),
        )
        colors = sorted(component["color"] for component in out["components"])
        self.assertEqual(colors, ["#00b795", "#143861"])


if __name__ == "__main__":
    unittest.main()
