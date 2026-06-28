"""Convert two-color antialiased PNG line icons into optimized centerline SVGs.

Example:
    uv run --no-sync python recipes/two_color_centerline_icons.py INPUT_DIR OUT_DIR
"""

from __future__ import annotations

import argparse
from concurrent.futures import ProcessPoolExecutor, as_completed
import json
import math
from dataclasses import dataclass
from pathlib import Path
from statistics import median
from xml.sax.saxutils import escape

from svgo import (
    CenterlineOptions,
    TraceOptions,
    centerline_path_data,
    filled_loops,
    format_number as fmt_number,
    normalize_color,
    optimize_path_data,
    point_distance,
    polygon_area,
    polyline_lengths,
    polyline_subpaths,
    radial_centerline_candidate as build_radial_centerline_candidate,
    serialize_polyline_subpaths,
    stitch_subpaths,
    trace_png_components,
    turn_stats,
)


DEFAULT_MAIN = "#143861"
DEFAULT_ACCENT = "#00b795"


@dataclass
class CenterlineCandidate:
    d: str
    stroke_width: float
    kind: str
    segments: int
    subpaths: int
    lengths: list[float]
    max_turn: float
    sharp_turns: int


@dataclass
class SvgComponent:
    color: str
    d: str
    stroke_width: float
    area: int
    bbox: dict
    kind: str


def optimize_d(d: str, decimals: int) -> str:
    return optimize_path_data(d, decimals)


def radial_closed_candidate(path_d: str, args: argparse.Namespace) -> CenterlineCandidate | None:
    result = build_radial_centerline_candidate(
        path_d,
        samples=args.radial_samples,
        simplify=args.radial_simplify,
        decimals=args.decimals,
        fallback_stroke_width=float(args.stroke_width or 31),
    )
    if result is None:
        return None
    d, stroke_width = result
    return CenterlineCandidate(
        d=d,
        stroke_width=stroke_width,
        kind="radial-smooth",
        segments=d.count("C") + d.count("L"),
        subpaths=1,
        lengths=[],
        max_turn=0,
        sharp_turns=0,
    )


def centerline_candidate(path_d: str, args: argparse.Namespace, *, mode: str, polyline: bool, simplify: float, bridge_gap: float = 0.0) -> CenterlineCandidate:
    d, stroke_width, _ctx = centerline_path_data(
        path_d,
        CenterlineOptions(
            mode=mode,
            scale=args.center_scale,
            max_size=args.max_size,
            simplify=simplify,
            min_length=args.min_length,
            decimals=args.decimals,
            polyline=polyline,
            bridge_gap=bridge_gap,
        ),
    )
    raw_d = d
    if polyline and mode == "all" and args.stitch_all:
        stitch_gap = args.stitch_gap if args.stitch_gap > 0 else max(24.0, stroke_width * 2.2)
        stitched = stitch_subpaths(polyline_subpaths(raw_d), stitch_gap)
        raw_d = serialize_polyline_subpaths(stitched, args.decimals)
    lengths = polyline_lengths(raw_d) if polyline else []
    subpaths = len(lengths) if polyline else raw_d.count("M")
    segments, max_turn, sharp_turns = turn_stats(raw_d) if polyline else (raw_d.count("L") + raw_d.count("l") + raw_d.count("C") + raw_d.count("c"), 0.0, 0)
    d = optimize_d(raw_d, args.decimals)
    return CenterlineCandidate(
        d=d,
        stroke_width=stroke_width,
        kind=f"{mode}-{'polyline' if polyline else 'smooth'}",
        segments=segments,
        subpaths=subpaths,
        lengths=lengths,
        max_turn=max_turn,
        sharp_turns=sharp_turns,
    )


def branch_probe(path_d: str, args: argparse.Namespace) -> CenterlineCandidate | None:
    try:
        d, stroke_width, _ctx = centerline_path_data(
            path_d,
            CenterlineOptions(
                mode="all",
                scale=args.branch_probe_scale,
                max_size=args.branch_probe_max_size,
                simplify=args.linear_simplify,
                min_length=args.min_length,
                decimals=1,
                polyline=True,
            ),
        )
    except Exception:
        return None
    lengths = polyline_lengths(d)
    segments, max_turn, sharp_turns = turn_stats(d)
    return CenterlineCandidate(
        d=d,
        stroke_width=stroke_width,
        kind="probe-all-polyline",
        segments=segments,
        subpaths=len(lengths),
        lengths=lengths,
        max_turn=max_turn,
        sharp_turns=sharp_turns,
    )


