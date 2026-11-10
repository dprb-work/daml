#!/usr/bin/env python3
"""Admit a real prepared bundle, then reject isolated corruptions before effects.

This does not build, extract to disk, install, upload, or invoke publication.
It first requires the independently supplied source/version/run/manifest identity
to pass against all eight real payloads. Temporary copies are destroyed after each
case; no fake executable, WASM module, native library, or wheel is a positive fixture.
Staging regressions use only fixture external-service adapters with the actual
admitted local bundle; all production admission/orchestration policies stay real.
"""

import argparse
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

from test_release_delivery import load_policy, prepare_origin, release_environment

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


def rewrite_browser_tar(path: Path, replacements: dict[str, bytes | None]) -> None:
    """Corrupt a temporary real archive while preserving unrelated member bytes."""
    remaining = dict(replacements)
    temporary = path.with_name(path.name + ".mutated")
    with tarfile.open(path, "r:gz") as source, tarfile.open(temporary, "w:gz") as target:
        for member in source.getmembers():
            if member.name in remaining:
                contents = remaining.pop(member.name)
                if contents is None:
                    continue
                member.size = len(contents)
                target.addfile(member, io.BytesIO(contents))
            else:
                stream = source.extractfile(member) if member.isfile() else None
                if stream is None:
                    target.addfile(member)
                else:
                    with stream:
                        target.addfile(member, stream)
        for name, contents in remaining.items():
            if contents is None:
                raise AssertionError(f"requested corruption member does not exist: {name}")
            member = tarfile.TarInfo(name)
            member.size = len(contents)
            member.mode = 0o644
            target.addfile(member, io.BytesIO(contents))
    temporary.replace(path)


def rewrite_vsix_license(path: Path, contents: bytes | None) -> None:
    """Alter only the legal member of a temporary real VSCE-produced archive."""
    temporary = path.with_name(path.name + ".mutated")
    found = False
    with zipfile.ZipFile(path) as source, zipfile.ZipFile(temporary, "w") as target:
        for member in source.infolist():
            if member.filename == "extension/LICENSE.txt":
                found = True
                if contents is not None:
                    target.writestr(member, contents)
            else:
                target.writestr(member, source.read(member))
    if not found:
        raise AssertionError("real VSIX baseline has no extension/LICENSE.txt")
    temporary.replace(path)


def reseal_payload(directory: Path, filename: str) -> None:
    """Refresh envelopes and artifact binding so inner metadata must fail."""
    path = directory / filename
    checksum = digest(path)
    size = path.stat().st_size
    (directory / f"{filename}.sha256").write_text(f"{checksum}  {filename}\n", encoding="utf-8")

    def change(manifest: dict) -> None:
        for field in ("assets", "security"):
            records = manifest[field] if field == "assets" else manifest[field]["assets"]
            for asset in records:
                if asset["filename"] == filename:
                    asset["sha256"] = checksum
                    asset["size"] = size
                    break
            else:
                raise AssertionError(f"missing real baseline asset in {field}: {filename}")

    rewrite_manifest(directory, change)

