# svgo

`svgo` is a Rust-backed SVG toolkit distributed as a Python package with a
single command line interface. It focuses on practical SVG asset work: path
editing, document optimization, measurement, inspection, validation,
sanitization, viewport editing, PNG tracing, approximate centerline
reconstruction, and reusable declarative recipes.

For normal use, run the published CLI directly:

```powershell
uvx svgo --help
uvx svgo <command> --help
```

## Features

- Edit raw SVG path data or `d` attributes inside SVG files.
- Apply ordered path operations such as translate, scale, affine matrix,
  rotate, relative/absolute serialization, subpath reversal, origin changes,
  cubic conversion, and path-data optimization.
- Optimize SVG documents with built-in SVGO-style cleanup and minification
  passes.
- Convert shapes to paths, flatten supported transforms, inline simple CSS,
  remove editor metadata, sanitize unsafe markup, and edit root viewport
  metadata.
- Measure path and SVG bounds, lengths, and point-at-length coordinates.
- Trace non-interlaced 8-bit PNG icons into filled SVG paths, including
  component-level JSON for recipe pipelines.
- Reconstruct approximate centerline strokes from filled outlines.
- Build reusable JSON recipes that run existing `svgo` commands in sequence.

## Installation

From PyPI:

```powershell
uvx svgo --help
uvx svgo --version
```

From this repository:

```powershell
uv sync
uv run --no-sync svgo --help
```

The Rust binary target can also be built directly:

```powershell
cargo build --bin svgo
target\debug\svgo.exe --help
```

## Command Line

Use `-h`/`--help` for general help and `svgo <command> --help` for full
command-specific options. Use `-v`/`--version` to print the package version.

```powershell
svgo --help
svgo -h
svgo --version
svgo -v
svgo path --help
svgo opt --help
```

Commands:

| Command | Alias | Purpose |
| --- | --- | --- |
| `path` | `p` | Edit raw path data or SVG path attributes. |
| `opt` | `o` | Optimize SVG documents. |
| `trace` | `t` | Trace PNG images into filled SVG paths. |
| `trace2` | `t2` | Trace PNG images with VTracer-compatible option names. |
| `center` | `c` | Reconstruct approximate centerline strokes. |
| `info` | `i` | Inspect SVG metadata as JSON. |
| `validate` | `v` | Validate SVG XML and structure. |
| `measure` | `m` | Measure path and SVG geometry. |
| `sanitize` | `s` | Remove unsafe SVG content. |
| `viewbox` | `b` | Edit `viewBox`, `width`, and `height` metadata. |
| `convert` | `x` | Convert shapes, transforms, styles, and editor markup. |
| `plugins` | `l` | List optimizer plugins. |
| `recipe` | `r` | Run declarative SVG conversion recipes. |

### Path Editing

Path operations are applied in the same order they are provided:

```powershell
svgo path --path "M10 10h5v5z" --op optimize:safe --minify
svgo p --path "M0 0H10V10Z" --op translate:2,-1 --op relative
svgo p --input icon.svg --output edited.svg --select 0,2 --op "matrix(-1,0,0,1,30,0)"
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

The affine matrix follows SVG convention:

```text
x' = a*x + c*y + e
y' = b*x + d*y + f
```

Arcs are converted to cubic Beziers when an arbitrary affine transform cannot
be represented as an SVG arc.

### SVG Optimization

`svgo opt` optimizes complete SVG documents. `svgo path --svgo` applies the
same optimizer around path edits.

```powershell
svgo opt --input icon.svg --output icon.min.svg --svgo-precision 3 --svgo-multipass
svgo o -i icon.svg --svgo-disable cleanupIds --svgo-plugin removeDimensions
svgo path -i icon.svg --op optimize:safe --svgo --svgo-pretty
svgo plugins
```

Common optimizer options:

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
- `--svgo-list-plugins`
- `--svgo-config FILE`

`--svgo-config` is accepted by the CLI, but JavaScript config files are not
executed.

### PNG Tracing

`trace` decodes non-interlaced 8-bit PNG files, groups visible pixels, traces
connected component boundaries, and writes filled SVG paths.

```powershell
svgo trace --input icon.png --output traced.svg --mode palette --max-colors 8 --min-area 8
svgo t -i icon.png --components-json --palette "#143861,#00b795" --drop-white
```

Useful options:

- `--mode palette|alpha|exact`
- `--curve-mode pixel|exact`
- `--components-json`
- `--drop-white`
- `--alpha-threshold N`
- `--white-threshold N`
- `--quantize N`
- `--max-colors N`
- `--min-area N`
- `--scale N`
- `--decimals N`
- `--palette "#143861,#00b795"`
- `--title TEXT`

`trace2` accepts VTracer-compatible option names and maps them to the native
svgo tracer:

```powershell
svgo trace2 --input icon.png --output traced.svg --curve-mode spline --filter-speckle 4
svgo t2 -i icon.png --color-mode binary --path-precision 6
```

### Centerline Reconstruction

Centerline reconstruction converts filled stroke outlines into approximate
stroked paths. The pipeline flattens path data, rasterizes the filled shape,
skeletonizes the raster mask, optionally bridges nearby gaps, estimates stroke
width, and traces skeleton chains.

```powershell
svgo center --path "M0 0L100 0L100 20L0 20Z" --emit d
svgo c --input traced.svg --output centerline.svg --svg-paths all --mode all --bridge-gap 12
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
- `--decimals N`
- `--polyline`
- `--fill-rule evenodd|nonzero`
- `--svg-paths first|all`
- `--keep-failed`
- `--bridge-gap N`

