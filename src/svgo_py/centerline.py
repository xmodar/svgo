"""Filled-outline to stroked-centerline reconstruction."""

from __future__ import annotations

import math
import re
import xml.etree.ElementTree as ET
from collections import deque
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

from .pathdata import Point

try:  # Optional acceleration; the pure-Python path is always available.
    import numpy as _np
except Exception:  # pragma: no cover - depends on optional dependency.
    _np = None

COMMAND_RE = re.compile(
    r"[AaCcHhLlMmQqSsTtVvZz]|[-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?"
)
PATH_ATTR_RE = re.compile(r"\bd\s*=\s*([\"'])([\s\S]*?)\1")
NEIGHBORS = [(-1, 0), (-1, 1), (0, 1), (1, 1), (1, 0), (1, -1), (0, -1), (-1, -1)]


class CenterlineError(ValueError):
    """Raised when centerline reconstruction cannot continue."""


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


def read_path_data(path: str | Path) -> str:
    text = Path(path).read_text(encoding="utf-8").strip()
    match = PATH_ATTR_RE.search(text)
    if match:
        return match.group(2).strip()
    return text


def is_command(token: str) -> bool:
    return len(token) == 1 and token.isalpha()


def as_float(token: str) -> float:
    try:
        return float(token)
    except ValueError as exc:
        raise CenterlineError(f"Invalid numeric token: {token}") from exc


def cubic_point(p0: Point, p1: Point, p2: Point, p3: Point, t: float) -> Point:
    mt = 1.0 - t
    return Point(
        mt**3 * p0.x + 3 * mt**2 * t * p1.x + 3 * mt * t**2 * p2.x + t**3 * p3.x,
        mt**3 * p0.y + 3 * mt**2 * t * p1.y + 3 * mt * t**2 * p2.y + t**3 * p3.y,
    )


def quad_point(p0: Point, p1: Point, p2: Point, t: float) -> Point:
    mt = 1.0 - t
    return Point(mt**2 * p0.x + 2 * mt * t * p1.x + t**2 * p2.x, mt**2 * p0.y + 2 * mt * t * p1.y + t**2 * p2.y)


def angle_between(ux: float, uy: float, vx: float, vy: float) -> float:
    return math.atan2(ux * vy - uy * vx, ux * vx + uy * vy)


def distance(a: Point, b: Point) -> float:
    return math.hypot(a.x - b.x, a.y - b.y)


def arc_points(
    p0: Point,
    rx: float,
    ry: float,
    x_axis_rotation: float,
    large_arc: int,
    sweep: int,
    p1: Point,
    curve_samples: int,
) -> list[Point]:
    rx = abs(rx)
    ry = abs(ry)
    if rx == 0 or ry == 0 or distance(p0, p1) == 0:
        return [p1]

    phi = math.radians(x_axis_rotation % 360.0)
    cos_phi = math.cos(phi)
    sin_phi = math.sin(phi)
    dx = (p0.x - p1.x) / 2.0
    dy = (p0.y - p1.y) / 2.0
    x1p = cos_phi * dx + sin_phi * dy
    y1p = -sin_phi * dx + cos_phi * dy

    radius_check = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry)
    if radius_check > 1:
        scale = math.sqrt(radius_check)
        rx *= scale
        ry *= scale

    sign = -1.0 if large_arc == sweep else 1.0
    numerator = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p
    denominator = rx * rx * y1p * y1p + ry * ry * x1p * x1p
    factor = 0.0 if denominator == 0 else sign * math.sqrt(max(0.0, numerator / denominator))
    cxp = factor * (rx * y1p / ry)
    cyp = factor * (-ry * x1p / rx)
    cx = cos_phi * cxp - sin_phi * cyp + (p0.x + p1.x) / 2.0
    cy = sin_phi * cxp + cos_phi * cyp + (p0.y + p1.y) / 2.0

    ux = (x1p - cxp) / rx
    uy = (y1p - cyp) / ry
    vx = (-x1p - cxp) / rx
    vy = (-y1p - cyp) / ry
    theta1 = angle_between(1.0, 0.0, ux, uy)
    delta = angle_between(ux, uy, vx, vy)
    if not sweep and delta > 0:
        delta -= math.tau
    elif sweep and delta < 0:
        delta += math.tau

    approx_length = max(rx, ry) * abs(delta)
    n = max(4, min(240, int(math.ceil(approx_length / 8.0)), curve_samples * 4))
    points: list[Point] = []
    for step in range(1, n + 1):
        theta = theta1 + delta * (step / n)
        x = cos_phi * rx * math.cos(theta) - sin_phi * ry * math.sin(theta) + cx
        y = sin_phi * rx * math.cos(theta) + cos_phi * ry * math.sin(theta) + cy
        points.append(Point(x, y))
    return points


