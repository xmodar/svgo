"""Command-line interface for svgo."""

from __future__ import annotations

import argparse
import copy
import json
import re
import sys
from pathlib import Path

from .centerline import CenterlineError, CenterlineOptions, build_output, centerline_path_data, centerline_svg_text, read_path_data
from .inspect_svg import convert_shapes_svg, flatten_svg, get_svg_info, inline_styles_svg, sanitize_svg, to_plain_svg, validate_svg
from .measure import metrics_json, path_metrics, point_at_length, svg_metrics
from .pathdata import PathData, PathDataError
from .raster_trace import RasterTraceError, TraceOptions, trace_png
from .svg_optimize import BUILTIN_PLUGINS, OptimizeOptions, SvgOptimizeError, optimize_svg, parse_plugin_spec
from .viewport import fit_viewbox_svg, resize_svg, set_viewbox_svg


def add_svgo_options(parser: argparse.ArgumentParser, include_flag: bool = True) -> None:
    if include_flag:
        parser.add_argument("--svgo", action="store_true", help="Run Python SVGO-style optimization.")
    parser.add_argument("--svgo-order", choices=("before", "after"), default="after", help="Run optimization before or after path operations.")
    parser.add_argument("--svgo-config", help="Load a JSON or TOML optimizer config.")
    parser.add_argument("--svgo-preset", choices=("default", "none"), default="default", help="Use default preset or only explicit plugins.")
    parser.add_argument("--svgo-plugin", action="append", default=[], help="Add a built-in plugin, optionally NAME:JSON.")
    parser.add_argument("--svgo-disable", action="append", default=[], help="Disable a default preset plugin.")
    parser.add_argument("--svgo-precision", type=int, help="Set global float precision.")
    parser.add_argument("--svgo-multipass", action="store_true", help="Repeat optimization while output shrinks.")
    parser.add_argument("--svgo-pretty", action="store_true", help="Pretty-print SVG output.")
    parser.add_argument("--svgo-indent", type=int, default=2, help="Indent width for pretty output.")
    parser.add_argument("--svgo-eol", choices=("lf", "crlf"), help="Output line ending.")
    parser.add_argument("--svgo-final-newline", action="store_true", help="End SVG output with a newline.")
    parser.add_argument("--svgo-datauri", choices=("base64", "enc", "unenc"), help="Emit a data URI.")
    parser.add_argument("--svgo-list-plugins", action="store_true", help="Print available plugin names.")


def add_path_options(parser: argparse.ArgumentParser) -> None:
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--path", help="Raw SVG path d data.")
    source.add_argument("--input", "-i", help="Input SVG file or text file containing path data.")
    parser.add_argument("--output", "-o", help="Write output to this file instead of stdout.")
    parser.add_argument("--select", default="all", help="Path d attribute selection: all, N, or N,N.")
    parser.add_argument("--op", action="append", default=[], help="Ordered path operation. Repeat to compose operations.")
    parser.add_argument("--decimals", type=int, default=4, help="Decimal places for path output.")
    parser.add_argument("--minify", action="store_true", help="Remove optional path whitespace and leading zeroes.")
    add_svgo_options(parser)


def path_parser(prog: str = "svgo path") -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog=prog,
        description="Edit raw SVG path data or path d attributes in an SVG file.",
    )
    add_path_options(parser)
    return parser


def opt_parser(prog: str = "svgo opt") -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog=prog, description="Optimize an SVG file using the Python SVGO-style optimizer.")
    parser.add_argument("--input", "-i", required=True, help="Input SVG file.")
    parser.add_argument("--output", "-o", help="Write output to this file instead of stdout.")
    add_svgo_options(parser, include_flag=False)
    return parser


