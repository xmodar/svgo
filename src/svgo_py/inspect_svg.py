"""SVG validation, information, and structural conversion helpers."""

from __future__ import annotations

import asyncio
import json
import re
import xml.etree.ElementTree as ET
from collections import Counter
from pathlib import Path
from typing import Any

from .svg_optimize import OptimizeOptions, PluginSpec, SvgOptimizeError, local_name, optimize_svg, parse_style

EDITOR_NAMESPACES = ("inkscape", "sodipodi", "sketch", "figma")
SHAPE_NAMES = {"rect", "circle", "ellipse", "line", "polyline", "polygon"}
KNOWN_SVG_ELEMENTS = {
    "svg",
    "g",
    "defs",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "text",
    "tspan",
    "image",
    "style",
    "script",
    "metadata",
    "desc",
    "title",
    "linearGradient",
    "radialGradient",
    "stop",
    "clipPath",
    "mask",
    "pattern",
    "symbol",
    "use",
    "marker",
}


def validate_svg(svg_input: str | Path, *, strict: bool = False) -> dict[str, Any]:
    """Validate SVG XML and report structural warnings."""
    text, source, read_error = read_svg_text(svg_input)
    if read_error:
        return {"valid": False, "issues": [{"level": "error", "reason": read_error}], "error": read_error}

    issues: list[dict[str, str]] = []
    try:
        root = ET.fromstring(text.strip())
    except ET.ParseError as exc:
        reason = f"XML parse error: {exc}"
        return {"valid": False, "issues": [{"level": "error", "reason": reason}], "error": reason}

    if local_name(root.tag) != "svg":
        issues.append({"level": "error", "reason": "Root element is not <svg>"})
    if not (root.attrib.get("viewBox") or (root.attrib.get("width") and root.attrib.get("height"))):
        issues.append({"level": "warning", "reason": "SVG has neither viewBox nor width/height dimensions"})

    for element in root.iter():
        name = local_name(element.tag)
        if name == "script":
            issues.append({"level": "error", "reason": "SVG contains a <script> element"})
        if name not in KNOWN_SVG_ELEMENTS:
            issues.append({"level": "warning", "reason": f"Unknown or uncommon SVG element: {name}"})
        for key, value in element.attrib.items():
            attr = local_name(key)
            if attr.lower().startswith("on"):
                issues.append({"level": "error", "reason": f"Event handler attribute on <{name}>: {attr}"})
            if "href" in attr and value.strip().lower().startswith("javascript:"):
                issues.append({"level": "error", "reason": f"Potentially unsafe href on <{name}>"})
            if attr == "style" and re.search(r"javascript:|vbscript:|expression\s*\(", value, flags=re.I):
                issues.append({"level": "error", "reason": f"Potentially unsafe style on <{name}>"})
            if any(ns in key for ns in EDITOR_NAMESPACES):
                issues.append({"level": "warning", "reason": f"Editor-specific attribute on <{name}>: {attr}"})

    has_error = any(issue["level"] == "error" for issue in issues)
    valid = not issues if strict else not has_error
    return {"valid": valid, "issues": issues, "error": None, "source": source}


async def validate_svg_async(svg_input: str | Path, *, strict: bool = False) -> dict[str, Any]:
    """Validate SVG input in an executor for async batch workflows."""
    loop = asyncio.get_running_loop()
    return await loop.run_in_executor(None, lambda: validate_svg(svg_input, strict=strict))


def get_svg_info(svg_input: str | Path) -> dict[str, Any]:
    """Return structured SVG metadata and element counts."""
    text, source, read_error = read_svg_text(svg_input)
    if read_error:
        return {"error": read_error}
    try:
        root = ET.fromstring(text.strip())
    except ET.ParseError as exc:
        return {"error": f"XML parse error: {exc}"}

    counts = Counter(local_name(element.tag) for element in root.iter())
    fonts = sorted(collect_fonts(root))
    width = root.attrib.get("width")
    height = root.attrib.get("height")
    view_box = root.attrib.get("viewBox")
    return {
        "source": source,
        "width": width,
        "height": height,
        "viewBox": view_box,
        "elements": sum(counts.values()),
        "element_counts": dict(sorted(counts.items())),
        "paths": counts.get("path", 0),
        "shapes": sum(counts.get(name, 0) for name in SHAPE_NAMES),
        "text": counts.get("text", 0) + counts.get("tspan", 0),
        "images": counts.get("image", 0),
        "fonts": fonts,
        "bytes": len(text.encode("utf-8")),
    }


def to_plain_svg(svg_text: str, *, precision: int | None = None) -> str:
    """Remove common editor metadata, editor attributes, comments, and empty containers."""
    return optimize_svg(
        svg_text,
        OptimizeOptions(
            preset="none",
            plugins=[
                PluginSpec("removeComments"),
                PluginSpec("removeMetadata"),
                PluginSpec("removeEditorsNSData"),
                PluginSpec("cleanupAttrs"),
                PluginSpec("removeEmptyContainers"),
                PluginSpec("sortAttrs"),
            ],
            float_precision=precision,
        ),
    )