def sample_count(points: Iterable[Point], samples: int) -> int:
    pts = list(points)
    length = sum(distance(pts[i], pts[i + 1]) for i in range(len(pts) - 1))
    return max(4, min(160, int(math.ceil(length / 8.0)), samples))


def append_line(subpath: list[Point], point: Point) -> None:
    if not subpath or distance(subpath[-1], point) > 1e-9:
        subpath.append(point)


def flatten_path(path_data: str, curve_samples: int) -> list[list[Point]]:
    tokens = COMMAND_RE.findall(path_data)
    if not tokens:
        raise CenterlineError("No SVG path tokens found")

    subpaths: list[list[Point]] = []
    subpath: list[Point] = []
    i = 0
    command = ""
    current = Point(0.0, 0.0)
    start = Point(0.0, 0.0)
    last_cubic_ctrl: Point | None = None
    last_quad_ctrl: Point | None = None

    def has_numbers(count: int) -> bool:
        return i + count <= len(tokens) and all(not is_command(tokens[i + j]) for j in range(count))

    def read_point(relative: bool) -> Point:
        nonlocal i, current
        x = as_float(tokens[i])
        y = as_float(tokens[i + 1])
        i += 2
        return Point(current.x + x, current.y + y) if relative else Point(x, y)

    def read_number(label: str) -> float:
        nonlocal i
        if i >= len(tokens) or is_command(tokens[i]):
            raise CenterlineError(f"Arc command requires {label}")
        value = as_float(tokens[i])
        i += 1
        return value

    def read_arc_flag(label: str) -> int:
        nonlocal i
        if i >= len(tokens) or is_command(tokens[i]):
            raise CenterlineError(f"Arc command requires {label}")
        token = tokens[i]
        if token in ("0", "1"):
            i += 1
            return int(token)
        if token[0] in ("0", "1"):
            flag = int(token[0])
            tokens[i] = token[1:]
            if not tokens[i]:
                i += 1
            return flag
        raise CenterlineError(f"Arc command {label} must be 0 or 1")

    while i < len(tokens):
        if is_command(tokens[i]):
            command = tokens[i]
            i += 1
        elif not command:
            raise CenterlineError("Path data must begin with a command")

        upper = command.upper()
        relative = command.islower()

        if upper == "M":
            if not has_numbers(2):
                raise CenterlineError("M command requires x y")
            if subpath:
                subpaths.append(subpath)
            subpath = []
            current = read_point(relative)
            start = current
            append_line(subpath, current)
            while has_numbers(2):
                current = read_point(relative)
                append_line(subpath, current)
            command = "l" if relative else "L"
            last_cubic_ctrl = None
            last_quad_ctrl = None
        elif upper == "L":
            while has_numbers(2):
                current = read_point(relative)
                append_line(subpath, current)
            last_cubic_ctrl = None
            last_quad_ctrl = None
        elif upper == "H":
            while has_numbers(1):
                x = as_float(tokens[i])
                i += 1
                current = Point(current.x + x, current.y) if relative else Point(x, current.y)
                append_line(subpath, current)
            last_cubic_ctrl = None
            last_quad_ctrl = None
        elif upper == "V":
            while has_numbers(1):
                y = as_float(tokens[i])
                i += 1
                current = Point(current.x, current.y + y) if relative else Point(current.x, y)
                append_line(subpath, current)
            last_cubic_ctrl = None
            last_quad_ctrl = None
        elif upper == "C":
            while has_numbers(6):
                p0 = current
                p1 = read_point(relative)
                p2 = read_point(relative)
                p3 = read_point(relative)
                n = sample_count((p0, p1, p2, p3), curve_samples)
                for step in range(1, n + 1):
                    append_line(subpath, cubic_point(p0, p1, p2, p3, step / n))
                current = p3
                last_cubic_ctrl = p2
                last_quad_ctrl = None
        elif upper == "S":
            while has_numbers(4):
                p0 = current
                p1 = Point(2 * current.x - last_cubic_ctrl.x, 2 * current.y - last_cubic_ctrl.y) if last_cubic_ctrl else current
                p2 = read_point(relative)
                p3 = read_point(relative)
                n = sample_count((p0, p1, p2, p3), curve_samples)
                for step in range(1, n + 1):
                    append_line(subpath, cubic_point(p0, p1, p2, p3, step / n))
                current = p3
                last_cubic_ctrl = p2
                last_quad_ctrl = None
        elif upper == "Q":
            while has_numbers(4):
                p0 = current
                p1 = read_point(relative)
                p2 = read_point(relative)
                n = sample_count((p0, p1, p2), curve_samples)
                for step in range(1, n + 1):
                    append_line(subpath, quad_point(p0, p1, p2, step / n))
                current = p2
                last_quad_ctrl = p1
                last_cubic_ctrl = None
        elif upper == "T":
            while has_numbers(2):
                p0 = current
                p1 = Point(2 * current.x - last_quad_ctrl.x, 2 * current.y - last_quad_ctrl.y) if last_quad_ctrl else current
                p2 = read_point(relative)
                n = sample_count((p0, p1, p2), curve_samples)
                for step in range(1, n + 1):
                    append_line(subpath, quad_point(p0, p1, p2, step / n))
                current = p2
                last_quad_ctrl = p1
                last_cubic_ctrl = None
        elif upper == "Z":
            append_line(subpath, start)
            current = start
            last_cubic_ctrl = None
            last_quad_ctrl = None
        elif upper == "A":
            while i < len(tokens) and not is_command(tokens[i]):
                p0 = current
                rx = read_number("rx")
                ry = read_number("ry")
                rotation = read_number("x-axis-rotation")
                large_arc = read_arc_flag("large-arc-flag")
                sweep = read_arc_flag("sweep-flag")
                endpoint = read_point(relative)
                for point in arc_points(p0, rx, ry, rotation, large_arc, sweep, endpoint, curve_samples):
                    append_line(subpath, point)
                current = endpoint
            last_cubic_ctrl = None
            last_quad_ctrl = None
        else:
            raise CenterlineError(f"Unsupported path command: {command}")

    if subpath:
        subpaths.append(subpath)
    if not subpaths:
        raise CenterlineError("Path did not produce any drawable subpaths")
    return subpaths


