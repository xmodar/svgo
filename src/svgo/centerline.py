"""Thin Python bindings for Rust centerline reconstruction."""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass
from pathlib import Path

from . import _svgo as _rust

CenterlineError = ValueError


@dataclass(frozen=True)
class RasterContext:
    min_x: float
    min_y: float
    scale: float
    pad: int
    width: int
    height: int


@dataclass(frozen=True)
class CenterlineOptions:
    emit: str = "path"
    mode: str = "longest"
    scale: float = 2.0
    max_size: int = 1600
    curve_samples: int = 24
    simplify: float = 6.0
    min_length: float = 20.0
    stroke_width: str = "auto"
    linecap: str = "round"
    linejoin: str = "round"
    decimals: int = 3
    polyline: bool = False
    fill_rule: str = "evenodd"
    svg_paths: str = "first"
    keep_failed: bool = False
    bridge_gap: float = 0.0


def _options_json(options: CenterlineOptions | None) -> str | None:
    return None if options is None else json.dumps(asdict(options))


def read_path_data(path: str | Path) -> str:
    text = Path(path).read_text(encoding="utf-8").strip()
    marker = 'd="'
    if marker in text:
        start = text.index(marker) + len(marker)
        end = text.index('"', start)
        return text[start:end].strip()
    return text


def centerline_path_data(path_data: str, options: CenterlineOptions | None = None):
    result = json.loads(_rust.centerline_path_data_json(path_data, _options_json(options)))
    return result["d"], result["stroke_width"], RasterContext(**result["ctx"])


def centerline_svg_text(svg_text: str, options: CenterlineOptions | None = None) -> str:
    return _rust.centerline_svg_text(svg_text, _options_json(options))


def build_output(d: str, emit: str, stroke_width: float, options: CenterlineOptions, ctx: RasterContext) -> str:
    if emit == "d":
        return d
    style = f"fill: none; stroke-linecap: {options.linecap}; stroke-width: {stroke_width:.{options.decimals}f}px; stroke-linejoin: {options.linejoin};"
    path = f'<path style="{style}" d="{d}"/>'
    if emit == "path":
        return path
    return f'<svg xmlns="http://www.w3.org/2000/svg">{path}</svg>'
