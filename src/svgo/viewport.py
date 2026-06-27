"""Thin Python bindings for Rust SVG viewport helpers."""

from __future__ import annotations

from pathlib import Path
from typing import Sequence

from . import _svgo as _rust

SvgOptimizeError = ValueError


def set_viewbox_svg(svg_text: str, viewbox: str | Sequence[float], *, precision: int | None = None, remove_dimensions: bool = False) -> str:
    if not isinstance(viewbox, str):
        viewbox = " ".join(str(float(value)) for value in viewbox)
    return _rust.set_viewbox_svg(svg_text, viewbox, precision, remove_dimensions)


def fit_viewbox_svg(svg_text: str, *, padding: float = 0.0, precision: int | None = None, remove_dimensions: bool = False) -> str:
    return _rust.fit_viewbox_svg(svg_text, padding, precision, remove_dimensions)


def resize_svg(svg_text: str, *, width: str | float | None = None, height: str | float | None = None) -> str:
    return _rust.resize_svg(svg_text, None if width is None else str(width), None if height is None else str(height))


def read_svg_file(path: str | Path) -> str:
    return Path(path).read_text(encoding="utf-8")
