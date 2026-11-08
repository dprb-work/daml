#!/usr/bin/env python3
"""Verify one DAML wheel, then drive real consumers in a disposable installation."""

from __future__ import annotations

import argparse
import base64
import csv
import hashlib
import io
import json
import os
import re
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import tomllib
import zipfile
from email import policy
from email.parser import BytesParser
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[3]
PAYLOAD = "daml/_distribution/"

# Runs only in the newly created environment, never through the source checkout.
CONSUMER = r"""
import importlib.metadata
import importlib.util
import json
from pathlib import Path
import sys
import sysconfig

import daml
import daml.daml as native
import daml._type_schema as adapter


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


prefix = Path(sys.prefix).resolve()
require(sys.prefix != sys.base_prefix, "consumer is not in a virtual environment")
require(sys.implementation.name == "cpython", "only standard CPython is supported")
require(
    not sysconfig.get_config_var("Py_GIL_DISABLED"),
    "free-threaded Python is unsupported",
)
origins = {}
for module in (daml, native, adapter):
    origin = Path(module.__file__).resolve()
    require(origin.is_relative_to(prefix), f"module escaped venv: {origin}")
    origins[module.__name__] = str(origin)
distribution = importlib.metadata.distribution("daml-config")
require(
    Path(distribution.locate_file("")).resolve().is_relative_to(prefix),
    "metadata escaped venv",
)
require(
    not any(
        Path(path).resolve().is_relative_to(Path(sys.argv[2]))
        for path in sys.path if path
    ),
    "checkout leaked into sys.path",
)
require(not list(prefix.rglob("*.egg-link")), "editable egg-link installed")
for direct in prefix.rglob("direct_url.json"):
    require(
        not json.loads(direct.read_text()).get("dir_info", {}).get("editable", False),
        "editable distribution installed",
    )
result = {"origins": origins, "version": distribution.version,
          "python": sys.version, "executable": sys.executable}
if sys.argv[1] == "base":
    require(
        importlib.util.find_spec("pydantic") is None,
        "Pydantic leaked into base environment",
    )
    value = daml.parse('{name: "demo"\nretries: 3}').to_python()
    require(
        type(value) is dict and value == {"name": "demo", "retries": 3},
        "base parse failed",
    )
    result.update(pydantic_absent=True, parsed=value)
else:
    # Deliberately defer the optional dependency until the no-Pydantic base run passed.
    from dataclasses import dataclass
    from pydantic import BaseModel

    class PydanticWorker(BaseModel):
        name: str
        retries: int = 3

    @dataclass
    class DataclassWorker:
        name: str
        retries: int = 3

    workflows = {}
    for kind, worker in (("pydantic", PydanticWorker), ("dataclass", DataclassWorker)):
        schemas = daml.Schemas.compose([daml.schema_from_types({"app.Worker": worker})])
        values = {}
        for label, source, expected in (
            ("default", '!app.Worker {name: "demo"}', 3),
            ("override", '!app.Worker {name: "demo"\nretries: 5}', 5),
        ):
            value = (
                schemas.resolve(daml.parse(source), expected="app.Worker").to_python()
            )
            require(
                type(value) is dict and value == {"name": "demo", "retries": expected},
                f"{kind} {label} did not return plain resolved values",
            )
            require(
                type(value["name"]) is str and type(value["retries"]) is int,
                "result is not JSON-shaped",
            )
            require(json.loads(json.dumps(value)) == value, "result is not JSON-shaped")
            values[label] = value
        try:
            schemas.resolve(
                daml.parse('!app.Worker {name: "demo"\nretries: "many"}'),
                expected="app.Worker",
            )
        except daml.ResolveError as error:
            require(
                error.path == "/retries",
                f"{kind} rejection has wrong path: {error.path}",
            )
            values["invalid_path"] = error.path
        else:
            raise RuntimeError(f"{kind} accepted invalid string retries")
        workflows[kind] = values
    result.update(
        workflows=workflows,
        pydantic_version=importlib.metadata.version("pydantic"),
    )
print(json.dumps(result))
"""


