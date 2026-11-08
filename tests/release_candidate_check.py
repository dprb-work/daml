#!/usr/bin/env python3
"""Admit a real prepared bundle, then reject isolated corruptions before effects.

This does not build, extract, install, upload, or invoke the publisher. It first
requires the independently supplied source/version/run/manifest identity to pass
against all eight real payloads. Temporary copies are destroyed after each case;
no fake executable, WASM module, native library, or wheel is a positive fixture.
"""

import argparse
import hashlib
import importlib.util
import json
import shutil
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def rewrite_manifest(directory: Path, change) -> None:
    path = directory / "RELEASE.json"
    manifest = json.loads(path.read_text(encoding="utf-8"))
    change(manifest)
    path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def flip_byte(path: Path) -> None:
    with path.open("r+b") as stream:
        original = stream.read(1)
        if not original:
            raise AssertionError(f"positive baseline contains empty payload: {path.name}")
        stream.seek(0)
        stream.write(bytes([original[0] ^ 1]))


def check_candidate(args: argparse.Namespace) -> dict:
    spec = importlib.util.spec_from_file_location("release_common", ROOT / "scripts" / "release_common.py")
    common = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(common)
    candidate = args.directory.resolve(strict=True)
    binding = {
        "version": args.version,
        "source_sha": args.source_sha,
        "prepare_run_id": args.prepare_run_id,
        "manifest_sha256": args.manifest_sha256,
    }
    # A negative case is meaningful only after these exact real bytes are admitted.
    common.validate_bundle(candidate, **binding)
    manifest = json.loads((candidate / "RELEASE.json").read_text(encoding="utf-8"))
    filenames = [entry["filename"] for entry in manifest["assets"]]
    if len(filenames) != 8 or len(set(filenames)) != 8:
        raise AssertionError("the real positive baseline must contain eight distinct payloads")
    passed = []

    def reject_binding(label: str, **change) -> None:
        try:
            common.validate_bundle(candidate, **{**binding, **change})
        except ValueError:
            passed.append(label)
            return
        raise AssertionError(f"incorrect candidate binding admitted: {label}")

    def reject_copy(label: str, change, **rebind) -> None:
        with tempfile.TemporaryDirectory(prefix="daml-release-corruption-") as temporary:
            directory = Path(temporary) / "candidate"
            shutil.copytree(candidate, directory, symlinks=True)
            change(directory)
            # Reapprove the mutated manifest digest so semantic-policy checks,
            # not just the outer digest, must reject malformed manifest content.
            manifest_path = directory / "RELEASE.json"
            changed_binding = {**binding, **rebind}
            if manifest_path.is_file():
                changed_binding["manifest_sha256"] = digest(manifest_path)
            try:
                common.validate_bundle(directory, **changed_binding)
            except (ValueError, FileNotFoundError):
                passed.append(label)
                return
            raise AssertionError(f"corrupted candidate admitted: {label}")

    reject_binding("approved-manifest-digest", manifest_sha256="0" * 64)
    reject_binding("approved-source", source_sha="f" * 40)
    reject_binding("approved-version", version="99.98.97")
    reject_binding("approved-preparation-run", prepare_run_id=args.prepare_run_id + 1)
    for filename in filenames:
        reject_copy(f"payload-bytes:{filename}", lambda directory, name=filename: flip_byte(directory / name))
        reject_copy(f"missing-payload:{filename}", lambda directory, name=filename: (directory / name).unlink())
        reject_copy(f"missing-sidecar:{filename}", lambda directory, name=filename: (directory / f"{name}.sha256").unlink())
        reject_copy(
            f"wrong-sidecar:{filename}",
            lambda directory, name=filename: (directory / f"{name}.sha256").write_text(
                f"{'0' * 64}  {name}\n", encoding="utf-8",
            ),
        )

    reject_copy("extra-payload", lambda directory: (directory / "unapproved.zip").write_bytes(b"extra"))
    reject_copy("extra-directory", lambda directory: (directory / "unexpected").mkdir())
    reject_copy("missing-manifest", lambda directory: (directory / "RELEASE.json").unlink())
    reject_copy("invalid-manifest-json", lambda directory: (directory / "RELEASE.json").write_text("{", encoding="utf-8"))

    def duplicate_key(directory: Path) -> None:
        path = directory / "RELEASE.json"
        contents = path.read_text(encoding="utf-8")
        path.write_text('{"schema":1,' + contents.lstrip()[1:], encoding="utf-8")

    reject_copy("duplicate-manifest-key", duplicate_key)
    for field, value in (
        ("schema", 2), ("schema", True), ("repository", "untrusted/daml"),
        ("version", "0.0.0"), ("source_sha", "short"),
        ("prepare_run_id", args.prepare_run_id + 1), ("prepare_run_id", True),
        ("run_attempt", 0), ("run_attempt", True), ("unapproved_field", "extra"),
    ):
        reject_copy(
            f"manifest-field:{field}:{value}",
            lambda directory, key=field, replacement=value: rewrite_manifest(
                directory, lambda data: data.__setitem__(key, replacement),
            ),
        )
    reject_copy(
        "forged-source-with-new-approved-digest",
        lambda directory: rewrite_manifest(directory, lambda data: data.__setitem__("source_sha", "f" * 40)),
        source_sha="f" * 40,
    )
    reject_copy(
        "missing-manifest-asset",
        lambda directory: rewrite_manifest(directory, lambda data: data["assets"].pop()),
    )
    reject_copy(
        "duplicate-manifest-asset",
        lambda directory: rewrite_manifest(directory, lambda data: data["assets"].append(dict(data["assets"][0]))),
    )
    for field, value in (
        ("filename", "../outside.whl"), ("filename", "/absolute.whl"),
        ("sha256", "0" * 64), ("sha256", "short"), ("size", 0), ("size", True),
        ("kind", "unapproved"), ("target", "unapproved"), ("unapproved_field", "extra"),
    ):
        reject_copy(
            f"asset-field:{field}:{value}",
            lambda directory, key=field, replacement=value: rewrite_manifest(
                directory, lambda data: data["assets"][0].__setitem__(key, replacement),
            ),
        )

    def symlink_payload(directory: Path) -> None:
        path = directory / filenames[0]
        path.unlink()
        path.symlink_to(candidate / filenames[0])

    reject_copy("symlink-payload", symlink_payload)
    # Detect accidental writes to the supplied baseline, not just its temporary copies.
    common.validate_bundle(candidate, **binding)
    return {
        "status": "passed",
        "candidate": str(candidate),
        **binding,
        "payloads": filenames,
        "positive_baseline": "admitted-before-and-after",
        "rejected_cases": passed,
        "temporary_copies": "removed",
        "publication": "not-invoked",
        "runtime_installation": "not-exercised",
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--prepare-run-id", required=True, type=int)
    parser.add_argument("--manifest-sha256", required=True)
    args = parser.parse_args()
    print(json.dumps(check_candidate(args), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