def check_staging_boundary(args: argparse.Namespace, candidate: Path, manifest: dict, common) -> list[str]:
    """Exercise tag-before-draft orchestration with real bytes and isolated ports."""
    with patch.dict(sys.modules, {"release_common": common}):
        publisher = load_policy("release_publish")
    run, workflow = prepare_origin()
    run.update(
        id=args.prepare_run_id, head_sha=args.source_sha,
        run_attempt=manifest["run_attempt"],
    )
    environment, policies = release_environment()
    tag = f"v{args.version}"
    exact_reference = {
        "ref": f"refs/tags/{tag}",
        "object": {"type": "commit", "sha": args.source_sha},
    }
    artifact_id = 87654321
    cases = ("tag-permission-denied", "tag-not-observable", "wrong-tag-source", "wrong-tag-object", "exact-tag")
    passed = []
    for case in cases:
        with tempfile.TemporaryDirectory(prefix="daml-stage-boundary-") as temporary:
            output = Path(temporary) / "github-output"
            state = {"tag_attempted": False, "reference": None, "tag_observed": False, "draft": None}
            denied = RuntimeError("fixture GitHub POST git/refs: HTTP 403")
            missing = RuntimeError("fixture GitHub GET tag ref: HTTP 404")
            context = {
                "GITHUB_ACTIONS": "true",
                "GITHUB_EVENT_NAME": "workflow_dispatch",
                "GITHUB_REPOSITORY": "dprb-work/daml",
                "GITHUB_REF": "refs/heads/main",
                "GITHUB_SHA": args.source_sha,
                "GITHUB_WORKFLOW_REF": "dprb-work/daml/.github/workflows/publish-release.yml@refs/heads/main",
                "GITHUB_OUTPUT": str(output),
                "RELEASE_TARGET": "github-and-pypi",
            }
            responses = {
                "environments/release": environment,
                "environments/release/deployment-branch-policies?per_page=100": policies,
                "actions/workflows/prepare-release.yml": workflow,
                f"actions/runs/{args.prepare_run_id}": run,
                f"compare/{args.source_sha}...main": {
                    "status": "identical", "merge_base_commit": {"sha": args.source_sha},
                },
                f"actions/runs/{args.prepare_run_id}/artifacts?per_page=100&page=1": {
                    "total_count": 1,
                    "artifacts": [{
                        "id": artifact_id, "name": "daml-release-candidate", "expired": False,
                        "digest": "sha256:" + "a" * 64,
                        "workflow_run": {"id": args.prepare_run_id, "head_sha": args.source_sha},
                    }],
                },
                "": {"default_branch": "main"},
                "git/ref/heads/main": {"object": {"type": "commit", "sha": args.source_sha}},
            }

            def github(path: str, *, method="GET", data=None, missing_ok=False):
                if method == "POST" and path == "git/refs":
                    state["tag_attempted"] = True
                    if data != {"ref": f"refs/tags/{tag}", "sha": args.source_sha}:
                        raise AssertionError("tag write does not name the exact approved commit")
                    if case == "tag-permission-denied":
                        raise denied
                    reference = json.loads(json.dumps(exact_reference))
                    if case == "wrong-tag-source":
                        reference["object"]["sha"] = "f" * 40
                    elif case == "wrong-tag-object":
                        reference["object"]["type"] = "tag"
                    state["reference"] = reference
                    return reference
                if method != "GET":
                    raise AssertionError(f"unexpected external mutation: {method} {path}")
                if path == f"git/ref/tags/{tag}":
                    if state["tag_attempted"]:
                        state["tag_observed"] = True
                        if case == "tag-not-observable":
                            raise missing
                    return state["reference"]
                if path == f"releases/tags/{tag}":
                    return state["draft"]
                if path == "releases?per_page=100&page=1":
                    return [] if state["draft"] is None else [state["draft"]]
                return responses[path]

            def release_command(command, **kwargs):
                # A safety adapter at the external gh effect boundary, not a
                # successful fake qualification or a replacement stage policy.
                if command[:3] != ["gh", "release", "create"]:
                    raise AssertionError(f"unexpected subprocess effect: {command}")
                if not state["tag_observed"] or state["reference"] != exact_reference:
                    raise AssertionError("draft creation attempted before exact tag materialization")
                assets = []
                for filename in command[4:command.index("--repo")]:
                    path = Path(filename)
                    assets.append({
                        "name": path.name, "state": "uploaded", "size": path.stat().st_size,
                        "digest": "sha256:" + digest(path),
                    })
                state["draft"] = {
                    "id": 23456789, "draft": "--draft" in command,
                    "tag_name": command[3],
                    "target_commitish": command[command.index("--target") + 1],
                    "assets": assets,
                }
                return subprocess.CompletedProcess(command, 0)

            stage_args = argparse.Namespace(
                version=args.version, source_sha=args.source_sha,
                prepare_run_id=args.prepare_run_id, manifest_sha256=args.manifest_sha256,
                artifact_id=artifact_id, bundle=candidate,
                publication_target="github-and-pypi",
            )
            failure = None
            with (
                patch.dict(os.environ, context),
                patch.object(publisher, "github", side_effect=github),
                patch.object(publisher, "pypi_version", return_value=None),
                patch.object(publisher.subprocess, "run", side_effect=release_command),
                redirect_stdout(io.StringIO()),
            ):
                try:
                    publisher.stage_github(stage_args)
                except RuntimeError as exc:
                    failure = exc
            if not state["tag_attempted"]:
                raise AssertionError(f"case did not reach tag-permission boundary: {case}")
            if case == "exact-tag":
                if failure is not None:
                    raise failure
                values = dict(line.split("=", 1) for line in output.read_text(encoding="utf-8").splitlines())
                if state["draft"] is None or values.get("release_id") != "23456789":
                    raise AssertionError("exact tag did not produce a bound draft output")
            else:
                if failure is None or state["draft"] is not None or output.exists():
                    raise AssertionError(f"unsafe draft/output after failed tag admission: {case}")
                if case == "tag-permission-denied" and failure is not denied:
                    raise AssertionError("permission case failed before the intended 403 boundary")
                if case == "tag-not-observable" and failure is not missing:
                    raise AssertionError("missing-tag case failed before the intended 404 boundary")
                if case.startswith("wrong-tag") and not state["tag_observed"]:
                    raise AssertionError("wrong-tag case did not reach reference verification")
            passed.append(case)
    return passed


