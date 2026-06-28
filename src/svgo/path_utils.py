"""Reusable SVG path, polyline, and icon conversion helpers."""

from __future__ import annotations

import math
import re
from collections.abc import Iterable, Sequence
from statistics import median

from .pathdata import PathData, fmt_number, parse_path

Point = tuple[float, float]


def normalize_color(value: str) -> str:
    """Normalize a six-digit hex color to lowercase ``#rrggbb`` form."""

    color = value.strip().lower()
    if not color.startswith("#"):
        color = "#" + color
    if not re.fullmatch(r"#[0-9a-f]{6}", color):
        raise ValueError(f"invalid color: {value}")
    return color


def optimize_path_data(path_data: str, decimals: int = 3, profile: str | None = "safe") -> str:
    """Optimize and minify an SVG path data string."""

    return PathData.parse(path_data).optimize(profile).to_string(decimals=decimals, minify=True)


def format_number(value: float, decimals: int = 3, *, minify: bool = False) -> str:
    """Format an SVG number with trimmed trailing zeroes."""

    return fmt_number(value, decimals, minify=minify)


def _point(values: Sequence[object]) -> Point:
    if len(values) < 2:
        raise ValueError("point values must contain at least two coordinates")
    return (float(values[-2]), float(values[-1]))


def _as_points(points: Iterable[Sequence[float]]) -> list[Point]:
    return [(float(point[0]), float(point[1])) for point in points]


def polyline_subpaths(path_data: str, *, close_paths: bool = True) -> list[list[Point]]:
    """Return command endpoints grouped by subpath.

    The parser normalizes SVG commands to absolute coordinates. Curves and arcs
    contribute their endpoint, so this helper is intended for path data that is
    already polyline-like or where endpoint topology is enough.
    """

    subpaths: list[list[Point]] = []
    current: list[Point] = []
    start: Point | None = None
    for item in parse_path(path_data):
        command = str(item["command"]).upper()
        args = item["args"]  # type: ignore[assignment]
        if command == "M":
            if current:
                subpaths.append(current)
            current = [_point(args)]  # type: ignore[arg-type]
            start = current[0]
        elif command == "Z":
            if current and close_paths and start is not None and point_distance(current[-1], start) > 1e-9:
                current.append(start)
            if current:
                subpaths.append(current)
            current = []
            start = None
        elif current and args:
            current.append(_point(args))  # type: ignore[arg-type]
    if current:
        subpaths.append(current)
    return subpaths


def filled_loops(path_data: str) -> list[list[Point]]:
    """Return closed loops from filled path data using command endpoints."""

    loops: list[list[Point]] = []
    current: list[Point] = []
    for item in parse_path(path_data):
        command = str(item["command"]).upper()
        args = item["args"]  # type: ignore[assignment]
        if command == "M":
            if len(current) >= 3:
                loops.append(current)
            current = [_point(args)]  # type: ignore[arg-type]
        elif command == "Z":
            if len(current) >= 3:
                loops.append(current)
            current = []
        elif current and args:
            current.append(_point(args))  # type: ignore[arg-type]
    if len(current) >= 3:
        loops.append(current)
    return loops


def point_distance(a: Point, b: Point) -> float:
    return math.hypot(b[0] - a[0], b[1] - a[1])


def point_line_distance(point: Point, start: Point, end: Point) -> float:
    vx = end[0] - start[0]
    vy = end[1] - start[1]
    wx = point[0] - start[0]
    wy = point[1] - start[1]
    denom = vx * vx + vy * vy
    if denom <= 1e-12:
        return point_distance(point, start)
    t = max(0.0, min(1.0, (wx * vx + wy * vy) / denom))
    px = start[0] + t * vx
    py = start[1] + t * vy
    return math.hypot(point[0] - px, point[1] - py)


def polygon_area(points: Sequence[Point]) -> float:
    pts = list(points)
    if len(pts) < 3:
        return 0.0
    return sum((a[0] * b[1]) - (b[0] * a[1]) for a, b in zip(pts, pts[1:] + pts[:1])) / 2


