"""Pure-Python SVGO-style SVG cleanup and minification."""

from __future__ import annotations

import base64
import json
import re
import urllib.parse
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from .pathdata import PathData, PathDataError, fmt_number, parse_transform

SVG_NS = "http://www.w3.org/2000/svg"
XLINK_NS = "http://www.w3.org/1999/xlink"

ET.register_namespace("", SVG_NS)
ET.register_namespace("xlink", XLINK_NS)

DEFAULT_PRESET_PLUGINS = [
    "removeDoctype",
    "removeXMLProcInst",
    "removeComments",
    "removeDeprecatedAttrs",
    "removeMetadata",
    "removeEditorsNSData",
    "cleanupAttrs",
    "mergeStyles",
    "inlineStyles",
    "minifyStyles",
    "cleanupIds",
    "removeUselessDefs",
    "cleanupNumericValues",
    "convertColors",
    "removeUnknownsAndDefaults",
    "removeNonInheritableGroupAttrs",
    "removeUselessStrokeAndFill",
    "cleanupEnableBackground",
    "removeHiddenElems",
    "removeEmptyText",
    "convertShapeToPath",
    "convertEllipseToCircle",
    "moveElemsAttrsToGroup",
    "moveGroupAttrsToElems",
    "collapseGroups",
    "convertPathData",
    "convertTransform",
    "removeEmptyAttrs",
    "removeEmptyContainers",
    "mergePaths",
    "removeUnusedNS",
    "sortAttrs",
    "sortDefsChildren",
    "removeDesc",
]

EXTRA_PLUGINS = [
    "addAttributesToSVGElement",
    "addClassesToSVGElement",
    "cleanupListOfValues",
    "convertOneStopGradients",
    "convertStyleToAttrs",
    "prefixIds",
    "removeAttributesBySelector",
    "removeAttrs",
    "removeDimensions",
    "removeElementsByAttr",
    "removeOffCanvasPaths",
    "removeRasterImages",
    "removeScripts",
    "removeStyleElement",
    "removeTitle",
    "removeViewBox",
    "removeXlink",
    "removeXMLNS",
    "reusePaths",
]

BUILTIN_PLUGINS = DEFAULT_PRESET_PLUGINS + [name for name in EXTRA_PLUGINS if name not in DEFAULT_PRESET_PLUGINS]


class SvgOptimizeError(ValueError):
    """Raised when SVG optimization cannot continue."""


@dataclass(frozen=True)
class PluginSpec:
    name: str
    params: dict[str, Any] = field(default_factory=dict)


@dataclass
class OptimizeOptions:
    preset: str = "default"
    plugins: list[PluginSpec | str] = field(default_factory=list)
    disabled: set[str] = field(default_factory=set)
    float_precision: int | None = None
    multipass: bool = False
    pretty: bool = False
    indent: int = 2
    eol: str | None = None
    final_newline: bool = False
    datauri: str | None = None
    config: str | Path | None = None


def parse_plugin_spec(spec: str) -> PluginSpec:
    if ":" not in spec:
        return PluginSpec(spec)
    name, raw = spec.split(":", 1)
    try:
        params = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise SvgOptimizeError(f"Invalid JSON params for plugin {name}: {exc}") from exc
    if not isinstance(params, dict):
        raise SvgOptimizeError(f"Plugin params for {name} must be a JSON object")
    return PluginSpec(name, params)


def load_config(path: str | Path) -> dict[str, Any]:
    source = Path(path)
    suffix = source.suffix.lower()
    text = source.read_text(encoding="utf-8")
    if suffix == ".json":
        data = json.loads(text)
    elif suffix == ".toml":
        import tomllib

        data = tomllib.loads(text)
    else:
        raise SvgOptimizeError("--svgo-config supports JSON and TOML files in the Python implementation")
    if not isinstance(data, dict):
        raise SvgOptimizeError("SVGO config must be an object")
    return data