def trace_parser(prog: str = "svgo trace") -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog=prog, description="Trace a PNG icon into filled SVG paths.")
    parser.add_argument("--input", "-i", required=True, help="Input PNG file.")
    parser.add_argument("--output", "-o", help="Write SVG output to this file instead of stdout.")
    parser.add_argument("--mode", choices=("palette", "alpha", "exact"), default="palette", help="Tracing mode.")
    parser.add_argument("--alpha-threshold", type=int, default=16, help="Minimum alpha to include a pixel.")
    parser.add_argument("--white-threshold", type=int, default=250, help="RGB threshold for --drop-white.")
    parser.add_argument("--drop-white", action="store_true", help="Treat near-white pixels as background.")
    parser.add_argument("--quantize", type=int, default=24, help="Color bucket size.")
    parser.add_argument("--max-colors", type=int, default=8, help="Maximum dominant colors in palette mode.")
    parser.add_argument("--min-area", type=int, default=4, help="Drop components smaller than this many pixels.")
    parser.add_argument("--scale", type=float, default=1.0, help="Scale output coordinates.")
    parser.add_argument("--decimals", type=int, default=3, help="Decimal places for coordinates.")
    parser.add_argument("--title", help="Optional SVG title.")
    return parser


def center_parser(prog: str = "svgo center") -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog=prog, description="Convert a filled SVG outline to an approximate centerline stroke path.")
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--path", help="Raw SVG path d data.")
    source.add_argument("--input", "-i", help="SVG file or text file containing path data.")
    parser.add_argument("--output", "-o", help="Write output to this file instead of stdout.")
    parser.add_argument("--emit", choices=("path", "svg", "d"), default="path", help="Output path element, full SVG, or d data.")
    parser.add_argument("--mode", choices=("longest", "all"), default="longest", help="Trace longest chain or all chains.")
    parser.add_argument("--scale", type=float, default=2.0, help="Raster pixels per SVG unit before max-size limiting.")
    parser.add_argument("--max-size", type=int, default=1600, help="Maximum raster width or height; 0 disables limiting.")
    parser.add_argument("--curve-samples", type=int, default=24, help="Base samples per curve segment.")
    parser.add_argument("--simplify", type=float, default=6.0, help="Douglas-Peucker tolerance in SVG units.")
    parser.add_argument("--min-length", type=float, default=20.0, help="Drop chains shorter than this SVG-unit length.")
    parser.add_argument("--stroke-width", default="auto", help="Stroke width in SVG units or auto.")
    parser.add_argument("--linecap", default="round", help="stroke-linecap value.")
    parser.add_argument("--linejoin", default="round", help="stroke-linejoin value.")
    parser.add_argument("--decimals", type=int, default=3, help="Decimal places.")
    parser.add_argument("--polyline", action="store_true", help="Emit L commands instead of cubic smoothing.")
    parser.add_argument("--fill-rule", choices=("evenodd",), default="evenodd", help="Rasterizer fill rule.")
    parser.add_argument("--svg-paths", choices=("first", "all"), default="first", help="For SVG input, convert first path or all paths.")
    parser.add_argument("--keep-failed", action="store_true", help="Keep failed original paths in --svg-paths all mode.")
    return parser


def info_parser(prog: str = "svgo info") -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog=prog, description="Print structured SVG metadata and element counts.")
    parser.add_argument("--input", "-i", required=True, help="Input SVG file.")
    parser.add_argument("--output", "-o", help="Write JSON output to this file instead of stdout.")
    parser.add_argument("--compact", action="store_true", help="Emit compact JSON.")
    return parser


def validate_parser(prog: str = "svgo validate") -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog=prog, description="Validate SVG XML and report structural issues.")
    parser.add_argument("--input", "-i", required=True, help="Input SVG file.")
    parser.add_argument("--output", "-o", help="Write validation report to this file instead of stdout.")
    parser.add_argument("--strict", action="store_true", help="Treat warnings as invalid.")
    parser.add_argument("--json", action="store_true", help="Emit JSON instead of a text report.")
    parser.add_argument("--compact", action="store_true", help="Emit compact JSON with --json.")
    return parser


def measure_parser(prog: str = "svgo measure") -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog=prog, description="Measure SVG path length, bounds, and point-at-length data.")
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--path", help="Raw SVG path d data.")
    source.add_argument("--input", "-i", help="Input SVG file or text file containing path data.")
    parser.add_argument("--output", "-o", help="Write JSON output to this file instead of stdout.")
    parser.add_argument("--at", type=float, help="Return point coordinates at this distance along a single path.")
    parser.add_argument("--decimals", type=int, help="Round numeric output to this many decimals.")
    parser.add_argument("--error", type=float, default=0.01, help="Maximum cubic-length approximation error.")
    parser.add_argument("--compact", action="store_true", help="Emit compact JSON.")
    return parser