def combined_complex_candidate(path_d: str, args: argparse.Namespace) -> CenterlineCandidate:
    longest_d, stroke_width, _ctx = centerline_path_data(
        path_d,
        CenterlineOptions(
            mode="longest",
            scale=args.center_scale,
            max_size=args.max_size,
            simplify=args.linear_simplify,
            min_length=args.min_length,
            decimals=args.decimals,
            polyline=True,
        ),
    )
    all_d, _all_width, _all_ctx = centerline_path_data(
        path_d,
        CenterlineOptions(
            mode="all",
            scale=args.center_scale,
            max_size=args.max_size,
            simplify=args.linear_simplify,
            min_length=args.min_length,
            decimals=args.decimals,
            polyline=True,
        ),
    )
    d = optimize_d(longest_d + " " + all_d, args.decimals)
    return CenterlineCandidate(
        d=d,
        stroke_width=stroke_width,
        kind="combined-complex",
        segments=d.count("L") + d.count("l"),
        subpaths=d.count("M") + d.count("m"),
        lengths=[],
        max_turn=0,
        sharp_turns=0,
    )


def dot_candidate(raw: dict, args: argparse.Namespace) -> CenterlineCandidate | None:
    bbox = raw["pixel_bbox"]
    width = float(bbox["width"])
    height = float(bbox["height"])
    if width <= 0 or height <= 0:
        return None
    aspect = max(width, height) / min(width, height)
    if max(width, height) > args.dot_max_size or aspect > args.dot_max_aspect:
        return None
    cx = float(bbox["x"]) + width / 2
    cy = float(bbox["y"]) + height / 2
    d = f"M{fmt_number(cx, args.decimals)} {fmt_number(cy, args.decimals)}h0"
    return CenterlineCandidate(
        d=d,
        stroke_width=(width + height) / 2,
        kind="dot",
        segments=1,
        subpaths=1,
        lengths=[],
        max_turn=0,
        sharp_turns=0,
    )


def solid_line_candidate(raw: dict, args: argparse.Namespace) -> CenterlineCandidate | None:
    bbox = raw["pixel_bbox"]
    width = float(bbox["width"])
    height = float(bbox["height"])
    if max(width, height) > args.solid_line_max_size or raw["d"].count("M") != 1:
        return None
    loops = filled_loops(raw["d"])
    if not loops:
        return None
    points = loops[0]
    cx = sum(p[0] for p in points) / len(points)
    cy = sum(p[1] for p in points) / len(points)
    cov_xx = sum((p[0] - cx) * (p[0] - cx) for p in points) / len(points)
    cov_xy = sum((p[0] - cx) * (p[1] - cy) for p in points) / len(points)
    cov_yy = sum((p[1] - cy) * (p[1] - cy) for p in points) / len(points)
    angle = 0.5 * math.atan2(2 * cov_xy, cov_xx - cov_yy)
    axis = (math.cos(angle), math.sin(angle))
    projections = [(p[0] - cx) * axis[0] + (p[1] - cy) * axis[1] for p in points]
    lo = min(projections)
    hi = max(projections)
    span = hi - lo
    if span <= 1e-6:
        return dot_candidate(raw, args)
    stroke_width = float(raw["area"]) / span
    if span <= stroke_width * 1.6:
        return dot_candidate(raw, args)
    inset = min(span * 0.25, stroke_width * 0.5)
    a_proj = lo + inset
    b_proj = hi - inset
    if b_proj <= a_proj:
        return dot_candidate(raw, args)
    x1 = cx + axis[0] * a_proj
    y1 = cy + axis[1] * a_proj
    x2 = cx + axis[0] * b_proj
    y2 = cy + axis[1] * b_proj
    d = "M{} {}L{} {}".format(
        fmt_number(x1, args.decimals),
        fmt_number(y1, args.decimals),
        fmt_number(x2, args.decimals),
        fmt_number(y2, args.decimals),
    )
    return CenterlineCandidate(
        d=optimize_d(d, args.decimals),
        stroke_width=stroke_width,
        kind="solid-line",
        segments=1,
        subpaths=1,
        lengths=[point_distance((x1, y1), (x2, y2))],
        max_turn=0,
        sharp_turns=0,
    )