def merge_config(options: OptimizeOptions) -> OptimizeOptions:
    if not options.config:
        return options
    data = load_config(options.config)
    merged = OptimizeOptions(**{**options.__dict__})
    if "floatPrecision" in data:
        merged.float_precision = int(data["floatPrecision"])
    if "multipass" in data:
        merged.multipass = bool(data["multipass"])
    if "datauri" in data:
        merged.datauri = str(data["datauri"])
    if "plugins" in data and isinstance(data["plugins"], list):
        merged.plugins.extend(normalize_plugin(item) for item in data["plugins"])
    js2svg = data.get("js2svg")
    if isinstance(js2svg, dict):
        merged.pretty = bool(js2svg.get("pretty", merged.pretty))
        if "indent" in js2svg:
            merged.indent = int(js2svg["indent"])
        if "eol" in js2svg:
            merged.eol = str(js2svg["eol"])
        if "finalNewline" in js2svg:
            merged.final_newline = bool(js2svg["finalNewline"])
    return merged


def normalize_plugin(plugin: PluginSpec | str | dict[str, Any]) -> PluginSpec:
    if isinstance(plugin, PluginSpec):
        return plugin
    if isinstance(plugin, str):
        return parse_plugin_spec(plugin)
    if isinstance(plugin, dict):
        name = plugin.get("name")
        if not isinstance(name, str):
            raise SvgOptimizeError("Plugin object requires a string name")
        params = plugin.get("params", {})
        if not isinstance(params, dict):
            raise SvgOptimizeError(f"Plugin params for {name} must be an object")
        return PluginSpec(name, params)
    raise SvgOptimizeError(f"Unsupported plugin spec: {plugin!r}")


def effective_plugins(options: OptimizeOptions) -> list[PluginSpec]:
    plugins: list[PluginSpec] = []
    if options.preset == "default":
        plugins.extend(PluginSpec(name) for name in DEFAULT_PRESET_PLUGINS if name not in options.disabled)
    elif options.preset != "none":
        raise SvgOptimizeError("--svgo-preset must be default or none")
    plugins.extend(normalize_plugin(plugin) for plugin in options.plugins)
    return plugins


def optimize_svg(svg_text: str, options: OptimizeOptions | None = None) -> str:
    options = merge_config(options or OptimizeOptions())
    if options.float_precision is not None:
        options.float_precision = min(max(0, int(options.float_precision)), 20)

    passes = 10 if options.multipass else 1
    result = svg_text
    for _ in range(passes):
        previous = result
        result = optimize_once(result, options)
        if not options.multipass or len(result) >= len(previous):
            if len(result) >= len(previous):
                result = previous if previous.strip() else result
            break

    if options.datauri:
        result = to_data_uri(result, options.datauri)
    if options.eol == "crlf":
        result = result.replace("\n", "\r\n")
    elif options.eol == "lf":
        result = result.replace("\r\n", "\n")
    if options.final_newline and not result.endswith(("\n", "\r\n")):
        result += "\r\n" if options.eol == "crlf" else "\n"
    return result


def optimize_once(svg_text: str, options: OptimizeOptions) -> str:
    plugins = effective_plugins(options)
    names = {plugin.name for plugin in plugins}
    text = svg_text
    if "removeDoctype" in names:
        text = re.sub(r"<!DOCTYPE[\s\S]*?>", "", text, flags=re.I)
    if "removeXMLProcInst" in names:
        text = re.sub(r"<\?xml[\s\S]*?\?>", "", text, flags=re.I)
    if "removeComments" in names:
        text = re.sub(r"<!--[\s\S]*?-->", "", text)

    try:
        root = ET.fromstring(text.strip())
    except ET.ParseError as exc:
        raise SvgOptimizeError(f"Could not parse SVG: {exc}") from exc

    for plugin in plugins:
        apply_plugin(root, plugin, options)

    if options.pretty:
        ET.indent(root, space=" " * max(0, options.indent))
    out = ET.tostring(root, encoding="unicode", short_empty_elements=True)
    if not options.pretty:
        out = re.sub(r">\s+<", "><", out)
        out = out.replace(" />", "/>")
    if "removeXMLNS" in names:
        out = re.sub(r'\s+xmlns(:\w+)?="[^"]*"', "", out)
    return out.strip()


