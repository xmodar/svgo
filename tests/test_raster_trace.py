import unittest

from svgo.raster_trace import Image, TraceOptions, trace_image


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


if __name__ == "__main__":
    unittest.main()
