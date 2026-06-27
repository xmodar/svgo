"""SVG path parsing, editing, serialization, and lightweight optimization."""

from __future__ import annotations

import math
import re
from dataclasses import dataclass, replace
from typing import Iterable, Sequence

COMMAND_RE = re.compile(
    r"[AaCcHhLlMmQqSsTtVvZz]|[-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?"
)

Matrix = tuple[float, float, float, float, float, float]

OPTIMIZE_ALIASES = {
    "remove-useless": "removeUselessCommands",
    "remove-useless-commands": "removeUselessCommands",
    "removeUselessCommands": "removeUselessCommands",
    "use-shorthands": "useShorthands",
    "useShorthands": "useShorthands",
    "use-hv": "useHorizontalAndVerticalLines",
    "use-horizontal-vertical": "useHorizontalAndVerticalLines",
    "use-horizontal-and-vertical-lines": "useHorizontalAndVerticalLines",
    "useHorizontalAndVerticalLines": "useHorizontalAndVerticalLines",
    "use-relative-absolute": "useRelativeAbsolute",
    "useRelativeAbsolute": "useRelativeAbsolute",
    "use-reverse": "useReverse",
    "useReverse": "useReverse",
    "use-close-path": "useClosePath",
    "useClosePath": "useClosePath",
    "remove-orphan-dots": "removeOrphanDots",
    "removeOrphanDots": "removeOrphanDots",
}


class PathDataError(ValueError):
    """Raised when SVG path data is malformed or an operation cannot apply."""


@dataclass(frozen=True)
class Point:
    x: float
    y: float

    def transform(self, matrix: Matrix) -> "Point":
        a, b, c, d, e, f = matrix
        return Point(a * self.x + c * self.y + e, b * self.x + d * self.y + f)

    def close_to(self, other: "Point", eps: float = 1e-9) -> bool:
        return abs(self.x - other.x) <= eps and abs(self.y - other.y) <= eps


@dataclass(frozen=True)
class Segment:
    cmd: str
    start: Point
    end: Point
    values: tuple[float | int | Point, ...] = ()
    index: int = 0

    def reversed(self) -> "Segment":
        if self.cmd == "M":
            return Segment("M", self.end, self.end, (), self.index)
        if self.cmd == "L":
            return Segment("L", self.end, self.start, (), self.index)
        if self.cmd == "C":
            c1, c2 = self.values
            assert isinstance(c1, Point) and isinstance(c2, Point)
            return Segment("C", self.end, self.start, (c2, c1), self.index)
        if self.cmd == "Q":
            (c,) = self.values
            assert isinstance(c, Point)
            return Segment("Q", self.end, self.start, (c,), self.index)
        if self.cmd == "A":
            rx, ry, rotation, large_arc, sweep = self.values
            return Segment("A", self.end, self.start, (rx, ry, rotation, large_arc, 0 if sweep else 1), self.index)
        if self.cmd == "Z":
            return Segment("Z", self.end, self.start, (), self.index)
        raise PathDataError(f"Unsupported segment for reverse: {self.cmd}")

    def transformed(self, matrix: Matrix) -> list["Segment"]:
        if self.cmd == "M":
            p = self.end.transform(matrix)
            return [Segment("M", p, p, (), self.index)]
        if self.cmd == "L":
            return [Segment("L", self.start.transform(matrix), self.end.transform(matrix), (), self.index)]
        if self.cmd == "C":
            c1, c2 = self.values
            assert isinstance(c1, Point) and isinstance(c2, Point)
            return [
                Segment(
                    "C",
                    self.start.transform(matrix),
                    self.end.transform(matrix),
                    (c1.transform(matrix), c2.transform(matrix)),
                    self.index,
                )
            ]
        if self.cmd == "Q":
            (c,) = self.values
            assert isinstance(c, Point)
            return [
                Segment(
                    "Q",
                    self.start.transform(matrix),
                    self.end.transform(matrix),
                    (c.transform(matrix),),
                    self.index,
                )
            ]
        if self.cmd == "A":
            return [seg.transformed(matrix)[0] for seg in arc_to_cubic_segments(self)]
        if self.cmd == "Z":
            return [Segment("Z", self.start.transform(matrix), self.end.transform(matrix), (), self.index)]
        raise PathDataError(f"Unsupported segment for transform: {self.cmd}")