def sanitize_parser(prog: str = "svgo sanitize") -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog=prog, description="Remove scripts, event handlers, and unsafe links from SVG input.")
    parser.add_argument("--input", "-i", required=True, help="Input SVG file.")
    parser.add_argument("--output", "-o", help="Write output SVG to this file instead of stdout.")
    parser.add_argument("--precision", type=int, help="Numeric precision for generated values.")
    parser.add_argument("--remove-external-refs", action="store_true", help="Remove http(s), protocol-relative, and external CSS URL references.")
    parser.add_argument("--disallow-data-images", action="store_true", help="Remove data: image references as unsafe links.")
    parser.add_argument("--remove-styles", action="store_true", help="Remove style elements and style attributes.")
    parser.add_argument("--remove-raster-images", action="store_true", help="Remove image elements.")
    return parser


def viewbox_parser(prog: str = "svgo viewbox") -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog=prog, description="Set, fit, or resize the root SVG viewBox and dimensions.")
    parser.add_argument("--input", "-i", required=True, help="Input SVG file.")
    parser.add_argument("--output", "-o", help="Write output SVG to this file instead of stdout.")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--set", dest="set_viewbox", help='Set viewBox to "min-x min-y width height".')
    mode.add_argument("--fit-content", action="store_true", help="Fit viewBox to measured geometry bounds.")
    parser.add_argument("--padding", type=float, default=0.0, help="Padding around measured bounds for --fit-content.")
    parser.add_argument("--width", help="Set root width attribute.")
    parser.add_argument("--height", help="Set root height attribute.")
    parser.add_argument("--remove-dimensions", action="store_true", help="Remove root width and height after setting/fitting viewBox.")
    parser.add_argument("--precision", type=int, help="Numeric precision for viewBox values.")
    return parser


def convert_parser(prog: str = "svgo convert") -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog=prog, description="Convert SVG structure: plain cleanup, shape-to-path conversion, and transform flattening.")
    parser.add_argument("--input", "-i", required=True, help="Input SVG file.")
    parser.add_argument("--output", "-o", help="Write output SVG to this file instead of stdout.")
    parser.add_argument("--precision", type=int, help="Numeric precision for generated path data.")
    parser.add_argument("--to-plain", action="store_true", help="Remove editor metadata and editor-specific attributes.")
    parser.add_argument("--shapes-to-paths", action="store_true", help="Convert rect/circle/ellipse/line/polyline/polygon to path elements.")
    parser.add_argument("--flatten-transforms", action="store_true", help="Bake supported transforms into coordinates.")
    parser.add_argument("--flatten-groups", action="store_true", help="Collapse empty unstyled groups.")
    parser.add_argument("--inline-styles", action="store_true", help="Inline simple style-element rules into presentation attributes.")
    parser.add_argument("--sanitize", action="store_true", help="Remove scripts, event handlers, and unsafe links before conversion.")
    parser.add_argument("--all", action="store_true", help="Enable all conversion passes.")
    return parser


