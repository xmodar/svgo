"""VTracer-backed raster image tracing."""

from __future__ import annotations

import shutil
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path


class VTracerTraceError(ValueError):
    """Raised when VTracer tracing cannot continue."""


@dataclass(frozen=True)
class VTracerOptions:
    color_mode: str = "color"
    hierarchical: str = "stacked"
    color_precision: int = 6
    gradient_step: int = 16
    filter_speckle: int = 4
    curve_mode: str = "spline"
    corner_threshold: int = 60
    segment_length: float = 4.0
    max_iterations: int = 10
    splice_threshold: int = 45
    path_precision: int = 8


def trace_image_vtracer(path: str | Path, options: VTracerOptions | None = None) -> str:
    options = options or VTracerOptions()
    validate_options(options)
    image_path = Path(path)
    if not image_path.exists():
        raise VTracerTraceError(f"Input file does not exist: {image_path}")

    try:
        return trace_with_python_package(image_path, options)
    except ImportError:
        return trace_with_cli(image_path, options)


def validate_options(options: VTracerOptions) -> None:
    if options.color_mode not in {"color", "binary"}:
        raise VTracerTraceError("--color-mode must be color or binary")
    if options.hierarchical not in {"stacked", "cutout"}:
        raise VTracerTraceError("--hierarchical must be stacked or cutout")
    if options.curve_mode not in {"pixel", "polygon", "spline"}:
        raise VTracerTraceError("--curve-mode must be pixel, polygon, or spline")
    if not 1 <= options.color_precision <= 8:
        raise VTracerTraceError("--color-precision must be between 1 and 8")
    if options.gradient_step < 0:
        raise VTracerTraceError("--gradient-step must be non-negative")
    if options.filter_speckle < 0:
        raise VTracerTraceError("--filter-speckle must be non-negative")
    if options.segment_length <= 0:
        raise VTracerTraceError("--segment-length must be greater than zero")
    if options.max_iterations < 0:
        raise VTracerTraceError("--max-iterations must be non-negative")
    if options.path_precision < 0:
        raise VTracerTraceError("--path-precision must be non-negative")


def trace_with_python_package(image_path: Path, options: VTracerOptions) -> str:
    try:
        import vtracer  # type: ignore[import-not-found]
    except ImportError as exc:
        raise ImportError from exc

    image_format = image_path.suffix.lower().lstrip(".")
    if image_format == "jpg":
        image_format = "jpeg"
    if not image_format:
        image_format = "png"

    kwargs = vtracer_kwargs(options)
    if hasattr(vtracer, "convert_raw_image_to_svg"):
        return str(vtracer.convert_raw_image_to_svg(image_path.read_bytes(), img_format=image_format, **kwargs))

    if hasattr(vtracer, "convert_image_to_svg_py"):
        with tempfile.NamedTemporaryFile(suffix=".svg", delete=False) as tmp:
            output_path = Path(tmp.name)
        try:
            vtracer.convert_image_to_svg_py(str(image_path), str(output_path), **kwargs)
            return output_path.read_text(encoding="utf-8")
        finally:
            try:
                output_path.unlink()
            except OSError:
                pass

    raise VTracerTraceError("Installed vtracer package does not expose a supported SVG conversion API")


def trace_with_cli(image_path: Path, options: VTracerOptions) -> str:
    binary = shutil.which("vtracer")
    if not binary:
        raise VTracerTraceError(
            "svgo trace2 requires the VTracer Python package or vtracer CLI. "
            "Run with `uv run --with vtracer svgo trace2 ...`, install `vtracer`, or use `svgo trace`."
        )

    with tempfile.NamedTemporaryFile(suffix=".svg", delete=False) as tmp:
        output_path = Path(tmp.name)
    try:
        cmd = [
            binary,
            "--input",
            str(image_path),
            "--output",
            str(output_path),
            "--colormode",
            options.color_mode,
            "--hierarchical",
            options.hierarchical,
            "--mode",
            options.curve_mode,
            "--filter_speckle",
            str(options.filter_speckle),
            "--color_precision",
            str(options.color_precision),
            "--layer_difference",
            str(options.gradient_step),
            "--corner_threshold",
            str(options.corner_threshold),
            "--length_threshold",
            str(options.segment_length),
            "--splice_threshold",
            str(options.splice_threshold),
            "--path_precision",
            str(options.path_precision),
        ]
        if options.max_iterations:
            cmd.extend(["--max_iterations", str(options.max_iterations)])
        result = subprocess.run(cmd, capture_output=True, text=True, check=False)
        if result.returncode != 0:
            message = result.stderr.strip() or result.stdout.strip() or f"vtracer exited with code {result.returncode}"
            raise VTracerTraceError(message)
        return output_path.read_text(encoding="utf-8")
    finally:
        try:
            output_path.unlink()
        except OSError:
            pass


def vtracer_kwargs(options: VTracerOptions) -> dict[str, object]:
    return {
        "colormode": options.color_mode,
        "hierarchical": options.hierarchical,
        "mode": "none" if options.curve_mode == "pixel" else options.curve_mode,
        "filter_speckle": options.filter_speckle,
        "color_precision": options.color_precision,
        "layer_difference": options.gradient_step,
        "corner_threshold": options.corner_threshold,
        "length_threshold": options.segment_length,
        "max_iterations": options.max_iterations,
        "splice_threshold": options.splice_threshold,
        "path_precision": options.path_precision,
    }
