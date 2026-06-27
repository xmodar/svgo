"""Geometry and matrix helpers for SVG path workflows."""

from __future__ import annotations

import math
from typing import Sequence

from .pathdata import (
    Matrix,
    PathData,
    coerce_matrix,
    fmt_number,
    matrix_multiply,
    rotate_matrix,
    scale_matrix,
    transform_path,
    translate_matrix,
)

_PRECISION = 20


def set_precision(decimals: int) -> None:
    """Set the default decimal precision used by geometry helpers."""
    global _PRECISION
    _PRECISION = max(0, int(decimals))


def get_precision() -> int:
    """Return the current geometry-helper decimal precision."""
    return _PRECISION


def get_kappa() -> float:
    """Return the cubic Bezier kappa constant for circular arcs."""
    return 4.0 * (math.sqrt(2.0) - 1.0) / 3.0


def identity(size: int = 3) -> list[list[float]]:
    """Return a square identity matrix."""
    if size <= 0:
        raise ValueError("identity size must be positive")
    return [[1.0 if row == col else 0.0 for col in range(size)] for row in range(size)]


def translate_2d(tx: float, ty: float = 0.0) -> Matrix:
    """Create an SVG affine translation matrix."""
    return translate_matrix(float(tx), float(ty))


def scale_2d(sx: float, sy: float | None = None) -> Matrix:
    """Create an SVG affine scale matrix."""
    return scale_matrix(float(sx), float(sx if sy is None else sy))


def rotate_2d(angle: float, cx: float = 0.0, cy: float = 0.0, *, degrees: bool = False) -> Matrix:
    """Create an SVG affine rotation matrix. Angles are radians by default."""
    deg = float(angle) if degrees else math.degrees(float(angle))
    return rotate_matrix(float(cx), float(cy), deg)


def multiply_matrices(left: Matrix | Sequence[float] | Sequence[Sequence[float]], right: Matrix | Sequence[float] | Sequence[Sequence[float]]) -> Matrix:
    """Multiply two SVG affine or row-major 3x3 matrices."""
    return matrix_multiply(coerce_matrix(left), coerce_matrix(right))


def transform_2d(matrix: Matrix | Sequence[float] | Sequence[Sequence[float]], x: float, y: float) -> tuple[float, float]:
    """Apply an SVG affine matrix to a point."""
    a, b, c, d, e, f = coerce_matrix(matrix)
    return a * float(x) + c * float(y) + e, b * float(x) + d * float(y) + f


def matrix_to_3x3(matrix: Matrix | Sequence[float] | Sequence[Sequence[float]]) -> list[list[float]]:
    """Return a row-major 3x3 representation of an SVG affine matrix."""
    a, b, c, d, e, f = coerce_matrix(matrix)
    return [[a, c, e], [b, d, f], [0.0, 0.0, 1.0]]


def rect_to_path(
    x: float = 0.0,
    y: float = 0.0,
    width: float = 0.0,
    height: float = 0.0,
    rx: float = 0.0,
    ry: float | None = None,
    *,
    decimals: int | None = None,
    minify: bool = False,
) -> str:
    """Convert an SVG rectangle to path data."""
    width = float(width)
    height = float(height)
    if width < 0 or height < 0:
        raise ValueError("rect width and height must be non-negative")
    if width == 0 or height == 0:
        return ""
    x = float(x)
    y = float(y)
    rx = min(max(float(rx), 0.0), width / 2.0)
    ry = min(max(float(rx if ry is None else ry), 0.0), height / 2.0)
    if rx == 0 and ry == 0:
        return _format_path(f"M{x} {y}L{x + width} {y}L{x + width} {y + height}L{x} {y + height}Z", decimals, minify)

    k = get_kappa()
    ox = rx * k
    oy = ry * k
    raw = (
        f"M{x + rx} {y}L{x + width - rx} {y}"
        f"C{x + width - rx + ox} {y} {x + width} {y + ry - oy} {x + width} {y + ry}"
        f"L{x + width} {y + height - ry}"
        f"C{x + width} {y + height - ry + oy} {x + width - rx + ox} {y + height} {x + width - rx} {y + height}"
        f"L{x + rx} {y + height}"
        f"C{x + rx - ox} {y + height} {x} {y + height - ry + oy} {x} {y + height - ry}"
        f"L{x} {y + ry}"
        f"C{x} {y + ry - oy} {x + rx - ox} {y} {x + rx} {y}Z"
    )
    return _format_path(raw, decimals, minify)