class TokenStream:
    def __init__(self, path_data: str) -> None:
        self.tokens = COMMAND_RE.findall(path_data)
        self.i = 0

    def has_more(self) -> bool:
        return self.i < len(self.tokens)

    def peek(self) -> str | None:
        return self.tokens[self.i] if self.i < len(self.tokens) else None

    def next(self) -> str:
        if self.i >= len(self.tokens):
            raise PathDataError("Unexpected end of path data")
        token = self.tokens[self.i]
        self.i += 1
        return token

    def has_numbers(self, count: int) -> bool:
        return self.i + count <= len(self.tokens) and all(not is_command(self.tokens[self.i + j]) for j in range(count))

    def number(self, label: str) -> float:
        token = self.next()
        if is_command(token):
            raise PathDataError(f"{label} requires a number")
        try:
            return float(token)
        except ValueError as exc:
            raise PathDataError(f"Invalid numeric token: {token}") from exc

    def flag(self, label: str) -> int:
        if self.i >= len(self.tokens):
            raise PathDataError(f"Arc command requires {label}")
        token = self.tokens[self.i]
        if is_command(token):
            raise PathDataError(f"Arc command requires {label}")
        if token in ("0", "1"):
            self.i += 1
            return int(token)
        if token[0] in ("0", "1"):
            flag = int(token[0])
            self.tokens[self.i] = token[1:]
            if not self.tokens[self.i]:
                self.i += 1
            return flag
        raise PathDataError(f"Arc command {label} must be 0 or 1")


def is_command(token: str) -> bool:
    return len(token) == 1 and token.isalpha()


def parse_optimize_options(profile: str | None) -> dict[str, bool]:
    if not profile or profile == "safe":
        return {
            "removeUselessCommands": True,
            "useShorthands": True,
            "useHorizontalAndVerticalLines": True,
            "useRelativeAbsolute": True,
        }
    if profile == "size":
        options = parse_optimize_options("safe")
        options["useReverse"] = True
        return options
    if profile == "closed":
        options = parse_optimize_options("safe")
        options["useClosePath"] = True
        return options
    if profile == "all":
        return {
            "removeUselessCommands": True,
            "removeOrphanDots": True,
            "useShorthands": True,
            "useHorizontalAndVerticalLines": True,
            "useRelativeAbsolute": True,
            "useReverse": True,
            "useClosePath": True,
        }

    options: dict[str, bool] = {}
    for raw in profile.split(","):
        name = raw.strip()
        if not name:
            continue
        key = OPTIMIZE_ALIASES.get(name)
        if not key:
            raise PathDataError(f"Unknown optimize option: {name}")
        options[key] = True
    return options


def matrix_multiply(left: Matrix, right: Matrix) -> Matrix:
    a1, b1, c1, d1, e1, f1 = left
    a2, b2, c2, d2, e2, f2 = right
    return (
        a1 * a2 + c1 * b2,
        b1 * a2 + d1 * b2,
        a1 * c2 + c1 * d2,
        b1 * c2 + d1 * d2,
        a1 * e2 + c1 * f2 + e1,
        b1 * e2 + d1 * f2 + f1,
    )


def translate_matrix(dx: float, dy: float) -> Matrix:
    return (1.0, 0.0, 0.0, 1.0, dx, dy)


def scale_matrix(kx: float, ky: float) -> Matrix:
    return (kx, 0.0, 0.0, ky, 0.0, 0.0)


def rotate_matrix(ox: float, oy: float, degrees: float) -> Matrix:
    radians = math.radians(degrees)
    cos_v = math.cos(radians)
    sin_v = math.sin(radians)
    around_origin: Matrix = (cos_v, sin_v, -sin_v, cos_v, 0.0, 0.0)
    return matrix_multiply(translate_matrix(ox, oy), matrix_multiply(around_origin, translate_matrix(-ox, -oy)))


def parse_matrix_values(text: str) -> Matrix:
    parts = [part for part in re.split(r"[,\s]+", text.strip()) if part]
    if len(parts) != 6:
        raise PathDataError("matrix requires 6 comma- or space-separated numbers")
    try:
        return tuple(float(part) for part in parts)  # type: ignore[return-value]
    except ValueError as exc:
        raise PathDataError(f"Invalid matrix value in {text!r}") from exc


