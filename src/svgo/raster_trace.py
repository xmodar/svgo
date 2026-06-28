"""Thin Python bindings for Rust PNG tracing."""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass
from pathlib import Path

from . import _svgo as _rust

RasterTraceError = ValueError


@dataclass(frozen=True)
class Image:
    width: int
    height: int
    pixels: list[tuple[int, int, int, int]]


@dataclass(frozen=True)
class TraceOptions:
    mode: str = "palette"
    alpha_threshold: int = 16
    white_threshold: int = 250
    drop_white: bool = False
    quantize: int = 24
    max_colors: int = 8
    min_area: int = 4
    scale: float = 1.0
    decimals: int = 3
    title: str | None = None
    curve_mode: str = "pixel"
    palette: tuple[str, ...] = ()


def _options_json(options: TraceOptions | None) -> str | None:
    return None if options is None else json.dumps(asdict(options))


def trace_image(image: Image, options: TraceOptions | None = None) -> str:
    return _rust.trace_image(json.dumps(asdict(image)), _options_json(options))


def trace_png(path: str | Path, options: TraceOptions | None = None) -> str:
    return _rust.trace_png(str(path), _options_json(options))


def trace_image_components(image: Image, options: TraceOptions | None = None) -> dict:
    return json.loads(_rust.trace_image_components_json(json.dumps(asdict(image)), _options_json(options)))


def trace_png_components(path: str | Path, options: TraceOptions | None = None) -> dict:
    return json.loads(_rust.trace_png_components_json(str(path), _options_json(options)))