def line_to_path(x1: float, y1: float, x2: float, y2: float, *, decimals: int | None = None, minify: bool = False) -> str:
    """Convert an SVG line to path data."""
    return _format_path(f"M{x1} {y1}L{x2} {y2}", decimals, minify)


def circle_to_path(cx: float, cy: float, r: float, *, decimals: int | None = None, minify: bool = False) -> str:
    """Convert an SVG circle to cubic Bezier path data."""
    r = float(r)
    if r < 0:
        raise ValueError("circle radius must be non-negative")
    return ellipse_to_path(cx, cy, r, r, decimals=decimals, minify=minify)


def ellipse_to_path(cx: float, cy: float, rx: float, ry: float, *, decimals: int | None = None, minify: bool = False) -> str:
    """Convert an SVG ellipse to cubic Bezier path data."""
    rx = float(rx)
    ry = float(ry)
    if rx < 0 or ry < 0:
        raise ValueError("ellipse radii must be non-negative")
    if rx == 0 or ry == 0:
        return ""
    cx = float(cx)
    cy = float(cy)
    ox = rx * get_kappa()
    oy = ry * get_kappa()
    raw = (
        f"M{cx} {cy - ry}"
        f"C{cx + ox} {cy - ry} {cx + rx} {cy - oy} {cx + rx} {cy}"
        f"C{cx + rx} {cy + oy} {cx + ox} {cy + ry} {cx} {cy + ry}"
        f"C{cx - ox} {cy + ry} {cx - rx} {cy + oy} {cx - rx} {cy}"
        f"C{cx - rx} {cy - oy} {cx - ox} {cy - ry} {cx} {cy - ry}Z"
    )
    return _format_path(raw, decimals, minify)


def polyline_to_path(points: Sequence[Sequence[float]], *, decimals: int | None = None, minify: bool = False) -> str:
    """Convert a sequence of points to open polyline path data."""
    pairs = _point_pairs(points)
    if not pairs:
        return ""
    raw = "M" + "L".join(f"{x} {y}" for x, y in pairs)
    return _format_path(raw, decimals, minify)


def polygon_to_path(points: Sequence[Sequence[float]], *, decimals: int | None = None, minify: bool = False) -> str:
    """Convert a sequence of points to closed polygon path data."""
    path = polyline_to_path(points, decimals=decimals, minify=minify)
    return path + ("Z" if minify else " Z") if path else ""


def transform_geometry_path(path_data: str, matrix: Matrix | Sequence[float] | Sequence[Sequence[float]], *, decimals: int | None = None, minify: bool = False) -> str:
    """Transform path data using the geometry helper default precision."""
    return transform_path(path_data, matrix, _decimals(decimals), minify)


def _point_pairs(points: Sequence[Sequence[float]]) -> list[tuple[float, float]]:
    pairs: list[tuple[float, float]] = []
    for point in points:
        if len(point) != 2:
            raise ValueError("points must contain two numeric values")
        pairs.append((float(point[0]), float(point[1])))
    return pairs


def _format_path(path_data: str, decimals: int | None, minify: bool) -> str:
    return PathData.parse(path_data).to_string(_decimals(decimals), minify)


def _decimals(decimals: int | None) -> int:
    return _PRECISION if decimals is None else max(0, int(decimals))


def format_number(value: float, decimals: int | None = None, minify: bool = False) -> str:
    """Format a number with the current geometry-helper precision."""
    return fmt_number(float(value), _decimals(decimals), minify)