def bounds(subpaths: list[list[Point]]) -> tuple[float, float, float, float]:
    xs = [p.x for subpath in subpaths for p in subpath]
    ys = [p.y for subpath in subpaths for p in subpath]
    return min(xs), min(ys), max(xs), max(ys)


def make_context(subpaths: list[list[Point]], scale: float, max_size: int) -> RasterContext:
    min_x, min_y, max_x, max_y = bounds(subpaths)
    if scale <= 0:
        raise CenterlineError("--scale must be greater than zero")
    svg_width = max(max_x - min_x, 1.0)
    svg_height = max(max_y - min_y, 1.0)
    if max_size > 0:
        scale = min(scale, max_size / max(svg_width, svg_height))
    pad = max(8, int(math.ceil(8 * scale)))
    width = int(math.ceil(svg_width * scale)) + pad * 2 + 2
    height = int(math.ceil(svg_height * scale)) + pad * 2 + 2
    return RasterContext(min_x=min_x, min_y=min_y, scale=scale, pad=pad, width=width, height=height)


def to_pixel(point: Point, ctx: RasterContext) -> tuple[float, float]:
    return ((point.x - ctx.min_x) * ctx.scale + ctx.pad, (point.y - ctx.min_y) * ctx.scale + ctx.pad)


def to_svg(pixel: tuple[int, int], ctx: RasterContext) -> Point:
    row, col = pixel
    return Point((col + 0.5 - ctx.pad) / ctx.scale + ctx.min_x, (row + 0.5 - ctx.pad) / ctx.scale + ctx.min_y)


