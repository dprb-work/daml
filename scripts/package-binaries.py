#!/usr/bin/env python3
"""Build and package the native DAML command-line binaries."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "Cargo.toml"


def run(command: list[str], *, cwd: Path = ROOT, env: dict[str, str] | None = None) -> str:
    try:
        result = subprocess.run(
            command,
            cwd=cwd,
            env=env,
            check=True,
            text=True,
            encoding="utf-8",
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except FileNotFoundError as exc:
        raise RuntimeError(f"required command not found: {command[0]}") from exc
    except subprocess.CalledProcessError as exc:
        detail = exc.stderr.strip() or exc.stdout.strip()
        raise RuntimeError(f"command failed ({' '.join(command)}): {detail}") from exc
    return result.stdout


def native_target() -> tuple[str, str]:
    details = run(["rustc", "-vV"])
    values = dict(line.split(": ", 1) for line in details.splitlines() if ": " in line)
    host = values.get("host")
    if not host:
        raise RuntimeError("rustc -vV did not report a host target")
    version = run(["rustc", "--version"]).strip()
    return host, version


def cargo_metadata(host: str) -> tuple[str, Path]:
    output = run(
        [
            "cargo", "metadata", "--locked", "--all-features", "--filter-platform", host,
            "--format-version", "1", "--manifest-path", str(MANIFEST),
        ]
    )
    metadata = json.loads(output)
    package = next((item for item in metadata["packages"] if Path(item["manifest_path"]) == MANIFEST), None)
    if package is None:
        raise RuntimeError(f"Cargo metadata did not include {MANIFEST}")
    return package["version"], Path(metadata["target_directory"])


def git_build_info() -> tuple[str, bool, int]:
    commit = run(["git", "rev-parse", "HEAD"]).strip()
    if len(commit) != 40:
        raise RuntimeError(f"unexpected full git commit id: {commit!r}")
    dirty = bool(run(["git", "status", "--porcelain", "--untracked-files=normal"]).strip())
    epoch_text = run(["git", "show", "-s", "--format=%ct", "HEAD"]).strip()
    try:
        epoch = int(epoch_text)
    except ValueError as exc:
        raise RuntimeError(f"invalid git commit timestamp: {epoch_text!r}") from exc
    return commit, dirty, epoch


def copy_binaries(stage: Path, target_dir: Path, host: str) -> None:
    bindir = stage / "bin"
    bindir.mkdir()
    suffix = ".exe" if os.name == "nt" else ""
    for name in ("daml-fmt", "daml-lsp"):
        source = target_dir / host / "release" / f"{name}{suffix}"
        if not source.is_file():
            raise RuntimeError(f"cargo build did not produce required binary: {source}")
        shutil.copyfile(source, bindir / source.name)
        (bindir / source.name).chmod(0o755)



def write_build(path: Path, *, version: str, commit: str, dirty: bool, host: str, rustc: str) -> None:
    data = {
        "version": version,
        "git_commit": commit,
        "git_dirty": dirty,
        "target": host,
        "rustc": rustc,
        "build_features": ["lsp"],
    }
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8", newline="\n")


def archive_tar(stage: Path, destination: Path, epoch: int) -> None:
    with destination.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=epoch) as compressed:
            with tarfile.open(fileobj=compressed, mode="w|", format=tarfile.USTAR_FORMAT) as archive:
                for path in sorted(stage.rglob("*"), key=lambda item: item.relative_to(stage.parent).as_posix()):
                    relative = path.relative_to(stage.parent).as_posix()
                    info = archive.gettarinfo(str(path), arcname=relative)
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    info.mtime = epoch
                    info.mode = 0o755 if path.is_dir() or path.parent.name == "bin" else 0o644
                    if path.is_file():
                        with path.open("rb") as source:
                            archive.addfile(info, source)
                    else:
                        archive.addfile(info)


def archive_zip(stage: Path, destination: Path, epoch: int) -> None:
    timestamp = datetime.fromtimestamp(min(max(epoch, 315532800), 4354819198), tz=timezone.utc)
    date_time = (timestamp.year, timestamp.month, timestamp.day, timestamp.hour, timestamp.minute, timestamp.second)
    with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in sorted(stage.rglob("*"), key=lambda item: item.relative_to(stage.parent).as_posix()):
            relative = path.relative_to(stage.parent).as_posix()
            info = zipfile.ZipInfo(relative + ("/" if path.is_dir() else ""), date_time)
            info.create_system = 3
            mode = 0o40755 if path.is_dir() else 0o100755 if path.parent.name == "bin" else 0o100644
            info.external_attr = mode << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            if path.is_dir():
                archive.writestr(info, b"")
            else:
                with path.open("rb") as source, archive.open(info, "w", force_zip64=True) as target:
                    shutil.copyfileobj(source, target)

def checksum(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return f"{digest.hexdigest()}  {path.name}\n"


def package(out: Path) -> Path:
    host, rustc = native_target()
    version, target_dir = cargo_metadata(host)
    commit, dirty, epoch = git_build_info()
    base = f"daml-{version}-{host}"
    suffix = ".zip" if os.name == "nt" else ".tar.gz"
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".daml-package-", dir=out) as temporary:
        temporary_path = Path(temporary)
        stage = temporary_path / base
        stage.mkdir()
        run(["cargo", "build", "--release", "--locked", "--features", "lsp", "--bins", "--target", host])
        copy_binaries(stage, target_dir, host)
        write_build(stage / "BUILD.json", version=version, commit=commit, dirty=dirty, host=host, rustc=rustc)
        shutil.copyfile(ROOT / "SOURCES.md", stage / "SOURCES.md")
        shutil.copyfile(ROOT / "LICENSE", stage / "LICENSE")
        notices = run([
            sys.executable, "-B", str(ROOT / "scripts" / "cargo-notices.py"),
            "--manifest-path", str(MANIFEST), "--target", host, "--all-features",
        ])
        (stage / "THIRD-PARTY-NOTICES.txt").write_text(notices, encoding="utf-8", newline="\n")
        temporary_archive = temporary_path / (base + suffix)
        if suffix == ".zip":
            archive_zip(stage, temporary_archive, epoch)
        else:
            archive_tar(stage, temporary_archive, epoch)
        final_archive = out / temporary_archive.name
        os.replace(temporary_archive, final_archive)
        checksum_path = out / (final_archive.name + ".sha256")
        temporary_checksum = temporary_path / checksum_path.name
        temporary_checksum.write_text(checksum(final_archive), encoding="ascii", newline="\n")
        os.replace(temporary_checksum, checksum_path)
    return out / (base + suffix)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=ROOT / "dist", help="output directory (default: repository dist/)")
    args = parser.parse_args()
    try:
        archive = package(args.out.expanduser().resolve())
    except (OSError, RuntimeError, KeyError, ValueError, json.JSONDecodeError, tarfile.TarError, zipfile.BadZipFile) as exc:
        print(f"package-binaries: error: {exc}", file=sys.stderr)
        return 1
    print(archive)
    print(archive.with_name(archive.name + ".sha256"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
