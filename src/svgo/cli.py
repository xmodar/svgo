"""Python entry point shim for the Rust CLI."""

from __future__ import annotations

import sys


def _version_text() -> str:
    from . import __version__

    return f"svgo {__version__}"


def main(argv: list[str] | None = None) -> int:
    args = list(sys.argv[1:] if argv is None else argv)
    if args and args[0] in {"--version", "-V", "version"}:
        sys.stdout.write(_version_text() + "\n")
        return 0

    from . import _svgo as _rust

    code, stdout, stderr = _rust.cli_run(args)
    if stdout:
        sys.stdout.write(stdout + ("" if stdout.endswith("\n") else "\n"))
    if stderr:
        sys.stderr.write(stderr + ("" if stderr.endswith("\n") else "\n"))
    return int(code)