Centerline output is approximate by design. Render and inspect production icons
before final minification.

### Inspection, Validation, And Measurement

```powershell
svgo info --input icon.svg
svgo i -i icon.svg --compact

svgo validate --input icon.svg
svgo validate -i icon.svg --strict --json

svgo measure --path "M0 0H10V10H0Z" --decimals 3
svgo m --input icon.svg --compact
svgo m --path "M0 0H10V10" --at 15
```

`validate` returns exit code `1` for invalid input. With `--strict`, warnings
also make the command fail.

### Sanitizing, Viewports, And Conversion

```powershell
svgo sanitize --input unsafe.svg --output safe.svg --remove-external-refs
svgo s -i unsafe.svg --remove-styles --remove-raster-images

svgo viewbox --input icon.svg --set "0 0 24 24" --remove-dimensions
svgo b -i icon.svg --fit-content --padding 1 --precision 2
svgo b -i icon.svg --width 48 --height 48

svgo convert --input shapes.svg --output paths.svg
svgo x -i drawing.svg -o plain.svg --to-plain
svgo x -i transformed.svg -o flat.svg --shapes-to-paths --flatten-transforms
svgo x -i source.svg -o converted.svg --all --precision 3
```

Conversion flags:

- `--to-plain`
- `--shapes-to-paths`
- `--flatten-transforms`
- `--flatten-groups`
- `--inline-styles`
- `--sanitize`
- `--all`
- `--precision N`

With no conversion flag, `convert` defaults to `--shapes-to-paths`.

## Declarative Recipes

Use `svgo recipe` when a workflow needs repeated steps, batch processing, or a
report. A recipe is JSON with a `steps` array. Each step uses a normal command
name and the long CLI option names converted to JSON keys.

Create a starter recipe:

```powershell
uvx svgo recipe init --kind cleanup --output cleanup.svgo.json
uvx svgo recipe init --kind centerline-icons --output centerline-icons.svgo.json
```

Run a recipe on one file or a directory:

```powershell
uvx svgo recipe run --recipe cleanup.svgo.json --input source.svg --output source.min.svg --report report.json
uvx svgo recipe run --recipe centerline-icons.svgo.json --input input-pngs --output output-svgs --report report.json
```

Minimal cleanup recipe:

```json
{
  "name": "svg-cleanup",
  "outputExtension": ".svg",
  "steps": [
    { "command": "sanitize", "removeExternalRefs": true },
    { "command": "convert", "all": true, "precision": 3 },
    { "command": "viewbox", "fitContent": true, "padding": 1, "removeDimensions": true, "precision": 3 },
    { "command": "validate", "strict": true },
    { "command": "opt", "multipass": true, "precision": 3 }
  ]
}
```

PNG centerline recipe:

```json
{
  "name": "centerline-icons",
  "outputExtension": ".svg",
  "steps": [
    {
      "command": "trace",
      "mode": "palette",
      "palette": ["#143861", "#00b795"],
      "dropWhite": true,
      "whiteThreshold": 245,
      "alphaThreshold": 16,
      "minArea": 80,
      "decimals": 1
    },
    {
      "command": "center",
      "svgPaths": "all",
      "mode": "all",
      "polyline": true,
      "bridgeGap": 12,
      "keepFailed": true,
      "strokeWidth": "auto",
      "decimals": 2
    },
    { "command": "opt", "multipass": true, "precision": 2 }
  ]
}
```

Supported recipe step commands are `validate`, `sanitize`, `convert`,
`viewbox`, `path`, `trace`, `trace2`, `center`, `opt`, `info`, and `measure`.
Recipes preserve input files and write to the requested output path or output
directory.

## Project Layout

- `rust/src/lib.rs`: crate entry point, shared imports, and feature file
  includes.
- `rust/src/core.rs`: shared errors, points, segments, and matrix primitives.
- `rust/src/pathdata.rs`: path parsing, path operations, serialization, and
  PyO3 path bindings.
- `rust/src/geometry.rs`: shape-to-path and matrix geometry helpers.
- `rust/src/measure.rs`: path and SVG measurement.
- `rust/src/svg.rs`: SVG parsing, optimization, sanitization, conversion, and
  viewport utilities.
- `rust/src/trace.rs`: PNG decoding, native tracing, and VTracer-style options.
- `rust/src/centerline.rs`: raster skeletonization and centerline generation.
- `rust/src/cli.rs`: command-line argument handling, recipe execution, and
  help text.
- `rust/src/python.rs`: PyO3 module registration.
- `rust/src/bin/svgo.rs`: Rust binary entry point.
- `src/svgo/*.py`: Python bindings, dataclass options, and helper utilities.
- `recipes/`: reusable JSON recipe examples for `svgo recipe run`.
- `tests/`: Python tests for the Rust-backed package surface and CLI.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, verification,
release, and publishing notes.

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
