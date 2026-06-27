"""Command-line interface for svgo."""

from __future__ import annotations

import argparse
import copy
import re
import sys
from pathlib import Path

from .centerline import CenterlineError, CenterlineOptions, build_output, centerline_path_data, centerline_svg_text, read_path_data
from .pathdata import PathData, PathDataError
from .raster_trace import RasterTraceError, TraceOptions, trace_png
from .svg_optimize import BUILTIN_PLUGINS, OptimizeOptions, SvgOptimizeError, optimize_svg, parse_plugin_spec


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
    sub.add_parser("plugins", aliases=["l"], help="List built-in optimizer plugin names.")
    return parser


def copy_arguments(source: argparse.ArgumentParser, target: argparse.ArgumentParser) -> None:
    # Argparse has no public clone API. This replays the relevant option actions
    # from small throwaway parsers used by the main subcommands.
    for action in source._actions:  # noqa: SLF001 - intentional argparse interop.
        if not action.option_strings or action.dest == "help":
            continue
        kwargs: dict[str, object] = {
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


def plugin_list_text() -> str:
    return "\n".join(f"{name}\t{'preset' if name == 'preset-default' else 'plugin'}" for name in BUILTIN_PLUGINS)


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
    parser.print_help()
    return 1