def apply_plugin(root: ET.Element, plugin: PluginSpec, options: OptimizeOptions) -> None:
    name = plugin.name
    params = plugin.params
    if name not in BUILTIN_PLUGINS and name != "preset-default":
        raise SvgOptimizeError(f"Unknown SVGO plugin: {name}")
    if name in {"removeDoctype", "removeXMLProcInst", "removeComments", "removeUnusedNS", "reusePaths", "preset-default"}:
        return
    if name in {"removeMetadata", "removeDesc", "removeTitle", "removeScripts", "removeStyleElement", "removeRasterImages"}:
        tags = {
            "removeMetadata": {"metadata"},
            "removeDesc": {"desc"},
            "removeTitle": {"title"},
            "removeScripts": {"script"},
            "removeStyleElement": {"style"},
            "removeRasterImages": {"image"},
        }[name]
        remove_by_local_name(root, tags)
    elif name == "removeEditorsNSData":
        remove_editor_attrs(root)
    elif name == "cleanupAttrs":
        cleanup_attrs(root)
    elif name in {"mergeStyles", "minifyStyles", "inlineStyles"}:
        minify_styles(root)
    elif name == "cleanupIds":
        cleanup_ids(root)
    elif name == "removeUselessDefs":
        remove_empty_defs(root)
    elif name in {"cleanupNumericValues", "cleanupListOfValues"}:
        cleanup_numeric_values(root, options.float_precision)
    elif name == "convertColors":
        convert_colors(root)
    elif name == "removeUnknownsAndDefaults":
        remove_defaults(root)
    elif name == "removeNonInheritableGroupAttrs":
        remove_non_inheritable_group_attrs(root)
    elif name == "removeUselessStrokeAndFill":
        remove_useless_stroke_fill(root)
    elif name == "cleanupEnableBackground":
        remove_attr_everywhere(root, "enable-background")
    elif name == "removeHiddenElems":
        remove_hidden(root)
    elif name == "removeEmptyText":
        remove_empty_text(root)
    elif name in {"convertShapeToPath", "convertEllipseToCircle"}:
        convert_shapes_to_paths(root, options)
    elif name == "convertTransform":
        convert_transforms(root, options)
    elif name == "convertPathData":
        convert_path_data(root, options)
    elif name == "removeEmptyAttrs":
        remove_empty_attrs(root)
    elif name == "removeEmptyContainers":
        remove_empty_containers(root)
    elif name == "collapseGroups":
        collapse_groups(root)
    elif name in {"moveElemsAttrsToGroup", "moveGroupAttrsToElems", "convertOneStopGradients"}:
        return
    elif name == "mergePaths":
        merge_paths(root)
    elif name == "sortAttrs":
        sort_attrs(root)
    elif name == "sortDefsChildren":
        sort_defs_children(root)
    elif name == "addAttributesToSVGElement":
        for key, value in params.items():
            root.set(str(key), str(value))
    elif name == "addClassesToSVGElement":
        classes = params.get("classNames") or params.get("classes") or params.get("class")
        if isinstance(classes, str):
            classes = [classes]
        if isinstance(classes, list):
            existing = root.attrib.get("class", "")
            root.set("class", " ".join(part for part in [existing, *map(str, classes)] if part).strip())
    elif name == "convertStyleToAttrs":
        convert_style_to_attrs(root)
    elif name == "prefixIds":
        prefix_ids(root, str(params.get("prefix", "prefix")))
    elif name == "removeAttrs":
        remove_attrs_plugin(root, params)
    elif name == "removeAttributesBySelector":
        remove_attrs_plugin(root, params)
    elif name == "removeDimensions":
        root.attrib.pop("width", None)
        root.attrib.pop("height", None)
    elif name == "removeElementsByAttr":
        remove_elements_by_attr(root, params)
    elif name == "removeOffCanvasPaths":
        return
    elif name == "removeViewBox":
        root.attrib.pop("viewBox", None)
    elif name == "removeXlink":
        remove_xlink(root)
    elif name == "removeXMLNS":
        return


def local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def namespace(tag: str) -> str:
    return tag[1:].split("}", 1)[0] if tag.startswith("{") else ""


def qname(ns: str, name: str) -> str:
    return f"{{{ns}}}{name}" if ns else name


def walk(root: ET.Element) -> list[ET.Element]:
    return list(root.iter())


def parent_map(root: ET.Element) -> dict[ET.Element, ET.Element]:
    return {child: parent for parent in root.iter() for child in list(parent)}


