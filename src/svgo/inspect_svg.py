"""Thin Python bindings for Rust SVG inspection and conversion."""

from __future__ import annotations

import asyncio
import json
from pathlib import Path
from typing import Any

from . import _svgo as _rust


def validate_svg(svg_input: str | Path, *, strict: bool = False) -> dict[str, Any]:
    return json.loads(_rust.validate_svg_json(str(svg_input), strict))


async def validate_svg_async(svg_input: str | Path, *, strict: bool = False) -> dict[str, Any]:
    loop = asyncio.get_running_loop()
    return await loop.run_in_executor(None, lambda: validate_svg(svg_input, strict=strict))


def get_svg_info(svg_input: str | Path) -> dict[str, Any]:
    return json.loads(_rust.get_svg_info_json(str(svg_input)))


def to_plain_svg(svg_text: str, *, precision: int | None = None) -> str:
    return _rust.to_plain_svg(svg_text, precision)


def sanitize_svg(svg_text: str, *, precision: int | None = None, remove_external_refs: bool = False, allow_data_images: bool = True, remove_styles: bool = False, remove_raster_images: bool = False) -> str:
    return _rust.sanitize_svg(svg_text, precision, remove_external_refs, allow_data_images, remove_styles, remove_raster_images)


def inline_styles_svg(svg_text: str, *, precision: int | None = None, remove_style_elements: bool = True) -> str:
    return _rust.inline_styles_svg(svg_text, precision, remove_style_elements)


def convert_shapes_svg(svg_text: str, *, precision: int | None = None) -> str:
    return _rust.convert_shapes_svg(svg_text, precision)


def flatten_svg(svg_text: str, *, precision: int | None = None, flatten_transforms: bool = True, flatten_groups: bool = True, shapes_to_paths: bool = True, plain: bool = False) -> str:
    return _rust.flatten_svg(svg_text, precision, flatten_transforms, flatten_groups, shapes_to_paths, plain)


def read_svg_text(svg_input: str | Path) -> tuple[str, str | None, str | None]:
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


def info_json(svg_input: str | Path, *, indent: int | None = 2) -> str:
    return json.dumps(get_svg_info(svg_input), indent=indent, sort_keys=True)


conversion_error = ValueError
