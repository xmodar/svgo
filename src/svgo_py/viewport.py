"""SVG viewport and viewBox helpers."""

from __future__ import annotations

import re
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Sequence

from .measure import svg_metrics
from .pathdata import fmt_number
from .svg_optimize import SVG_NS, SvgOptimizeError, local_name

ET.register_namespace("", SVG_NS)


def set_viewbox_svg(
    svg_text: str,
    viewbox: str | Sequence[float],
    *,
    precision: int | None = None,
    remove_dimensions: bool = False,
) -> str:
    """Set the root SVG ``viewBox`` value."""

    root = parse_svg_root(svg_text)
    values = parse_viewbox(viewbox)
    if values[2] < 0 or values[3] < 0:
        raise SvgOptimizeError("viewBox width and height must be non-negative")
    root.set("viewBox", format_viewbox(values, precision))
    if remove_dimensions:
        root.attrib.pop("width", None)
        root.attrib.pop("height", None)
    return serialize_root(root)


def fit_viewbox_svg(
    svg_text: str,
    *,
    padding: float = 0.0,
    precision: int | None = None,
    remove_dimensions: bool = False,
) -> str:
    """Set ``viewBox`` to the measured geometry bounds plus optional padding."""

    metrics = svg_metrics(svg_text)
    if metrics.get("error"):
        raise SvgOptimizeError(str(metrics["error"]))
    bbox = metrics.get("bbox")
    if not isinstance(bbox, dict):
        raise SvgOptimizeError("Cannot fit viewBox: SVG has no measurable geometry")
    pad = max(0.0, float(padding))
    values = (
        float(bbox["x"]) - pad,
        float(bbox["y"]) - pad,
        float(bbox["width"]) + pad * 2.0,
        float(bbox["height"]) + pad * 2.0,
    )
    return set_viewbox_svg(svg_text, values, precision=precision, remove_dimensions=remove_dimensions)


def resize_svg(svg_text: str, *, width: str | float | None = None, height: str | float | None = None) -> str:
    """Set root SVG width and/or height attributes."""

    root = parse_svg_root(svg_text)
    if "viewBox" not in root.attrib:
        inferred = infer_viewbox_from_dimensions(root)
        if inferred:
            root.set("viewBox", format_viewbox(inferred, None))
    if width is not None:
        root.set("width", format_dimension(width))
    if height is not None:
        root.set("height", format_dimension(height))
    return serialize_root(root)


def parse_svg_root(svg_text: str) -> ET.Element:
    try:
        root = ET.fromstring(svg_text.strip())
    except ET.ParseError as exc:
        raise SvgOptimizeError(f"Could not parse SVG: {exc}") from exc
    if local_name(root.tag) != "svg":
        raise SvgOptimizeError("Root element is not <svg>")
    return root


def parse_viewbox(viewbox: str | Sequence[float]) -> tuple[float, float, float, float]:
    if isinstance(viewbox, str):
        values = [float(part) for part in re.findall(r"[-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?", viewbox)]
    else:
        values = [float(part) for part in viewbox]
    if len(values) != 4:
        raise SvgOptimizeError("viewBox requires four numbers: min-x min-y width height")
    return values[0], values[1], values[2], values[3]


def infer_viewbox_from_dimensions(root: ET.Element) -> tuple[float, float, float, float] | None:
    width = parse_dimension(root.attrib.get("width"))
    height = parse_dimension(root.attrib.get("height"))
    if width is None or height is None:
        return None
    return 0.0, 0.0, width, height


def parse_dimension(value: str | None) -> float | None:
    if not value:
        return None
    match = re.match(r"\s*([-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?)", value)
    if not match:
        return None
    number = float(match.group(1))
    return number if number >= 0 else None


def format_dimension(value: str | float) -> str:
    if isinstance(value, str):
        return value
    return fmt_number(float(value), 6, False)


def format_viewbox(values: Sequence[float], precision: int | None) -> str:
    decimals = 6 if precision is None else max(0, int(precision))
    return " ".join(fmt_number(float(value), decimals, False) for value in values)


def serialize_root(root: ET.Element) -> str:
    return ET.tostring(root, encoding="unicode", short_empty_elements=True).strip()


def read_svg_file(path: str | Path) -> str:
    """Read SVG text from disk."""

    return Path(path).read_text(encoding="utf-8")