def build_main_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="svgo", description="Pure-Python SVG operations CLI.")
    sub = parser.add_subparsers(dest="command")
    path_sub = sub.add_parser("path", aliases=["p"], help="Edit path data or path d attributes.")
    add_path_options(path_sub)
    opt_sub = sub.add_parser("opt", aliases=["o"], help="Optimize an SVG file.")
    opt_sub.add_argument("--input", "-i", required=True, help="Input SVG file.")
    opt_sub.add_argument("--output", "-o", help="Write output to this file instead of stdout.")
    add_svgo_options(opt_sub, include_flag=False)
    trace_sub = sub.add_parser("trace", aliases=["t"], help="Trace a PNG into SVG.")
    copy_arguments(trace_parser(), trace_sub)
    center_sub = sub.add_parser("center", aliases=["c"], help="Reconstruct centerline strokes.")
    copy_arguments(center_parser(), center_sub)
    info_sub = sub.add_parser("info", aliases=["i"], help="Show SVG metadata and element counts.")
    copy_arguments(info_parser(), info_sub)
    validate_sub = sub.add_parser("validate", aliases=["v"], help="Validate SVG XML and report issues.")
    copy_arguments(validate_parser(), validate_sub)
    measure_sub = sub.add_parser("measure", aliases=["m"], help="Measure path/SVG bounds and lengths.")
    copy_arguments(measure_parser(), measure_sub)
    sanitize_sub = sub.add_parser("sanitize", aliases=["s"], help="Remove active or unsafe SVG content.")
    copy_arguments(sanitize_parser(), sanitize_sub)
    viewbox_sub = sub.add_parser("viewbox", aliases=["b"], help="Set, fit, or resize root SVG viewport data.")
    copy_arguments(viewbox_parser(), viewbox_sub)
    convert_sub = sub.add_parser("convert", aliases=["x"], help="Convert shapes, flatten transforms, or remove editor data.")
    copy_arguments(convert_parser(), convert_sub)
    sub.add_parser("plugins", aliases=["l"], help="List built-in optimizer plugin names.")
    return parser


def copy_arguments(source: argparse.ArgumentParser, target: argparse.ArgumentParser) -> None:
    # Argparse has no public clone API. This replays the relevant option actions
    # from small throwaway parsers used by the main subcommands.
    for action in source._actions:  # noqa: SLF001 - intentional argparse interop.
        if not action.option_strings or action.dest == "help":
            continue
        kwargs: dict[str, object] = {
            "dest": action.dest,
            "help": action.help,
            "default": action.default,
            "required": action.required,
        }
        if isinstance(action, argparse._StoreAction):  # noqa: SLF001
            kwargs["type"] = action.type
            kwargs["choices"] = action.choices
            kwargs["nargs"] = action.nargs
        elif isinstance(action, argparse._StoreTrueAction):  # noqa: SLF001
            kwargs["action"] = "store_true"
        target.add_argument(*action.option_strings, **{k: v for k, v in kwargs.items() if v is not None})


def build_optimize_options(args: argparse.Namespace, force: bool = False) -> OptimizeOptions:
    return OptimizeOptions(
        preset=getattr(args, "svgo_preset", "default"),
        plugins=[parse_plugin_spec(spec) for spec in getattr(args, "svgo_plugin", [])],
        disabled=set(getattr(args, "svgo_disable", [])),
        float_precision=getattr(args, "svgo_precision", None),
        multipass=bool(getattr(args, "svgo_multipass", False)),
        pretty=bool(getattr(args, "svgo_pretty", False)),
        indent=int(getattr(args, "svgo_indent", 2)),
        eol=getattr(args, "svgo_eol", None),
        final_newline=bool(getattr(args, "svgo_final_newline", False)),
        datauri=getattr(args, "svgo_datauri", None),
        config=getattr(args, "svgo_config", None),
    )


def should_run_svgo(args: argparse.Namespace, force: bool = False) -> bool:
    return force or bool(getattr(args, "svgo", False))


def edit_path_data(path_data: str, args: argparse.Namespace) -> str:
    result = path_data.strip()
    if should_run_svgo(args) and args.svgo_order == "before":
        result = optimize_path_data(result, args)
    path = PathData.parse(result)
    for op in args.op:
        path.apply_operation(op, args.decimals)
    result = path.to_string(args.decimals, args.minify)
    if should_run_svgo(args) and args.svgo_order == "after":
        result = optimize_path_data(result, args)
    return result


def optimize_path_data(path_data: str, args: argparse.Namespace) -> str:
    if getattr(args, "svgo_datauri", None):
        raise SvgOptimizeError("--svgo-datauri cannot be used when optimizing raw path d data")
    svg = f'<svg xmlns="http://www.w3.org/2000/svg"><path d="{escape_attr(path_data)}"/></svg>'
    optimized = optimize_svg(svg, build_optimize_options(args))
    match = re.search(r"\bd\s*=\s*([\"'])([\s\S]*?)\1", optimized)
    if not match:
        raise SvgOptimizeError("Optimizer removed or could not return a path d attribute")
    return match.group(2)