def rasterize(subpaths: list[list[Point]], ctx: RasterContext) -> set[tuple[int, int]]:
    pixel_paths = [[to_pixel(p, ctx) for p in subpath] for subpath in subpaths if len(subpath) >= 2]
    filled: set[tuple[int, int]] = set()
    for row in range(ctx.height):
        y = row + 0.5
        intersections: list[float] = []
        for points in pixel_paths:
            for (x1, y1), (x2, y2) in zip(points, points[1:]):
                if y1 == y2:
                    continue
                if (y1 <= y < y2) or (y2 <= y < y1):
                    t = (y - y1) / (y2 - y1)
                    intersections.append(x1 + t * (x2 - x1))
        intersections.sort()
        for left, right in zip(intersections[0::2], intersections[1::2]):
            start = max(0, int(math.ceil(left - 0.5)))
            end = min(ctx.width - 1, int(math.floor(right - 0.5)))
            for col in range(start, end + 1):
                filled.add((row, col))
    if not filled:
        raise CenterlineError("Rasterization produced an empty mask")
    return filled


def zhang_suen_thin(mask: set[tuple[int, int]], height: int, width: int) -> set[tuple[int, int]]:
    foreground = set(mask)

    def values(pixel: tuple[int, int]) -> list[int]:
        row, col = pixel
        return [1 if (row + dr, col + dc) in foreground else 0 for dr, dc in NEIGHBORS]

    changed = True
    while changed:
        changed = False
        for step in (0, 1):
            remove: list[tuple[int, int]] = []
            for row, col in foreground:
                if row <= 0 or col <= 0 or row >= height - 1 or col >= width - 1:
                    continue
                p2, p3, p4, p5, p6, p7, p8, p9 = values((row, col))
                b = p2 + p3 + p4 + p5 + p6 + p7 + p8 + p9
                if b < 2 or b > 6:
                    continue
                seq = [p2, p3, p4, p5, p6, p7, p8, p9, p2]
                a = sum(1 for idx in range(8) if seq[idx] == 0 and seq[idx + 1] == 1)
                if a != 1:
                    continue
                if step == 0:
                    if p2 * p4 * p6 != 0 or p4 * p6 * p8 != 0:
                        continue
                else:
                    if p2 * p4 * p8 != 0 or p2 * p6 * p8 != 0:
                        continue
                remove.append((row, col))
            if remove:
                foreground.difference_update(remove)
                changed = True
    if not foreground:
        raise CenterlineError("Skeletonization produced an empty centerline")
    return foreground


def chamfer_distance(mask: set[tuple[int, int]], height: int, width: int) -> dict[tuple[int, int], float]:
    if _np is not None and height * width >= 4096:
        return chamfer_distance_numpy(mask, height, width)
    return chamfer_distance_python(mask, height, width)


