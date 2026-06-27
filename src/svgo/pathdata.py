"""Thin Python bindings for Rust SVG path data operations."""

from __future__ import annotations

import json
from collections.abc import Sequence

from . import _svgo as _rust

PathDataError = ValueError
Matrix = tuple[float, float, float, float, float, float]


def _matrix_values(matrix: Sequence[float] | Sequence[Sequence[float]]) -> list[float]:
    values = list(matrix)
    if len(values) == 3 and all(isinstance(row, Sequence) for row in values):
        return [float(item) for row in values for item in row]  # type: ignore[arg-type]
    return [float(item) for item in values]  # type: ignore[arg-type]


class PathData:
    """Parsed path handle backed by Rust."""

    def __init__(self, inner):
        self._inner = inner

    @classmethod
    def parse(cls, path_data: str) -> "PathData":
        return cls(_rust.PathData.parse(path_data))

    def apply_operation(self, operation: str, decimals: int = 4) -> "PathData":
        self._inner.apply_operation(operation, decimals)
        return self

    def transform(self, matrix: Sequence[float] | Sequence[Sequence[float]]) -> "PathData":
        self._inner.transform(_matrix_values(matrix))
        return self

    def translate(self, dx: float, dy: float) -> "PathData":
        self._inner.translate(dx, dy)
        return self

    def scale(self, kx: float, ky: float) -> "PathData":
        self._inner.scale(kx, ky)
        return self

    def rotate(self, ox: float, oy: float, degrees: float) -> "PathData":
        self._inner.rotate(ox, oy, degrees)
        return self

    def set_relative(self, relative: bool) -> "PathData":
        self._inner.set_relative(relative)
        return self

    def reverse(self, item_index: int | None = None) -> "PathData":
        self._inner.reverse(item_index)
        return self

    def change_origin(self, item_index: int, subpath: bool = False) -> "PathData":
        self._inner.change_origin(item_index, subpath)
        return self

    def optimize(self, profile: str | None = "safe") -> "PathData":
        self._inner.optimize(profile)
        return self

    def to_cubics(self) -> "PathData":
        self._inner.to_cubics()
        return self

    def to_string(self, decimals: int = 4, minify: bool = False) -> str:
        return self._inner.to_string(decimals, minify)

    def command_items(self) -> list[dict[str, object]]:
        return json.loads(self._inner.command_items_json())


def parse_path(path_data: str) -> list[dict[str, object]]:
    return json.loads(_rust.parse_path_json(path_data))


def path_to_absolute(path_data: str, decimals: int = 4, minify: bool = False) -> str:
    return _rust.path_to_absolute(path_data, decimals, minify)


def path_to_relative(path_data: str, decimals: int = 4, minify: bool = False) -> str:
    return _rust.path_to_relative(path_data, decimals, minify)


def path_to_string(path_data: str, decimals: int = 4, minify: bool = False) -> str:
    return PathData.parse(path_data).to_string(decimals, minify)


def transform_path(path_data: str, matrix: Sequence[float] | Sequence[Sequence[float]], decimals: int = 4, minify: bool = False) -> str:
    return _rust.transform_path(path_data, _matrix_values(matrix), decimals, minify)


def path_to_cubics(path_data: str, decimals: int = 4, minify: bool = False) -> str:
    return _rust.path_to_cubics(path_data, decimals, minify)


def coerce_matrix(matrix: Sequence[float] | Sequence[Sequence[float]]) -> Matrix:
    values = _matrix_values(matrix)
    if len(values) == 6:
        return tuple(values)  # type: ignore[return-value]
    if len(values) == 9:
        return (values[0], values[3], values[1], values[4], values[2], values[5])
    raise ValueError("matrix must contain 6 SVG affine values or 9 row-major 3x3 values")


def fmt_number(value: float, decimals: int, minify: bool = False) -> str:
    text = f"{round(float(value), decimals):.{decimals}f}".rstrip("0").rstrip(".")
    if not text or text == "-0":
        text = "0"
    if minify and text.startswith("0."):
        return text[1:]
    if minify and text.startswith("-0."):
        return "-." + text[3:]
    return text


parse_transform = None
