"""Path and SVG measurement helpers."""

from __future__ import annotations

import json
import math
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable

from .inspect_svg import read_svg_text
from .pathdata import (
    Matrix,
    PathData,
    PathDataError,
    Point,
    Segment,
    arc_to_cubic_segments,
    matrix_multiply,
    quadratic_to_cubic_segment,
    transform_path,
)
from .svg_optimize import OptimizeOptions, local_name, parse_transform, shape_to_path_d

IDENTITY_MATRIX: Matrix = (1.0, 0.0, 0.0, 1.0, 0.0, 0.0)


@dataclass(frozen=True)
class Bounds:
    """Axis-aligned bounding box for measured geometry."""

    x: float
    y: float
    x2: float
    y2: float

    @property
    def width(self) -> float:
        return self.x2 - self.x

    @property
    def height(self) -> float:
        return self.y2 - self.y

    @property
    def cx(self) -> float:
        return (self.x + self.x2) / 2.0

    @property
    def cy(self) -> float:
        return (self.y + self.y2) / 2.0


@dataclass(frozen=True)
class MeasureSegment:
    """Drawable line or cubic segment used for measurement."""

    kind: str
    start: Point
    end: Point
    controls: tuple[Point, ...] = ()


def path_length(path_data: str, *, error: float = 0.01) -> float:
    """Return the approximate arc length of SVG path data."""

    return sum(segment_length(segment, error) for segment in measurable_segments(path_data))


def path_bbox(path_data: str, *, decimals: int | None = None) -> dict[str, float] | None:
    """Return the path bounding box as a dictionary, or ``None`` for empty paths."""

    bounds = segments_bounds(measurable_segments(path_data))
    return bounds_dict(bounds, decimals) if bounds else None


def point_at_length(path_data: str, distance: float, *, error: float = 0.01) -> dict[str, float]:
    """Return a point on a path at the requested distance along the path."""

    target = float(distance)
    segments = measurable_segments(path_data)
    if not segments:
        raise ValueError("Path contains no drawable segments")
    if target <= 0:
        first = segments[0].start
        return {"x": first.x, "y": first.y}

    remaining = target
    last = segments[-1].end
    for segment in segments:
        length = segment_length(segment, error)
        if remaining <= length:
            point = point_on_segment_at_length(segment, remaining, max(error / 10.0, 1e-6))
            return {"x": point.x, "y": point.y}
        remaining -= length
        last = segment.end
    return {"x": last.x, "y": last.y}


def path_metrics(path_data: str, *, decimals: int | None = None, error: float = 0.01) -> dict[str, Any]:
    """Return length, bounding box, and segment counts for path data."""

    segments = measurable_segments(path_data)
    bounds = segments_bounds(segments)
    length = sum(segment_length(segment, error) for segment in segments)
    return {
        "length": round_number(length, decimals),
        "bbox": bounds_dict(bounds, decimals) if bounds else None,
        "segments": len(segments),
    }


def svg_metrics(svg_input: str | Path, *, decimals: int | None = None, error: float = 0.01) -> dict[str, Any]:
    """Measure all path/basic-shape geometry in an SVG document."""

    text, source, read_error = read_svg_text(svg_input)
    if read_error:
        return {"error": read_error, "source": source}
    try:
        root = ET.fromstring(text.strip())
    except ET.ParseError as exc:
        return {"error": f"XML parse error: {exc}", "source": source}

    paths: list[dict[str, Any]] = []
    warnings: list[str] = []
    total_length = 0.0
    for index, item in enumerate(iter_svg_paths(root, warnings, decimals)):
        total_length += path_length(item["d"], error=error)
        metrics = path_metrics(item["d"], decimals=decimals, error=error)
        metrics.update(
            {
                "index": index,
                "element": item["element"],
                "id": item.get("id"),
                "d": item["d"],
            }
        )
        paths.append(metrics)

    overall = union_bounds(entry["bbox"] for entry in paths if entry.get("bbox"))
    return {
        "source": source,
        "length": round_number(total_length, decimals),
        "bbox": bounds_dict(overall, decimals) if overall else None,
        "paths": paths,
        "path_count": len(paths),
        "warnings": warnings,
    }


