# svgo

`svgo` is a pure-Python SVG toolchain modeled after the local `svgo` Codex
skill. This first beta provides importable APIs and a documented CLI for:

- ordered SVG path edits: translate, scale, matrix, rotate, relative/absolute
  serialization, reverse, origin changes, and optimizer profiles;
- whole-file SVG cleanup and minification with Python implementations of the
  commonly used SVGO-style operations;
- simple PNG icon tracing into filled SVG paths;
- filled-outline to stroked-centerline reconstruction.

The package has no required runtime dependencies. If `numpy` is already
installed, centerline distance transforms use the accelerated array path where
possible; otherwise the deterministic Python fallback is used.

## Install And Run

From this repository:

```bash
uv run python -m svgo_py --help
uv run svgo --help
```

## CLI Overview

`svgo` is organized into short subcommands. Each subcommand has a one-letter
alias:

```bash
uv run svgo path   --path "<d>" [--op OP ...] [--svgo]              # alias: p
uv run svgo path   --input icon.svg --output icon.out.svg --select all|N|N,N --op OP
uv run svgo opt    --input icon.svg --output icon.min.svg [SVGO-style options] # alias: o
uv run svgo trace  --input icon.png --output traced.svg [trace options]        # alias: t
uv run svgo center --input outline.svg --output stroke.svg [centerline options] # alias: c
uv run svgo plugins                                                        # alias: l
```

Every command supports `--help`.

## Path Operations

Path operations are applied in order with repeated `--op` flags:

```bash
uv run svgo path --path "M10 10h5v5z" --op "matrix(-1,0,0,1,30,0)" --minify
uv run svgo p --input icon.svg --output edited.svg --select 0,2 --op translate:2,-1 --op optimize:safe
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
reflections, rotations, scales, and skews remain fully Python based.

## SVG Optimization

`svgo opt` and `svgo path --svgo` expose SVGO-style flags:

```bash
uv run svgo opt --input icon.svg --output icon.min.svg --svgo-multipass --svgo-precision 3
uv run svgo o --input icon.svg --svgo-disable cleanupIds --svgo-plugin removeDimensions
uv run svgo opt --input icon.svg --svgo-preset none --svgo-plugin convertShapeToPath --svgo-plugin sortAttrs
uv run svgo l
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
shell out to Node.

## PNG Tracing

PNG tracing decodes non-interlaced 8-bit PNGs with the standard library,
groups visible pixels, traces connected component boundaries, and writes
filled SVG paths.

```bash
uv run svgo trace --input icon.png --output traced.svg --mode palette --max-colors 8 --quantize 24 --min-area 8
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
uv run svgo center --path "M0 0L100 0L100 20L0 20Z" --emit path
uv run svgo c --input traced.svg --output centerline.svg --svg-paths all --mode all --simplify 4
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

## Library API

```python
from svgo_py import PathData, optimize_svg, trace_png, centerline_path_data

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
