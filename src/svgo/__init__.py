"""Pure-Python SVG path editing, optimization, tracing, and centerline APIs."""

from .centerline import centerline_path_data, centerline_svg_text
from .geometry import (
    circle_to_path,
    ellipse_to_path,
    get_kappa,
    get_precision,
    identity,
    line_to_path,
    matrix_to_3x3,
    multiply_matrices,
    polygon_to_path,
    polyline_to_path,
    rect_to_path,
    rotate_2d,
    scale_2d,
    set_precision,
    transform_2d,
    transform_geometry_path,
    translate_2d,
)
from .inspect_svg import (
    convert_shapes_svg,
    flatten_svg,
    get_svg_info,
    inline_styles_svg,
    sanitize_svg,
    to_plain_svg,
    validate_svg,
    validate_svg_async,
)
from .measure import metrics_json, path_bbox, path_length, path_metrics, point_at_length, svg_metrics
from .pathdata import PathData, parse_path, path_to_absolute, path_to_cubics, path_to_relative, path_to_string, transform_path
from .raster_trace import trace_png
from .svg_optimize import BUILTIN_PLUGINS, OptimizeOptions, optimize_svg
from .viewport import fit_viewbox_svg, resize_svg, set_viewbox_svg

__all__ = [
    "BUILTIN_PLUGINS",
    "OptimizeOptions",
    "PathData",
    "circle_to_path",
    "centerline_path_data",
    "centerline_svg_text",
    "convert_shapes_svg",
    "ellipse_to_path",
    "flatten_svg",
    "fit_viewbox_svg",
    "get_kappa",
    "get_precision",
    "get_svg_info",
    "identity",
    "inline_styles_svg",
    "line_to_path",
    "matrix_to_3x3",
    "metrics_json",
    "multiply_matrices",
    "optimize_svg",
    "parse_path",
    "path_bbox",
    "path_length",
    "path_metrics",
    "path_to_absolute",
    "path_to_cubics",
    "path_to_relative",
    "path_to_string",
    "point_at_length",
    "polygon_to_path",
    "polyline_to_path",
    "rect_to_path",
    "resize_svg",
    "rotate_2d",
    "sanitize_svg",
    "scale_2d",
    "set_precision",
    "set_viewbox_svg",
    "svg_metrics",
    "to_plain_svg",
    "transform_2d",
    "transform_geometry_path",
    "transform_path",
    "trace_png",
    "translate_2d",
    "validate_svg",
    "validate_svg_async",
]

__version__ = "0.1.0b1"
