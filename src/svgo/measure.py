"""Thin Python bindings for Rust path and SVG measurement."""

from __future__ import annotations

import json
from typing import Any

from . import _svgo as _rust


def path_length(path_data: str, *, error: float = 0.01) -> float:
    return _rust.path_length(path_data, error)


def path_bbox(path_data: str, *, decimals: int | None = None) -> dict[str, float] | None:
    return json.loads(_rust.path_bbox_json(path_data, decimals))


def point_at_length(path_data: str, distance: float, *, error: float = 0.01) -> dict[str, float]:
    return json.loads(_rust.point_at_length_json(path_data, distance, error))


def path_metrics(path_data: str, *, decimals: int | None = None, error: float = 0.01) -> dict[str, Any]:
    return json.loads(_rust.path_metrics_json(path_data, decimals, error))


def svg_metrics(svg_input: str, *, decimals: int | None = None, error: float = 0.01) -> dict[str, Any]:
    return json.loads(_rust.svg_metrics_json(str(svg_input), decimals, error))


def metrics_json(metrics: dict[str, Any], *, compact: bool = False) -> str:
    return json.dumps(metrics, indent=None if compact else 2, sort_keys=True)
