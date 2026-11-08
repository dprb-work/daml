#!/usr/bin/env python3
"""Read-only source and release-candidate admission policy (stdlib only)."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import stat
import tarfile
import tomllib
import zipfile
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from email import policy
from email.parser import BytesParser
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parent.parent
REPOSITORY = "dprb-work/daml"
TARGETS = {
    "x86_64-unknown-linux-gnu": "manylinux_2_35_x86_64",
    "aarch64-apple-darwin": "macosx_11_0_arm64",
    "x86_64-pc-windows-msvc": "win_amd64",
}


def require(condition: object, message: str) -> None:
    if not condition:
        raise ValueError(message)


def canonical_version(version: object) -> str:
    require(
        isinstance(version, str)
        and re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version),
        f"unsupported release version: {version!r}; expected canonical x.y.z",
    )
    return version


def source_identity(source_sha: str) -> None:
    require(
        isinstance(source_sha, str) and re.fullmatch(r"[0-9a-f]{40}", source_sha),
        "source_sha must be a full lowercase 40-hex commit",
    )


def _object(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON key: {key}")
        result[key] = value
    return result


def load_json(content: bytes) -> dict:
    value = json.loads(content, object_pairs_hook=_object)
    require(isinstance(value, dict), "JSON document must be an object")
    return value


def check_versions(root: Path = ROOT) -> str:
    root = Path(root)
    versions = {}
    for name in (
        "Cargo.toml",
        "bindings/python/Cargo.toml",
        "bindings/browser/Cargo.toml",
    ):
        versions[name] = tomllib.loads((root / name).read_text(encoding="utf-8"))[
            "package"
        ]["version"]
    for name in ("bindings/browser/package.json", "integrations/vscode/package.json"):
        versions[name] = load_json((root / name).read_bytes())["version"]
    for version in versions.values():
        canonical_version(version)
    require(len(set(versions.values())) == 1, f"shipping versions disagree: {versions}")
    return next(iter(versions.values()))


def expected_assets(version: str) -> dict[str, dict[str, str]]:
    canonical_version(version)
    assets = {}
    for target, platform in TARGETS.items():
        extension = ".zip" if target == "x86_64-pc-windows-msvc" else ".tar.gz"
        assets[f"daml-{version}-{target}{extension}"] = {
            "kind": "native",
            "target": target,
        }
        assets[f"daml_config-{version}-cp312-abi3-{platform}.whl"] = {
            "kind": "python",
            "target": target,
        }
    assets[f"dprb-work-daml-browser-{version}.tgz"] = {
        "kind": "browser",
        "target": "wasm32-unknown-unknown",
    }
    assets[f"definitively-asserted-daml-{version}.vsix"] = {
        "kind": "vsix",
        "target": "universal",
    }
    return dict(sorted(assets.items()))


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check_sidecar(path: Path, digest: str) -> None:
    sidecar = path.with_name(path.name + ".sha256")
    require(
        not sidecar.is_symlink() and sidecar.is_file(),
        f"missing/unsafe checksum: {sidecar}",
    )
    require(
        sidecar.read_bytes() == f"{digest}  {path.name}\n".encode("ascii"),
        f"checksum or sidecar filename mismatch: {path.name}",
    )


def _member_name(name: str, directory: bool) -> str:
    path = PurePosixPath(name)
    require(
        name == path.as_posix() + ("/" if directory else "")
        and not path.is_absolute()
        and bool(path.parts)
        and ".." not in path.parts
        and not any(c in name for c in ("\\", ":", "\x00")),
        f"unsafe archive member: {name!r}",
    )
    require(
        all(
            not part.endswith((" ", "."))
            and not any(ord(c) < 32 for c in part)
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
        f"nonportable archive member: {name!r}",
    )
    return path.as_posix()


@contextmanager
def _archive(path: Path) -> Iterator[tuple[set[str], Callable[[str], bytes]]]:
    """Index safe regular members without extracting or executing payloads."""
    is_zip = path.suffix in (".zip", ".whl", ".vsix")
    with zipfile.ZipFile(path) if is_zip else tarfile.open(path, "r:gz") as archive:
        files = {}
        seen = set()
        total = 0
        members = archive.infolist() if is_zip else archive.getmembers()
        require(len(members) <= 10000, "too many archive members")
        for member in members:
            directory = member.is_dir() if is_zip else member.isdir()
            raw = member.orig_filename if is_zip else member.name
            # Tar directory names do not necessarily carry a trailing slash.
            name = _member_name(
                raw if is_zip or not directory else raw.rstrip("/") + "/", directory
            )
            if is_zip:
                require(
                    raw == member.filename and not member.flag_bits & 1,
                    "normalized/encrypted ZIP member",
                )
                require(
                    stat.S_IFMT(member.external_attr >> 16)
                    in (0, stat.S_IFDIR if directory else stat.S_IFREG),
                    f"special archive member: {name}",
                )
                size = member.file_size
            else:
                require(directory or member.isfile(), f"special archive member: {name}")
                require(not member.issparse(), f"sparse archive member: {name}")
                size = member.size
            require(
                name.casefold() not in seen,
                f"duplicate/case-colliding archive member: {name}",
            )
            seen.add(name.casefold())
            total += size
            require(
                0 <= size <= 256 * 1024 * 1024 and total <= 512 * 1024 * 1024,
                "oversized archive",
            )
            if not directory:
                files[name] = member
        file_names = {name.casefold() for name in files}
        for name in seen:
            require(
                not any(
                    parent.as_posix().casefold() in file_names
                    for parent in PurePosixPath(name).parents
                ),
                f"file/directory collision: {name}",
            )

        def read_member(name: str) -> bytes:
            require(name in files, f"missing archive member: {name}")
            member = files[name]
            size = member.file_size if is_zip else member.size
            require(size <= 16 * 1024 * 1024, f"oversized metadata: {name}")
            stream = archive.open(member) if is_zip else archive.extractfile(member)
            require(stream is not None, f"unreadable archive member: {name}")
            with stream:
                return stream.read()

        yield set(files), read_member


def _provenance(
    build: dict, *, version: str, source_sha: str, target: str | None
) -> None:
    require(
        build.get("version") == version
        and build.get("git_commit") == source_sha
        and build.get("git_dirty") is False,
        "stale, dirty, or mixed BUILD provenance",
    )
    if target is not None:
        require(build.get("target") == target, "BUILD target mismatch")


def _legal(read: Callable[[str], bytes], prefix: str) -> None:
    for name in ("LICENSE", "SOURCES.md"):
        require(
            read(prefix + name) == (ROOT / name).read_bytes(),
            f"noncanonical legal payload: {prefix}{name}",
        )
    notices = read(prefix + "THIRD-PARTY-NOTICES.txt")
    require(
        bool(notices.strip()) and b"Permission is hereby granted" in notices,
        "missing dependency notices/license text",
    )


def inspect_payload(
    path: Path, *, version: str, source_sha: str, kind: str, target: str
) -> None:
    """Admit producer identity and staged metadata without installing the archive."""
    with _archive(path) as (files, read):
        if kind == "native":
            prefix = f"daml-{version}-{target}/"
            suffix = ".exe" if target == "x86_64-pc-windows-msvc" else ""
            require(
                files
                == {
                    prefix + name
                    for name in (
                        "BUILD.json",
                        "LICENSE",
                        "SOURCES.md",
                        "THIRD-PARTY-NOTICES.txt",
                        f"bin/daml-fmt{suffix}",
                        f"bin/daml-lsp{suffix}",
                    )
                },
                "native archive file set differs",
            )
            build = load_json(read(prefix + "BUILD.json"))
            _provenance(build, version=version, source_sha=source_sha, target=target)
            require(
                build.get("build_features") == ["lsp"], "native build features differ"
            )
            _legal(read, prefix)
        elif kind == "python":
            prefix = "daml/_distribution/"
            info = f"daml_config-{version}.dist-info/"
            require(
                all(name.startswith(("daml/", info)) for name in files),
                "unexpected wheel root",
            )
            require(
                {
                    "daml/__init__.py",
                    "daml/daml.pyi",
                    "daml/py.typed",
                    "daml/_type_schema.py",
                    info + "RECORD",
                }
                <= files,
                "missing wheel package/metadata",
            )
            build = load_json(read(prefix + "BUILD.json"))
            _provenance(build, version=version, source_sha=source_sha, target=target)
            require(
                build.get("distribution") == "daml-config"
                and build.get("profile") == "release",
                "incorrect Python build identity",
            )
            metadata = BytesParser(policy=policy.default).parsebytes(
                read(info + "METADATA")
            )
            require(
                metadata.get_all("Name") == ["daml-config"]
                and metadata.get_all("Version") == [version],
                "wheel METADATA identity mismatch",
            )
            wheel = BytesParser(policy=policy.default).parsebytes(read(info + "WHEEL"))
            require(
                wheel.get_all("Root-Is-Purelib") == ["false"]
                and wheel.get_all("Tag") == [f"cp312-abi3-{TARGETS[target]}"],
                "wheel ABI/platform mismatch",
            )
            _legal(read, prefix)
        elif kind == "browser":
            prefix = "package/"
            outputs = {
                "dist/daml_browser.js",
                "dist/daml_browser.d.ts",
                "dist/daml_browser_bg.wasm",
                "dist/daml_browser_bg.wasm.d.ts",
            }
            require(
                files
                == {
                    prefix + name
                    for name in outputs
                    | {
                        "BUILD.json",
                        "LICENSE",
                        "SOURCES.md",
                        "THIRD-PARTY-NOTICES.txt",
                        "README.md",
                        "package.json",
                    }
                },
                "browser archive file set differs",
            )
            package = load_json(read(prefix + "package.json"))
            require(
                package.get("name") == "@dprb-work/daml-browser"
                and package.get("version") == version,
                "browser package identity mismatch",
            )
            build = load_json(read(prefix + "BUILD.json"))
            _provenance(build, version=version, source_sha=source_sha, target=target)
            require(
                build.get("package") == "@dprb-work/daml-browser"
                and build.get("profile") == "release",
                "browser build identity mismatch",
            )
            require(
                isinstance(build.get("outputs"), dict)
                and set(build["outputs"]) == outputs,
                "browser output provenance differs",
            )
            for name, digest in build["outputs"].items():
                require(
                    hashlib.sha256(read(prefix + name)).hexdigest() == digest,
                    "browser output hash mismatch",
                )
            require(
                isinstance(build.get("package_inputs"), dict)
                and set(build["package_inputs"])
                == {"LICENSE", "SOURCES.md", "README.md", "THIRD-PARTY-NOTICES.txt"},
                "browser package input provenance differs",
            )
            for name, digest in build["package_inputs"].items():
                require(
                    hashlib.sha256(read(prefix + name)).hexdigest() == digest,
                    "browser package input hash mismatch",
                )
            _legal(read, prefix)
        elif kind == "vsix":
            prefix = "extension/"
            require(
                {
                    "[Content_Types].xml",
                    "extension.vsixmanifest",
                    prefix + "package.json",
                    prefix + "dist/extension.js",
                    prefix + "dist/terminateProcess.sh",
                }
                <= files,
                "VSIX lacks installable extension payload",
            )
            package = load_json(read(prefix + "package.json"))
            require(
                package.get("name") == "definitively-asserted-daml"
                and package.get("version") == version
                and package.get("publisher") == "dprb-work",
                "VSIX package identity mismatch",
            )
            build = load_json(read(prefix + "BUILD.json"))
            _provenance(build, version=version, source_sha=source_sha, target=None)
            require(build.get("kind") == "vsix", "VSIX BUILD kind mismatch")
            _legal(read, prefix)
        else:
            raise ValueError(f"unsupported asset kind: {kind}")


def validate_bundle(
    directory: Path,
    *,
    version: str,
    source_sha: str,
    manifest_sha256: str | None = None,
    prepare_run_id: int | None = None,
) -> dict:
    canonical_version(version)
    source_identity(source_sha)
    require(
        check_versions() == version,
        "bundle version differs from source shipping manifests",
    )
    directory = Path(directory)
    require(
        directory.is_dir() and not directory.is_symlink(),
        "release bundle must be a real directory",
    )
    expected = expected_assets(version)
    expected_files = {
        "RELEASE.json",
        *expected,
        *(name + ".sha256" for name in expected),
    }
    entries = list(directory.iterdir())
    require(
        {path.name for path in entries} == expected_files
        and all(path.is_file() and not path.is_symlink() for path in entries),
        "release file set differs",
    )
    manifest_path = directory / "RELEASE.json"
    if manifest_sha256 is not None:
        require(
            isinstance(manifest_sha256, str)
            and re.fullmatch(r"[0-9a-f]{64}", manifest_sha256),
            "manifest_sha256 must be a lowercase SHA-256",
        )
        require(
            sha256(manifest_path) == manifest_sha256,
            "approved manifest digest mismatch",
        )
    manifest = load_json(manifest_path.read_bytes())
    require(
        set(manifest)
        == {
            "schema",
            "repository",
            "source_sha",
            "version",
            "prepare_run_id",
            "run_attempt",
            "assets",
        },
        "release manifest fields differ",
    )
    require(
        type(manifest["schema"]) is int
        and manifest["schema"] == 1
        and manifest["repository"] == REPOSITORY
        and manifest["source_sha"] == source_sha
        and manifest["version"] == version,
        "release manifest identity mismatch",
    )
    require(
        type(manifest["prepare_run_id"]) is int
        and manifest["prepare_run_id"] > 0
        and type(manifest["run_attempt"]) is int
        and manifest["run_attempt"] > 0,
        "invalid preparation run identity",
    )
    if prepare_run_id is not None:
        require(
            type(prepare_run_id) is int
            and prepare_run_id > 0
            and manifest["prepare_run_id"] == prepare_run_id,
            "preparation run mismatch",
        )
    require(
        isinstance(manifest["assets"], list)
        and len(manifest["assets"]) == len(expected),
        "expected eight assets",
    )
    seen = set()
    for asset in manifest["assets"]:
        require(
            isinstance(asset, dict)
            and set(asset) == {"filename", "sha256", "size", "kind", "target"},
            "release asset fields differ",
        )
        name = asset["filename"]
        require(
            isinstance(name, str) and name in expected and name not in seen,
            "unknown/duplicate asset",
        )
        seen.add(name)
        require(
            {key: asset[key] for key in ("kind", "target")} == expected[name],
            "asset kind/target differs",
        )
        path = directory / name
        digest = sha256(path)
        require(
            asset["sha256"] == digest
            and type(asset["size"]) is int
            and asset["size"] == path.stat().st_size
            and asset["size"] > 0,
            "asset digest/size mismatch",
        )
        check_sidecar(path, digest)
        inspect_payload(path, version=version, source_sha=source_sha, **expected[name])
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["check-version"])
    parser.parse_args()
    print(check_versions())


if __name__ == "__main__":
    main()