def check_publication_targets(args: argparse.Namespace, candidate: Path, manifest: dict, common) -> list[str]:
    """Exercise both effect targets using real admitted bytes and API-shaped ports."""
    with patch.dict(sys.modules, {"release_common": common}):
        publisher = load_policy("release_publish")
    tag = f"v{args.version}"
    exact_reference = {
        "ref": f"refs/tags/{tag}",
        "object": {"type": "commit", "sha": args.source_sha},
    }
    assets = [
        {
            "name": path.name, "state": "uploaded", "size": path.stat().st_size,
            "digest": "sha256:" + digest(path),
        }
        for path in sorted(candidate.iterdir())
    ]
    exact_draft = {
        "id": 23456789, "draft": True, "tag_name": tag,
        "target_commitish": args.source_sha, "assets": assets,
    }
    exact_pypi = {"urls": [
        {
            "filename": asset["filename"], "digests": {"sha256": asset["sha256"]},
            "yanked": False, "packagetype": "bdist_wheel",
        }
        for asset in manifest["assets"] if asset["kind"] == "python"
    ]}
    cases = [
        ("preflight", "exact"),
        ("plan", "exact"),
        ("stage-github", "exact"),
        ("promote", "exact"),
    ]
    for command in ("stage-github", "promote"):
        cases.extend((command, failure) for failure in (
            "moved-main", "weakened-environment", "wrong-source",
            "wrong-manifest-digest", "replaced-artifact", "stale-attempt", "mutated-payload",
        ))
    cases.extend(("stage-github", failure) for failure in ("existing-tag", "existing-release"))
    cases.extend(("promote", failure) for failure in (
        "wrong-draft-source", "public-draft", "missing-draft-asset",
        "wrong-draft-asset-hash", "wrong-tag-source", "missing-tag",
    ))
    passed = []
    # Build the download response from the real bundle, without executing it.
    with tempfile.TemporaryFile() as archive:
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_STORED) as zipped:
            for path in sorted(candidate.iterdir()):
                zipped.write(path, path.name)
        archive.seek(0)
        archive_digest = "sha256:" + hashlib.file_digest(archive, "sha256").hexdigest()
        for target in ("github-only", "github-and-pypi"):
            target_cases = list(cases)
            if target == "github-and-pypi":
                target_cases.extend((
                    ("preflight", "existing-pypi"),
                    ("stage-github", "existing-pypi"),
                    ("promote", "missing-pypi"),
                    ("promote", "wrong-pypi-hash"),
                    ("promote", "missing-pypi-wheel"),
                ))
            for command, case in target_cases:
                with tempfile.TemporaryDirectory(prefix="daml-publication-target-") as temporary:
                    root = Path(temporary)
                    output = root / "github-output"
                    registry = root / "pypi"
                    run, workflow = prepare_origin()
                    run.update(
                        id=args.prepare_run_id, head_sha=args.source_sha,
                        run_attempt=manifest["run_attempt"],
                    )
                    environment, policies = release_environment()
                    artifact_id = 87654321
                    artifact = {
                        "id": artifact_id, "name": "daml-release-candidate", "expired": False,
                        "digest": archive_digest,
                        "workflow_run": {"id": args.prepare_run_id, "head_sha": args.source_sha},
                    }
                    responses = {
                        "environments/release": environment,
                        "environments/release/deployment-branch-policies?per_page=100": policies,
                        "actions/workflows/prepare-release.yml": workflow,
                        f"actions/runs/{args.prepare_run_id}": run,
                        f"compare/{args.source_sha}...main": {
                            "status": "identical", "merge_base_commit": {"sha": args.source_sha},
                        },
                        f"actions/runs/{args.prepare_run_id}/artifacts?per_page=100&page=1": {
                            "total_count": 1, "artifacts": [artifact],
                        },
                        "": {"default_branch": "main"},
                        "git/ref/heads/main": {"object": {"type": "commit", "sha": args.source_sha}},
                    }
                    state = {
                        "reference": json.loads(json.dumps(exact_reference)) if command == "promote" else None,
                        "release": json.loads(json.dumps(exact_draft)) if command == "promote" else None,
                        "writes": [],
                    }
                    observation = json.loads(json.dumps(exact_pypi)) if command == "promote" else None
                    if case == "moved-main":
                        responses["git/ref/heads/main"]["object"]["sha"] = "f" * 40
                    elif case == "weakened-environment":
                        environment["protection_rules"][0]["prevent_self_review"] = False
                    elif case == "wrong-source":
                        run["head_sha"] = "f" * 40
                    elif case == "replaced-artifact":
                        artifact["id"] += 1
                    elif case == "stale-attempt":
                        run["run_attempt"] += 1
                    elif case == "existing-tag":
                        state["reference"] = exact_reference
                    elif case == "existing-release":
                        state["release"] = exact_draft
                    elif case == "wrong-draft-source":
                        state["release"]["target_commitish"] = "f" * 40
                    elif case == "public-draft":
                        state["release"]["draft"] = False
                    elif case == "missing-draft-asset":
                        state["release"]["assets"].pop()
                    elif case == "wrong-draft-asset-hash":
                        state["release"]["assets"][0]["digest"] = "sha256:" + "f" * 64
                    elif case == "wrong-tag-source":
                        state["reference"]["object"]["sha"] = "f" * 40
                    elif case == "missing-tag":
                        state["reference"] = None
                    elif case == "existing-pypi":
                        observation = exact_pypi
                    elif case == "missing-pypi":
                        observation = None
                    elif case == "wrong-pypi-hash":
                        observation["urls"][0]["digests"]["sha256"] = "f" * 64
                    elif case == "missing-pypi-wheel":
                        observation["urls"].pop()
                    context = {
                        "GITHUB_ACTIONS": "true",
                        "GITHUB_EVENT_NAME": "workflow_dispatch",
                        "GITHUB_REPOSITORY": "dprb-work/daml",
                        "GITHUB_REF": "refs/heads/main",
                        "GITHUB_SHA": args.source_sha,
                        "GITHUB_WORKFLOW_REF": "dprb-work/daml/.github/workflows/publish-release.yml@refs/heads/main",
                        "GITHUB_OUTPUT": str(output),
                        "RELEASE_TARGET": target,
                    }

                    def github(path: str, *, method="GET", data=None, missing_ok=False):
                        if method == "POST" and path == "git/refs":
                            if data != {"ref": f"refs/tags/{tag}", "sha": args.source_sha}:
                                raise AssertionError("tag effect differs from the approved source")
                            state["writes"].append("tag")
                            state["reference"] = json.loads(json.dumps(exact_reference))
                            return state["reference"]
                        if method == "PATCH" and path == f"releases/{exact_draft['id']}":
                            if data != {"draft": False}:
                                raise AssertionError("unexpected release mutation")
                            state["writes"].append("publish")
                            state["release"]["draft"] = False
                            return state["release"]
                        if method != "GET":
                            raise AssertionError(f"unexpected GitHub effect: {method} {path}")
                        if path == f"git/ref/tags/{tag}":
                            return state["reference"]
                        if path in (f"releases/tags/{tag}", f"releases/{exact_draft['id']}"):
                            return state["release"]
                        if path == "releases?per_page=100&page=1":
                            return [] if state["release"] is None else [state["release"]]
                        return responses[path]

                    def release_command(command_args, **kwargs):
                        if command_args == [
                            *publisher.GH_API,
                            f"{publisher.API_ROOT}/actions/artifacts/{artifact_id}/zip",
                        ]:
                            archive.seek(0)
                            shutil.copyfileobj(archive, kwargs["stdout"])
                            return subprocess.CompletedProcess(command_args, 0)
                        if command_args[:3] != ["gh", "release", "create"]:
                            raise AssertionError(f"unexpected subprocess effect: {command_args}")
                        if state["reference"] != exact_reference:
                            raise AssertionError("draft before exact tag")
                        state["writes"].append("draft")
                        uploaded = []
                        for filename in command_args[4:command_args.index("--repo")]:
                            path = Path(filename)
                            uploaded.append({
                                "name": path.name, "state": "uploaded", "size": path.stat().st_size,
                                "digest": "sha256:" + digest(path),
                            })
                        state["release"] = {
                            "id": exact_draft["id"], "draft": "--draft" in command_args,
                            "tag_name": command_args[3],
                            "target_commitish": command_args[command_args.index("--target") + 1],
                            "assets": uploaded,
                        }
                        return subprocess.CompletedProcess(command_args, 0)

                    def pypi(version):
                        if target == "github-only":
                            raise AssertionError("GitHub-only publication contacted PyPI")
                        if version != args.version:
                            raise AssertionError("registry lookup does not use the approved version")
                        return observation

                    approved_digest = "0" * 64 if case == "wrong-manifest-digest" else args.manifest_sha256
                    argv = [
                        "release_publish.py", command, "--publication-target", target,
                        "--version", args.version, "--source-sha", args.source_sha,
                        "--prepare-run-id", str(args.prepare_run_id),
                        "--manifest-sha256", approved_digest, "--artifact-id", str(artifact_id),
                    ]
                    if command == "preflight":
                        argv.extend(["--out", str(root / "candidate")])
                        if target == "github-and-pypi":
                            argv.extend(["--wheel-dir", str(registry)])
                    elif command in {"stage-github", "promote"}:
                        bundle_path = candidate
                        if case == "mutated-payload":
                            bundle_path = root / "mutated"
                            shutil.copytree(candidate, bundle_path)
                            flip_byte(bundle_path / manifest["assets"][0]["filename"])
                        argv.extend(["--bundle", str(bundle_path)])
                    if command == "promote":
                        argv.extend(["--release-id", str(exact_draft["id"])])
                    failure = None
                    with (
                        patch.dict(os.environ, context, clear=True),
                        patch.object(sys, "argv", argv),
                        patch.object(publisher, "github", side_effect=github),
                        patch.object(publisher, "pypi_version", side_effect=pypi),
                        patch.object(publisher.subprocess, "run", side_effect=release_command),
                        patch.object(publisher.urllib.request, "urlopen", side_effect=AssertionError("unexpected live network request")),
                        redirect_stdout(io.StringIO()),
                    ):
                        try:
                            publisher.main()
                        except (RuntimeError, ValueError) as exc:
                            failure = exc
                    identity = f"{target}:{command}:{case}"
                    if case != "exact":
                        if failure is None or state["writes"] or output.exists():
                            raise AssertionError(f"failed admission allowed effects or outputs: {identity}")
                    else:
                        if failure is not None:
                            raise failure
                        expected_writes = {
                            "preflight": [], "plan": [], "stage-github": ["tag", "draft"],
                            "promote": ["publish"],
                        }[command]
                        if state["writes"] != expected_writes:
                            raise AssertionError(f"publication effects differ: {identity}")
                        if command == "stage-github":
                            if state["release"]["assets"] != assets or state["release"]["draft"] is not True:
                                raise AssertionError(f"staged release differs from the unchanged bundle: {identity}")
                        if command == "preflight":
                            downloaded = {path.name: digest(path) for path in (root / "candidate").iterdir()}
                            expected = {path.name: digest(path) for path in candidate.iterdir()}
                            if downloaded != expected:
                                raise AssertionError(f"preflight changed the approved bundle: {identity}")
                            if target == "github-and-pypi":
                                actual_wheels = {path.name: digest(path) for path in registry.iterdir()}
                                expected_wheels = {
                                    asset["filename"]: asset["sha256"]
                                    for asset in manifest["assets"] if asset["kind"] == "python"
                                }
                                if actual_wheels != expected_wheels:
                                    raise AssertionError("registry directory does not contain exactly unchanged approved wheels")
                    if target == "github-only" and registry.exists():
                        raise AssertionError(f"GitHub-only publication prepared registry wheels: {identity}")
                    passed.append(identity)
    return passed