def should_use_all(candidate: CenterlineCandidate, args: argparse.Namespace, outline_loops: int) -> bool:
    if candidate.subpaths <= 1:
        return True
    if outline_loops > 1 and candidate.subpaths <= args.max_loop_subpaths:
        return True
    if candidate.subpaths > args.max_all_subpaths:
        return False
    lengths = sorted(candidate.lengths, reverse=True)
    if len(lengths) < 2:
        return False
    return lengths[1] >= args.branch_min_length


def choose_candidate(path_d: str, args: argparse.Namespace, outline_loops: int) -> CenterlineCandidate:
    if args.radial_closed and outline_loops == 2:
        radial = radial_closed_candidate(path_d, args)
        if radial is not None:
            return radial
    if args.combine_complex and outline_loops >= args.combine_complex_min_loops:
        return combined_complex_candidate(path_d, args)
    probe = branch_probe(path_d, args) if args.preserve_branches else None
    mode = "all" if probe is not None and should_use_all(probe, args, outline_loops) else "longest"
    bridge_gap = args.center_bridge_gap if mode == "all" and outline_loops > 1 else 0.0
    poly = centerline_candidate(path_d, args, mode=mode, polyline=True, simplify=args.linear_simplify, bridge_gap=bridge_gap)
    if poly.segments <= 2:
        return poly
    if mode == "all" and outline_loops > 1 and poly.segments <= args.max_loop_polyline_segments:
        return poly
    if poly.segments <= args.max_polyline_segments and poly.sharp_turns > 0 and poly.max_turn >= args.sharp_turn:
        return poly
    smooth = centerline_candidate(path_d, args, mode=mode, polyline=False, simplify=args.curve_simplify, bridge_gap=bridge_gap)
    return smooth


def convert_png(input_path: Path, output_path: Path, args: argparse.Namespace) -> dict:
    main = normalize_color(args.main)
    accent = normalize_color(args.accent)
    color_order = {main: 0, accent: 1}
    trace = trace_png_components(
        input_path,
        TraceOptions(
            mode="palette",
            palette=(main, accent),
            drop_white=True,
            white_threshold=args.white_threshold,
            alpha_threshold=args.alpha_threshold,
            min_area=args.min_area,
            decimals=args.trace_decimals,
        ),
    )
    raw_components = sorted(
        trace["components"],
        key=lambda c: (color_order.get(c["color"].lower(), 99), -int(c["area"]), c["pixel_bbox"]["y"], c["pixel_bbox"]["x"]),
    )

    components: list[SvgComponent] = []
    errors: list[str] = []
    for raw in raw_components:
        color = raw["color"].lower()
        try:
            candidate = choose_candidate(raw["d"], args, raw["d"].count("M"))
        except Exception as exc:
            candidate = dot_candidate(raw, args) or solid_line_candidate(raw, args)
            if candidate is not None:
                components.append(
                    SvgComponent(
                        color=color,
                        d=candidate.d,
                        stroke_width=candidate.stroke_width,
                        area=int(raw["area"]),
                        bbox=raw["pixel_bbox"],
                        kind=candidate.kind,
                    )
                )
                continue
            errors.append(f"{color} component area={raw['area']}: {exc}")
            if not args.keep_failed:
                continue
            candidate = CenterlineCandidate(
                d=optimize_d(raw["d"], args.decimals),
                stroke_width=float(args.stroke_width or 1),
                kind="filled-fallback",
                segments=0,
                subpaths=0,
                lengths=[],
                max_turn=0,
                sharp_turns=0,
            )
        components.append(
            SvgComponent(
                color=color,
                d=candidate.d,
                stroke_width=candidate.stroke_width,
                area=int(raw["area"]),
                bbox=raw["pixel_bbox"],
                kind=candidate.kind,
            )
        )

    if errors and not args.keep_failed:
        raise RuntimeError(f"{input_path.name}: centerline failed for {len(errors)} component(s): {'; '.join(errors)}")
    if not components:
        raise RuntimeError(f"{input_path.name}: no centerline components generated")

    stroke_width = float(args.stroke_width) if args.stroke_width else round(median(c.stroke_width for c in components))
    svg = build_svg(trace["viewBox"], components, [main, accent], stroke_width, args.decimals)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(svg, encoding="utf-8")
    return {
        "input": str(input_path),
        "output": str(output_path),
        "components": len(components),
        "stroke_width": stroke_width,
        "paths_by_color": {color: sum(1 for c in components if c.color == color) for color in [main, accent]},
        "kinds": {kind: sum(1 for c in components if c.kind == kind) for kind in sorted({c.kind for c in components})},
        "errors": errors,
    }