class PathData:
    """Parsed SVG path data with mutating editing and optimization operations."""

    def __init__(self, segments: list[Segment], relative: bool = False) -> None:
        self.segments = segments
        self.relative = relative
        self.optimize_flags: dict[str, bool] = {}

    @classmethod
    def parse(cls, path_data: str) -> "PathData":
        stream = TokenStream(path_data)
        if not stream.tokens:
            raise PathDataError("No SVG path tokens found")

        segments: list[Segment] = []
        command = ""
        current = Point(0.0, 0.0)
        subpath_start = Point(0.0, 0.0)
        last_cubic_ctrl: Point | None = None
        last_quad_ctrl: Point | None = None
        index = 0

        def read_point(relative: bool) -> Point:
            nonlocal current
            x = stream.number("x")
            y = stream.number("y")
            if relative:
                return Point(current.x + x, current.y + y)
            return Point(x, y)

        while stream.has_more():
            token = stream.peek()
            if token and is_command(token):
                command = stream.next()
            elif not command:
                raise PathDataError("Path data must begin with a command")

            upper = command.upper()
            relative = command.islower()

            if upper == "M":
                if not stream.has_numbers(2):
                    raise PathDataError("M command requires x y")
                current = read_point(relative)
                subpath_start = current
                segments.append(Segment("M", current, current, (), index))
                index += 1
                while stream.has_numbers(2):
                    start = current
                    current = read_point(relative)
                    segments.append(Segment("L", start, current, (), index))
                    index += 1
                command = "l" if relative else "L"
                last_cubic_ctrl = None
                last_quad_ctrl = None
            elif upper == "L":
                while stream.has_numbers(2):
                    start = current
                    current = read_point(relative)
                    segments.append(Segment("L", start, current, (), index))
                    index += 1
                last_cubic_ctrl = None
                last_quad_ctrl = None
            elif upper == "H":
                while stream.has_numbers(1):
                    start = current
                    x = stream.number("x")
                    current = Point(current.x + x, current.y) if relative else Point(x, current.y)
                    segments.append(Segment("L", start, current, (), index))
                    index += 1
                last_cubic_ctrl = None
                last_quad_ctrl = None
            elif upper == "V":
                while stream.has_numbers(1):
                    start = current
                    y = stream.number("y")
                    current = Point(current.x, current.y + y) if relative else Point(current.x, y)
                    segments.append(Segment("L", start, current, (), index))
                    index += 1
                last_cubic_ctrl = None
                last_quad_ctrl = None
            elif upper == "C":
                while stream.has_numbers(6):
                    start = current
                    c1 = read_point(relative)
                    c2 = read_point(relative)
                    current = read_point(relative)
                    segments.append(Segment("C", start, current, (c1, c2), index))
                    index += 1
                    last_cubic_ctrl = c2
                    last_quad_ctrl = None
            elif upper == "S":
                while stream.has_numbers(4):
                    start = current
                    c1 = Point(2 * current.x - last_cubic_ctrl.x, 2 * current.y - last_cubic_ctrl.y) if last_cubic_ctrl else current
                    c2 = read_point(relative)
                    current = read_point(relative)
                    segments.append(Segment("C", start, current, (c1, c2), index))
                    index += 1
                    last_cubic_ctrl = c2
                    last_quad_ctrl = None
            elif upper == "Q":
                while stream.has_numbers(4):
                    start = current
                    c = read_point(relative)
                    current = read_point(relative)
                    segments.append(Segment("Q", start, current, (c,), index))
                    index += 1
                    last_quad_ctrl = c
                    last_cubic_ctrl = None
            elif upper == "T":
                while stream.has_numbers(2):
                    start = current
                    c = Point(2 * current.x - last_quad_ctrl.x, 2 * current.y - last_quad_ctrl.y) if last_quad_ctrl else current
                    current = read_point(relative)
                    segments.append(Segment("Q", start, current, (c,), index))
                    index += 1
                    last_quad_ctrl = c
                    last_cubic_ctrl = None
            elif upper == "A":
                while stream.has_more() and stream.peek() and not is_command(stream.peek()):
                    start = current
                    rx = stream.number("rx")
                    ry = stream.number("ry")
                    rotation = stream.number("x-axis-rotation")
                    large_arc = stream.flag("large-arc-flag")
                    sweep = stream.flag("sweep-flag")
                    current = read_point(relative)
                    segments.append(Segment("A", start, current, (abs(rx), abs(ry), rotation, large_arc, sweep), index))
                    index += 1
                last_cubic_ctrl = None
                last_quad_ctrl = None
            elif upper == "Z":
                segments.append(Segment("Z", current, subpath_start, (), index))
                index += 1
                current = subpath_start
                last_cubic_ctrl = None
                last_quad_ctrl = None
            else:
                raise PathDataError(f"Unsupported path command: {command}")

        if not segments:
            raise PathDataError("Path did not produce any commands")
        return cls(reindex_segments(segments))

    def clone(self) -> "PathData":
        other = PathData(list(self.segments), self.relative)
        other.optimize_flags = dict(self.optimize_flags)
        return other

    def transform(self, matrix: Matrix) -> "PathData":
        transformed: list[Segment] = []
        for segment in self.segments:
            transformed.extend(segment.transformed(matrix))
        self.segments = reindex_segments(recompute_starts(transformed))
        return self

    def translate(self, dx: float, dy: float) -> "PathData":
        return self.transform(translate_matrix(dx, dy))

    def scale(self, kx: float, ky: float) -> "PathData":
        return self.transform(scale_matrix(kx, ky))

    def rotate(self, ox: float, oy: float, degrees: float) -> "PathData":
        return self.transform(rotate_matrix(ox, oy, degrees))

    def set_relative(self, relative: bool) -> "PathData":
        self.relative = relative
        return self

    def reverse(self, item_index: int | None = None) -> "PathData":
        groups = subpath_groups(self.segments)
        selected = range(len(groups)) if item_index is None else [group_index_for_item(groups, item_index)]
        selected_set = set(selected)
        result: list[Segment] = []
        for group_index, group in enumerate(groups):
            result.extend(reverse_group(group) if group_index in selected_set else group)
        self.segments = reindex_segments(recompute_starts(result))
        return self

    def change_origin(self, item_index: int, subpath: bool = False) -> "PathData":
        groups = subpath_groups(self.segments)
        if subpath:
            if item_index < 0 or item_index >= len(groups):
                raise PathDataError(f"origin subpath index {item_index} is out of range")
            self.segments = reindex_segments([seg for group in groups[item_index:] + groups[:item_index] for seg in group])
            return self

        group_index = group_index_for_item(groups, item_index)
        group = groups[group_index]
        groups[group_index] = rotate_group_origin(group, item_index)
        self.segments = reindex_segments(recompute_starts([seg for g in groups for seg in g]))
        return self

    def optimize(self, profile: str | None = "safe") -> "PathData":
        options = parse_optimize_options(profile)
        if options.get("removeUselessCommands") or options.get("removeOrphanDots"):
            self.segments = remove_useless(self.segments, remove_orphan_dots=options.get("removeOrphanDots", False))
        if options.get("useClosePath"):
            self.segments = close_matching_subpaths(self.segments)
        if options.get("useReverse"):
            self.segments = choose_shorter_reversal(self.segments)
        self.optimize_flags.update(options)
        self.segments = reindex_segments(recompute_starts(self.segments))
        return self

    def to_string(self, decimals: int = 4, minify: bool = False) -> str:
        return serialize_path(self.segments, decimals, minify, self.relative, self.optimize_flags)

    def apply_operation(self, operation: str, decimals: int = 4) -> "PathData":
        name, rest = split_operation(operation)
        if name == "translate":
            dx, dy = parse_number_list(rest, 2, operation)
            return self.translate(dx, dy)
        if name == "scale":
            kx, ky = parse_number_list(rest, 2, operation)
            return self.scale(kx, ky)
        if name == "matrix":
            return self.transform(parse_matrix_values(rest))
        if name == "rotate":
            ox, oy, degrees = parse_number_list(rest, 3, operation)
            return self.rotate(ox, oy, degrees)
        if name == "relative":
            return self.set_relative(True)
        if name == "absolute":
            return self.set_relative(False)
        if name == "reverse":
            return self.reverse(parse_non_negative_int(rest, "reverse itemIndex") if rest else None)
        if name == "origin":
            parts = rest.split(":")
            if not parts or not parts[0]:
                raise PathDataError("origin requires itemIndex")
            return self.change_origin(parse_non_negative_int(parts[0], "origin itemIndex"), len(parts) > 1 and parts[1] == "subpath")
        if name == "optimize":
            return self.optimize(rest or "safe")
        raise PathDataError(f"Unknown operation: {name}")