def selected_indexes(select: str, count: int) -> set[int]:
    if select == "all":
        return set(range(count))
    indexes: set[int] = set()
    for part in select.split(","):
        part = part.strip()
        if not part:
            continue
        try:
            index = int(part)
        except ValueError as exc:
            raise PathDataError(f"select index must be a non-negative integer: {part}") from exc
        if index < 0 or index >= count:
            raise PathDataError(f"select index {index} is out of range; file has {count} path d attributes")
        indexes.add(index)
    return indexes


def edit_svg_path_attributes(text: str, args: argparse.Namespace) -> str:
    matches = list(re.finditer(r"\bd\s*=\s*([\"'])([\s\S]*?)\1", text))
    if not matches and args.op:
        raise PathDataError("No path d attributes found in SVG input")
    selected = selected_indexes(args.select, len(matches))
    output: list[str] = []
    cursor = 0
    for index, match in enumerate(matches):
        quote = match.group(1)
        path_data = match.group(2)
        output.append(text[cursor : match.start()])
        if index in selected and args.op:
            child_args = copy.copy(args)
            child_args.svgo = False
            edited = edit_path_data(path_data, child_args)
            output.append(f"d={quote}{edited}{quote}")
        else:
            output.append(match.group(0))
        cursor = match.end()
    output.append(text[cursor:])
    return "".join(output)


def edit_svg_text(text: str, args: argparse.Namespace) -> str:
    result = text
    if should_run_svgo(args) and args.svgo_order == "before":
        result = optimize_svg(result, build_optimize_options(args))
    result = edit_svg_path_attributes(result, args)
    if should_run_svgo(args) and args.svgo_order == "after":
        result = optimize_svg(result, build_optimize_options(args))
    return result


def run_path(args: argparse.Namespace) -> str:
    if args.svgo_list_plugins:
        return plugin_list_text()
    if args.path and args.input:
        raise PathDataError("Use either --path or --input, not both")
    if not args.path and not args.input:
        raise PathDataError("Provide --path or --input")
    if args.svgo_datauri and args.svgo_order == "before" and args.op:
        raise SvgOptimizeError("--svgo-datauri cannot run before path operations")

    if args.path:
        return edit_path_data(args.path, args)
    text = Path(args.input).read_text(encoding="utf-8")
    if re.search(r"<path\b|<svg\b", text, flags=re.I):
        return edit_svg_text(text, args)
    return edit_path_data(text.strip(), args)


def run_optimize(args: argparse.Namespace) -> str:
    if args.svgo_list_plugins:
        return plugin_list_text()
    text = Path(args.input).read_text(encoding="utf-8")
    return optimize_svg(text, build_optimize_options(args, force=True))


def run_trace(args: argparse.Namespace) -> str:
    options = TraceOptions(
        mode=args.mode,
        alpha_threshold=args.alpha_threshold,
        white_threshold=args.white_threshold,
        drop_white=args.drop_white,
        quantize=args.quantize,
        max_colors=args.max_colors,
        min_area=args.min_area,
        scale=args.scale,
        decimals=args.decimals,
        title=args.title,
    )
    return trace_png(args.input, options)


def centerline_options(args: argparse.Namespace) -> CenterlineOptions:
    return CenterlineOptions(
        emit=args.emit,
        mode=args.mode,
        scale=args.scale,
        max_size=args.max_size,
        curve_samples=args.curve_samples,
        simplify=args.simplify,
        min_length=args.min_length,
        stroke_width=args.stroke_width,
        linecap=args.linecap,
        linejoin=args.linejoin,
        decimals=args.decimals,
        polyline=args.polyline,
        fill_rule=args.fill_rule,
        svg_paths=args.svg_paths,
        keep_failed=args.keep_failed,
    )