def polygon_centroid(points: Sequence[Point]) -> Point:
    pts = list(points)
    if not pts:
        raise ValueError("cannot calculate centroid of an empty polygon")
    area = polygon_area(pts)
    if abs(area) <= 1e-9:
        return (sum(p[0] for p in pts) / len(pts), sum(p[1] for p in pts) / len(pts))
    cx = 0.0
    cy = 0.0
    for a, b in zip(pts, pts[1:] + pts[:1]):
        cross_value = (a[0] * b[1]) - (b[0] * a[1])
        cx += (a[0] + b[0]) * cross_value
        cy += (a[1] + b[1]) * cross_value
    factor = 1 / (6 * area)
    return (cx * factor, cy * factor)


def cross(a: Point, b: Point) -> float:
    return a[0] * b[1] - a[1] * b[0]


def ray_intersections(center: Point, direction: Point, loops: Sequence[Sequence[Point]]) -> list[float]:
    """Return positive ray distances where a ray intersects polygon loops."""

    hits: list[float] = []
    for loop in loops:
        pts = list(loop)
        for a, b in zip(pts, pts[1:] + pts[:1]):
            segment = (b[0] - a[0], b[1] - a[1])
            denom = cross(direction, segment)
            if abs(denom) <= 1e-9:
                continue
            offset = (a[0] - center[0], a[1] - center[1])
            t = cross(offset, segment) / denom
            u = cross(offset, direction) / denom
            if t > 1e-6 and -1e-6 <= u <= 1 + 1e-6:
                hits.append(t)
    hits.sort()
    deduped: list[float] = []
    for hit in hits:
        if not deduped or abs(hit - deduped[-1]) > 1e-3:
            deduped.append(hit)
    return deduped


def polyline_lengths(path_or_subpaths: str | Sequence[Sequence[Point]]) -> list[float]:
    subpaths = polyline_subpaths(path_or_subpaths, close_paths=False) if isinstance(path_or_subpaths, str) else path_or_subpaths
    return [
        sum(point_distance(a, b) for a, b in zip(points, points[1:]))
        for points in subpaths
        if len(points) >= 2
    ]


def turn_stats(path_or_subpaths: str | Sequence[Sequence[Point]], sharp_turn: float = 35.0) -> tuple[int, float, int]:
    """Return ``(segment_count, max_turn_degrees, sharp_turn_count)``."""

    subpaths = polyline_subpaths(path_or_subpaths, close_paths=False) if isinstance(path_or_subpaths, str) else path_or_subpaths
    segments = 0
    max_turn = 0.0
    sharp_turns = 0
    for points in subpaths:
        segments += max(0, len(points) - 1)
        for a, b, c in zip(points, points[1:], points[2:]):
            v1 = (b[0] - a[0], b[1] - a[1])
            v2 = (c[0] - b[0], c[1] - b[1])
            len1 = math.hypot(*v1)
            len2 = math.hypot(*v2)
            if len1 <= 1e-9 or len2 <= 1e-9:
                continue
            cos_theta = max(-1.0, min(1.0, (v1[0] * v2[0] + v1[1] * v2[1]) / (len1 * len2)))
            turn = math.degrees(math.acos(cos_theta))
            max_turn = max(max_turn, turn)
            if turn >= sharp_turn:
                sharp_turns += 1
    return segments, max_turn, sharp_turns


