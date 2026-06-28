# svgo

`svgo` is a Rust-backed SVG toolchain with Python bindings and a single
command-line interface. It edits SVG path data, optimizes SVG documents,
converts geometry, measures paths, validates and sanitizes markup, traces PNG
icons, and reconstructs approximate centerline strokes from filled outlines.

The Python package is intentionally thin. Parsing, path operations, tracing,
optimization, and centerline reconstruction run in the native `svgo._svgo`
extension, while Python modules provide stable dataclass options, importable
helpers, and recipe-friendly utilities.

## Features

- Edit SVG path data with ordered operations: translate, scale, affine matrix,
  rotate, relative/absolute serialization, subpath reversal, origin changes,
  conversion to cubics, and optimization profiles.
- Optimize whole SVG files with built-in SVGO-style cleanup and minification
  passes.
- Convert SVG shapes to paths, flatten supported transforms, inline simple CSS
  styles, remove editor metadata, sanitize unsafe markup, and edit root
  `viewBox`, `width`, and `height` values.
- Measure path/SVG bounds, lengths, and point-at-length coordinates.
- Trace simple PNG icons into filled SVG paths, including component-level JSON
  output and fixed palette snapping for multi-color icon recipes.
- Reconstruct approximate stroked centerlines from filled outlines with a Rust
  rasterize, skeletonize, bridge, and trace pipeline.
- Use reusable Python utilities for icon recipes: color normalization, loop and
  polyline extraction, polygon metrics, endpoint stitching, radial centerline
  candidates, Douglas-Peucker simplification, radial-distance simplification,
  collinearity cleanup, and adaptive rounding.

## Installation

From PyPI, once a release is available:

```powershell
uvx svgo --help
uv run --with svgo svgo --help
```

From this repository:

```powershell
uv sync
uv run --no-sync svgo --help
```

## CLI

The command is organized into short subcommands, each with a one-letter alias:

```powershell
svgo path     --path "<d>" [--op OP ...] [--svgo]                         # alias: p
svgo path     --input icon.svg --output icon.out.svg --select all|N|N,N --op OP
svgo opt      --input icon.svg --output icon.min.svg [optimization flags]  # alias: o
svgo trace    --input icon.png --output traced.svg [trace flags]           # alias: t
svgo trace2   --input icon.png --output traced.svg [VTracer flags]         # alias: t2
svgo center   --input outline.svg --output stroke.svg [centerline flags]   # alias: c
svgo info     --input icon.svg                                             # alias: i
svgo validate --input icon.svg [--strict]                                  # alias: v
svgo measure  --input icon.svg                                             # alias: m
svgo sanitize --input icon.svg --output safe.svg                           # alias: s
svgo viewbox  --input icon.svg --fit-content --output fitted.svg           # alias: b
svgo convert  --input icon.svg --output converted.svg [conversion flags]   # alias: x
svgo plugins                                                              # alias: l
```

Every command supports `--help`:

```powershell
svgo --help
svgo path --help
svgo opt --help
svgo trace --help
svgo trace2 --help
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

```powershell
svgo path --path "M10 10h5v5z" --op "matrix(-1,0,0,1,30,0)" --op optimize:safe --minify
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

Arbitrary affine transforms on arcs are handled by converting arcs to cubic
Beziers before applying the transform.

## SVG Optimization

`svgo opt` optimizes SVG documents. `svgo path --svgo` applies the same
optimizer after path edits:

