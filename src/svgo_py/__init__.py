"""Pure-Python SVG path editing, optimization, tracing, and centerline APIs."""

from .centerline import centerline_path_data, centerline_svg_text
from .pathdata import PathData
from .raster_trace import trace_png
from .svg_optimize import BUILTIN_PLUGINS, OptimizeOptions, optimize_svg

__all__ = [
    "BUILTIN_PLUGINS",
    "OptimizeOptions",
    "PathData",
    "centerline_path_data",
    "centerline_svg_text",
    "optimize_svg",
    "trace_png",
]

__version__ = "0.1.0b1"