def split_operation(operation: str) -> tuple[str, str]:
    matrix_call = re.match(r"^matrix\((.*)\)$", operation, re.S)
    if matrix_call:
        return "matrix", matrix_call.group(1)
    if ":" not in operation:
        return operation, ""
    name, rest = operation.split(":", 1)
    return name, rest


def parse_number_list(text: str, expected: int, label: str) -> list[float]:
    parts = [part.strip() for part in text.split(",") if part.strip()]
    if len(parts) != expected:
        raise PathDataError(f"{label} requires {expected} comma-separated numbers")
    try:
        return [float(part) for part in parts]
    except ValueError as exc:
        raise PathDataError(f"{label} contains an invalid number") from exc


def parse_non_negative_int(text: str, label: str) -> int:
    try:
        value = int(text)
    except ValueError as exc:
        raise PathDataError(f"{label} must be a non-negative integer: {text}") from exc
    if value < 0:
        raise PathDataError(f"{label} must be a non-negative integer: {text}")
    return value


def reindex_segments(segments: list[Segment]) -> list[Segment]:
    return [replace(segment, index=i) for i, segment in enumerate(segments)]


def recompute_starts(segments: list[Segment]) -> list[Segment]:
    result: list[Segment] = []
    current = Point(0.0, 0.0)
    subpath_start = Point(0.0, 0.0)
    for segment in segments:
        if segment.cmd == "M":
            current = segment.end
            subpath_start = segment.end
            result.append(replace(segment, start=current, end=current))
            continue
        if segment.cmd == "Z":
            result.append(replace(segment, start=current, end=subpath_start))
            current = subpath_start
            continue
        result.append(replace(segment, start=current))
        current = segment.end
    return result


