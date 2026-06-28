# Contributing

This project uses Rust for the native implementation and thin Python modules
for packaging, dataclass options, and recipe-friendly helpers. Use `uv` for
Python commands.

## Development Setup

Install dependencies and run the Python test suite:

```powershell
uv sync
uv run --no-sync python -m unittest discover -s tests
```

Run Rust formatting and checks:

```powershell
cargo fmt
cargo check --all-targets
cargo test
```

Build or refresh the local Python extension with maturin:

```powershell
uv run --with maturin maturin develop --manifest-path Cargo.toml
```

On Windows, local GNU verification can use:

```powershell
$env:RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-gnu"
cargo check --all-targets
```

The package targets Python 3.11 and newer.

## Release Builds

Build Python distributions:

```powershell
uv build
uv run --with maturin maturin build --manifest-path Cargo.toml --out dist
```

Before tagging a release, update all package version references together:

- `Cargo.toml`
- `Cargo.lock`
- `pyproject.toml`
- Python fallback/test expectations when applicable

Then run the verification suite before committing:

```powershell
cargo fmt --check
cargo check --all-targets
uv run --no-sync python -m unittest discover -s tests
```

## Publishing

PyPI publishing is handled by GitHub Actions Trusted Publishing through
`.github/workflows/publish.yml`. The pending publisher must match this
repository, the `publish.yml` workflow filename, and the `pypi` environment.

To publish a release, commit the version bump and push a matching tag:

```powershell
git tag v0.4.0
git push origin v0.4.0
```

The workflow verifies that the pushed tag equals `v{project.version}`, runs the
test suite, builds ABI3 binary wheels and a source distribution, then publishes
to PyPI with Trusted Publishing.