def chamfer_distance_numpy(mask: set[tuple[int, int]], height: int, width: int) -> dict[tuple[int, int], float]:
    inf = 1e12
    dist = _np.zeros((height, width), dtype=float)
    if mask:
        rows, cols = zip(*mask)
        dist[list(rows), list(cols)] = inf

    root2 = math.sqrt(2.0)
    for row in range(height):
        for col in range(width):
            if dist[row, col] == 0:
                continue
            best = dist[row, col]
            if row > 0:
                best = min(best, dist[row - 1, col] + 1.0)
                if col > 0:
                    best = min(best, dist[row - 1, col - 1] + root2)
                if col + 1 < width:
                    best = min(best, dist[row - 1, col + 1] + root2)
            if col > 0:
                best = min(best, dist[row, col - 1] + 1.0)
            dist[row, col] = best

    for row in range(height - 1, -1, -1):
        for col in range(width - 1, -1, -1):
            if dist[row, col] == 0:
                continue
            best = dist[row, col]
            if row + 1 < height:
                best = min(best, dist[row + 1, col] + 1.0)
                if col > 0:
                    best = min(best, dist[row + 1, col - 1] + root2)
                if col + 1 < width:
                    best = min(best, dist[row + 1, col + 1] + root2)
            if col + 1 < width:
                best = min(best, dist[row, col + 1] + 1.0)
            dist[row, col] = best
    return {pixel: float(dist[pixel[0], pixel[1]]) for pixel in mask}


def chamfer_distance_python(mask: set[tuple[int, int]], height: int, width: int) -> dict[tuple[int, int], float]:
    inf = 1e12
    dist = [[0.0] * width for _ in range(height)]
    for row in range(height):
        for col in range(width):
            dist[row][col] = inf if (row, col) in mask else 0.0

    root2 = math.sqrt(2.0)
    for row in range(height):
        for col in range(width):
            if dist[row][col] == 0:
                continue
            best = dist[row][col]
            if row > 0:
                best = min(best, dist[row - 1][col] + 1.0)
                if col > 0:
                    best = min(best, dist[row - 1][col - 1] + root2)
                if col + 1 < width:
                    best = min(best, dist[row - 1][col + 1] + root2)
            if col > 0:
                best = min(best, dist[row][col - 1] + 1.0)
            dist[row][col] = best

    for row in range(height - 1, -1, -1):
        for col in range(width - 1, -1, -1):
            if dist[row][col] == 0:
                continue
            best = dist[row][col]
            if row + 1 < height:
                best = min(best, dist[row + 1][col] + 1.0)
                if col > 0:
                    best = min(best, dist[row + 1][col - 1] + root2)
                if col + 1 < width:
                    best = min(best, dist[row + 1][col + 1] + root2)
            if col + 1 < width:
                best = min(best, dist[row][col + 1] + 1.0)
            dist[row][col] = best

    return {pixel: dist[pixel[0]][pixel[1]] for pixel in mask}


def skeleton_neighbors(pixel: tuple[int, int], skeleton: set[tuple[int, int]]) -> list[tuple[int, int]]:
    row, col = pixel
    return [(row + dr, col + dc) for dr, dc in NEIGHBORS if (row + dr, col + dc) in skeleton]


def connected_components(skeleton: set[tuple[int, int]]) -> list[set[tuple[int, int]]]:
    remaining = set(skeleton)
    components: list[set[tuple[int, int]]] = []
    while remaining:
        first = remaining.pop()
        component = {first}
        queue = deque([first])
        while queue:
            pixel = queue.popleft()
            for neighbor in skeleton_neighbors(pixel, skeleton):
                if neighbor in remaining:
                    remaining.remove(neighbor)
                    component.add(neighbor)
                    queue.append(neighbor)
        components.append(component)
    return components


def farthest_path(component: set[tuple[int, int]], start: tuple[int, int]) -> list[tuple[int, int]]:
    def bfs(origin: tuple[int, int]) -> tuple[tuple[int, int], dict[tuple[int, int], tuple[int, int] | None]]:
        parents: dict[tuple[int, int], tuple[int, int] | None] = {origin: None}
        queue = deque([origin])
        last = origin
        while queue:
            pixel = queue.popleft()
            last = pixel
            for neighbor in skeleton_neighbors(pixel, component):
                if neighbor not in parents:
                    parents[neighbor] = pixel
                    queue.append(neighbor)
        return last, parents

    a, _ = bfs(start)
    b, parents = bfs(a)
    path = [b]
    while parents[path[-1]] is not None:
        path.append(parents[path[-1]])  # type: ignore[arg-type]
    path.reverse()
    return path