def measurable_segments(path_data: str) -> list[MeasureSegment]:
    """Convert path data to drawable line/cubic measurement segments."""

    parsed = PathData.parse(path_data)
    segments: list[MeasureSegment] = []
    for segment in parsed.segments:
        if segment.cmd == "M":
            continue
        if segment.cmd == "L":
            append_line_segment(segments, segment.start, segment.end)
        elif segment.cmd == "Q":
            cubic = quadratic_to_cubic_segment(segment.start, segment.values[0], segment.end, segment.index)
            append_cubic_segment(segments, cubic)
        elif segment.cmd == "C":
            append_cubic_segment(segments, segment)
        elif segment.cmd == "A":
            for cubic in arc_to_cubic_segments(segment):
                if cubic.cmd == "L":
                    append_line_segment(segments, cubic.start, cubic.end)
                else:
                    append_cubic_segment(segments, cubic)
        elif segment.cmd == "Z":
            append_line_segment(segments, segment.start, segment.end)
        else:
            raise PathDataError(f"Unsupported segment for measurement: {segment.cmd}")
    return segments


def append_line_segment(segments: list[MeasureSegment], start: Point, end: Point) -> None:
    if not start.close_to(end):
        segments.append(MeasureSegment("line", start, end))


def append_cubic_segment(segments: list[MeasureSegment], segment: Segment) -> None:
    c1, c2 = segment.values
    if not isinstance(c1, Point) or not isinstance(c2, Point):
        raise PathDataError("Cubic segment controls must be points")
    if segment.start.close_to(segment.end) and segment.start.close_to(c1) and segment.end.close_to(c2):
        return
    segments.append(MeasureSegment("cubic", segment.start, segment.end, (c1, c2)))


def segment_length(segment: MeasureSegment, error: float) -> float:
    if segment.kind == "line":
        return distance(segment.start, segment.end)
    c1, c2 = segment.controls
    return cubic_length(segment.start, c1, c2, segment.end, max(float(error), 1e-9))


def point_on_segment_at_length(segment: MeasureSegment, target: float, error: float) -> Point:
    if segment.kind == "line":
        length = distance(segment.start, segment.end)
        if length == 0:
            return segment.end
        t = max(0.0, min(1.0, target / length))
        return lerp(segment.start, segment.end, t)

    c1, c2 = segment.controls
    total = cubic_length(segment.start, c1, c2, segment.end, error)
    if total == 0:
        return segment.end
    lo = 0.0
    hi = 1.0
    for _ in range(32):
        mid = (lo + hi) / 2.0
        left = split_cubic(segment.start, c1, c2, segment.end, mid)[0]
        length = cubic_length(*left, error)
        if length < target:
            lo = mid
        else:
            hi = mid
    return cubic_point(segment.start, c1, c2, segment.end, (lo + hi) / 2.0)


def segments_bounds(segments: Iterable[MeasureSegment]) -> Bounds | None:
    points: list[Point] = []
    for segment in segments:
        if segment.kind == "line":
            points.extend((segment.start, segment.end))
        else:
            c1, c2 = segment.controls
            for t in cubic_extrema(segment.start, c1, c2, segment.end):
                points.append(cubic_point(segment.start, c1, c2, segment.end, t))
    if not points:
        return None
    xs = [point.x for point in points]
    ys = [point.y for point in points]
    return Bounds(min(xs), min(ys), max(xs), max(ys))


def bounds_dict(bounds: Bounds | dict[str, float] | None, decimals: int | None) -> dict[str, float] | None:
    if bounds is None:
        return None
    if isinstance(bounds, dict):
        return {key: round_number(value, decimals) for key, value in bounds.items()}
    return {
        "x": round_number(bounds.x, decimals),
        "y": round_number(bounds.y, decimals),
        "x2": round_number(bounds.x2, decimals),
        "y2": round_number(bounds.y2, decimals),
        "width": round_number(bounds.width, decimals),
        "height": round_number(bounds.height, decimals),
        "cx": round_number(bounds.cx, decimals),
        "cy": round_number(bounds.cy, decimals),
    }


def union_bounds(items: Iterable[dict[str, float] | Bounds | None]) -> Bounds | None:
    bounds: list[Bounds] = []
    for item in items:
        if item is None:
            continue
        if isinstance(item, Bounds):
            bounds.append(item)
        else:
            bounds.append(Bounds(float(item["x"]), float(item["y"]), float(item["x2"]), float(item["y2"])))
    if not bounds:
        return None
    return Bounds(
        min(item.x for item in bounds),
        min(item.y for item in bounds),
        max(item.x2 for item in bounds),
        max(item.y2 for item in bounds),
    )