def subpath_groups(segments: list[Segment]) -> list[list[Segment]]:
    groups: list[list[Segment]] = []
    current: list[Segment] = []
    for segment in segments:
        if segment.cmd == "M" and current:
            groups.append(current)
            current = [segment]
        else:
            current.append(segment)
    if current:
        groups.append(current)
    return groups


def group_index_for_item(groups: list[list[Segment]], item_index: int) -> int:
    for group_index, group in enumerate(groups):
        if any(segment.index == item_index for segment in group):
            return group_index
    raise PathDataError(f"item index {item_index} is out of range")


def reverse_group(group: list[Segment]) -> list[Segment]:
    if not group or group[0].cmd != "M":
        return group
    drawing = [segment for segment in group[1:] if segment.cmd != "Z"]
    if not drawing:
        return group
    closed = any(segment.cmd == "Z" for segment in group)
    start = group[0].end
    if closed and not drawing[-1].end.close_to(start):
        drawing = drawing + [Segment("L", drawing[-1].end, start, (), drawing[-1].index)]
    new_start = start if closed else drawing[-1].end
    reversed_draw = [segment.reversed() for segment in reversed(drawing)]
    result = [Segment("M", new_start, new_start, (), group[0].index)] + reversed_draw
    if closed:
        result.append(Segment("Z", result[-1].end, new_start, (), group[-1].index))
    return recompute_starts(result)


def rotate_group_origin(group: list[Segment], item_index: int) -> list[Segment]:
    if not group or group[0].cmd != "M":
        return group
    drawing = [segment for segment in group[1:] if segment.cmd != "Z"]
    closed = any(segment.cmd == "Z" for segment in group)
    if not closed:
        raise PathDataError("origin can only rotate closed subpaths")
    if not drawing:
        return group
    target = next((i for i, segment in enumerate(drawing) if segment.index == item_index), None)
    if target is None:
        raise PathDataError(f"origin item index {item_index} is not a drawable item in its subpath")
    start = drawing[target].start
    rotated = drawing[target:] + drawing[:target]
    result = [Segment("M", start, start, (), group[0].index)] + rotated + [Segment("Z", rotated[-1].end, start, (), group[-1].index)]
    return recompute_starts(result)