def require(condition: object, message: str) -> None:
    if not condition:
        raise ValueError(message)


def digest_file(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def safe_member(member: zipfile.ZipInfo) -> str:
    name = member.filename
    path = PurePosixPath(name)
    canonical = str(path) + ("/" if member.is_dir() else "")
    require(
        name == canonical
        and not path.is_absolute()
        and ".." not in path.parts
        and "\\" not in name
        and ":" not in name
        and "\x00" not in name
        and bool(path.parts),
        f"unsafe wheel member: {name!r}",
    )
    require(
        all(
            not part.endswith((" ", "."))
            and not any(ord(char) < 32 for char in part)
            and part.split(".")[0].upper()
            not in {
                "CON",
                "PRN",
                "AUX",
                "NUL",
                *(f"COM{i}" for i in range(1, 10)),
                *(f"LPT{i}" for i in range(1, 10)),
            }
            for part in path.parts
        ),
        f"unsafe portable wheel member: {name!r}",
    )
    mode = stat.S_IFMT(member.external_attr >> 16)
    require(
        mode in (0, stat.S_IFDIR if member.is_dir() else stat.S_IFREG),
        f"special wheel member: {name}",
    )
    require(not member.flag_bits & 1, f"encrypted wheel member: {name}")
    return str(path)


def verify_record(files: dict[str, bytes], info: str) -> None:
    record = info + "/RECORD"
    require(record in files, "wheel is missing RECORD")
    rows = csv.reader(io.StringIO(files[record].decode("utf-8"), newline=""))
    seen: set[str] = set()
    for row in rows:
        require(len(row) == 3, "invalid RECORD row")
        name, checksum, size = row
        require(
            name in files and name not in seen,
            f"unknown/duplicate RECORD member: {name}",
        )
        seen.add(name)
        if name == record:
            require(checksum == size == "", "RECORD must not hash itself")
            continue
        expected = (
            base64.urlsafe_b64encode(hashlib.sha256(files[name]).digest())
            .rstrip(b"=")
            .decode()
        )
        require(
            checksum == "sha256=" + expected and size == str(len(files[name])),
            f"RECORD digest/size mismatch: {name}",
        )
    require(seen == files.keys(), "RECORD does not cover the exact wheel payload")


def platform_contract(
    platform_tag: str, target: str, binary: bytes, build: dict
) -> None:
    tags = platform_tag.split(".")
    if target == "x86_64-unknown-linux-gnu":
        require(
            binary[:4] == b"\x7fELF"
            and binary[4:6] == b"\x02\x01"
            and struct.unpack_from("<H", binary, 18)[0] == 62,
            "Linux extension is not x86-64 ELF",
        )
        floors = []
        for tag in tags:
            match = re.fullmatch(r"manylinux_2_(\d+)_x86_64", tag)
            legacy = {
                "manylinux1_x86_64": 5,
                "manylinux2010_x86_64": 12,
                "manylinux2014_x86_64": 17,
            }
            require(
                match is not None or tag in legacy,
                "Linux wheel needs an audited manylinux tag",
            )
            floor = int(match[1]) if match else legacy[tag]
            require(floor <= 35, "Linux wheel exceeds the glibc 2.35 floor")
            floors.append((2, floor))
        references = {
            tuple(map(int, match))
            for match in re.findall(rb"GLIBC_(\d+)\.(\d+)", binary)
        }
        require(
            references and max(references) <= min(floors),
            "extension GLIBC references exceed its declared manylinux floor",
        )
    elif target == "aarch64-apple-darwin":
        require(
            binary[:4] == b"\xcf\xfa\xed\xfe"
            and struct.unpack_from("<I", binary, 4)[0] == 0x0100000C,
            "macOS extension is not ARM64 Mach-O",
        )
        require(
            tags == ["macosx_11_0_arm64"]
            and build["macos_deployment_target"] == "11.0",
            "macOS wheel must honestly declare its explicit "
            "11.0 ARM64 deployment target",
        )
        # Inspect LC_BUILD_VERSION/LC_VERSION_MIN_MACOSX, not just the filename.
        offset = 32
        versions = []
        for _ in range(struct.unpack_from("<I", binary, 16)[0]):
            command, size = struct.unpack_from("<II", binary, offset)
            require(
                size >= 8 and offset + size <= len(binary),
                "invalid Mach-O load command",
            )
            if command in (0x32, 0x24):
                version = struct.unpack_from(
                    "<I", binary, offset + (12 if command == 0x32 else 8)
                )[0]
                versions.append((version >> 16, (version >> 8) & 255, version & 255))
            offset += size
        require(
            versions and max(versions) <= (11, 0, 0),
            "Mach-O minimum OS exceeds wheel tag",
        )
    elif target == "x86_64-pc-windows-msvc":
        require(
            tags == ["win_amd64"] and binary[:2] == b"MZ", "invalid Windows x64 wheel"
        )
        offset = struct.unpack_from("<I", binary, 60)[0]
        require(
            binary[offset : offset + 4] == b"PE\0\0"
            and struct.unpack_from("<H", binary, offset + 4)[0] == 0x8664,
            "Windows extension is not AMD64 PE",
        )
    else:
        raise ValueError(f"unsupported native target: {target}")
    if target != "aarch64-apple-darwin":
        require(
            build["macos_deployment_target"] is None,
            "unexpected macOS deployment target",
        )


def inspect_wheel(wheel: Path) -> dict:
    require(
        wheel.is_absolute() and wheel.suffix == ".whl", "supply an absolute wheel path"
    )
    digest = digest_file(wheel)
    sidecar = wheel.with_name(wheel.name + ".sha256").read_text(encoding="ascii")
    require(
        sidecar == f"{digest}  {wheel.name}\n",
        "wheel checksum/sidecar filename does not match",
    )
    project = tomllib.loads((ROOT / "bindings/python/pyproject.toml").read_text())
    cargo = tomllib.loads((ROOT / "bindings/python/Cargo.toml").read_text())
    version = cargo["package"]["version"]
    minimum = project["project"]["requires-python"]
    abi_features = cargo["dependencies"]["pyo3"]["features"]
    abi = [
        feature.removeprefix("abi3-py")
        for feature in abi_features
        if feature.startswith("abi3-py")
    ]
    require(
        len(abi) == 1 and minimum == f">={abi[0][0]}.{abi[0][1:]}",
        "manifest Python minimum and abi3 feature disagree",
    )
    parts = wheel.name[:-4].split("-")
    require(len(parts) == 5, "unexpected wheel filename/build tag")
    distribution, wheel_version, python_tag, abi_tag, platform_tag = parts
    require(
        distribution == "daml_config" and project["project"]["name"] == "daml-config",
        "distribution must be daml-config (import remains daml)",
    )
    require(
        wheel_version == version and python_tag == "cp" + abi[0] and abi_tag == "abi3",
        "wheel version/Python stable ABI does not match manifests",
    )
    info = f"daml_config-{version}.dist-info"
    files: dict[str, bytes] = {}
    seen: set[str] = set()
    with zipfile.ZipFile(wheel) as bundle:
        for member in bundle.infolist():
            name = safe_member(member)
            require(
                name.casefold() not in seen,
                f"duplicate/case-colliding wheel member: {name}",
            )
            seen.add(name.casefold())
            require(
                PurePosixPath(name).parts[0] in ("daml", info),
                f"unexpected wheel root: {name}",
            )
            if not member.is_dir():
                files[name] = bundle.read(member)
    for name in files:
        require(
            not any(
                parent.as_posix() in files for parent in PurePosixPath(name).parents
            ),
            f"wheel file/directory collision: {name}",
        )
    verify_record(files, info)
    metadata = BytesParser(policy=policy.default).parsebytes(files[info + "/METADATA"])
    for key, expected in (
        ("Name", "daml-config"),
        ("Version", version),
        ("Requires-Python", minimum),
    ):
        require(
            metadata.get_all(key) == [expected],
            f"METADATA {key} differs from accepted manifest",
        )
    require(
        "types" in metadata.get_all("Provides-Extra", []),
        "wheel is missing types extra",
    )
    requirements = metadata.get_all("Requires-Dist", [])
    expected_extra = project["project"]["optional-dependencies"]["types"]
    require(
        len(requirements) == len(expected_extra) == 1,
        "unexpected base/types dependencies",
    )
    dependency = re.fullmatch(
        r"pydantic\s*([^;]+);\s*extra\s*==\s*['\"]types['\"]", requirements[0]
    )
    require(
        dependency is not None
        and {item.strip() for item in dependency[1].split(",")}
        == {
            item.strip()
            for item in expected_extra[0].removeprefix("pydantic").split(",")
        },
        "types dependency differs from manifest or is not optional",
    )
    wheel_metadata = BytesParser(policy=policy.default).parsebytes(
        files[info + "/WHEEL"]
    )
    require(
        wheel_metadata.get_all("Root-Is-Purelib") == ["false"],
        "native wheel claims pure Python",
    )
    require(
        set(wheel_metadata.get_all("Tag", []))
        == {f"{python_tag}-{abi_tag}-{tag}" for tag in platform_tag.split(".")},
        "WHEEL tags disagree with filename",
    )
    source_root = (
        ROOT / "bindings/python" / project["tool"]["maturin"]["python-source"] / "daml"
    )
    expected_sources = {
        "daml/" + source.relative_to(source_root).as_posix(): source.read_bytes()
        for source in source_root.rglob("*")
        if source.is_file()
        and (source.suffix in (".py", ".pyi") or source.name == "py.typed")
        and "_distribution" not in source.relative_to(source_root).parts
    }
    require(
        {"daml/__init__.py", "daml/daml.pyi", "daml/py.typed", "daml/_type_schema.py"}
        <= expected_sources.keys(),
        "source package is missing required import/stub/schema files",
    )
    for name, expected in expected_sources.items():
        require(
            files.get(name) == expected, f"wheel source/stub payload differs: {name}"
        )
    for name in ("LICENSE", "SOURCES.md"):
        require(
            files.get(PAYLOAD + name) == (ROOT / name).read_bytes(),
            f"noncanonical {name}",
        )
    notices = files[PAYLOAD + "THIRD-PARTY-NOTICES.txt"].decode("utf-8")
    template = (ROOT / "scripts/third-party-notices.hbs").read_text()
    require(
        notices.startswith(template.split("{{#each crates}}")[0])
        and "License texts\n" in notices
        and "Permission is hereby granted" in notices,
        "missing canonical generated dependency notices/license texts",
    )
    lock = tomllib.loads((ROOT / "bindings/python/Cargo.lock").read_text())
    for name in ("pyo3", "serde_json"):
        packages = [package for package in lock["package"] if package["name"] == name]
        require(
            len(packages) == 1
            and f"{name} {packages[0]['version']}\nLicense:" in notices,
            f"notices omit locked {name} dependency",
        )
    build = json.loads(files[PAYLOAD + "BUILD.json"])
    require(isinstance(build, dict), "BUILD.json must be an object")
    require(
        build.get("version") == version
        and build.get("distribution") == "daml-config"
        and build.get("profile") == "release",
        "incorrect BUILD distribution/version/profile",
    )
    require(type(build.get("git_dirty")) is bool, "BUILD git_dirty must be boolean")
    require(
        isinstance(build.get("git_commit"), str)
        and re.fullmatch(r"[0-9a-f]{40}", build["git_commit"]),
        "invalid BUILD commit",
    )
    commit = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    require(build["git_commit"] == commit, "wheel belongs to a different source commit")
    backend = project["build-system"]["requires"]
    require(
        len(backend) == 1 and backend[0].startswith("maturin=="),
        "unexpected wheel backend",
    )
    require(
        build.get("maturin") == backend[0].replace("==", " "), "incorrect BUILD maturin"
    )
    require(
        isinstance(build.get("rustc"), str) and build["rustc"].startswith("rustc "),
        "missing BUILD Rust identity",
    )
    require(
        build.get("auditwheel") == "check",
        "BUILD must record audited, non-relabelled packaging",
    )
    require(
        build.get("compatibility")
        == (
            ["pypi", "manylinux_2_35"]
            if build.get("target") == "x86_64-unknown-linux-gnu"
            else ["pypi"]
        ),
        "BUILD compatibility does not match the accepted platform policy",
    )
    require(
        build.get("notices")
        == {
            "generator": "scripts/cargo-notices.py",
            "tool": "cargo-about 0.9.2",
            "manifest": "bindings/python/Cargo.toml",
            "target": build.get("target"),
        },
        "BUILD notices provenance does not identify "
        "the canonical Python dependency graph",
    )
    native = [
        name
        for name in files
        if name.startswith("daml/daml.") and name.endswith((".so", ".pyd"))
    ]
    require(len(native) == 1, "wheel must contain exactly one native daml extension")
    require(
        native[0].endswith(".abi3.so")
        if not platform_tag.startswith("win")
        else native[0] == "daml/daml.pyd",
        "extension does not use the stable ABI filename",
    )
    allowed = (
        set(expected_sources)
        | {native[0]}
        | {
            PAYLOAD + name
            for name in (
                "LICENSE",
                "SOURCES.md",
                "THIRD-PARTY-NOTICES.txt",
                "BUILD.json",
            )
        }
    )
    require(
        all(name in allowed or name.startswith(info + "/") for name in files),
        "wheel contains unexpected import/runtime payload",
    )
    platform_contract(platform_tag, build["target"], files[native[0]], build)
    return {
        "wheel": str(wheel),
        "wheel_sha256": digest,
        "build": build,
        "tags": wheel_metadata.get_all("Tag"),
        "requires_python": minimum,
        "checked_files": len(files),
    }


def run(command: list[str], cwd: Path, env: dict[str, str]) -> str:
    completed = subprocess.run(
        command, cwd=cwd, env=env, capture_output=True, text=True
    )
    if completed.stdout:
        print(
            completed.stdout,
            file=sys.stderr,
            end="" if completed.stdout.endswith("\n") else "\n",
        )
    if completed.stderr:
        print(
            completed.stderr,
            file=sys.stderr,
            end="" if completed.stderr.endswith("\n") else "\n",
        )
    completed.check_returncode()
    return completed.stdout


def installed_workflows(wheel: Path, interpreter: str, receipt: dict) -> None:
    uv = shutil.which("uv")
    require(uv, "uv is required on PATH; verifier never installs build tools")
    uv = str(Path(uv).resolve())
    python = shutil.which(interpreter)
    require(python, f"Python interpreter not found: {interpreter}")
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.upper().startswith(("PYTHON", "UV_"))
        and key not in ("VIRTUAL_ENV", "CONDA_PREFIX")
    }
    env.update(UV_PYTHON_DOWNLOADS="never")
    # Explicit system temp placement avoids TMPDIR pointing back into a checkout.
    temp_root = Path(tempfile.gettempdir()).resolve()
    if temp_root.is_relative_to(ROOT):
        temp_root = ROOT.parent
    with tempfile.TemporaryDirectory(
        prefix="daml-wheel-contracts-", dir=temp_root
    ) as temporary:
        scratch = Path(temporary).resolve()
        require(not scratch.is_relative_to(ROOT), "scratch must be outside checkout")
        receipt["scratch"] = str(scratch)
        receipt["cleanup"] = "pending"
        venv = scratch / "venv"
        run(
            [
                uv,
                "--no-config",
                "venv",
                "--python",
                str(Path(python).resolve()),
                str(venv),
            ],
            scratch,
            env,
        )
        executable = venv / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
        probe = run(
            [
                str(executable),
                "-I",
                "-c",
                "import json,platform,sys,sysconfig; print(json.dumps({"
                "'implementation':sys.implementation.name,'version':list(sys.version_info[:2]),"
                "'free_threaded':bool(sysconfig.get_config_var('Py_GIL_DISABLED')),"
                "'platform':sys.platform,'machine':platform.machine().lower(),"
                "'libc':platform.libc_ver(),'macos':platform.mac_ver()[0]}))",
            ],
            scratch,
            env,
        )
        identity = json.loads(probe)
        minimum = list(
            map(int, receipt["requires_python"].removeprefix(">=").split("."))
        )
        require(
            identity["implementation"] == "cpython"
            and identity["version"] >= minimum
            and not identity["free_threaded"],
            "use standard CPython 3.12+, not PyPy/free-threaded",
        )
        host = (identity["platform"], identity["machine"])
        expected_hosts = {
            "x86_64-unknown-linux-gnu": {("linux", "x86_64")},
            "aarch64-apple-darwin": {("darwin", "arm64")},
            "x86_64-pc-windows-msvc": {("win32", "amd64")},
        }
        require(
            host in expected_hosts[receipt["build"]["target"]],
            "wheel target does not match interpreter host",
        )
        if host[0] == "linux":
            require(
                identity["libc"][0] == "glibc"
                and tuple(map(int, identity["libc"][1].split(".")[:2])) >= (2, 35),
                "candidate needs a glibc 2.35+ host (musl is unsupported)",
            )
        if host[0] == "darwin":
            require(
                tuple(map(int, identity["macos"].split(".")[:2])) >= (11, 0),
                "candidate needs macOS 11.0+",
            )
        receipt["interpreter"] = identity
        # No Cargo/Rust commands can be discovered by the installer. uv is absolute;
        # --only-binary :all: forbids source-build backends, including extras.
        empty_path = scratch / "no-build-tools"
        empty_path.mkdir()
        env["PATH"] = str(empty_path)
        install = [
            uv,
            "--no-config",
            "pip",
            "install",
            "--python",
            str(executable),
            "--only-binary",
            ":all:",
        ]
        # Snapshot the already inspected bytes: neither stage can substitute
        # an index package.
        candidate = scratch / wheel.name
        shutil.copyfile(wheel, candidate)
        require(
            digest_file(candidate) == receipt["wheel_sha256"],
            "candidate changed after inspection",
        )
        run([*install, "--no-index", "--no-deps", str(candidate)], scratch, env)
        receipt["base"] = json.loads(
            run(
                [str(executable), "-I", "-c", CONSUMER, "base", str(ROOT)], scratch, env
            )
        )
        require(
            receipt["base"]["version"] == receipt["build"]["version"],
            "installed version mismatch",
        )
        run([*install, f"daml-config[types] @ {candidate.as_uri()}"], scratch, env)
        receipt["types"] = json.loads(
            run(
                [str(executable), "-I", "-c", CONSUMER, "types", str(ROOT)],
                scratch,
                env,
            )
        )
        require(
            digest_file(candidate) == receipt["wheel_sha256"],
            "installed candidate changed",
        )
    receipt["cleanup"] = "removed" if not scratch.exists() else "failed"
    require(receipt["cleanup"] == "removed", "temporary installation survived cleanup")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wheel", type=Path)
    parser.add_argument(
        "--python", default=sys.executable, help="standard CPython 3.12+ interpreter"
    )
    args = parser.parse_args()
    receipt: dict = {
        "status": "failed",
        "verification": "daml-installed-wheel-v1",
        "wheel": str(args.wheel),
    }
    try:
        candidate = args.wheel.expanduser().resolve()
        receipt["wheel"] = str(candidate)
        receipt.update(inspect_wheel(candidate))
        installed_workflows(candidate, args.python, receipt)
        receipt["status"] = "passed"
    except (
        OSError,
        ValueError,
        KeyError,
        struct.error,
        zipfile.BadZipFile,
        subprocess.CalledProcessError,
    ) as error:
        receipt["error"] = f"{type(error).__name__}: {error}"
    finally:
        if "scratch" in receipt:
            receipt["cleanup"] = (
                "removed" if not Path(receipt["scratch"]).exists() else "failed"
            )
        print(json.dumps(receipt, indent=2))
    return 0 if receipt["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