def remove_by_local_name(root: ET.Element, names: set[str]) -> None:
    parents = parent_map(root)
    for element in list(root.iter()):
        if element is root:
            continue
        if local_name(element.tag) in names:
            parents[element].remove(element)


def remove_editor_attrs(root: ET.Element) -> None:
    editor_namespaces = ("inkscape", "sodipodi", "sketch", "figma")
    for element in walk(root):
        for key in list(element.attrib):
            if any(ns in key for ns in editor_namespaces) or key.startswith("data-"):
                element.attrib.pop(key, None)


def cleanup_attrs(root: ET.Element) -> None:
    for element in walk(root):
        for key, value in list(element.attrib.items()):
            cleaned = " ".join(value.split())
            element.set(key, cleaned)


def minify_styles(root: ET.Element) -> None:
    for element in walk(root):
        style = element.attrib.get("style")
        if not style:
            continue
        parts = []
        for part in style.split(";"):
            if ":" not in part:
                continue
            key, value = part.split(":", 1)
            parts.append(f"{key.strip()}:{value.strip()}")
        if parts:
            element.set("style", ";".join(parts))
        else:
            element.attrib.pop("style", None)


def parse_style(style: str | None) -> dict[str, str]:
    result: dict[str, str] = {}
    if not style:
        return result
    for part in style.split(";"):
        if ":" not in part:
            continue
        key, value = part.split(":", 1)
        result[key.strip()] = value.strip()
    return result


def style_text(style: dict[str, str]) -> str:
    return ";".join(f"{key}:{value}" for key, value in style.items())


def cleanup_ids(root: ET.Element) -> None:
    serialized = ET.tostring(root, encoding="unicode")
    for element in walk(root):
        ident = element.attrib.get("id")
        if not ident:
            continue
        references = serialized.count(f"#{ident}") + serialized.count(f"url(&quot;#{ident}&quot;)") + serialized.count(f"url('#{ident}')")
        if references == 0:
            element.attrib.pop("id", None)


def remove_empty_defs(root: ET.Element) -> None:
    changed = True
    while changed:
        changed = False
        parents = parent_map(root)
        for element in list(root.iter()):
            if element is root:
                continue
            if local_name(element.tag) == "defs" and len(list(element)) == 0:
                parents[element].remove(element)
                changed = True


def cleanup_numeric_values(root: ET.Element, precision: int | None) -> None:
    decimals = 4 if precision is None else precision
    numeric_attr_names = {
        "x",
        "y",
        "x1",
        "y1",
        "x2",
        "y2",
        "cx",
        "cy",
        "r",
        "rx",
        "ry",
        "width",
        "height",
        "stroke-width",
        "opacity",
        "fill-opacity",
        "stroke-opacity",
    }
    for element in walk(root):
        for key, value in list(element.attrib.items()):
            lname = local_name(key)
            if lname == "d":
                continue
            if lname in numeric_attr_names or re.fullmatch(r"[-+0-9.eE\s,]+", value):
                element.set(key, reformat_number_list(value, decimals))


def reformat_number_list(value: str, decimals: int) -> str:
    def repl(match: re.Match[str]) -> str:
        return fmt_number(float(match.group(0)), decimals, True)

    return re.sub(r"[-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?", repl, value)


def convert_colors(root: ET.Element) -> None:
    for element in walk(root):
        for key, value in list(element.attrib.items()):
            element.set(key, min_color(value))
        style = parse_style(element.attrib.get("style"))
        if style:
            for key, value in list(style.items()):
                style[key] = min_color(value)
            element.set("style", style_text(style))


def min_color(value: str) -> str:
    named = {"white": "#fff", "black": "#000", "red": "red", "none": "none", "currentColor": "currentColor"}
    if value in named:
        return named[value]
    match = re.fullmatch(r"#([0-9a-fA-F]{6})", value.strip())
    if match:
        hexv = match.group(1).lower()
        if hexv[0] == hexv[1] and hexv[2] == hexv[3] and hexv[4] == hexv[5]:
            return f"#{hexv[0]}{hexv[2]}{hexv[4]}"
        return f"#{hexv}"
    rgb = re.fullmatch(r"rgb\(\s*(\d+),\s*(\d+),\s*(\d+)\s*\)", value)
    if rgb:
        return min_color("#" + "".join(f"{int(part):02x}" for part in rgb.groups()))
    return value