def remove_useless(segments: list[Segment], remove_orphan_dots: bool = False) -> list[Segment]:
    result: list[Segment] = []
    group_has_draw = False
    for segment in segments:
        if segment.cmd == "M":
            if remove_orphan_dots and result and result[-1].cmd == "M" and not group_has_draw:
                result.pop()
            result.append(segment)
            group_has_draw = False
            continue
        if segment.cmd == "Z":
            if group_has_draw:
                result.append(segment)
            continue
        if segment.start.close_to(segment.end) and segment.cmd in {"L", "Q", "C"}:
            continue
        result.append(segment)
        group_has_draw = True
    if remove_orphan_dots and result and result[-1].cmd == "M" and not group_has_draw:
        result.pop()
    return result


def close_matching_subpaths(segments: list[Segment]) -> list[Segment]:
    result: list[Segment] = []
    for group in subpath_groups(segments):
        if not group or group[0].cmd != "M":
            result.extend(group)
            continue
        if any(segment.cmd == "Z" for segment in group):
            result.extend(group)
            continue
        drawing = group[1:]
        if drawing and drawing[-1].end.close_to(group[0].end):
            result.extend(group[:-1])
            result.append(Segment("Z", drawing[-2].end if len(drawing) > 1 else group[0].end, group[0].end, (), drawing[-1].index))
        else:
            result.extend(group)
    return result


def choose_shorter_reversal(segments: list[Segment]) -> list[Segment]:
    result: list[Segment] = []
    flags = {"useRelativeAbsolute": True, "useHorizontalAndVerticalLines": True, "useShorthands": True}
    for group in subpath_groups(segments):
        normal = serialize_path(group, 4, True, False, flags)
        reversed_group = reverse_group(group)
        reversed_text = serialize_path(reversed_group, 4, True, False, flags)
        result.extend(reversed_group if len(reversed_text) < len(normal) else group)
    return result


def serialize_path(
    segments: list[Segment],
    decimals: int,
    minify: bool,
    relative_mode: bool,
    optimize_flags: dict[str, bool] | None = None,
) -> str:
    flags = optimize_flags or {}
    parts: list[str] = []
    current = Point(0.0, 0.0)
    subpath_start = Point(0.0, 0.0)
    last_cubic_ctrl: Point | None = None
    last_quad_ctrl: Point | None = None

    for segment in recompute_starts(segments):
        absolute = segment_to_text(
            segment,
            current,
            subpath_start,
            decimals,
            minify,
            False,
            flags,
            last_cubic_ctrl,
            last_quad_ctrl,
        )
        relative = segment_to_text(
            segment,
            current,
            subpath_start,
            decimals,
            minify,
            True,
            flags,
            last_cubic_ctrl,
            last_quad_ctrl,
        )
        if flags.get("useRelativeAbsolute"):
            text = relative if len(relative) < len(absolute) else absolute
        else:
            text = relative if relative_mode else absolute
        parts.append(text)

        if segment.cmd == "M":
            current = segment.end
            subpath_start = segment.end
            last_cubic_ctrl = None
            last_quad_ctrl = None
        elif segment.cmd == "Z":
            current = subpath_start
            last_cubic_ctrl = None
            last_quad_ctrl = None
        else:
            current = segment.end
            if segment.cmd == "C":
                c2 = segment.values[1]
                assert isinstance(c2, Point)
                last_cubic_ctrl = c2
                last_quad_ctrl = None
            elif segment.cmd == "Q":
                c = segment.values[0]
                assert isinstance(c, Point)
                last_quad_ctrl = c
                last_cubic_ctrl = None
            else:
                last_cubic_ctrl = None
                last_quad_ctrl = None

    joiner = "" if minify else " "
    return joiner.join(part for part in parts if part)


