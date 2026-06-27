# svgo

`svgo` is a pure-Python SVG toolchain for path editing, SVG optimization,
PNG icon tracing, centerline reconstruction, geometry conversion, matrix
transforms, measurement, sanitization, viewBox/viewport edits, and SVG
inspection. It ships as an importable Python package and as a single `svgo`
command-line program.

The package has no required runtime dependencies. If `numpy` is installed,
some centerline distance-transform work can use accelerated array operations;
otherwise the standard-library fallback is used.

## Features

- Edit SVG path data with ordered operations: translate, scale, affine matrix,
  rotate, relative/absolute serialization, subpath reversal, origin changes,
  and path optimization profiles.
- Optimize whole SVG files with Python implementations of common
  SVGO-style cleanup and minification operations.
- Trace simple PNG icons into filled SVG paths without shelling out to
  external tracing tools.
- Convert filled stroke outlines into approximate stroked centerlines.
- Convert SVG geometry primitives to path data and create common affine
  matrices from Python.
- Measure path/SVG length, bounds, and point-at-length coordinates.
- Set, fit, and resize root SVG `viewBox`, `width`, and `height` values.
- Validate SVG XML, inspect dimensions/element counts/fonts, and run
  structural conversions such as shape-to-path conversion, plain cleanup,
  CSS style inlining, sanitization, and transform flattening.
- Use the same functionality from Python APIs or from the documented CLI.

## Installation

From PyPI, once published:

```bash
python -m pip install svgo
```

From this repository:

```bash
python -m pip install .
```

For local development with `uv`:

```bash
uv sync
uv run svgo --help
```

## CLI

The CLI entry point is `svgo`. It is organized into short subcommands, each
with a one-letter alias:

```bash
svgo path   --path "<d>" [--op OP ...] [--svgo]                         # alias: p
svgo path   --input icon.svg --output icon.out.svg --select all|N|N,N --op OP
svgo opt    --input icon.svg --output icon.min.svg [optimization options] # alias: o
svgo trace  --input icon.png --output traced.svg [trace options]          # alias: t
svgo center --input outline.svg --output stroke.svg [centerline options]  # alias: c
svgo info   --input icon.svg                                             # alias: i
svgo validate --input icon.svg [--strict]                                # alias: v
svgo measure --input icon.svg                                            # alias: m
svgo sanitize --input icon.svg --output safe.svg                         # alias: s
svgo viewbox --input icon.svg --fit-content --output fitted.svg          # alias: b
svgo convert --input icon.svg --output converted.svg [conversion options] # alias: x
svgo plugins                                                             # alias: l
```

Every command supports `--help`:

```bash
svgo --help
svgo path --help
svgo opt --help
svgo trace --help
svgo center --help
svgo info --help
svgo validate --help
svgo measure --help
svgo sanitize --help
svgo viewbox --help
svgo convert --help
```

## Path Editing

Path operations are applied in order with repeated `--op` flags:

```bash
svgo path --path "M10 10h5v5z" --op "matrix(-1,0,0,1,30,0)" --minify
svgo p --input icon.svg --output edited.svg --select 0,2 --op translate:2,-1 --op optimize:safe
```

Supported operations:

- `translate:dx,dy`
- `scale:kx,ky`
- `matrix:a,b,c,d,e,f` or `matrix(a,b,c,d,e,f)`
- `rotate:ox,oy,degrees`
- `relative`
- `absolute`
- `reverse` or `reverse:itemIndex`
- `origin:itemIndex` or `origin:itemIndex:subpath`
- `cubics`, `cubic`, `to-cubics`, or `toCubics`
- `optimize:safe`, `optimize:size`, `optimize:closed`, `optimize:all`
- `optimize:remove-useless,use-shorthands,use-hv,use-relative-absolute,use-reverse,use-close-path,remove-orphan-dots`

The affine matrix uses SVG convention:

```text
x' = a*x + c*y + e
y' = b*x + d*y + f
```

Arcs are converted to cubic Beziers during arbitrary affine transforms so
reflections, rotations, scales, and skews stay fully Python based.

## SVG Optimization

`svgo opt` optimizes SVG documents. `svgo path --svgo` applies the same SVG
optimizer after path edits:

```bash
svgo opt --input icon.svg --output icon.min.svg --svgo-multipass --svgo-precision 3
svgo o --input icon.svg --svgo-disable cleanupIds --svgo-plugin removeDimensions
svgo opt --input icon.svg --svgo-preset none --svgo-plugin convertShapeToPath --svgo-plugin sortAttrs
svgo l
```

Supported options include:

- `--svgo-preset default|none`
- `--svgo-plugin NAME[:JSON]`
- `--svgo-disable NAME`
- `--svgo-precision N`
- `--svgo-multipass`
- `--svgo-pretty`
- `--svgo-indent N`
- `--svgo-eol lf|crlf`
- `--svgo-final-newline`
- `--svgo-datauri base64|enc|unenc`
- `--svgo-config FILE`

`--svgo-config` accepts JSON files and Python-readable TOML files. JavaScript
SVGO configs are intentionally rejected because this implementation does not
execute Node.js.

## PNG Tracing

PNG tracing decodes non-interlaced 8-bit PNGs with the standard library,
groups visible pixels, traces connected-component boundaries, and writes
filled SVG paths.

```bash
svgo trace --input icon.png --output traced.svg --mode palette --max-colors 8 --quantize 24 --min-area 8
```