def run_centerline(args: argparse.Namespace) -> str:
    if bool(args.path) == bool(args.input):
        raise CenterlineError("Provide exactly one of --path or --input")
    options = centerline_options(args)
    if args.input and args.svg_paths == "all":
        text = Path(args.input).read_text(encoding="utf-8").strip()
        if "<svg" not in text[:500].lower():
            raise CenterlineError("--svg-paths all requires an SVG input file")
        return centerline_svg_text(text, options)
    path_data = args.path.strip() if args.path else read_path_data(args.input)
    d, stroke_width, ctx = centerline_path_data(path_data, options)
    return build_output(d, args.emit, stroke_width, options, ctx)


def run_info(args: argparse.Namespace) -> str:
    info = get_svg_info(Path(args.input))
    if "error" in info:
        raise SvgOptimizeError(str(info["error"]))
    return json.dumps(info, indent=None if args.compact else 2, sort_keys=True)


def run_validate(args: argparse.Namespace) -> tuple[str, bool]:
    result = validate_svg(Path(args.input), strict=args.strict)
    if args.json:
        return json.dumps(result, indent=None if args.compact else 2, sort_keys=True), bool(result["valid"])
    return format_validation_result(result), bool(result["valid"])


def run_measure(args: argparse.Namespace) -> str:
    error = max(float(args.error), 1e-9)
    if args.path:
        result = path_metrics(args.path, decimals=args.decimals, error=error)
        if args.at is not None:
            point = point_at_length(args.path, args.at, error=error)
            if args.decimals is not None:
                point = {key: round(value, args.decimals) for key, value in point.items()}
            result["point"] = point
        return metrics_json(result, compact=args.compact)

    text = Path(args.input).read_text(encoding="utf-8")
    if re.search(r"<path\b|<svg\b|<rect\b|<circle\b|<ellipse\b|<line\b|<polyline\b|<polygon\b", text, flags=re.I):
        result = svg_metrics(text, decimals=args.decimals, error=error)
        if args.at is not None:
            paths = result.get("paths")
            if not isinstance(paths, list) or len(paths) != 1:
                raise SvgOptimizeError("--at requires raw path input or an SVG with exactly one measurable path")
            d = paths[0].get("d")
            if not isinstance(d, str) or not d:
                raise SvgOptimizeError("--at could not read the measured path data")
            point = point_at_length(d, args.at, error=error)
            if args.decimals is not None:
                point = {key: round(value, args.decimals) for key, value in point.items()}
            result["point"] = point
        return metrics_json(result, compact=args.compact)

    result = path_metrics(text.strip(), decimals=args.decimals, error=error)
    if args.at is not None:
        point = point_at_length(text.strip(), args.at, error=error)
        if args.decimals is not None:
            point = {key: round(value, args.decimals) for key, value in point.items()}
        result["point"] = point
    return metrics_json(result, compact=args.compact)


def run_sanitize(args: argparse.Namespace) -> str:
    text = Path(args.input).read_text(encoding="utf-8")
    return sanitize_svg(
        text,
        precision=args.precision,
        remove_external_refs=args.remove_external_refs,
        allow_data_images=not args.disallow_data_images,
        remove_styles=args.remove_styles,
        remove_raster_images=args.remove_raster_images,
    )


def run_viewbox(args: argparse.Namespace) -> str:
    text = Path(args.input).read_text(encoding="utf-8")
    if args.fit_content:
        text = fit_viewbox_svg(text, padding=args.padding, precision=args.precision, remove_dimensions=args.remove_dimensions)
    elif args.set_viewbox:
        text = set_viewbox_svg(text, args.set_viewbox, precision=args.precision, remove_dimensions=args.remove_dimensions)
    elif args.remove_dimensions:
        raise SvgOptimizeError("--remove-dimensions requires --set or --fit-content")

    if args.width is not None or args.height is not None:
        text = resize_svg(text, width=args.width, height=args.height)
    if not (args.fit_content or args.set_viewbox or args.width is not None or args.height is not None):
        raise SvgOptimizeError("Provide --set, --fit-content, --width, or --height")
    return text