def segment_to_text(
    segment: Segment,
    current: Point,
    subpath_start: Point,
    decimals: int,
    minify: bool,
    relative: bool,
    flags: dict[str, bool],
    last_cubic_ctrl: Point | None,
    last_quad_ctrl: Point | None,
) -> str:
    del subpath_start
    if segment.cmd == "M":
        cmd = "m" if relative else "M"
        point = delta(segment.end, current) if relative else segment.end
        return command_text(cmd, point_numbers(point), decimals, minify)
    if segment.cmd == "Z":
        return "z" if relative else "Z"
    if segment.cmd == "L":
        end = delta(segment.end, current) if relative else segment.end
        if flags.get("useHorizontalAndVerticalLines"):
            if abs(segment.end.y - current.y) <= 1e-9:
                return command_text("h" if relative else "H", [end.x if relative else segment.end.x], decimals, minify)
            if abs(segment.end.x - current.x) <= 1e-9:
                return command_text("v" if relative else "V", [end.y if relative else segment.end.y], decimals, minify)
        return command_text("l" if relative else "L", point_numbers(end), decimals, minify)
    if segment.cmd == "C":
        c1, c2 = segment.values
        assert isinstance(c1, Point) and isinstance(c2, Point)
        if flags.get("useShorthands") and last_cubic_ctrl is not None:
            reflected = Point(2 * current.x - last_cubic_ctrl.x, 2 * current.y - last_cubic_ctrl.y)
            if c1.close_to(reflected):
                p2 = delta(c2, current) if relative else c2
                p = delta(segment.end, current) if relative else segment.end
                return command_text("s" if relative else "S", point_numbers(p2) + point_numbers(p), decimals, minify)
        values = [delta(c1, current), delta(c2, current), delta(segment.end, current)] if relative else [c1, c2, segment.end]
        return command_text("c" if relative else "C", flatten_points(values), decimals, minify)
    if segment.cmd == "Q":
        (c,) = segment.values
        assert isinstance(c, Point)
        if flags.get("useShorthands") and last_quad_ctrl is not None:
            reflected = Point(2 * current.x - last_quad_ctrl.x, 2 * current.y - last_quad_ctrl.y)
            if c.close_to(reflected):
                p = delta(segment.end, current) if relative else segment.end
                return command_text("t" if relative else "T", point_numbers(p), decimals, minify)
        values = [delta(c, current), delta(segment.end, current)] if relative else [c, segment.end]
        return command_text("q" if relative else "Q", flatten_points(values), decimals, minify)
    if segment.cmd == "A":
        rx, ry, rotation, large_arc, sweep = segment.values
        end = delta(segment.end, current) if relative else segment.end
        return command_text(
            "a" if relative else "A",
            [float(rx), float(ry), float(rotation), int(large_arc), int(sweep), end.x, end.y],
            decimals,
            minify,
        )
    raise PathDataError(f"Unsupported segment for serialization: {segment.cmd}")


def delta(point: Point, origin: Point) -> Point:
    return Point(point.x - origin.x, point.y - origin.y)


def point_numbers(point: Point) -> list[float]:
    return [point.x, point.y]


def flatten_points(points: Iterable[Point]) -> list[float]:
    result: list[float] = []
    for point in points:
        result.extend((point.x, point.y))
    return result


def command_text(command: str, numbers: Sequence[float | int], decimals: int, minify: bool) -> str:
    if not numbers:
        return command
    formatted = [fmt_number(float(number), decimals, minify) for number in numbers]
    if not minify:
        return command + " ".join(formatted)
    text = command
    previous = ""
    for number in formatted:
        if not previous:
            text += number
        elif number.startswith("-") or number.startswith(".") and previous[-1].isdigit():
            text += number
        else:
            text += " " + number
        previous = number
    return text


def fmt_number(value: float, decimals: int, minify: bool = False) -> str:
    if abs(value) < 10 ** (-(decimals + 1)):
        value = 0.0
    text = f"{round(value, decimals):.{decimals}f}".rstrip("0").rstrip(".")
    if not text or text == "-0":
        text = "0"
    if minify:
        if text.startswith("0."):
            text = text[1:]
        elif text.startswith("-0."):
            text = "-." + text[3:]
    return text


def angle_between(ux: float, uy: float, vx: float, vy: float) -> float:
    return math.atan2(ux * vy - uy * vx, ux * vx + uy * vy)


def arc_to_center(
    p0: Point,
    rx: float,
    ry: float,
    x_axis_rotation: float,
    large_arc: int,
    sweep: int,
    p1: Point,
) -> tuple[Point, float, float, float, float]:
    rx = abs(rx)
    ry = abs(ry)
    if rx == 0 or ry == 0 or p0.close_to(p1):
        raise PathDataError("Degenerate arc cannot be center-parameterized")

    phi = math.radians(x_axis_rotation % 360.0)
    cos_phi = math.cos(phi)
    sin_phi = math.sin(phi)
    dx = (p0.x - p1.x) / 2.0
    dy = (p0.y - p1.y) / 2.0
    x1p = cos_phi * dx + sin_phi * dy
    y1p = -sin_phi * dx + cos_phi * dy

    radius_check = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry)
    if radius_check > 1:
        factor = math.sqrt(radius_check)
        rx *= factor
        ry *= factor

    sign = -1.0 if large_arc == sweep else 1.0
    numerator = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p
    denominator = rx * rx * y1p * y1p + ry * ry * x1p * x1p
    coef = 0.0 if denominator == 0 else sign * math.sqrt(max(0.0, numerator / denominator))
    cxp = coef * (rx * y1p / ry)
    cyp = coef * (-ry * x1p / rx)
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
    return Point(cx, cy), rx, ry, phi, theta1, delta