```powershell
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
SVGO configs are intentionally not executed.

## PNG Tracing

`trace` decodes non-interlaced 8-bit PNG files, groups visible pixels, traces
connected-component boundaries, and writes filled SVG paths:

```powershell
svgo trace --input icon.png --output traced.svg --mode palette --curve-mode pixel --max-colors 8 --quantize 24 --min-area 8
```

Modes:

- `palette`: group pixels into dominant quantized colors.
- `alpha`: trace a single alpha mask using the most common visible color.
- `exact`: keep exact quantized color buckets.

Useful options:

- `--curve-mode pixel|exact`
- `--drop-white`
- `--alpha-threshold N`
- `--white-threshold N`
- `--quantize N`
- `--max-colors N`
- `--min-area N`
- `--scale N`
- `--decimals N`
- `--palette "#143861,#00b795"`
- `--components-json`
- `--title TEXT`

Component JSON is designed for recipes that need per-color or per-shape
post-processing before writing the final SVG:

```powershell
svgo trace --input icon.png --components-json --palette "#143861,#00b795" --drop-white
```

For higher-quality curve fitting, use `trace2`/`t2`. It calls the real
[VTracer](https://github.com/visioncortex/vtracer) Python package when
installed, or a `vtracer` CLI on `PATH`:

```powershell
uv run --with vtracer svgo trace2 --input icon.png --output traced.svg
svgo t2 --input icon.png --output traced.svg --curve-mode spline --filter-speckle 4 --color-precision 6 --gradient-step 16
```

## Centerline Reconstruction

Centerline reconstruction converts filled stroke outlines into approximate
stroked paths by flattening path data, rasterizing with even-odd fill,
skeletonizing, optionally bridging nearby skeleton gaps, estimating stroke
width, and tracing skeleton chains.

```powershell
svgo center --path "M0 0L100 0L100 20L0 20Z" --emit path
svgo c --input traced.svg --output centerline.svg --svg-paths all --mode all --simplify 4 --bridge-gap 12
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
- `--fill-rule evenodd|nonzero`
- `--svg-paths first|all`
- `--keep-failed`
- `--bridge-gap N`

Centerline output is intentionally approximate. Render and inspect production
icons before final minification.

## Inspection And Conversion

`svgo info` prints structured JSON metadata:

```powershell
svgo info --input icon.svg
svgo i --input icon.svg --compact
```

`svgo validate` checks SVG XML and reports structural issues. Warnings do not
make the command fail unless `--strict` is used:

```powershell
svgo validate --input icon.svg
svgo v --input icon.svg --strict --json
```

`svgo measure` reports path/SVG length and bounds:

```powershell
svgo measure --path "M0 0H10V10H0Z" --decimals 3
svgo m --input icon.svg --compact
svgo m --path "M0 0H10V10" --at 15
```

`svgo sanitize` removes active or unsafe content while keeping normal static
SVG geometry:

```powershell
svgo sanitize --input icon.svg --output icon.safe.svg
svgo s --input icon.svg --remove-external-refs --remove-styles
```

`svgo viewbox` edits root viewport metadata:

```powershell
svgo viewbox --input icon.svg --set "0 0 24 24" --remove-dimensions
svgo b --input icon.svg --fit-content --padding 1 --precision 2
svgo b --input icon.svg --width 48 --height 48
```

`svgo convert` runs structural conversions:

```powershell
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
from svgo import (
    CenterlineOptions,
    PathData,
    TraceOptions,
    centerline_path_data,
    fit_viewbox_svg,
    optimize_svg,
    path_metrics,
    rect_to_path,
    sanitize_svg,
    trace_png,
    trace_png_components,
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
x, y = transform_2d(translate_2d(10, 5), 1, 2)
report = validate_svg("<svg viewBox='0 0 10 10'/>")
metrics = path_metrics("M0 0H10V10H0Z", decimals=3)
safe_svg = sanitize_svg("<svg onload='x()'><path d='M0 0H1'/></svg>")
fitted_svg = fit_viewbox_svg("<svg><path d='M2 3H6V7H2Z'/></svg>")
components = trace_png_components("icon.png", TraceOptions(mode="palette", min_area=8))
d, stroke_width, ctx = centerline_path_data("M0 0H30V6H0Z", CenterlineOptions(polyline=True))
```

Recipe utilities are also importable from `svgo`:

```python
from svgo import (
    filled_loops,
    normalize_color,
    optimize_path_data,
    radial_centerline_candidate,
    remove_collinear_points,
    serialize_polyline_subpaths,
    simplify_rdp,
    stitch_subpaths,
)

color = normalize_color("143861")
loops = filled_loops("M0 0H10V10H0Z")
points = remove_collinear_points([(0, 0), (5, 0), (10, 0), (10, 5)])
simple = simplify_rdp(points, "0.5")
candidate = radial_centerline_candidate("M0 0H20V20H0Z M5 5H15V15H5Z")
```