def sanitize_svg(
    svg_text: str,
    *,
    precision: int | None = None,
    remove_external_refs: bool = False,
    allow_data_images: bool = True,
    remove_styles: bool = False,
    remove_raster_images: bool = False,
) -> str:
    """Remove active content and optionally external references from SVG text."""
    plugins = [
        PluginSpec("removeComments"),
        PluginSpec("removeScripts"),
        PluginSpec("removeScriptElement"),
        PluginSpec("removeEventAttributes"),
        PluginSpec("removeUnsafeLinks", {"removeExternal": remove_external_refs, "allowDataImages": allow_data_images}),
        PluginSpec("cleanupAttrs"),
        PluginSpec("removeEmptyAttrs"),
        PluginSpec("removeEmptyContainers"),
        PluginSpec("sortAttrs"),
    ]
    if remove_styles:
        plugins.insert(3, PluginSpec("removeStyleElement"))
        plugins.append(PluginSpec("removeAttrs", {"attrs": "style"}))
    if remove_raster_images:
        plugins.insert(3, PluginSpec("removeRasterImages"))
    return optimize_svg(svg_text, OptimizeOptions(preset="none", plugins=plugins, float_precision=precision))


def inline_styles_svg(svg_text: str, *, precision: int | None = None, remove_style_elements: bool = True) -> str:
    """Inline simple style-element rules into SVG presentation attributes."""
    return optimize_svg(
        svg_text,
        OptimizeOptions(
            preset="none",
            plugins=[
                PluginSpec("inlineStyles", {"removeStyleElement": remove_style_elements}),
                PluginSpec("convertStyleToAttrs"),
                PluginSpec("cleanupAttrs"),
                PluginSpec("removeEmptyContainers"),
                PluginSpec("sortAttrs"),
            ],
            float_precision=precision,
        ),
    )


def convert_shapes_svg(svg_text: str, *, precision: int | None = None) -> str:
    """Convert SVG basic shapes to path elements."""
    return optimize_svg(
        svg_text,
        OptimizeOptions(preset="none", plugins=[PluginSpec("convertShapeToPath"), PluginSpec("sortAttrs")], float_precision=precision),
    )


def flatten_svg(
    svg_text: str,
    *,
    precision: int | None = None,
    flatten_transforms: bool = True,
    flatten_groups: bool = True,
    shapes_to_paths: bool = True,
    plain: bool = False,
) -> str:
    """Bake supported transforms into coordinates and optionally simplify SVG structure."""
    plugins: list[PluginSpec] = []
    if plain:
        plugins.extend(
            [
                PluginSpec("removeComments"),
                PluginSpec("removeMetadata"),
                PluginSpec("removeEditorsNSData"),
                PluginSpec("cleanupAttrs"),
            ]
        )
    if shapes_to_paths:
        plugins.append(PluginSpec("convertShapeToPath"))
    if flatten_transforms:
        plugins.append(PluginSpec("convertTransform"))
    if flatten_groups:
        plugins.append(PluginSpec("collapseGroups"))
    plugins.extend([PluginSpec("removeEmptyContainers"), PluginSpec("sortAttrs")])
    return optimize_svg(svg_text, OptimizeOptions(preset="none", plugins=plugins, float_precision=precision))


def read_svg_text(svg_input: str | Path) -> tuple[str, str | None, str | None]:
    """Read SVG input from a path or accept raw SVG text."""
    if isinstance(svg_input, Path):
        if not svg_input.exists():
            return "", str(svg_input), f"File not found: {svg_input}"
        return svg_input.read_text(encoding="utf-8"), str(svg_input), None
    text = str(svg_input)
    if text.lstrip().startswith("<"):
        return text, None, None
    path = Path(text)
    if path.exists():
        return path.read_text(encoding="utf-8"), str(path), None
    return text, None, None


def collect_fonts(root: ET.Element) -> set[str]:
    fonts: set[str] = set()
    for element in root.iter():
        if "font-family" in element.attrib:
            fonts.update(split_font_families(element.attrib["font-family"]))
        style = parse_style(element.attrib.get("style"))
        if "font-family" in style:
            fonts.update(split_font_families(style["font-family"]))
        if local_name(element.tag) == "style" and element.text:
            for match in re.finditer(r"font-family\s*:\s*([^;}{]+)", element.text, flags=re.I):
                fonts.update(split_font_families(match.group(1)))
    return fonts


def split_font_families(value: str) -> list[str]:
    return [part.strip().strip("\"'") for part in value.split(",") if part.strip()]


def info_json(svg_input: str | Path, *, indent: int | None = 2) -> str:
    """Serialize SVG info as JSON."""
    return json.dumps(get_svg_info(svg_input), indent=indent, sort_keys=True)


def conversion_error(message: str) -> SvgOptimizeError:
    """Create an optimizer-compatible conversion error."""
    return SvgOptimizeError(message)
