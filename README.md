# svgo

`svgo` is now implemented in Rust. The Python package is a thin PyO3-backed
binding layer, and all SVG/path/tracing/centerline work is delegated to the
native `svgo._svgo` extension. A Rust `svgo` binary target is also provided for
CLI use.

The project intentionally does not preserve the old pure-Python internals.

## What It Does

- Parses, serializes, transforms, reverses, re-origins, optimizes, and converts
  SVG path data.
- Optimizes SVG documents with built-in SVGO-style cleanup passes.
- Converts basic SVG shapes to paths, flattens transforms, inlines styles,
  sanitizes unsafe content, edits viewBox/viewport data, and reports metadata.
- Measures path/SVG bounds, lengths, and point-at-length coordinates.
- Traces simple PNG icons into SVG paths in Rust.
- Provides VTracer-style tracing controls through the native Rust tracer.
- Reconstructs approximate centerline strokes from filled outlines with a Rust
  rasterize/skeletonize/trace pipeline.

## Layout

- `rust/src/lib.rs`: Rust implementation plus PyO3 exports.
- `rust/src/bin/svgo.rs`: Rust CLI binary target.
- `src/svgo/*.py`: thin Python shims and dataclass option wrappers.
- `tests/`: Python tests that exercise the Rust-backed package surface.

## Build

Use `uv` for Python commands.

```powershell
uv run --no-project --with maturin maturin build --manifest-path Cargo.toml --out dist
```

On Windows, the GNU build used during local verification required:

- Rust toolchain: `stable-x86_64-pc-windows-gnu`
- MinGW/binutils on `PATH` (`dlltool`, linker runtime DLLs)

The Rust binary can be built directly:

```powershell
cargo build --bin svgo
```

## Test

```powershell
uv run --no-sync python -m unittest discover -s tests
```

## Python API

The familiar top-level imports remain as thin wrappers:

```python
from svgo import PathData, optimize_svg, path_metrics, trace_png

path = PathData.parse("M0 0H10V10Z")
path.apply_operation("translate:2,3")
print(path.to_string(decimals=2, minify=True))
```

Those wrappers perform marshaling only. The computation happens in Rust.

## CLI

The Rust binary target supports the existing short command family:

```powershell
target\debug\svgo.exe path --path "M10 10h5v5z" --op optimize:safe --minify
target\debug\svgo.exe opt --input icon.svg --output icon.min.svg
target\debug\svgo.exe trace --input icon.png --output traced.svg
target\debug\svgo.exe center --path "M0 0L30 0L30 6L0 6Z" --emit d
```