The lower-level modules are:

- `svgo.pathdata`
- `svgo.path_utils`
- `svgo.geometry`
- `svgo.measure`
- `svgo.viewport`
- `svgo.inspect_svg`
- `svgo.svg_optimize`
- `svgo.raster_trace`
- `svgo.centerline`
- `svgo.vtracer_trace`

## Recipes

`recipes/two_color_centerline_icons.py` converts two-color antialiased PNG line
icons into grouped centerline SVGs. It traces components with a fixed palette,
chooses a centerline strategy for each filled component, stitches fragmented
chains, preserves useful branches, and emits one grouped stroke per color.

```powershell
uv run --no-sync python recipes/two_color_centerline_icons.py input-pngs output-svgs --jobs 4 --report report.json
```

The recipe is intentionally built on public `svgo` APIs so the same helpers can
be reused in other conversion pipelines.

## Layout

- `rust/src/lib.rs`: Rust implementation plus PyO3 exports.
- `rust/src/bin/svgo.rs`: Rust CLI binary target.
- `src/svgo/*.py`: Python bindings, option dataclasses, and helper utilities.
- `recipes/`: higher-level conversion workflows built from public APIs.
- `tests/`: Python tests that exercise the Rust-backed package surface.

## Reference Tools

These projects are useful reference points for SVG feature coverage, command
shape, API expectations, and simplification behavior:

- [SVGO](https://github.com/svg/svgo): Node.js SVG optimizer and plugin
  ecosystem.
- [Scour](https://github.com/scour-project/scour): Python SVG optimizer and
  cleaner.
- [svgpathtools](https://github.com/mathandy/svgpathtools): Python path,
  Bezier, geometry, length, and bounds utilities.
- [svg.path](https://github.com/regebro/svg.path): Python SVG path parser and
  path object model.
- [svg-matrix-python](https://github.com/Emasoft/svg-matrix-python): Python
  wrapper around SVG matrix conversion and validation workflows.
- [Yqnn/svg-path-editor](https://github.com/Yqnn/svg-path-editor): SVG path
  editing UI and path operation reference implementation.
- [svg-path-commander](https://github.com/thednp/svg-path-commander):
  TypeScript path parsing, normalization, geometry, and transformation tools.
- [herrstrietzel/svg-path-simplify](https://github.com/herrstrietzel/svg-path-simplify):
  JavaScript path simplifier covering Bezier and line reduction, adaptive
  rounding, polygon simplification, arc conversion, shape conversion, transform
  baking, and SVG cleanup.
- [Iconify Tools](https://github.com/iconify/tools): TypeScript SVG import,
  validation, cleanup, and export tooling.
- [resvg/usvg](https://github.com/linebender/resvg): Rust SVG rendering and
  static SVG simplification reference implementation.
- [VTracer](https://github.com/visioncortex/vtracer): Rust raster-to-vector
  tracing tool.

## Development

Use `uv` for Python commands.

```powershell
uv run --no-sync python -m unittest discover -s tests
uv build
```

Build the wheel through maturin:

```powershell
uv run --no-project --with maturin maturin build --manifest-path Cargo.toml --out dist
```

The Rust binary can be built directly:

```powershell
cargo build --bin svgo
```

On Windows, the GNU build used during local verification required:

- Rust toolchain: `stable-x86_64-pc-windows-gnu`
- MinGW/binutils on `PATH` (`dlltool`, linker runtime DLLs)

## Publishing

PyPI publishing is handled by GitHub Actions Trusted Publishing through
`.github/workflows/publish.yml`. The PyPI pending publisher must match this
repository, the `publish.yml` workflow filename, and the `pypi` environment.

To publish a release, update `project.version`, commit the change, and push a
matching tag:

```powershell
git tag v0.2.0
git push origin v0.2.0
```

The workflow verifies that the pushed tag equals `v{project.version}`, runs the
test suite, builds the wheel and source distribution with `uv`, then publishes
to PyPI with Trusted Publishing.

The package targets Python 3.11 and newer.