def stitch_subpaths(subpaths: Sequence[Sequence[Point]], max_gap: float) -> list[list[Point]]:
    """Join subpaths whose endpoints are within ``max_gap`` SVG units."""

    remaining = [_as_points(path) for path in subpaths if len(path) >= 2]
    stitched: list[list[Point]] = []
    while remaining:
        current = remaining.pop(0)
        changed = True
        while changed:
            changed = False
            best: tuple[float, int, str] | None = None
            for idx, candidate in enumerate(remaining):
                options = [
                    (point_distance(current[-1], candidate[0]), idx, "end-start"),
                    (point_distance(current[-1], candidate[-1]), idx, "end-end"),
                    (point_distance(current[0], candidate[-1]), idx, "start-end"),
                    (point_distance(current[0], candidate[0]), idx, "start-start"),
                ]
                local_best = min(options, key=lambda item: item[0])
                if local_best[0] <= max_gap and (best is None or local_best[0] < best[0]):
                    best = local_best
            if best is None:
                continue
            _distance, idx, mode = best
            other = remaining.pop(idx)
            if mode == "end-start":
                current.extend(other)
            elif mode == "end-end":
                current.extend(reversed(other))
            elif mode == "start-end":
                current = other + current
            else:
                current = list(reversed(other)) + current
            changed = True
        if point_distance(current[0], current[-1]) <= max_gap:
            current[-1] = current[0]
        stitched.append(current)
    return stitched


def serialize_polyline_subpaths(subpaths: Sequence[Sequence[Point]], decimals: int = 3) -> str:
    pieces: list[str] = []
    for points in subpaths:
        if len(points) < 2:
            continue
        parts = [f"M{fmt_number(points[0][0], decimals)} {fmt_number(points[0][1], decimals)}"]
        for point in points[1:]:
            parts.append(f"L{fmt_number(point[0], decimals)} {fmt_number(point[1], decimals)}")
        pieces.append(" ".join(parts))
    return " ".join(pieces)


def simplify_points(points: Sequence[Point], tolerance: float) -> list[Point]:
    """Simplify points with Douglas-Peucker using an absolute SVG-unit tolerance."""

    pts = _as_points(points)
    if len(pts) <= 2 or tolerance <= 0:
        return pts
    start = pts[0]
    end = pts[-1]
    max_distance = -1.0
    index = 0
    for i, point in enumerate(pts[1:-1], start=1):
        distance = point_line_distance(point, start, end)
        if distance > max_distance:
            max_distance = distance
            index = i
    if max_distance > tolerance:
        left = simplify_points(pts[: index + 1], tolerance)
        right = simplify_points(pts[index:], tolerance)
        return left[:-1] + right
    return [start, end]


def simplify_closed_points(points: Sequence[Point], tolerance: float) -> list[Point]:
    pts = _as_points(points)
    if len(pts) <= 4 or tolerance <= 0:
        return pts
    if point_distance(pts[0], pts[-1]) <= 1e-9:
        pts = pts[:-1]
    start_index = min(range(len(pts)), key=lambda i: (pts[i][1], pts[i][0]))
    rotated = pts[start_index:] + pts[:start_index] + [pts[start_index]]
    simplified = simplify_points(rotated, tolerance)
    if simplified and point_distance(simplified[0], simplified[-1]) <= 1e-6:
        simplified = simplified[:-1]
    return simplified if len(simplified) >= 3 else pts


def _bbox_size(points: Sequence[Point]) -> tuple[float, float]:
    if not points:
        return (0.0, 0.0)
    xs = [p[0] for p in points]
    ys = [p[1] for p in points]
    return (max(xs) - min(xs), max(ys) - min(ys))


def round_to(value: float, decimals: float = 3) -> float:
    """Round a number, including svg-path-simplify-style stepped decimals."""

    if decimals < 0:
        return float(value)
    if decimals == 0:
        return float(round(value))
    whole = math.floor(decimals)
    if whole != decimals:
        fraction = round(decimals - whole, 2)
        if fraction > 0.5:
            fraction = math.floor(fraction / 0.5) * 0.5
        step = (10 ** -whole) * max(fraction, 1e-12)
        return round(round(value / step) * step, 8)
    factor = 10**int(decimals)
    return round(value * factor) / factor