def trace_skeleton(skeleton: set[tuple[int, int]], mode: str, min_length_px: float) -> list[list[tuple[int, int]]]:
    paths: list[list[tuple[int, int]]] = []
    for component in connected_components(skeleton):
        endpoints = [pixel for pixel in component if len(skeleton_neighbors(pixel, component)) <= 1]
        starts = endpoints or [next(iter(component))]
        if mode == "longest":
            paths.append(farthest_path(component, starts[0]))
        else:
            used_edges: set[frozenset[tuple[int, int]]] = set()
            nodes = {pixel for pixel in component if len(skeleton_neighbors(pixel, component)) != 2}
            if not nodes:
                paths.append(farthest_path(component, starts[0]))
                continue
            for node in nodes:
                for neighbor in skeleton_neighbors(node, component):
                    edge = frozenset((node, neighbor))
                    if edge in used_edges:
                        continue
                    chain = [node, neighbor]
                    used_edges.add(edge)
                    prev, current = node, neighbor
                    while current not in nodes:
                        candidates = [p for p in skeleton_neighbors(current, component) if p != prev]
                        if not candidates:
                            break
                        nxt = candidates[0]
                        used_edges.add(frozenset((current, nxt)))
                        chain.append(nxt)
                        prev, current = current, nxt
                    paths.append(chain)

    filtered = [path for path in paths if pixel_path_length(path) >= min_length_px]
    if not filtered:
        filtered = sorted(paths, key=pixel_path_length, reverse=True)[:1]
    if mode == "longest":
        return sorted(filtered, key=pixel_path_length, reverse=True)[:1]
    return sorted(filtered, key=pixel_path_length, reverse=True)


def pixel_path_length(path: list[tuple[int, int]]) -> float:
    return sum(math.hypot(path[i + 1][0] - path[i][0], path[i + 1][1] - path[i][1]) for i in range(len(path) - 1))


def point_line_distance(point: Point, start: Point, end: Point) -> float:
    vx = end.x - start.x
    vy = end.y - start.y
    wx = point.x - start.x
    wy = point.y - start.y
    denom = vx * vx + vy * vy
    if denom == 0:
        return math.hypot(point.x - start.x, point.y - start.y)
    t = max(0.0, min(1.0, (wx * vx + wy * vy) / denom))
    px = start.x + t * vx
    py = start.y + t * vy
    return math.hypot(point.x - px, point.y - py)


def simplify_points(points: list[Point], tolerance: float) -> list[Point]:
    if len(points) <= 2 or tolerance <= 0:
        return points
    start = points[0]
    end = points[-1]
    max_distance = -1.0
    index = 0
    for i in range(1, len(points) - 1):
        dist = point_line_distance(points[i], start, end)
        if dist > max_distance:
            max_distance = dist
            index = i
    if max_distance > tolerance:
        left = simplify_points(points[: index + 1], tolerance)
        right = simplify_points(points[index:], tolerance)
        return left[:-1] + right
    return [start, end]


def fmt(value: float, decimals: int) -> str:
    rounded = round(value, decimals)
    text = f"{rounded:.{decimals}f}".rstrip("0").rstrip(".")
    return text if text and text != "-0" else "0"


def serialize_polyline(points: list[Point], decimals: int) -> str:
    if not points:
        return ""
    parts = [f"M{fmt(points[0].x, decimals)} {fmt(points[0].y, decimals)}"]
    for point in points[1:]:
        parts.append(f"L{fmt(point.x, decimals)} {fmt(point.y, decimals)}")
    return " ".join(parts)


