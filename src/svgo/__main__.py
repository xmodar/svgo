"""Run the Rust-backed svgo CLI."""

from .cli import main

if __name__ == "__main__":
    raise SystemExit(main())