def auto_round(value: float, integer_threshold: float = 50) -> float:
    """Round using value-size-dependent accuracy inspired by svg-path-simplify."""

    magnitude = abs(value)
    if magnitude <= 1e-12:
        return 0.0
    if magnitude > integer_threshold * 2:
        decimals = 0
    elif magnitude > integer_threshold:
        decimals = 1
    else:
        decimals = len(str(math.ceil(500 / magnitude)))
    return round(value, decimals)


def detect_polyline_accuracy(points: Sequence[Point], threshold: float = 75.0, max_decimals: int = 8) -> int:
    """Suggest decimal precision from short segment sizes."""

    pts = _as_points(points)
    dims = [abs(b[0] - a[0]) + abs(b[1] - a[1]) for a, b in zip(pts, pts[1:]) if point_distance(a, b) > 1e-12]
    if not dims:
        return 0
    dims.sort()
    quarter = max(1, math.ceil(len(dims) * 0.25))
    low_average = sum(dims[:quarter]) / quarter
    mid = dims[len(dims) // 2]
    dim = (low_average + mid) * 0.5
    if dim <= 1e-12:
        return max_decimals
    decimals = 0 if dim > threshold * 1.5 else len(str(math.floor(threshold / dim)))
    return min(max(0, decimals), max_decimals)


def _quality_to_tolerance_squared(points: Sequence[Point], quality: float | str, width: float, height: float, *, divisor: float) -> tuple[float, bool]:
    absolute = isinstance(quality, str)
    value = float(quality)
    if absolute:
        return (value * value, True)
    if value >= 1:
        return (0.0, False)
    if width <= 0 and height <= 0:
        width, height = _bbox_size(points)
    tolerance = 1 - value
    if value > 0.5:
        tolerance /= 2
    scale = ((width + height) / 2) / divisor if width > 0 or height > 0 else 1.0
    return ((tolerance * scale) ** 2, False)


def simplify_radial_distance(points: Sequence[Point], quality: float | str = 0.9, width: float = 0, height: float = 0) -> list[Point]:
    """Fast radial-distance point reduction inspired by svg-path-simplify."""

    pts = _as_points(points)
    if len(pts) < 4:
        return pts
    tolerance_sq, absolute = _quality_to_tolerance_squared(pts, quality, width, height, divisor=25)
    if absolute and tolerance_sq <= 0:
        return pts
    if not absolute and tolerance_sq <= 0:
        return pts
    previous = pts[0]
    simplified = [previous]
    last = pts[-1]
    for point in pts[1:]:
        dist_sq = (point[0] - previous[0]) ** 2 + (point[1] - previous[1]) ** 2
        if dist_sq > tolerance_sq:
            simplified.append(point)
            previous = point
    if simplified[-1] != last:
        simplified.append(last)
    return simplified


def simplify_rdp(
    points: Sequence[Point],
    quality: float | str = 0.9,
    width: float = 0,
    height: float = 0,
    preserve_indices: Iterable[int] = (),
) -> list[Point]:
    """Quality-based Ramer-Douglas-Peucker reduction inspired by svg-path-simplify.

    Pass ``quality`` as a string, for example ``"2.5"``, to use an absolute
    SVG-unit tolerance.
    """

    pts = _as_points(points)
    if len(pts) < 4:
        return pts
    tolerance_sq, absolute = _quality_to_tolerance_squared(pts, quality, width, height, divisor=100)
    if absolute and tolerance_sq <= 0:
        return pts
    if not absolute and float(quality) >= 1:
        return pts
    preserved = set(preserve_indices)
    kept = [False] * len(pts)
    kept[0] = True
    kept[-1] = True
    stack = [(0, len(pts) - 1)]
    while stack:
        first, last = stack.pop()
        forced = next((idx for idx in range(first + 1, last) if idx in preserved), -1)
        if forced != -1:
            kept[forced] = True
            stack.append((forced, last))
            stack.append((first, forced))
            continue
        max_distance_sq = tolerance_sq
        index = -1
        for idx in range(first + 1, last):
            distance = point_line_distance(pts[idx], pts[first], pts[last])
            distance_sq = distance * distance
            if distance_sq > max_distance_sq:
                index = idx
                max_distance_sq = distance_sq
        if index != -1:
            kept[index] = True
            stack.append((index, last))
            stack.append((first, index))
    return [point for point, keep in zip(pts, kept) if keep]


def remove_collinear_points(points: Sequence[Point], tolerance: float = 0.0, *, closed: bool = False) -> list[Point]:
    """Remove zero-length and nearly collinear vertices from a polyline/polygon."""

    pts = _as_points(points)
    if len(pts) < 3:
        return pts
    if point_distance(pts[0], pts[-1]) <= 1e-9:
        pts = pts[:-1]
        closed = True
    if len(pts) < 3:
        return pts

    def is_flat(prev: Point, point: Point, next_point: Point) -> bool:
        return point_distance(prev, point) <= 1e-9 or point_distance(point, next_point) <= 1e-9 or point_line_distance(point, prev, next_point) <= tolerance

    if not closed:
        simplified = [pts[0]]
        for idx in range(1, len(pts) - 1):
            if not is_flat(simplified[-1], pts[idx], pts[idx + 1]):
                simplified.append(pts[idx])
        simplified.append(pts[-1])
        return simplified

    simplified = []
    for idx, point in enumerate(pts):
        prev = pts[idx - 1]
        next_point = pts[(idx + 1) % len(pts)]
        if not is_flat(prev, point, next_point):
            simplified.append(point)
    return simplified if len(simplified) >= 3 else pts


def serialize_smooth_closed(points: Sequence[Point], decimals: int = 3) -> str:
    """Serialize a closed Catmull-Rom-style smooth cubic path through points."""

    pts = _as_points(points)
    if len(pts) < 3:
        return serialize_polyline_subpaths([pts], decimals)
    parts = [f"M{fmt_number(pts[0][0], decimals)} {fmt_number(pts[0][1], decimals)}"]
    count = len(pts)
    for i in range(count):
        p0 = pts[(i - 1) % count]
        p1 = pts[i]
        p2 = pts[(i + 1) % count]
        p3 = pts[(i + 2) % count]
        c1 = (p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6)
        c2 = (p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6)
        parts.append(
            "C"
            + " ".join(
                [
                    fmt_number(c1[0], decimals),
                    fmt_number(c1[1], decimals),
                    fmt_number(c2[0], decimals),
                    fmt_number(c2[1], decimals),
                    fmt_number(p2[0], decimals),
                    fmt_number(p2[1], decimals),
                ]
            )
        )
    parts.append("Z")
    return "".join(parts)


def radial_centerline_candidate(
    path_data: str,
    *,
    samples: int = 160,
    simplify: float = 2.0,
    decimals: int = 3,
    fallback_stroke_width: float = 31.0,
) -> tuple[str, float] | None:
    """Build a smooth centerline candidate for two-loop radial closed outlines."""

    loops = filled_loops(path_data)
    if len(loops) < 2:
        return None
    loops = sorted(loops, key=lambda loop: abs(polygon_area(loop)), reverse=True)
    inner = loops[1]
    center = polygon_centroid(inner)
    points: list[Point] = []
    widths: list[float] = []
    for step in range(samples):
        angle = 2 * math.pi * step / samples
        direction = (math.cos(angle), math.sin(angle))
        hits = ray_intersections(center, direction, loops[:2])
        if len(hits) < 2:
            continue
        inner_t = hits[0]
        outer_t = hits[-1]
        if outer_t <= inner_t:
            continue
        mid = (inner_t + outer_t) / 2
        points.append((center[0] + direction[0] * mid, center[1] + direction[1] * mid))
        widths.append(outer_t - inner_t)
    if len(points) < 8:
        return None
    points = simplify_closed_points(points, simplify)
    d = optimize_path_data(serialize_smooth_closed(points, decimals), decimals)
    return (d, median(widths) if widths else fallback_stroke_width)
