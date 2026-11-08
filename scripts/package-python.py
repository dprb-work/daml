#!/usr/bin/env python3
"""Build one unpublished, native daml-config wheel and its SHA-256 sidecar."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import sysconfig
import tempfile
import tomllib
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PROJECT = ROOT / "bindings" / "python"
MANIFEST = PROJECT / "Cargo.toml"
PAYLOAD = PROJECT / "python" / "daml" / "_distribution"
MATURIN_VERSION = "maturin 1.15.0"
HOSTS = {
    "x86_64-unknown-linux-gnu": ("Linux", {"x86_64", "amd64"}),
    "aarch64-apple-darwin": ("Darwin", {"arm64", "aarch64"}),
    "x86_64-pc-windows-msvc": ("Windows", {"amd64", "x86_64"}),
}


def run(command: list[str], *, env: dict[str, str] | None = None) -> str:
    try:
        result = subprocess.run(
            command,
            cwd=ROOT,
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
    if host not in HOSTS:
        raise RuntimeError(
            f"unsupported rustc host: {host!r}; supported: {', '.join(HOSTS)}"
        )
    system, machines = HOSTS[host]
    if platform.system() != system or platform.machine().lower() not in machines:
        raise RuntimeError(
            f"Python and rustc must run natively on the same supported host: {host}"
        )
    if sys.maxsize <= 2**32:
        raise RuntimeError(
            "wheel production requires a native 64-bit Python interpreter"
        )
    return host, run(["rustc", "--version"]).strip()


@contextmanager
def staged_payload() -> Iterator[Path]:
    # Exclusive creation also prevents concurrent producers sharing this worktree.
    # Never overwrite, remove, or restore a pre-existing source/install payload.
    try:
        PAYLOAD.mkdir()
    except FileExistsError as exc:
        raise RuntimeError(
            "staging path already exists; preserve or remove it before building: "
            f"{PAYLOAD}"
        ) from exc
    try:
        yield PAYLOAD
    finally:
        original_error = sys.exception()
        try:
            shutil.rmtree(PAYLOAD)
        except OSError as exc:
            if original_error is None:
                raise
            print(
                f"package-python: could not clean staging path {PAYLOAD}: {exc}",
                file=sys.stderr,
            )


def package(out: Path) -> Path:
    if sys.version_info < (3, 12) or sys.implementation.name != "cpython":
        raise RuntimeError("run the producer with standard CPython 3.12 or newer")
    if sysconfig.get_config_var("Py_GIL_DISABLED"):
        raise RuntimeError(
            "free-threaded Python is not supported by this abi3 wheel producer"
        )
    # uv --with can expose the CLI on PATH outside Python's sysconfig scripts dir.
    # Resolve once so the version probe and build invoke the same executable.
    maturin_command = shutil.which("maturin")
    if maturin_command is None:
        raise RuntimeError(
            f"required command not found: maturin; install {MATURIN_VERSION} on PATH"
        )
    maturin = run([maturin_command, "--version"]).strip()
    if maturin != MATURIN_VERSION:
        raise RuntimeError(
            f"expected {MATURIN_VERSION} on PATH, found {maturin!r} "
            f"at {maturin_command}"
        )
    host, rustc = native_target()
    # Probe Cargo before staging or running the notice producer.
    run(["cargo", "--version"])
    manifest = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))
    version = manifest["package"]["version"]
    project = tomllib.loads((PROJECT / "pyproject.toml").read_text(encoding="utf-8"))
    distribution = project["project"]["name"]
    if distribution != "daml-config":
        raise RuntimeError(f"unexpected Python distribution name: {distribution!r}")
    commit = run(["git", "rev-parse", "HEAD"]).strip()
    if len(commit) != 40:
        raise RuntimeError(f"unexpected full git commit id: {commit!r}")
    dirty = bool(
        run(["git", "status", "--porcelain", "--untracked-files=normal"]).strip()
    )
    epoch = int(run(["git", "show", "-s", "--format=%ct", "HEAD"]).strip())
    compatibility = ["pypi"]
    if host == "x86_64-unknown-linux-gnu":
        # Ubuntu 22.04's glibc floor; maturin checks the actual linked symbols.
        # No fallback to an unaudited linux tag, repair, or manual retagging.
        compatibility.append("manylinux_2_35")
    deployment_target = "11.0" if host == "aarch64-apple-darwin" else None
    build = {
        "distribution": distribution,
        "version": version,
        "git_commit": commit,
        "git_dirty": dirty,
        "target": host,
        "rustc": rustc,
        "maturin": maturin,
        "profile": "release",
        "compatibility": compatibility,
        "auditwheel": "check",
        "macos_deployment_target": deployment_target,
        "notices": {
            "generator": "scripts/cargo-notices.py",
            "tool": "cargo-about 0.9.2",
            "manifest": "bindings/python/Cargo.toml",
            "target": host,
        },
    }
    out.mkdir(parents=True, exist_ok=True)
    if any(out.glob("*.whl")) or any(out.glob("*.whl.sha256")):
        raise RuntimeError(
            "output directory already contains wheel candidates; "
            f"use an empty directory: {out}"
        )
    env = os.environ.copy()
    env["PYO3_PYTHON"] = sys.executable
    env["SOURCE_DATE_EPOCH"] = str(epoch)
    if deployment_target:
        # Both the compiler/linker and maturin's tag calculation consume this.
        env["MACOSX_DEPLOYMENT_TARGET"] = deployment_target
    with tempfile.TemporaryDirectory(prefix=".daml-python-", dir=out) as temporary:
        temporary_path = Path(temporary)
        with staged_payload() as stage:
            shutil.copyfile(ROOT / "LICENSE", stage / "LICENSE")
            shutil.copyfile(ROOT / "SOURCES.md", stage / "SOURCES.md")
            notices = run(
                [
                    sys.executable,
                    "-B",
                    str(ROOT / "scripts" / "cargo-notices.py"),
                    "--manifest-path",
                    str(MANIFEST),
                    "--target",
                    host,
                ]
            )
            (stage / "THIRD-PARTY-NOTICES.txt").write_text(
                notices, encoding="utf-8", newline="\n"
            )
            (stage / "BUILD.json").write_text(
                json.dumps(build, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
                newline="\n",
            )
            run(
                [
                    maturin_command,
                    "build",
                    "--release",
                    "--locked",
                    "--manifest-path",
                    str(MANIFEST),
                    "--target",
                    host,
                    "--interpreter",
                    sys.executable,
                    "--compatibility",
                    *compatibility,
                    "--auditwheel",
                    "check",
                    "--out",
                    str(temporary_path),
                ],
                env=env,
            )
        wheels = list(temporary_path.glob("*.whl"))
        if len(wheels) != 1:
            raise RuntimeError(
                f"maturin must produce exactly one wheel, found {len(wheels)}"
            )
        wheel = wheels[0]
        prefix = f"daml_config-{version}-cp312-abi3-"
        if not wheel.name.startswith(prefix):
            raise RuntimeError(
                f"maturin produced an unexpected distribution/ABI: {wheel.name}"
            )
        with wheel.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        sidecar = temporary_path / (wheel.name + ".sha256")
        sidecar.write_text(f"{digest}  {wheel.name}\n", encoding="ascii", newline="\n")
        destination = out / wheel.name
        os.replace(wheel, destination)
        os.replace(sidecar, out / sidecar.name)
    return destination


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=ROOT / "dist" / "python",
        help="empty candidate output directory (default: repository dist/python/)",
    )
    args = parser.parse_args()
    try:
        wheel = package(args.out.expanduser().resolve())
    except (OSError, RuntimeError, KeyError, ValueError) as exc:
        print(f"package-python: error: {exc}", file=sys.stderr)
        return 1
    print(wheel)
    print(wheel.with_name(wheel.name + ".sha256"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
