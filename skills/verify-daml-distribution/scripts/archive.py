#!/usr/bin/env python3
"""Verify/extract a DAML bundle and exercise its installed executable contracts."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import subprocess
import tarfile
import tempfile
from typing import IO, cast
import zipfile


def safe_path(name: str) -> PurePosixPath:
    path = PurePosixPath(name)
    if path.is_absolute() or ".." in path.parts or "\\" in name or ":" in name or not path.parts:
        raise ValueError(f"unsafe archive member: {name}")
    return path


def store(source: IO[bytes], path: PurePosixPath, destination: Path, files: set[PurePosixPath]) -> None:
    if path in files:
        raise ValueError(f"duplicate archive member: {path}")
    files.add(path)
    target = destination.joinpath(*path.parts)
    target.parent.mkdir(parents=True, exist_ok=True)
    with target.open("xb") as output:
        shutil.copyfileobj(source, output)
    target.chmod(0o755 if path.parent.name == "bin" else 0o644)


def unpack(archive: Path, destination: Path) -> tuple[Path, dict[str, object]]:
    """Verify the sidecar and fixed bundle layout, installing only to a new path."""
    archive = archive.resolve()
    expected = archive.with_name(archive.name + ".sha256").read_text(encoding="ascii").split()
    with archive.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    if expected != [digest, archive.name]:
        raise ValueError("archive checksum or sidecar filename does not match")
    if destination.exists():
        raise ValueError(f"installation destination already exists: {destination}")
    destination.mkdir(parents=True)
    files: set[PurePosixPath] = set()
    is_zip = archive.name.endswith(".zip")
    if is_zip:
        with zipfile.ZipFile(archive) as bundle:
            for member in bundle.infolist():
                path = safe_path(member.filename)
                if member.is_dir():
                    continue
                if stat.S_IFMT(member.external_attr >> 16) not in (0, stat.S_IFREG):
                    raise ValueError(f"non-regular archive member: {path}")
                with bundle.open(member) as source:
                    store(source, path, destination, files)
    else:
        with tarfile.open(archive, "r:gz") as bundle:
            for member in bundle.getmembers():
                path = safe_path(member.name)
                if member.isdir():
                    continue
                if not member.isfile():
                    raise ValueError(f"non-regular archive member: {path}")
                source = bundle.extractfile(member)
                if source is None:
                    raise ValueError(f"archive file has no content: {path}")
                with source:
                    store(source, path, destination, files)
    roots = {path.parts[0] for path in files}
    if len(roots) != 1:
        raise ValueError("bundle must contain one top-level directory")
    root = roots.pop()
    suffix = ".exe" if is_zip else ""
    required = {PurePosixPath(root) / name for name in (
        "BUILD.json", "LICENSE", "SOURCES.md", "THIRD-PARTY-NOTICES.txt",
        f"bin/daml-lsp{suffix}", f"bin/daml-fmt{suffix}",
    )}
    if files != required:
        raise ValueError("bundle does not contain exactly the expected files")
    installed = destination / root
    loaded: object = json.loads((installed / "BUILD.json").read_text(encoding="utf-8"))
    if not isinstance(loaded, dict) or not all(isinstance(key, str) for key in loaded):
        raise ValueError("build manifest must be an object with string keys")
    build = cast(dict[str, object], loaded)
    for key in ("version", "target", "git_commit", "rustc"):
        if not isinstance(build.get(key), str) or not build[key]:
            raise ValueError(f"build manifest is missing {key}")
    if root != f"daml-{build['version']}-{build['target']}":
        raise ValueError("archive root does not match its build identity")
    return installed, {**build, "archive_sha256": digest}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    _ = parser.add_argument("archive", type=Path)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[3]
    with tempfile.TemporaryDirectory(prefix="daml-distribution-contracts-") as temporary:
        installed, build = unpack(args.archive, Path(temporary) / "install")
        env = os.environ.copy()
        env["DAML_TEST_BIN_DIR"] = str(installed / "bin")
        _ = subprocess.run([
            "cargo", "test", "--release", "--locked", "--features", "lsp",
            "--target", str(build["target"]), "--test", "lsp", "--test", "formatter_cli",
        ], cwd=repo, env=env, check=True)
        print(json.dumps({"installed_contracts": "passed", **build}, indent=2))


if __name__ == "__main__":
    main()