def build_svg(view_box: str, components: list[SvgComponent], colors: list[str], stroke_width: float, decimals: int) -> str:
    lines = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{escape(view_box)}">',
        f'  <g fill="none" stroke-linecap="round" stroke-linejoin="round" stroke-width="{fmt_number(stroke_width, decimals)}">',
    ]
    for color in colors:
        paths = [component for component in components if component.color == color]
        if not paths:
            continue
        lines.append(f'    <g stroke="{color}">')
        for component in paths:
            lines.append(f'      <path d="{escape(component.d)}"/>')
        lines.append("    </g>")
    lines.append("  </g>")
    lines.append("</svg>")
    return "\n".join(lines) + "\n"


def collect_inputs(input_path: Path) -> list[Path]:
    if input_path.is_file():
        return [input_path]
    return sorted(path for path in input_path.iterdir() if path.suffix.lower() == ".png")


def output_for(input_path: Path, root_input: Path, root_output: Path) -> Path:
    if root_input.is_file():
        if root_output.suffix.lower() == ".svg":
            return root_output
        return root_output / input_path.with_suffix(".svg").name
    return root_output / input_path.with_suffix(".svg").name


def convert_task(task: tuple[Path, Path, argparse.Namespace]) -> dict:
    png, out, args = task
    return convert_png(png, out, args)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Trace two-color PNG line icons into grouped centerline SVGs.")
    parser.add_argument("input", type=Path, help="PNG file or directory of PNG files")
    parser.add_argument("output", type=Path, nargs="?", help="SVG file or output directory")
    parser.add_argument("--main", default=DEFAULT_MAIN, help="Main stroke color")
    parser.add_argument("--accent", default=DEFAULT_ACCENT, help="Accent stroke color")
    parser.add_argument("--stroke-width", type=float, help="Fixed output stroke width; default is per-icon median estimate")
    parser.add_argument("--min-area", type=int, default=80, help="Drop traced raster components smaller than this pixel area")
    parser.add_argument("--min-length", type=float, default=5, help="Drop centerline chains shorter than this SVG-unit length")
    parser.add_argument("--white-threshold", type=int, default=245, help="Pixels at or above this RGB value are treated as background")
    parser.add_argument("--alpha-threshold", type=int, default=16, help="Pixels below this alpha are ignored")
    parser.add_argument("--trace-decimals", type=int, default=1, help="Decimals for intermediate filled component traces")
    parser.add_argument("--decimals", type=int, default=2, help="Decimals for final SVG path data")
    parser.add_argument("--center-scale", type=float, default=1.0, help="Raster scale for centerline reconstruction")
    parser.add_argument("--max-size", type=int, default=1200, help="Maximum centerline raster dimension")
    parser.add_argument("--center-bridge-gap", type=float, default=120.0, help="Rust skeleton bridge gap for multi-loop outlines")
    parser.add_argument("--linear-simplify", type=float, default=2.0, help="Simplification tolerance for straight/sharp components")
    parser.add_argument("--curve-simplify", type=float, default=2.0, help="Simplification tolerance for smoothed curved components")
    parser.add_argument("--radial-samples", type=int, default=160, help="Ray samples for two-loop closed-outline centerlines")
    parser.add_argument("--radial-simplify", type=float, default=2.0, help="Simplification tolerance for radial closed-outline centerlines")
    parser.add_argument("--radial-closed", action="store_true", dest="radial_closed", help="Use radial centerlines for exactly two-loop closed outlines")
    parser.add_argument("--no-radial-closed", action="store_false", dest="radial_closed", help=argparse.SUPPRESS)
    parser.set_defaults(radial_closed=False)
    parser.add_argument("--no-combine-complex", action="store_false", dest="combine_complex", help="Disable longest+all fallback for complex multi-loop components")
    parser.add_argument("--combine-complex-min-loops", type=int, default=3, help="Outline loop count that triggers the complex combined fallback")
    parser.add_argument("--max-polyline-segments", type=int, default=16, help="Choose polyline output only up to this segment count")
    parser.add_argument("--max-all-subpaths", type=int, default=6, help="Use centerline mode=all only when it creates at most this many subpaths")
    parser.add_argument("--max-loop-subpaths", type=int, default=200, help="Use centerline mode=all for multi-loop outlines up to this many subpaths")
    parser.add_argument("--max-loop-polyline-segments", type=int, default=400, help="Keep stitched multi-loop outlines as polylines up to this segment count")
    parser.add_argument("--branch-min-length", type=float, default=50.0, help="Use centerline mode=all only when its second-longest chain is at least this long")
    parser.add_argument("--branch-probe-scale", type=float, default=0.35, help="Low-resolution raster scale for branch detection")
    parser.add_argument("--branch-probe-max-size", type=int, default=500, help="Maximum branch-probe raster dimension")
    parser.add_argument("--stitch-gap", type=float, default=0.0, help="Endpoint distance used to reconnect fragmented all-mode centerline chains; 0 estimates from stroke width")
    parser.add_argument("--no-stitch-all", action="store_false", dest="stitch_all", help="Disable endpoint stitching for all-mode centerline chains")
    parser.add_argument("--no-preserve-branches", action="store_false", dest="preserve_branches", help="Skip branch probing and use longest centerlines only")
    parser.add_argument("--sharp-turn", type=float, default=35.0, help="Turn angle that marks a component as sharp/linear")
    parser.add_argument("--dot-max-size", type=float, default=64.0, help="Maximum bbox size for round dot fallback components")
    parser.add_argument("--dot-max-aspect", type=float, default=1.5, help="Maximum bbox aspect ratio for round dot fallback components")
    parser.add_argument("--solid-line-max-size", type=float, default=160.0, help="Maximum bbox size for compact solid-line fallback components")
    parser.add_argument("--keep-failed", action="store_true", help="Keep filled fallback paths for components that cannot be centerlined")
    parser.add_argument("--report", type=Path, help="Optional JSON report path")
    parser.add_argument("--jobs", type=int, default=1, help="Number of worker processes for directory conversion")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    args.main = normalize_color(args.main)
    args.accent = normalize_color(args.accent)
    input_path = args.input.resolve()
    if args.output:
        output_root = args.output.resolve()
    else:
        output_root = input_path.with_suffix(".svg") if input_path.is_file() else input_path.with_name(input_path.name + "-svg")

    inputs = collect_inputs(input_path)
    if not inputs:
        raise SystemExit(f"No PNG files found in {input_path}")

    tasks = [(png, output_for(png, input_path, output_root), args) for png in inputs]
    report = []
    if args.jobs <= 1 or len(tasks) == 1:
        for png, out, task_args in tasks:
            item = convert_png(png, out, task_args)
            report.append(item)
            print(f"{png.name} -> {out.name} ({item['components']} paths, stroke {fmt_number(item['stroke_width'], args.decimals)})")
    else:
        with ProcessPoolExecutor(max_workers=args.jobs) as executor:
            futures = {executor.submit(convert_task, task): task[0] for task in tasks}
            for future in as_completed(futures):
                png = futures[future]
                item = future.result()
                report.append(item)
                print(f"{png.name} -> {Path(item['output']).name} ({item['components']} paths, stroke {fmt_number(item['stroke_width'], args.decimals)})")
        report.sort(key=lambda item: item["input"])

    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