def remove_defaults(root: ET.Element) -> None:
    defaults = {"version": "1.1", "type": "text/css"}
    for element in walk(root):
        for key, value in list(defaults.items()):
            if element.attrib.get(key) == value:
                element.attrib.pop(key, None)


def remove_non_inheritable_group_attrs(root: ET.Element) -> None:
    for element in walk(root):
        if local_name(element.tag) == "g":
            for key in ("x", "y", "width", "height"):
                element.attrib.pop(key, None)


def remove_useless_stroke_fill(root: ET.Element) -> None:
    for element in walk(root):
        if element.attrib.get("stroke") == "none":
            for key in ("stroke-width", "stroke-linecap", "stroke-linejoin", "stroke-opacity"):
                element.attrib.pop(key, None)
        if element.attrib.get("fill") == "none" and "stroke" not in element.attrib:
            element.attrib.pop("fill-opacity", None)


def remove_attr_everywhere(root: ET.Element, name: str) -> None:
    for element in walk(root):
        element.attrib.pop(name, None)


def remove_hidden(root: ET.Element) -> None:
    parents = parent_map(root)
    for element in list(root.iter()):
        if element is root:
            continue
        style = parse_style(element.attrib.get("style"))
        hidden = (
            element.attrib.get("display") == "none"
            or element.attrib.get("visibility") == "hidden"
            or style.get("display") == "none"
            or style.get("visibility") == "hidden"
        )
        if hidden:
            parents[element].remove(element)


def remove_empty_text(root: ET.Element) -> None:
    parents = parent_map(root)
    for element in list(root.iter()):
        if element is root:
            continue
        if local_name(element.tag) in {"text", "tspan"} and not "".join(element.itertext()).strip() and len(list(element)) == 0:
            parents[element].remove(element)


def parse_float(value: str | None, default: float = 0.0) -> float:
    if value is None:
        return default
    match = re.match(r"[-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?", value.strip())
    return float(match.group(0)) if match else default


def convert_shapes_to_paths(root: ET.Element, options: OptimizeOptions) -> None:
    ns = namespace(root.tag)
    for parent in list(root.iter()):
        children = list(parent)
        for index, child in enumerate(children):
            d = shape_to_path_d(child, options)
            if not d:
                continue
            attrs = {key: value for key, value in child.attrib.items() if local_name(key) not in SHAPE_ATTRS}
            attrs["d"] = d
            path = ET.Element(qname(ns, "path"), attrs)
            path.text = child.text
            path.tail = child.tail
            path.extend(list(child))
            parent.remove(child)
            parent.insert(index, path)


SHAPE_ATTRS = {"x", "y", "x1", "y1", "x2", "y2", "cx", "cy", "r", "rx", "ry", "width", "height", "points"}


