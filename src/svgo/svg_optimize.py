"""Thin Python bindings for the Rust SVG optimizer."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from . import _svgo as _rust

SvgOptimizeError = ValueError
BUILTIN_PLUGINS = json.loads(_rust.builtin_plugins_json())


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
    params = json.loads(raw)
    if not isinstance(params, dict):
        raise SvgOptimizeError(f"Plugin params for {name} must be a JSON object")
    return PluginSpec(name, params)


def _plugin_dict(plugin: PluginSpec | str) -> dict[str, Any]:
    if isinstance(plugin, str):
        plugin = parse_plugin_spec(plugin)
    return {"name": plugin.name, "params": plugin.params}


def _options_json(options: OptimizeOptions | None) -> str | None:
    if options is None:
        return None
    return json.dumps(
        {
            "preset": options.preset,
            "plugins": [_plugin_dict(plugin) for plugin in options.plugins],
            "disabled": sorted(options.disabled),
            "float_precision": options.float_precision,
            "multipass": options.multipass,
            "pretty": options.pretty,
            "indent": options.indent,
            "eol": options.eol,
            "final_newline": options.final_newline,
            "datauri": options.datauri,
        }
    )


def optimize_svg(svg_text: str, options: OptimizeOptions | None = None) -> str:
    return _rust.optimize_svg(svg_text, _options_json(options))


def local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1].rsplit(":", 1)[-1]


parse_transform = None
shape_to_path_d = None