def run_convert(args: argparse.Namespace) -> str:
    text = Path(args.input).read_text(encoding="utf-8")
    explicit = args.to_plain or args.shapes_to_paths or args.flatten_transforms or args.flatten_groups or args.inline_styles or args.sanitize or args.all
    plain = args.to_plain or args.all
    shapes_to_paths = args.shapes_to_paths or args.all or not explicit
    flatten_transforms = args.flatten_transforms or args.all
    flatten_groups = args.flatten_groups or args.all
    if args.sanitize or args.all:
        text = sanitize_svg(text, precision=args.precision)
    if args.inline_styles or args.all:
        text = inline_styles_svg(text, precision=args.precision)

    if plain and not (shapes_to_paths or flatten_transforms or flatten_groups):
        return to_plain_svg(text, precision=args.precision)
    if shapes_to_paths and not (plain or flatten_transforms or flatten_groups):
        return convert_shapes_svg(text, precision=args.precision)
    return flatten_svg(
        text,
        precision=args.precision,
        flatten_transforms=flatten_transforms,
        flatten_groups=flatten_groups,
        shapes_to_paths=shapes_to_paths,
        plain=plain,
    )


def plugin_list_text() -> str:
    return "\n".join(f"{name}\t{'preset' if name == 'preset-default' else 'plugin'}" for name in BUILTIN_PLUGINS)


def format_validation_result(result: dict[str, object]) -> str:
    lines = ["valid" if result.get("valid") else "invalid"]
    issues = result.get("issues")
    if isinstance(issues, list):
        for issue in issues:
            if isinstance(issue, dict):
                level = str(issue.get("level", "issue"))
                reason = str(issue.get("reason", ""))
                lines.append(f"{level}: {reason}")
    error = result.get("error")
    if error and not issues:
        lines.append(f"error: {error}")
    return "\n".join(lines)


def escape_attr(value: str) -> str:
    return value.replace("&", "&amp;").replace('"', "&quot;").replace("<", "&lt;").replace(">", "&gt;")


def write_or_print(text: str, output: str | None) -> None:
    if output:
        Path(output).write_text(text + ("" if text.endswith("\n") else "\n"), encoding="utf-8")
    else:
        sys.stdout.write(text + ("" if text.endswith("\n") else "\n"))


def handle_errors(prefix: str, fn, args: argparse.Namespace) -> int:
    try:
        text = fn(args)
        write_or_print(text, getattr(args, "output", None))
        return 0
    except (PathDataError, RasterTraceError, CenterlineError, SvgOptimizeError, OSError) as exc:
        print(f"{prefix}: {exc}", file=sys.stderr)
        return 1


def handle_validate(args: argparse.Namespace) -> int:
    try:
        text, valid = run_validate(args)
        write_or_print(text, getattr(args, "output", None))
        return 0 if valid else 1
    except OSError as exc:
        print(f"svgo validate: {exc}", file=sys.stderr)
        return 1


def main(argv: list[str] | None = None) -> int:
    parser = build_main_parser()
    args = parser.parse_args(argv)
    if not args.command:
        parser.print_help()
        return 0
    if args.command in {"plugins", "l"}:
        print(plugin_list_text())
        return 0
    if args.command in {"path", "p"}:
        return handle_errors("svgo path", run_path, args)
    if args.command in {"opt", "o"}:
        return handle_errors("svgo opt", run_optimize, args)
    if args.command in {"trace", "t"}:
        return handle_errors("svgo trace", run_trace, args)
    if args.command in {"center", "c"}:
        return handle_errors("svgo center", run_centerline, args)
    if args.command in {"info", "i"}:
        return handle_errors("svgo info", run_info, args)
    if args.command in {"validate", "v"}:
        return handle_validate(args)
    if args.command in {"measure", "m"}:
        return handle_errors("svgo measure", run_measure, args)
    if args.command in {"sanitize", "s"}:
        return handle_errors("svgo sanitize", run_sanitize, args)
    if args.command in {"viewbox", "b"}:
        return handle_errors("svgo viewbox", run_viewbox, args)
    if args.command in {"convert", "x"}:
        return handle_errors("svgo convert", run_convert, args)
    parser.print_help()
    return 1
