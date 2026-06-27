"""Python entry point shim for the Rust CLI."""

from __future__ import annotations

import sys

from . import _svgo as _rust


def main(argv: list[str] | None = None) -> int:
    code, stdout, stderr = _rust.cli_run(list(sys.argv[1:] if argv is None else argv))
    if stdout:
        sys.stdout.write(stdout + ("" if stdout.endswith("\n") else "\n"))
    if stderr:
        sys.stderr.write(stderr + ("" if stderr.endswith("\n") else "\n"))
    return int(code)