def check_candidate(args: argparse.Namespace) -> dict:
    common = load_policy("release_common")
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
        ("schema", 1), ("schema", 3), ("schema", True), ("repository", "untrusted/daml"),
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
        "missing-security-proof",
        lambda directory: rewrite_manifest(directory, lambda data: data.pop("security")),
    )
    for field, value in (
        ("status", "failed"), ("status", "incomplete"), ("schema", 2),
        ("source_sha", "f" * 40), ("policy_sha256", "0" * 64),
        ("rules_sha256", "0" * 64), ("scanned_at", "2000-01-01T00:00:00Z"),
        ("assets", []), ("checks", []),
    ):
        reject_copy(
            f"security-proof-field:{field}:{value}",
            lambda directory, key=field, replacement=value: rewrite_manifest(
                directory, lambda data: data["security"].__setitem__(key, replacement),
            ),
        )
    reject_copy(
        "security-proof-asset-binding",
        lambda directory: rewrite_manifest(
            directory, lambda data: data["security"]["assets"][0].__setitem__("sha256", "0" * 64),
        ),
    )
    for index, check in enumerate(manifest["security"]["checks"]):
        reject_copy(
            f"security-missing-required-check:{check['id']}",
            lambda directory, row=index: rewrite_manifest(
                directory, lambda data: data["security"]["checks"].pop(row),
            ),
        )
        for field, value in (
            ("status", "error"), ("findings", 1), ("coverage", {"complete": False}),
        ):
            reject_copy(
                f"security-check:{check['id']}:{field}",
                lambda directory, row=index, key=field, replacement=value: rewrite_manifest(
                    directory, lambda data: data["security"]["checks"][row].__setitem__(key, replacement),
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
    browser = next(asset["filename"] for asset in manifest["assets"] if asset["kind"] == "browser")
    with tarfile.open(candidate / browser, "r:gz") as archive:
        helper_names = [
            member.name for member in archive.getmembers()
            if member.isfile() and member.name.startswith("package/dist/snippets/")
            and member.name.endswith("/src/byte-view.js")
        ]
        build_stream = archive.extractfile("package/BUILD.json")
        if build_stream is None:
            raise AssertionError("real browser baseline has no BUILD.json")
        with build_stream:
            browser_build = json.load(build_stream)
    if not helper_names:
        raise AssertionError("real browser baseline is missing the landed byte-view helper")

    def corrupt_browser(directory: Path, replacements: dict[str, bytes | None]) -> None:
        rewrite_browser_tar(directory / browser, replacements)
        reseal_payload(directory, browser)

    for helper in helper_names:
        recorded = helper.removeprefix("package/")
        if recorded not in browser_build["outputs"]:
            raise AssertionError(f"real helper is not bound by BUILD output hashes: {helper}")
        reject_copy(
            f"browser-missing-helper:{helper}",
            lambda directory, name=helper: corrupt_browser(directory, {name: None}),
        )
        reject_copy(
            f"browser-helper-bytes-with-resealed-envelope:{helper}",
            lambda directory, name=helper: corrupt_browser(directory, {name: b"// deliberately corrupted helper\\n"}),
        )
        changed_build = json.loads(json.dumps(browser_build))
        changed_build["outputs"][recorded] = "0" * 64
        reject_copy(
            f"browser-helper-recorded-hash:{helper}",
            lambda directory, build=changed_build: corrupt_browser(
                directory, {"package/BUILD.json": json.dumps(build).encode("utf-8")},
            ),
        )
    reject_copy(
        "browser-extra-unexpected-output-with-resealed-envelope",
        lambda directory: corrupt_browser(directory, {"package/dist/unapproved-output.js": b"// unapproved\\n"}),
    )
    vsix = next(asset["filename"] for asset in manifest["assets"] if asset["kind"] == "vsix")
    with zipfile.ZipFile(candidate / vsix) as archive:
        if archive.read("extension/LICENSE.txt") != (ROOT / "LICENSE").read_bytes():
            raise AssertionError("real VSIX license does not match the canonical source license")

    def corrupt_vsix_license(directory: Path, contents: bytes | None) -> None:
        rewrite_vsix_license(directory / vsix, contents)
        reseal_payload(directory, vsix)

    reject_copy("vsix-missing-license-with-resealed-envelope", lambda directory: corrupt_vsix_license(directory, None))
    reject_copy(
        "vsix-noncanonical-license-with-resealed-envelope",
        lambda directory: corrupt_vsix_license(directory, b"deliberately noncanonical license\n"),
    )
    staging_cases = check_staging_boundary(args, candidate, manifest, common)
    target_cases = check_publication_targets(args, candidate, manifest, common)
    # Detect accidental writes to the supplied baseline, not just its temporary copies.
    common.validate_bundle(candidate, **binding)
    return {
        "status": "passed",
        "candidate": str(candidate),
        **binding,
        "payloads": filenames,
        "positive_baseline": "admitted-before-and-after",
        "rejected_cases": passed,
        "staging_policy_cases": staging_cases,
        "staging_external_effects": "fixture-adapters-only; no API/subprocess effect executed",
        "publication_target_cases": target_cases,
        "temporary_copies": "removed",
        "publication": "fixture-adapters-only; no live publication",
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
