"""Dependency-free PNG icon tracing into filled SVG paths."""

from __future__ import annotations

import struct
import zlib
from collections import Counter, defaultdict, deque
from dataclasses import dataclass
from pathlib import Path

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


class RasterTraceError(ValueError):
    """Raised when PNG tracing cannot continue."""


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


def paeth(left: int, up: int, up_left: int) -> int:
    p = left + up - up_left
    pa = abs(p - left)
    pb = abs(p - up)
    pc = abs(p - up_left)
    if pa <= pb and pa <= pc:
        return left
    if pb <= pc:
        return up
    return up_left


def read_png(path: Path) -> Image:
    data = path.read_bytes()
    if not data.startswith(PNG_SIGNATURE):
        raise RasterTraceError("Input is not a PNG file")

    offset = len(PNG_SIGNATURE)
    width = height = bit_depth = color_type = None
    palette: list[tuple[int, int, int]] = []
    transparency: bytes | None = None
    idat = bytearray()

    while offset + 8 <= len(data):
        length = struct.unpack(">I", data[offset : offset + 4])[0]
        chunk_type = data[offset + 4 : offset + 8]
        chunk = data[offset + 8 : offset + 8 + length]
        offset += 12 + length
        if chunk_type == b"IHDR":
            width, height, bit_depth, color_type, compression, filter_method, interlace = struct.unpack(">IIBBBBB", chunk)
            if compression != 0 or filter_method != 0 or interlace != 0:
                raise RasterTraceError("Only non-interlaced standard PNG files are supported")
        elif chunk_type == b"PLTE":
            palette = [tuple(chunk[i : i + 3]) for i in range(0, len(chunk), 3)]  # type: ignore[list-item]
        elif chunk_type == b"tRNS":
            transparency = chunk
        elif chunk_type == b"IDAT":
            idat.extend(chunk)
        elif chunk_type == b"IEND":
            break

    if width is None or height is None or bit_depth is None or color_type is None:
        raise RasterTraceError("PNG is missing IHDR")
    if bit_depth != 8:
        raise RasterTraceError("Only 8-bit PNG files are supported")

    channels_by_type = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}
    if color_type not in channels_by_type:
        raise RasterTraceError(f"Unsupported PNG color type: {color_type}")
    channels = channels_by_type[color_type]
    stride = width * channels
    raw = zlib.decompress(bytes(idat))
    rows: list[bytearray] = []
    src = 0
    for _ in range(height):
        filter_type = raw[src]
        src += 1
        row = bytearray(raw[src : src + stride])
        src += stride
        prev = rows[-1] if rows else bytearray(stride)
        for i in range(stride):
            left = row[i - channels] if i >= channels else 0
            up = prev[i]
            up_left = prev[i - channels] if i >= channels else 0
            if filter_type == 1:
                row[i] = (row[i] + left) & 0xFF
            elif filter_type == 2:
                row[i] = (row[i] + up) & 0xFF
            elif filter_type == 3:
                row[i] = (row[i] + ((left + up) // 2)) & 0xFF
            elif filter_type == 4:
                row[i] = (row[i] + paeth(left, up, up_left)) & 0xFF
            elif filter_type != 0:
                raise RasterTraceError(f"Unsupported PNG row filter: {filter_type}")
        rows.append(row)

    pixels: list[tuple[int, int, int, int]] = []
    for row in rows:
        for col in range(width):
            i = col * channels
            if color_type == 0:
                gray = row[i]
                pixels.append((gray, gray, gray, 255))
            elif color_type == 2:
                pixels.append((row[i], row[i + 1], row[i + 2], 255))
            elif color_type == 3:
                index = row[i]
                if index >= len(palette):
                    raise RasterTraceError("PNG palette index out of range")
                r, g, b = palette[index]
                a = transparency[index] if transparency is not None and index < len(transparency) else 255
                pixels.append((r, g, b, a))
            elif color_type == 4:
                gray, alpha = row[i], row[i + 1]
                pixels.append((gray, gray, gray, alpha))
            elif color_type == 6:
                pixels.append((row[i], row[i + 1], row[i + 2], row[i + 3]))
    return Image(width=width, height=height, pixels=pixels)


def visible(pixel: tuple[int, int, int, int], options: TraceOptions) -> bool:
    r, g, b, a = pixel
    if a < options.alpha_threshold:
        return False
    if options.drop_white and r >= options.white_threshold and g >= options.white_threshold and b >= options.white_threshold:
        return False
    return True


def quantized_rgb(pixel: tuple[int, int, int, int], step: int) -> tuple[int, int, int]:
    step = max(1, step)
    return tuple(min(255, int(round(channel / step) * step)) for channel in pixel[:3])


def nearest_color(color: tuple[int, int, int], palette: list[tuple[int, int, int]]) -> tuple[int, int, int]:
    return min(palette, key=lambda candidate: sum((color[i] - candidate[i]) ** 2 for i in range(3)))


def group_pixels(image: Image, options: TraceOptions) -> dict[tuple[int, int, int], set[tuple[int, int]]]:
    visible_pixels: list[tuple[int, int, tuple[int, int, int, int]]] = []
    for row in range(image.height):
        for col in range(image.width):
            pixel = image.pixels[row * image.width + col]
            if visible(pixel, options):
                visible_pixels.append((row, col, pixel))
    if not visible_pixels:
        raise RasterTraceError("No visible pixels found")

    groups: dict[tuple[int, int, int], set[tuple[int, int]]] = defaultdict(set)
    if options.mode == "alpha":
        color = Counter(quantized_rgb(pixel, options.quantize) for _row, _col, pixel in visible_pixels).most_common(1)[0][0]
        for row, col, _pixel in visible_pixels:
            groups[color].add((row, col))
        return groups

    if options.mode == "exact":
        for row, col, pixel in visible_pixels:
            groups[quantized_rgb(pixel, options.quantize)].add((row, col))
        return groups

    if options.mode != "palette":
        raise RasterTraceError("--mode must be palette, alpha, or exact")

    histogram = Counter(quantized_rgb(pixel, options.quantize) for _row, _col, pixel in visible_pixels)
    palette = [color for color, _count in histogram.most_common(max(1, options.max_colors))]
    for row, col, pixel in visible_pixels:
        groups[nearest_color(quantized_rgb(pixel, options.quantize), palette)].add((row, col))
    return groups


def components(mask: set[tuple[int, int]]) -> list[set[tuple[int, int]]]:
    remaining = set(mask)
    found: list[set[tuple[int, int]]] = []
    while remaining:
        first = remaining.pop()
        component = {first}
        queue = deque([first])
        while queue:
            row, col = queue.popleft()
            for neighbor in ((row - 1, col), (row + 1, col), (row, col - 1), (row, col + 1)):
                if neighbor in remaining:
                    remaining.remove(neighbor)
                    component.add(neighbor)
                    queue.append(neighbor)
        found.append(component)
    return found


def trace_edges(component: set[tuple[int, int]]) -> list[list[tuple[int, int]]]:
    edges: dict[tuple[int, int], list[tuple[int, int]]] = defaultdict(list)
    for row, col in component:
        if (row - 1, col) not in component:
            edges[(col, row)].append((col + 1, row))
        if (row, col + 1) not in component:
            edges[(col + 1, row)].append((col + 1, row + 1))
        if (row + 1, col) not in component:
            edges[(col + 1, row + 1)].append((col, row + 1))
        if (row, col - 1) not in component:
            edges[(col, row + 1)].append((col, row))

    loops: list[list[tuple[int, int]]] = []
    while edges:
        start = next(iter(edges))
        current = start
        loop = [start]
        while True:
            targets = edges.get(current)
            if not targets:
                break
            nxt = targets.pop()
            if not targets:
                del edges[current]
            loop.append(nxt)
            current = nxt
            if current == start:
                break
        if len(loop) > 3:
            loops.append(simplify_collinear(loop))
    return loops


def simplify_collinear(points: list[tuple[int, int]]) -> list[tuple[int, int]]:
    if len(points) <= 3:
        return points
    closed = points[0] == points[-1]
    body = points[:-1] if closed else points[:]
    changed = True
    while changed and len(body) > 2:
        changed = False
        simplified: list[tuple[int, int]] = []
        count = len(body)
        for i, point in enumerate(body):
            prev = body[(i - 1) % count]
            nxt = body[(i + 1) % count]
            if (prev[0] == point[0] == nxt[0]) or (prev[1] == point[1] == nxt[1]):
                changed = True
                continue
            simplified.append(point)
        body = simplified
    return body + [body[0]] if closed and body else body


def fmt(value: float, decimals: int) -> str:
    text = f"{round(value, decimals):.{decimals}f}".rstrip("0").rstrip(".")
    return text if text and text != "-0" else "0"


def path_from_loops(loops: list[list[tuple[int, int]]], scale: float, decimals: int) -> str:
    parts: list[str] = []
    for loop in loops:
        if len(loop) < 4:
            continue
        first = loop[0]
        parts.append(f"M{fmt(first[0] * scale, decimals)} {fmt(first[1] * scale, decimals)}")
        for point in loop[1:-1]:
            parts.append(f"L{fmt(point[0] * scale, decimals)} {fmt(point[1] * scale, decimals)}")
        parts.append("Z")
    return " ".join(parts)


def color_hex(color: tuple[int, int, int]) -> str:
    return "#{:02x}{:02x}{:02x}".format(*color)


def escape_attr(value: str) -> str:
    return value.replace("&", "&amp;").replace('"', "&quot;").replace("<", "&lt;").replace(">", "&gt;")


def build_svg(image: Image, groups: dict[tuple[int, int, int], set[tuple[int, int]]], options: TraceOptions) -> str:
    paths: list[str] = []
    for color, mask in sorted(groups.items(), key=lambda item: len(item[1]), reverse=True):
        loops: list[list[tuple[int, int]]] = []
        for component in components(mask):
            if len(component) < options.min_area:
                continue
            loops.extend(trace_edges(component))
        d = path_from_loops(loops, options.scale, options.decimals)
        if d:
            paths.append(f'<path fill="{color_hex(color)}" fill-rule="evenodd" d="{escape_attr(d)}"/>')
    if not paths:
        raise RasterTraceError("No traceable components survived --min-area")

    width = image.width * options.scale
    height = image.height * options.scale
    title = f"<title>{escape_attr(options.title)}</title>\n  " if options.title else ""
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {fmt(width, options.decimals)} {fmt(height, options.decimals)}">\n'
        f"  {title}" + "\n  ".join(paths) + "\n</svg>"
    )


def trace_image(image: Image, options: TraceOptions | None = None) -> str:
    options = options or TraceOptions()
    if options.curve_mode not in {"pixel", "exact"}:
        raise RasterTraceError("--curve-mode must be pixel or exact for svgo trace; use svgo trace2 for VTracer curve fitting")
    if options.alpha_threshold < 0 or options.alpha_threshold > 255:
        raise RasterTraceError("--alpha-threshold must be between 0 and 255")
    if options.max_colors < 1:
        raise RasterTraceError("--max-colors must be at least 1")
    if options.scale <= 0:
        raise RasterTraceError("--scale must be greater than zero")
    groups = group_pixels(image, options)
    return build_svg(image, groups, options)


def trace_png(path: str | Path, options: TraceOptions | None = None) -> str:
    return trace_image(read_png(Path(path)), options)
