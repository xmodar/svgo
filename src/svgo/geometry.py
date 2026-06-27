"""Thin Python bindings for Rust SVG geometry helpers."""

from __future__ import annotations

import json
import math
from collections.abc import Sequence

from . import _svgo as _rust
from .pathdata import _matrix_values

_PRECISION = 20


def set_precision(decimals: int) -> None:
    global _PRECISION
    _PRECISION = max(0, int(decimals))


def get_precision() -> int:
    return _PRECISION


def get_kappa() -> float:
    return 4.0 * (math.sqrt(2.0) - 1.0) / 3.0


def identity(size: int = 3) -> list[list[float]]:
    return _rust.identity(size)


def translate_2d(tx: float, ty: float = 0.0):
    return tuple(_rust.translate_2d(tx, ty))


def scale_2d(sx: float, sy: float | None = None):
    return tuple(_rust.scale_2d(sx, sy))


def rotate_2d(angle: float, cx: float = 0.0, cy: float = 0.0, *, degrees: bool = False):
    return tuple(_rust.rotate_2d(angle, cx, cy, degrees))


def multiply_matrices(left, right):
    return tuple(_rust.multiply_matrices(_matrix_values(left), _matrix_values(right)))


def transform_2d(matrix, x: float, y: float):
    return tuple(_rust.transform_2d(_matrix_values(matrix), x, y))


def matrix_to_3x3(matrix):
    return _rust.matrix_to_3x3(_matrix_values(matrix))


def rect_to_path(x: float = 0.0, y: float = 0.0, width: float = 0.0, height: float = 0.0, rx: float = 0.0, ry: float | None = None, *, decimals: int | None = None, minify: bool = False) -> str:
    return _rust.rect_to_path(x, y, width, height, rx, ry, _PRECISION if decimals is None else decimals, minify)


def line_to_path(x1: float, y1: float, x2: float, y2: float, *, decimals: int | None = None, minify: bool = False) -> str:
    return _rust.line_to_path(x1, y1, x2, y2, _PRECISION if decimals is None else decimals, minify)


def circle_to_path(cx: float, cy: float, r: float, *, decimals: int | None = None, minify: bool = False) -> str:
    return _rust.circle_to_path(cx, cy, r, _PRECISION if decimals is None else decimals, minify)


def ellipse_to_path(cx: float, cy: float, rx: float, ry: float, *, decimals: int | None = None, minify: bool = False) -> str:
    return _rust.ellipse_to_path(cx, cy, rx, ry, _PRECISION if decimals is None else decimals, minify)


def polyline_to_path(points: Sequence[Sequence[float]], *, decimals: int | None = None, minify: bool = False) -> str:
    return _rust.polyline_to_path(json.dumps(points), _PRECISION if decimals is None else decimals, minify)


def polygon_to_path(points: Sequence[Sequence[float]], *, decimals: int | None = None, minify: bool = False) -> str:
    return _rust.polygon_to_path(json.dumps(points), _PRECISION if decimals is None else decimals, minify)


def transform_geometry_path(path_data: str, matrix, *, decimals: int | None = None, minify: bool = False) -> str:
    return _rust.transform_path(path_data, _matrix_values(matrix), _PRECISION if decimals is None else decimals, minify)