def shape_to_path_d(element: ET.Element, options: OptimizeOptions) -> str | None:
    decimals = 4 if options.float_precision is None else options.float_precision
    name = local_name(element.tag)
    if name == "rect":
        x = parse_float(element.attrib.get("x"))
        y = parse_float(element.attrib.get("y"))
        width = parse_float(element.attrib.get("width"))
        height = parse_float(element.attrib.get("height"))
        if width <= 0 or height <= 0:
            return None
        rx = parse_float(element.attrib.get("rx"), parse_float(element.attrib.get("ry")))
        ry = parse_float(element.attrib.get("ry"), rx)
        rx = min(max(rx, 0.0), width / 2.0)
        ry = min(max(ry, 0.0), height / 2.0)
        if rx == 0 and ry == 0:
            return PathData.parse(f"M{x} {y}L{x + width} {y}L{x + width} {y + height}L{x} {y + height}Z").optimize("closed").to_string(decimals, True)
        d = (
            f"M{x + rx} {y}H{x + width - rx}A{rx} {ry} 0 0 1 {x + width} {y + ry}"
            f"V{y + height - ry}A{rx} {ry} 0 0 1 {x + width - rx} {y + height}"
            f"H{x + rx}A{rx} {ry} 0 0 1 {x} {y + height - ry}"
            f"V{y + ry}A{rx} {ry} 0 0 1 {x + rx} {y}Z"
        )
        return PathData.parse(d).to_string(decimals, True)
    if name == "line":
        return PathData.parse(
            f"M{parse_float(element.attrib.get('x1'))} {parse_float(element.attrib.get('y1'))}"
            f"L{parse_float(element.attrib.get('x2'))} {parse_float(element.attrib.get('y2'))}"
        ).to_string(decimals, True)
    if name in {"polyline", "polygon"}:
        points = parse_points(element.attrib.get("points", ""))
        if not points:
            return None
        suffix = "Z" if name == "polygon" else ""
        return PathData.parse("M" + "L".join(f"{x} {y}" for x, y in points) + suffix).to_string(decimals, True)
    if name == "circle":
        cx = parse_float(element.attrib.get("cx"))
        cy = parse_float(element.attrib.get("cy"))
        r = parse_float(element.attrib.get("r"))
        if r <= 0:
            return None
        return PathData.parse(f"M{cx - r} {cy}A{r} {r} 0 1 0 {cx + r} {cy}A{r} {r} 0 1 0 {cx - r} {cy}Z").to_string(decimals, True)
    if name == "ellipse":
        cx = parse_float(element.attrib.get("cx"))
        cy = parse_float(element.attrib.get("cy"))
        rx = parse_float(element.attrib.get("rx"))
        ry = parse_float(element.attrib.get("ry"))
        if rx <= 0 or ry <= 0:
            return None
        return PathData.parse(f"M{cx - rx} {cy}A{rx} {ry} 0 1 0 {cx + rx} {cy}A{rx} {ry} 0 1 0 {cx - rx} {cy}Z").to_string(decimals, True)
    return None


def parse_points(points: str) -> list[tuple[float, float]]:
    values = [float(part) for part in re.findall(r"[-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?", points)]
    return list(zip(values[0::2], values[1::2]))


def convert_transforms(root: ET.Element, options: OptimizeOptions) -> None:
    decimals = 4 if options.float_precision is None else options.float_precision
    for element in walk(root):
        transform = element.attrib.get("transform")
        if not transform:
            continue
        try:
            matrix = parse_transform(transform)
        except (PathDataError, ValueError):
            continue
        name = local_name(element.tag)
        if name == "path" and "d" in element.attrib:
            try:
                path = PathData.parse(element.attrib["d"]).transform(matrix)
            except PathDataError:
                continue
            element.set("d", path.to_string(decimals, True))
            element.attrib.pop("transform", None)
        elif name in {"rect", "line", "polyline", "polygon", "circle", "ellipse"}:
            d = shape_to_path_d(element, options)
            if not d:
                continue
            path = PathData.parse(d).transform(matrix)
            element.tag = qname(namespace(root.tag), "path")
            for attr in list(element.attrib):
                if local_name(attr) in SHAPE_ATTRS or attr == "transform":
                    element.attrib.pop(attr, None)
            element.set("d", path.to_string(decimals, True))


def convert_path_data(root: ET.Element, options: OptimizeOptions) -> None:
    decimals = 4 if options.float_precision is None else options.float_precision
    for element in walk(root):
        if local_name(element.tag) != "path" or "d" not in element.attrib:
            continue
        try:
            path = PathData.parse(element.attrib["d"]).optimize("safe")
        except PathDataError:
            continue
        element.set("d", path.to_string(decimals, True))


def remove_empty_attrs(root: ET.Element) -> None:
    for element in walk(root):
        for key, value in list(element.attrib.items()):
            if value == "":
                element.attrib.pop(key, None)


def remove_empty_containers(root: ET.Element) -> None:
    container_names = {"g", "defs", "clipPath", "mask", "pattern", "symbol", "marker"}
    changed = True
    while changed:
        changed = False
        parents = parent_map(root)
        for element in list(root.iter()):
            if element is root:
                continue
            if local_name(element.tag) in container_names and len(list(element)) == 0 and not (element.text or "").strip():
                parents[element].remove(element)
                changed = True


def collapse_groups(root: ET.Element) -> None:
    changed = True
    while changed:
        changed = False
        for parent in list(root.iter()):
            children = list(parent)
            for index, child in enumerate(children):
                if local_name(child.tag) == "g" and not child.attrib:
                    parent.remove(child)
                    for offset, grandchild in enumerate(list(child)):
                        parent.insert(index + offset, grandchild)
                    changed = True
                    break
            if changed:
                break