def arc_to_cubic_segments(segment: Segment) -> list[Segment]:
    if segment.cmd != "A":
        raise PathDataError("arc_to_cubic_segments requires an A segment")
    rx, ry, rotation, large_arc, sweep = segment.values
    try:
        center, rx, ry, phi, theta1, delta = arc_to_center(
            segment.start,
            float(rx),
            float(ry),
            float(rotation),
            int(large_arc),
            int(sweep),
            segment.end,
        )
    except PathDataError:
        return [Segment("L", segment.start, segment.end, (), segment.index)]

    cos_phi = math.cos(phi)
    sin_phi = math.sin(phi)
    pieces = max(1, int(math.ceil(abs(delta) / (math.pi / 2.0))))
    step = delta / pieces
    current = segment.start
    cubics: list[Segment] = []
    for i in range(pieces):
        t1 = theta1 + i * step
        t2 = t1 + step
        alpha = 4.0 / 3.0 * math.tan((t2 - t1) / 4.0)

        def point(theta: float) -> Point:
            return Point(
                center.x + cos_phi * rx * math.cos(theta) - sin_phi * ry * math.sin(theta),
                center.y + sin_phi * rx * math.cos(theta) + cos_phi * ry * math.sin(theta),
            )

        def derivative(theta: float) -> Point:
            return Point(
                -cos_phi * rx * math.sin(theta) - sin_phi * ry * math.cos(theta),
                -sin_phi * rx * math.sin(theta) + cos_phi * ry * math.cos(theta),
            )

        p1 = point(t1)
        p2 = point(t2)
        d1 = derivative(t1)
        d2 = derivative(t2)
        c1 = Point(p1.x + alpha * d1.x, p1.y + alpha * d1.y)
        c2 = Point(p2.x - alpha * d2.x, p2.y - alpha * d2.y)
        end = segment.end if i == pieces - 1 else p2
        cubics.append(Segment("C", current, end, (c1, c2), segment.index))
        current = end
    return cubics


def parse_transform(transform: str) -> Matrix:
    matrix: Matrix = (1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
    for name, raw in re.findall(r"([a-zA-Z]+)\(([^)]*)\)", transform):
        values = [float(part) for part in re.split(r"[,\s]+", raw.strip()) if part]
        op: Matrix
        if name == "matrix":
            if len(values) != 6:
                raise PathDataError("matrix() transform requires 6 values")
            op = tuple(values)  # type: ignore[assignment]
        elif name == "translate":
            if not values:
                raise PathDataError("translate() transform requires at least 1 value")
            op = translate_matrix(values[0], values[1] if len(values) > 1 else 0.0)
        elif name == "scale":
            if not values:
                raise PathDataError("scale() transform requires at least 1 value")
            op = scale_matrix(values[0], values[1] if len(values) > 1 else values[0])
        elif name == "rotate":
            if len(values) == 1:
                op = rotate_matrix(0.0, 0.0, values[0])
            elif len(values) == 3:
                op = rotate_matrix(values[1], values[2], values[0])
            else:
                raise PathDataError("rotate() transform requires 1 or 3 values")
        elif name == "skewX":
            if len(values) != 1:
                raise PathDataError("skewX() transform requires 1 value")
            op = (1.0, 0.0, math.tan(math.radians(values[0])), 1.0, 0.0, 0.0)
        elif name == "skewY":
            if len(values) != 1:
                raise PathDataError("skewY() transform requires 1 value")
            op = (1.0, math.tan(math.radians(values[0])), 0.0, 1.0, 0.0, 0.0)
        else:
            raise PathDataError(f"Unsupported transform: {name}")
        matrix = matrix_multiply(matrix, op)
    return matrix
