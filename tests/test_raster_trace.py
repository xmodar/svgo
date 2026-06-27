import unittest

from svgo.raster_trace import Image, RasterTraceError, TraceOptions, trace_image


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
        self.assertIn("L1 0", out)

    def test_rejects_non_pixel_curve_modes(self):
        image = Image(
            width=1,
            height=1,
            pixels=[(0, 0, 0, 255)],
        )
        with self.assertRaises(RasterTraceError):
            trace_image(image, TraceOptions(curve_mode="spline", min_area=1))


if __name__ == "__main__":
    unittest.main()