def merge_paths(root: ET.Element) -> None:
    for parent in list(root.iter()):
        merged: list[ET.Element] = []
        for child in list(parent):
            if local_name(child.tag) != "path" or "d" not in child.attrib:
                merged.append(child)
                continue
            if merged and local_name(merged[-1].tag) == "path" and same_path_attrs(merged[-1], child):
                merged[-1].set("d", (merged[-1].attrib.get("d", "") + " " + child.attrib.get("d", "")).strip())
                parent.remove(child)
            else:
                merged.append(child)


def same_path_attrs(left: ET.Element, right: ET.Element) -> bool:
    return {k: v for k, v in left.attrib.items() if local_name(k) != "d"} == {k: v for k, v in right.attrib.items() if local_name(k) != "d"}


def sort_attrs(root: ET.Element) -> None:
    for element in walk(root):
        items = sorted(element.attrib.items(), key=lambda item: (0 if local_name(item[0]) == "d" else 1, local_name(item[0])))
        element.attrib.clear()
        element.attrib.update(items)


def sort_defs_children(root: ET.Element) -> None:
    for element in walk(root):
        if local_name(element.tag) == "defs":
            children = sorted(list(element), key=lambda child: (local_name(child.tag), child.attrib.get("id", "")))
            element[:] = children


def convert_style_to_attrs(root: ET.Element) -> None:
    for element in walk(root):
        style = parse_style(element.attrib.get("style"))
        if not style:
            continue
        for key, value in style.items():
            if key not in element.attrib:
                element.set(key, value)
        element.attrib.pop("style", None)


def prefix_ids(root: ET.Element, prefix: str) -> None:
    mapping: dict[str, str] = {}
    for element in walk(root):
        ident = element.attrib.get("id")
        if ident:
            new = f"{prefix}{ident}"
            mapping[ident] = new
            element.set("id", new)
    if not mapping:
        return
    for element in walk(root):
        for key, value in list(element.attrib.items()):
            for old, new in mapping.items():
                value = value.replace(f"#{old}", f"#{new}").replace(f"url({old})", f"url({new})")
            element.set(key, value)


def remove_attrs_plugin(root: ET.Element, params: dict[str, Any]) -> None:
    attrs = params.get("attrs") or params.get("attributes") or params.get("name")
    if attrs is None:
        return
    if isinstance(attrs, str):
        patterns = [attrs]
    elif isinstance(attrs, list):
        patterns = [str(attr) for attr in attrs]
    else:
        return
    for element in walk(root):
        for key in list(element.attrib):
            lname = local_name(key)
            if any(attr_matches(lname, pattern) for pattern in patterns):
                element.attrib.pop(key, None)


def attr_matches(name: str, pattern: str) -> bool:
    if pattern == "*" or pattern == name:
        return True
    if pattern.endswith("*") and name.startswith(pattern[:-1]):
        return True
    if pattern.startswith("/") and pattern.endswith("/"):
        return re.search(pattern[1:-1], name) is not None
    return False


def remove_elements_by_attr(root: ET.Element, params: dict[str, Any]) -> None:
    attrs = params.get("attrs") or params
    if not isinstance(attrs, dict):
        return
    parents = parent_map(root)
    for element in list(root.iter()):
        if element is root:
            continue
        if all(element.attrib.get(str(key)) == str(value) for key, value in attrs.items()):
            parents[element].remove(element)


def remove_xlink(root: ET.Element) -> None:
    for element in walk(root):
        for key in list(element.attrib):
            if "xlink" in key:
                local = local_name(key)
                if local == "href" and "href" not in element.attrib:
                    element.set("href", element.attrib[key])
                element.attrib.pop(key, None)


def to_data_uri(svg_text: str, mode: str) -> str:
    if mode == "base64":
        return "data:image/svg+xml;base64," + base64.b64encode(svg_text.encode("utf-8")).decode("ascii")
    if mode == "enc":
        return "data:image/svg+xml," + urllib.parse.quote(svg_text, safe="/:;=,@-._~!$&'()*+")
    if mode == "unenc":
        return "data:image/svg+xml," + svg_text
    raise SvgOptimizeError("--svgo-datauri must be base64, enc, or unenc")