Modes:

- `palette`: group pixels into dominant quantized colors.
- `alpha`: trace a single alpha mask using the most common visible color.
- `exact`: keep exact quantized color buckets.

Useful options:

- `--drop-white`
- `--alpha-threshold N`
- `--white-threshold N`
- `--quantize N`
- `--max-colors N`
- `--min-area N`
- `--scale N`
- `--decimals N`
- `--title TEXT`

## Centerline Reconstruction

Centerline reconstruction converts filled stroke outlines into approximate
stroked paths by flattening path data, rasterizing with even-odd fill,
skeletonizing with Zhang-Suen thinning, estimating stroke width, and tracing
the skeleton.

```bash
svgo center --path "M0 0L100 0L100 20L0 20Z" --emit path
svgo c --input traced.svg --output centerline.svg --svg-paths all --mode all --simplify 4
```

Important options:

- `--emit path|svg|d`
- `--mode longest|all`
- `--scale N`
- `--max-size N`
- `--curve-samples N`
- `--simplify N`
- `--min-length N`
- `--stroke-width auto|N`
- `--linecap VALUE`
- `--linejoin VALUE`
- `--polyline`
- `--svg-paths first|all`
- `--keep-failed`

Centerline output is intentionally approximate. For production icon work,
render and inspect the result before final minification.

## Inspection And Conversion

`svgo info` prints structured JSON metadata:

```bash
svgo info --input icon.svg
svgo i --input icon.svg --compact
```

`svgo validate` checks SVG XML and reports structural issues. Warnings do not
make the command fail unless `--strict` is used:

```bash
svgo validate --input icon.svg
svgo v --input icon.svg --strict --json
```

`svgo measure` reports path/SVG length and axis-aligned bounds. It accepts raw
path data, SVG files, or text files containing path data:

```bash
svgo measure --path "M0 0H10V10H0Z" --decimals 3
svgo m --input icon.svg --compact
svgo m --path "M0 0H10V10" --at 15
```

`svgo sanitize` removes active or unsafe content while keeping normal static
SVG geometry:

```bash
svgo sanitize --input icon.svg --output icon.safe.svg
svgo s --input icon.svg --remove-external-refs --remove-styles
```

`svgo viewbox` edits root viewport metadata:

```bash
svgo viewbox --input icon.svg --set "0 0 24 24" --remove-dimensions
svgo b --input icon.svg --fit-content --padding 1 --precision 2
svgo b --input icon.svg --width 48 --height 48
```

`svgo convert` runs pure-Python structural conversions. With no conversion
flags it converts basic shapes to paths:

```bash
svgo convert --input shapes.svg --output paths.svg
svgo x --input drawing.svg --output plain.svg --to-plain
svgo x --input transformed.svg --output flat.svg --shapes-to-paths --flatten-transforms
svgo x --input styled.svg --output inline.svg --inline-styles
svgo x --input source.svg --output converted.svg --all --precision 3
```

Conversion options:

- `--to-plain`: remove common editor metadata and editor-specific attributes.
- `--shapes-to-paths`: convert `rect`, `circle`, `ellipse`, `line`,
  `polyline`, and `polygon` to `path`.
- `--flatten-transforms`: bake supported transforms into path coordinates.
- `--flatten-groups`: collapse empty unstyled groups.
- `--inline-styles`: inline simple style-element rules into presentation
  attributes.
- `--sanitize`: remove scripts, event handlers, and unsafe links before
  conversion.
- `--all`: enable every conversion pass.
- `--precision N`: control generated numeric precision.

## Python API

```python
from svgo_py import (
    PathData,
    centerline_path_data,
    circle_to_path,
    get_svg_info,
    fit_viewbox_svg,
    inline_styles_svg,
    optimize_svg,
    path_metrics,
    path_to_cubics,
    rect_to_path,
    resize_svg,
    sanitize_svg,
    set_viewbox_svg,
    trace_png,
    transform_2d,
    translate_2d,
    validate_svg,
)

path = PathData.parse("M0 0L10 0L10 10Z")
path.transform((1, 0, 0, 1, 2, -1))
path.optimize("safe")
print(path.to_string(decimals=3, minify=True))

svg = optimize_svg("<svg><rect width='10' height='10'/></svg>")
shape = rect_to_path(0, 0, 24, 12, rx=2, decimals=3, minify=True)
cubic = path_to_cubics("M0 0L10 0Q15 0 15 5", decimals=3, minify=True)
x, y = transform_2d(translate_2d(10, 5), 1, 2)
report = validate_svg("<svg viewBox='0 0 10 10'/>")
metrics = path_metrics("M0 0H10V10H0Z", decimals=3)
safe_svg = sanitize_svg("<svg onload='x()'><path d='M0 0H1'/></svg>")
fitted_svg = fit_viewbox_svg("<svg><path d='M2 3H6V7H2Z'/></svg>")
```

The lower-level modules are:

- `svgo_py.pathdata`
- `svgo_py.geometry`
- `svgo_py.inspect_svg`
- `svgo_py.svg_optimize`
- `svgo_py.raster_trace`
- `svgo_py.centerline`

## Development

This repository uses a small in-repo PEP 517 build backend and has no required
runtime dependencies.

```bash
uv run python -m unittest discover -s tests
uv build
```

The package targets Python 3.11 and newer.
