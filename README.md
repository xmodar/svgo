# svgo

`svgo` is a pure-Python SVG toolchain for path editing, SVG optimization,
PNG icon tracing, and centerline reconstruction. It ships as an importable
Python package and as a single `svgo` command-line program.

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
svgo plugins                                                             # alias: l
```

Every command supports `--help`:

```bash
svgo --help
svgo path --help
svgo opt --help
svgo trace --help
svgo center --help
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

## Python API

```python
from svgo_py import PathData, centerline_path_data, optimize_svg, trace_png

path = PathData.parse("M0 0L10 0L10 10Z")
path.transform((1, 0, 0, 1, 2, -1))
path.optimize("safe")
print(path.to_string(decimals=3, minify=True))

svg = optimize_svg("<svg><rect width='10' height='10'/></svg>")
```

The lower-level modules are:

- `svgo_py.pathdata`
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