def serialize_smooth(points: list[Point], decimals: int) -> str:
    if len(points) < 3:
        return serialize_polyline(points, decimals)
    parts = [f"M{fmt(points[0].x, decimals)} {fmt(points[0].y, decimals)}"]
    for i in range(len(points) - 1):
        p0 = points[i - 1] if i > 0 else points[i]
        p1 = points[i]
        p2 = points[i + 1]
        p3 = points[i + 2] if i + 2 < len(points) else p2
        c1 = Point(p1.x + (p2.x - p0.x) / 6.0, p1.y + (p2.y - p0.y) / 6.0)
        c2 = Point(p2.x - (p3.x - p1.x) / 6.0, p2.y - (p3.y - p1.y) / 6.0)
        parts.append(
            "C"
            f"{fmt(c1.x, decimals)} {fmt(c1.y, decimals)} "
            f"{fmt(c2.x, decimals)} {fmt(c2.y, decimals)} "
            f"{fmt(p2.x, decimals)} {fmt(p2.y, decimals)}"
        )
    return " ".join(parts)


def estimate_stroke_width(stroke_width: str, skeleton: set[tuple[int, int]], distances: dict[tuple[int, int], float], scale: float) -> float:
    if stroke_width != "auto":
        try:
            width = float(stroke_width)
        except ValueError as exc:
            raise CenterlineError("--stroke-width must be a number or 'auto'") from exc
        if width <= 0:
            raise CenterlineError("--stroke-width must be greater than zero")
        return width

    values = sorted(distances[pixel] / scale for pixel in skeleton if distances.get(pixel, 0) > 0)
    if not values:
        raise CenterlineError("Could not estimate stroke width from skeleton distances")
    lo = int(len(values) * 0.15)
    hi = max(lo + 1, int(len(values) * 0.85))
    core = values[lo:hi]
    radius = core[len(core) // 2]
    return radius * 2.0


def escape_attr(value: str) -> str:
    return value.replace("&", "&amp;").replace('"', "&quot;").replace("<", "&lt;").replace(">", "&gt;")


def parse_style(style: str | None) -> dict[str, str]:
    values: dict[str, str] = {}
    if not style:
        return values
    for part in style.split(";"):
        if ":" not in part:
            continue
        key, value = part.split(":", 1)
        values[key.strip()] = value.strip()
    return values


def style_or_attr(element: ET.Element, name: str, default: str | None = None) -> str | None:
    style = parse_style(element.attrib.get("style"))
    return style.get(name, element.attrib.get(name, default))


def local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def inherited_graphics(element: ET.Element, inherited: dict[str, str]) -> dict[str, str]:
    current = dict(inherited)
    style = parse_style(element.attrib.get("style"))
    for name in ("fill", "stroke", "opacity", "fill-opacity", "stroke-opacity"):
        value = style.get(name, element.attrib.get(name))
        if value is not None:
            current[name] = value
    return current


def centerline_path_data(path_data: str, options: CenterlineOptions | None = None) -> tuple[str, float, RasterContext]:
    options = options or CenterlineOptions()
    if options.mode not in {"longest", "all"}:
        raise CenterlineError("--mode must be longest or all")
    if options.fill_rule != "evenodd":
        raise CenterlineError("Only evenodd fill rule is supported")

    subpaths = flatten_path(path_data, options.curve_samples)
    ctx = make_context(subpaths, options.scale, options.max_size)
    mask = rasterize(subpaths, ctx)
    skeleton = zhang_suen_thin(mask, ctx.height, ctx.width)
    distances = chamfer_distance(mask, ctx.height, ctx.width)
    stroke_width = estimate_stroke_width(options.stroke_width, skeleton, distances, ctx.scale)

    min_length_px = max(0.0, options.min_length * ctx.scale)
    pixel_paths = trace_skeleton(skeleton, options.mode, min_length_px)
    svg_paths: list[str] = []
    for pixel_path in pixel_paths:
        points = [to_svg(pixel, ctx) for pixel in pixel_path]
        points = simplify_points(points, max(0.0, options.simplify))
        if len(points) < 2:
            continue
        svg_paths.append(serialize_polyline(points, options.decimals) if options.polyline else serialize_smooth(points, options.decimals))

    if not svg_paths:
        raise CenterlineError("No centerline paths survived simplification")
    return " ".join(svg_paths), stroke_width, ctx


def source_stroke_color(element: ET.Element, inherited: dict[str, str]) -> str:
    stroke = style_or_attr(element, "stroke", inherited.get("stroke"))
    if stroke and stroke != "none":
        return stroke
    fill = style_or_attr(element, "fill", inherited.get("fill"))
    if fill and fill != "none":
        return fill
    return "currentColor"


def source_opacity(element: ET.Element, inherited: dict[str, str]) -> str | None:
    stroke_opacity = style_or_attr(element, "stroke-opacity", inherited.get("stroke-opacity"))
    if stroke_opacity is not None:
        return stroke_opacity
    fill_opacity = style_or_attr(element, "fill-opacity", inherited.get("fill-opacity"))
    if fill_opacity is not None:
        return fill_opacity
    return style_or_attr(element, "opacity", inherited.get("opacity"))


def svg_attrs(root: ET.Element) -> str:
    attrs = ['xmlns="http://www.w3.org/2000/svg"']
    for name in ("viewBox", "width", "height"):
        value = root.attrib.get(name)
        if value is not None:
            attrs.append(f'{name}="{escape_attr(value)}"')
    return " ".join(attrs)


def centerline_svg_text(svg_text: str, options: CenterlineOptions | None = None) -> str:
    options = options or CenterlineOptions(svg_paths="all")
    try:
        root = ET.fromstring(svg_text)
    except ET.ParseError as error:
        raise CenterlineError(f"Could not parse SVG input: {error}") from error

    output_paths: list[str] = []

    def walk(element: ET.Element, inherited: dict[str, str]) -> None:
        current = inherited_graphics(element, inherited)
        if local_name(element.tag) == "path" and element.attrib.get("d"):
            d = element.attrib["d"]
            try:
                center_d, stroke_width, _ctx = centerline_path_data(d, options)
            except CenterlineError:
                if not options.keep_failed:
                    raise
                output_paths.append(f'<path d="{escape_attr(d)}" fill="{escape_attr(current.get("fill", "black"))}"/>')
                return
            color = source_stroke_color(element, current)
            opacity = source_opacity(element, current)
            style = (
                "fill: none; "
                f"stroke: {color}; "
                f"stroke-linecap: {options.linecap}; "
                f"stroke-width: {fmt(stroke_width, options.decimals)}px; "
                f"stroke-linejoin: {options.linejoin};"
            )
            if opacity is not None:
                style += f" stroke-opacity: {opacity};"
            output_paths.append(f'<path style="{escape_attr(style)}" d="{escape_attr(center_d)}"/>')
        for child in list(element):
            walk(child, current)

    walk(root, {})
    if not output_paths:
        raise CenterlineError("No path elements found in SVG input")
    return f'<svg {svg_attrs(root)}>\n  ' + "\n  ".join(output_paths) + "\n</svg>"


def build_output(d: str, emit: str, stroke_width: float, options: CenterlineOptions, ctx: RasterContext) -> str:
    if emit == "d":
        return d
    style = (
        "fill: none; "
        f"stroke-linecap: {options.linecap}; "
        f"stroke-width: {fmt(stroke_width, options.decimals)}px; "
        f"stroke-linejoin: {options.linejoin};"
    )
    path = f'<path style="{style}" d="{escape_attr(d)}"/>'
    if emit == "path":
        return path
    min_x = ctx.min_x
    min_y = ctx.min_y
    width = (ctx.width - ctx.pad * 2) / ctx.scale
    height = (ctx.height - ctx.pad * 2) / ctx.scale
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" '
        f'viewBox="{fmt(min_x, options.decimals)} {fmt(min_y, options.decimals)} {fmt(width, options.decimals)} {fmt(height, options.decimals)}">'
        f"{path}</svg>"
    )