def iter_svg_paths(root: ET.Element, warnings: list[str], decimals: int | None) -> Iterable[dict[str, str]]:
    options = OptimizeOptions(float_precision=decimals)

    def walk(element: ET.Element, inherited: Matrix) -> Iterable[dict[str, str]]:
        transform = element.attrib.get("transform")
        matrix = inherited
        if transform:
            try:
                matrix = matrix_multiply(inherited, parse_transform(transform))
            except (PathDataError, ValueError) as exc:
                warnings.append(f"Could not parse transform on <{local_name(element.tag)}>: {exc}")
        name = local_name(element.tag)
        d = element.attrib.get("d") if name == "path" else shape_to_path_d(element, options)
        if d:
            if matrix != IDENTITY_MATRIX:
                try:
                    d = transform_path(d, matrix, decimals or 4, True)
                except PathDataError as exc:
                    warnings.append(f"Could not transform <{name}> geometry: {exc}")
            yield {"element": name, "id": element.attrib.get("id", ""), "d": d}
        for child in list(element):
            yield from walk(child, matrix)

    yield from walk(root, IDENTITY_MATRIX)


def distance(a: Point, b: Point) -> float:
    return math.hypot(a.x - b.x, a.y - b.y)


def lerp(a: Point, b: Point, t: float) -> Point:
    return Point(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)


def cubic_point(p0: Point, p1: Point, p2: Point, p3: Point, t: float) -> Point:
    mt = 1.0 - t
    return Point(
        mt**3 * p0.x + 3.0 * mt**2 * t * p1.x + 3.0 * mt * t**2 * p2.x + t**3 * p3.x,
        mt**3 * p0.y + 3.0 * mt**2 * t * p1.y + 3.0 * mt * t**2 * p2.y + t**3 * p3.y,
    )


def split_cubic(p0: Point, p1: Point, p2: Point, p3: Point, t: float) -> tuple[tuple[Point, Point, Point, Point], tuple[Point, Point, Point, Point]]:
    p01 = lerp(p0, p1, t)
    p12 = lerp(p1, p2, t)
    p23 = lerp(p2, p3, t)
    p012 = lerp(p01, p12, t)
    p123 = lerp(p12, p23, t)
    p0123 = lerp(p012, p123, t)
    return (p0, p01, p012, p0123), (p0123, p123, p23, p3)


def cubic_length(p0: Point, p1: Point, p2: Point, p3: Point, error: float, depth: int = 0) -> float:
    chord = distance(p0, p3)
    control = distance(p0, p1) + distance(p1, p2) + distance(p2, p3)
    if depth >= 16 or control - chord <= error:
        return (control + chord) / 2.0
    left, right = split_cubic(p0, p1, p2, p3, 0.5)
    return cubic_length(*left, error / 2.0, depth + 1) + cubic_length(*right, error / 2.0, depth + 1)


def cubic_extrema(p0: Point, p1: Point, p2: Point, p3: Point) -> list[float]:
    roots = {0.0, 1.0}
    for values in ((p0.x, p1.x, p2.x, p3.x), (p0.y, p1.y, p2.y, p3.y)):
        roots.update(root for root in cubic_derivative_roots(*values) if 0.0 < root < 1.0)
    return sorted(roots)


def cubic_derivative_roots(p0: float, p1: float, p2: float, p3: float) -> list[float]:
    a = -p0 + 3.0 * p1 - 3.0 * p2 + p3
    b = 3.0 * p0 - 6.0 * p1 + 3.0 * p2
    c = -3.0 * p0 + 3.0 * p1
    qa = 3.0 * a
    qb = 2.0 * b
    qc = c
    if abs(qa) < 1e-12:
        if abs(qb) < 1e-12:
            return []
        return [-qc / qb]
    disc = qb * qb - 4.0 * qa * qc
    if disc < 0:
        return []
    root_disc = math.sqrt(max(0.0, disc))
    return [(-qb - root_disc) / (2.0 * qa), (-qb + root_disc) / (2.0 * qa)]


def round_number(value: float, decimals: int | None) -> float:
    if decimals is None:
        return value
    return round(value, max(0, int(decimals)))


def metrics_json(metrics: dict[str, Any], *, compact: bool = False) -> str:
    """Serialize a metrics dictionary as JSON."""

    return json.dumps(metrics, indent=None if compact else 2, sort_keys=True)
