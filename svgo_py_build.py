"""Minimal offline PEP 517 backend for this pure-Python package."""

from __future__ import annotations

import base64
import csv
import hashlib
import io
import tarfile
import zipfile
from pathlib import Path

NAME = "svgo"
NORMALIZED = "svgo"
VERSION = "0.1.0b1"
DIST_INFO = f"{NORMALIZED}-{VERSION}.dist-info"


def get_requires_for_build_wheel(config_settings=None):  # noqa: D401
    """Return build requirements."""
    return []


def get_requires_for_build_sdist(config_settings=None):  # noqa: D401
    """Return build requirements."""
    return []


def get_requires_for_build_editable(config_settings=None):  # noqa: D401
    """Return build requirements."""
    return []


def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
    dist_info = Path(metadata_directory) / DIST_INFO
    dist_info.mkdir(parents=True, exist_ok=True)
    write_metadata(dist_info)
    (dist_info / "RECORD").write_text("", encoding="utf-8")
    return DIST_INFO


def prepare_metadata_for_build_editable(metadata_directory, config_settings=None):
    return prepare_metadata_for_build_wheel(metadata_directory, config_settings)


def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    wheel_name = f"{NORMALIZED}-{VERSION}-py3-none-any.whl"
    wheel_path = Path(wheel_directory) / wheel_name
    records: list[tuple[str, str, str]] = []

    with zipfile.ZipFile(wheel_path, "w", compression=zipfile.ZIP_DEFLATED) as wheel:
        for source in sorted((Path("src") / "svgo_py").rglob("*.py")):
            arcname = source.relative_to("src").as_posix()
            data = source.read_bytes()
            wheel.writestr(arcname, data)
            records.append(record_entry(arcname, data))

        metadata_files = build_metadata_files()
        for arcname, data in metadata_files.items():
            wheel.writestr(arcname, data)
            records.append(record_entry(arcname, data))

        if Path("LICENSE").exists():
            arcname = f"{DIST_INFO}/licenses/LICENSE"
            data = Path("LICENSE").read_bytes()
            wheel.writestr(arcname, data)
            records.append(record_entry(arcname, data))

        record_name = f"{DIST_INFO}/RECORD"
        record_data = render_record(records, record_name)
        wheel.writestr(record_name, record_data)

    return wheel_name


def build_editable(wheel_directory, config_settings=None, metadata_directory=None):
    wheel_name = f"{NORMALIZED}-{VERSION}-py3-none-any.whl"
    wheel_path = Path(wheel_directory) / wheel_name
    records: list[tuple[str, str, str]] = []
    pth_name = f"{NORMALIZED}.pth"
    pth_data = (str((Path.cwd() / "src").resolve()) + "\n").encode("utf-8")

    with zipfile.ZipFile(wheel_path, "w", compression=zipfile.ZIP_DEFLATED) as wheel:
        wheel.writestr(pth_name, pth_data)
        records.append(record_entry(pth_name, pth_data))

        metadata_files = build_metadata_files()
        for arcname, data in metadata_files.items():
            wheel.writestr(arcname, data)
            records.append(record_entry(arcname, data))

        record_name = f"{DIST_INFO}/RECORD"
        record_data = render_record(records, record_name)
        wheel.writestr(record_name, record_data)

    return wheel_name


def build_sdist(sdist_directory, config_settings=None):
    sdist_name = f"{NORMALIZED}-{VERSION}.tar.gz"
    sdist_path = Path(sdist_directory) / sdist_name
    root = f"{NORMALIZED}-{VERSION}"
    include = [Path("pyproject.toml"), Path("README.md"), Path("LICENSE"), Path("svgo_py_build.py")]
    include.extend(sorted(Path("src").rglob("*")))
    include.extend(sorted(Path("tests").rglob("*")) if Path("tests").exists() else [])

    with tarfile.open(sdist_path, "w:gz") as tar:
        for path in include:
            if path.is_file() and "__pycache__" not in path.parts and path.suffix != ".pyc":
                tar.add(path, arcname=f"{root}/{path.as_posix()}")
    return sdist_name


def write_metadata(dist_info: Path) -> None:
    for relative, data in build_metadata_files().items():
        if not relative.startswith(f"{DIST_INFO}/"):
            continue
        target = dist_info / relative.split("/", 1)[1]
        target.write_bytes(data)


def build_metadata_files() -> dict[str, bytes]:
    readme = Path("README.md").read_text(encoding="utf-8") if Path("README.md").exists() else ""
    metadata = "\n".join(
        [
            "Metadata-Version: 2.4",
            f"Name: {NAME}",
            f"Version: {VERSION}",
            "Summary: Pure-Python SVG path editing, optimization, matrix geometry, measurement, sanitization, validation, tracing, and centerline reconstruction.",
            "Author: xmodar",
            "License: MIT",
            "Requires-Python: >=3.11",
            "Classifier: Development Status :: 4 - Beta",
            "Classifier: Environment :: Console",
            "Classifier: Intended Audience :: Developers",
            "Classifier: License :: OSI Approved :: MIT License",
            "Classifier: Programming Language :: Python :: 3",
            "Classifier: Programming Language :: Python :: 3.11",
            "Classifier: Programming Language :: Python :: 3.12",
            "Classifier: Topic :: Multimedia :: Graphics",
            "Classifier: Topic :: Software Development :: Libraries :: Python Modules",
            "Keywords: svg,path,optimizer,matrix,measurement,sanitize,validation,trace,centerline,cli",
            "Project-URL: Homepage, https://github.com/xmodar/svgo",
            "Project-URL: Repository, https://github.com/xmodar/svgo",
            "Project-URL: Issues, https://github.com/xmodar/svgo/issues",
            "Description-Content-Type: text/markdown",
            "",
            readme,
        ]
    ).encode("utf-8")
    wheel = "\n".join(
        [
            "Wheel-Version: 1.0",
            "Generator: svgo_py_build",
            "Root-Is-Purelib: true",
            "Tag: py3-none-any",
            "",
        ]
    ).encode("utf-8")
    entry_points = "\n".join(
        [
            "[console_scripts]",
            "svgo=svgo_py.cli:main",
            "",
        ]
    ).encode("utf-8")
    return {
        f"{DIST_INFO}/METADATA": metadata,
        f"{DIST_INFO}/WHEEL": wheel,
        f"{DIST_INFO}/entry_points.txt": entry_points,
    }


def record_entry(path: str, data: bytes) -> tuple[str, str, str]:
    digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode("ascii")
    return path, f"sha256={digest}", str(len(data))


def render_record(records: list[tuple[str, str, str]], record_name: str) -> bytes:
    output = io.StringIO()
    writer = csv.writer(output, lineterminator="\n")
    for row in records:
        writer.writerow(row)
    writer.writerow((record_name, "", ""))
    return output.getvalue().encode("utf-8")
