"""Thin Python bindings for Rust VTracer-style tracing."""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass
from pathlib import Path

from . import _svgo as _rust

VTracerTraceError = ValueError


@dataclass(frozen=True)
class VTracerOptions:
    color_mode: str = "color"
    hierarchical: str = "stacked"
    color_precision: int = 6
    gradient_step: int = 16
    filter_speckle: int = 4
    curve_mode: str = "spline"
    corner_threshold: int = 60
    segment_length: float = 4.0
    max_iterations: int = 10
    splice_threshold: int = 45
    path_precision: int = 8


def trace_image_vtracer(path: str | Path, options: VTracerOptions | None = None) -> str:
    return _rust.trace_image_vtracer(str(path), None if options is None else json.dumps(asdict(options)))
