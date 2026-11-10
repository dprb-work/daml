#!/usr/bin/env python3
"""Promote an approved canonical preparation, without building anything.

`check-environment` and `plan` perform external reads only. `preflight` also
stages the approved files locally. `stage-github` creates a draft with unchanged
assets; `promote` publishes only that draft. The explicit publication target
selects GitHub alone or GitHub plus PyPI. Only the latter prepares registry
wheels and requires exact public PyPI hashes after the official PyPA action.
Both write commands require the main-only protected publication workflow and
the approved publication target.

Publication is NOT atomic: a failure can leave a tag/draft/partial GitHub release
or, for github-and-pypi, some/all PyPI wheels. Never blindly rerun, delete,
overwrite, or use skip-existing. Run `plan` with the original four candidate
inputs and publication target, inspect the recorded public hashes and draft
assets, and obtain fresh owner authorization for remaining effects. This CLI
deliberately has no recovery write mode. For github-and-pypi, PyPI Trusted
Publisher setup must bind dprb-work/daml, publish-release.yml, environment
release, project daml-config.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
import zipfile
from pathlib import Path

from release_common import REPOSITORY, validate_bundle

PREPARE_WORKFLOW = ".github/workflows/prepare-release.yml"
PUBLISH_WORKFLOW = ".github/workflows/publish-release.yml"
BUNDLE_ARTIFACT = "daml-release-candidate"
API_ROOT = f"repos/{REPOSITORY}"
GH_API = ["gh", "api", "--hostname", "github.com"]
PUBLICATION_TARGETS = ("github-only", "github-and-pypi")
GITHUB_ONLY_NOTES = (
    "GitHub-only release: the Python wheels are attached to this GitHub release "
    "for installation. This release is not published to PyPI."
)
RECOVERY = (
    "External effects may already exist. Do not rerun publication or overwrite "
    "anything: use release_publish.py plan with the original version, source SHA, "
    "prepare run ID, manifest digest and publication target; obtain owner "
    "authorization for recovery."
)


def validate_prepare_run(
    run: dict, workflow: dict, *, source_sha: str, prepare_run_id: int
) -> None:
    """Admit only the repository's canonical successful main preparation run."""
    expected = {
        "id": prepare_run_id,
        "workflow_id": workflow.get("id"),
        "path": PREPARE_WORKFLOW,
        "event": "workflow_dispatch",
        "status": "completed",
        "conclusion": "success",
        "head_branch": "main",
        "head_sha": source_sha,
    }
    if workflow.get("path") != PREPARE_WORKFLOW or not isinstance(
        workflow.get("id"), int
    ):
        raise RuntimeError("canonical prepare-release.yml workflow identity is missing")
    for key, value in expected.items():
        if run.get(key) != value:
            raise RuntimeError(f"preparation run has untrusted {key}: {run.get(key)!r}")
    for key in ("repository", "head_repository"):
        if (run.get(key) or {}).get("full_name") != REPOSITORY:
            raise RuntimeError(f"preparation run {key} must be {REPOSITORY}")
    if type(run.get("run_attempt")) is not int or run["run_attempt"] < 1:
        raise RuntimeError("preparation run attempt is missing")


def validate_environment(environment: dict, policies: dict) -> None:
    """Require the real human gate; additional reviewers would permit bypass."""
    if environment.get("name") != "release":
        raise RuntimeError("protected release environment is missing")
    reviewers = [
        rule
        for rule in environment.get("protection_rules", [])
        if rule.get("type") == "required_reviewers"
    ]
    if len(reviewers) != 1 or reviewers[0].get("prevent_self_review") is not True:
        raise RuntimeError("release requires human review with prevent_self_review")
    allowed = reviewers[0].get("reviewers", [])
    if len(allowed) != 1 or (
        allowed[0].get("type"),
        (allowed[0].get("reviewer") or {}).get("login"),
    ) != ("User", "dprbook"):
        raise RuntimeError("release must require exactly human reviewer dprbook")
    if environment.get("deployment_branch_policy") != {
        "protected_branches": False,
        "custom_branch_policies": True,
    }:
        raise RuntimeError("release requires an explicit main-only deployment policy")
    branches = policies.get("branch_policies", [])
    if (
        policies.get("total_count") != 1
        or len(branches) != 1
        or (branches[0].get("name"), branches[0].get("type")) != ("main", "branch")
    ):
        raise RuntimeError("release deployment policies must allow only branch main")


def github(
    path: str,
    *,
    method: str = "GET",
    data: dict | None = None,
    missing_ok: bool = False,
):
    command = [
        *GH_API,
        "--method",
        method,
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "X-GitHub-Api-Version: 2022-11-28",
        f"{API_ROOT}/{path}".rstrip("/"),
    ]
    if data is not None:
        command.extend(["--input", "-"])
    result = subprocess.run(
        command,
        input=json.dumps(data) if data is not None else None,
        text=True,
        capture_output=True,
        check=False,
    )
    # gh preserves the JSON error response; only an actual 404 means absent.
    if result.returncode:
        try:
            error = json.loads(result.stdout)
        except json.JSONDecodeError:
            error = {}
        if missing_ok and error.get("status") == "404":
            return None
        raise RuntimeError(f"GitHub {method} {path} failed: {result.stderr.strip()}")
    return json.loads(result.stdout)


def check_environment() -> None:
    environment = github("environments/release", missing_ok=True)
    if environment is None:
        raise RuntimeError(
            "release environment does not exist; an admin must protect it first"
        )
    policies = github("environments/release/deployment-branch-policies?per_page=100")
    validate_environment(environment, policies)


def require_current_main(args: argparse.Namespace) -> None:
    repository = github("")
    reference = github("git/ref/heads/main")
    if (
        repository.get("default_branch") != "main"
        or (reference.get("object") or {}).get("sha") != args.source_sha
    ):
        raise RuntimeError(
            "publication requires the current default-main tip; prepare and approve again"
        )
    if os.environ.get("GITHUB_ACTIONS") == "true" and (
        os.environ.get("GITHUB_REF") != "refs/heads/main"
        or os.environ.get("GITHUB_SHA") != args.source_sha
        or os.environ.get("GITHUB_REPOSITORY") != REPOSITORY
    ):
        raise RuntimeError("dispatch must use main at exactly the approved source SHA")


def preparation(args: argparse.Namespace) -> tuple[dict, dict]:
    workflow = github("actions/workflows/prepare-release.yml")
    run = github(f"actions/runs/{args.prepare_run_id}")
    validate_prepare_run(
        run, workflow, source_sha=args.source_sha, prepare_run_id=args.prepare_run_id
    )
    comparison = github(f"compare/{args.source_sha}...main")
    if (
        comparison.get("status") not in {"ahead", "identical"}
        or (comparison.get("merge_base_commit") or {}).get("sha") != args.source_sha
    ):
        raise RuntimeError("qualified source is not an ancestor of current remote main")
    matches = []
    page = 1
    while True:
        response = github(
            f"actions/runs/{args.prepare_run_id}/artifacts?per_page=100&page={page}"
        )
        artifacts = response["artifacts"]
        matches.extend(item for item in artifacts if item["name"] == BUNDLE_ARTIFACT)
        if len(artifacts) < 100:
            break
        page += 1
    if len(matches) != 1:
        raise RuntimeError(
            "preparation must contain exactly one immutable release candidate"
        )
    artifact = matches[0]
    if artifact.get("expired") is not False or not re.fullmatch(
        r"sha256:[0-9a-f]{64}", artifact.get("digest") or ""
    ):
        raise RuntimeError(
            "candidate artifact is expired or lacks an immutable SHA-256 digest"
        )
    origin = artifact.get("workflow_run") or {}
    if (
        origin.get("id") != args.prepare_run_id
        or origin.get("head_sha") != args.source_sha
    ):
        raise RuntimeError(
            "candidate artifact provenance differs from approved preparation"
        )
    if type(artifact.get("id")) is not int or artifact["id"] < 1:
        raise RuntimeError("candidate artifact has no immutable ID")
    if args.artifact_id is not None and artifact["id"] != args.artifact_id:
        raise RuntimeError(
            "candidate artifact was replaced after preflight; new approval required"
        )
    return run, artifact


def bundle(args: argparse.Namespace, directory: Path) -> dict:
    return validate_bundle(
        directory,
        version=args.version,
        source_sha=args.source_sha,
        manifest_sha256=args.manifest_sha256,
        prepare_run_id=args.prepare_run_id,
    )


def download_candidate(args: argparse.Namespace, directory: Path) -> tuple[dict, dict]:
    run, artifact = preparation(args)
    directory.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryFile() as archive:
        result = subprocess.run(
            [*GH_API, f"{API_ROOT}/actions/artifacts/{artifact['id']}/zip"],
            stdout=archive,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode:
            raise RuntimeError(
                f"candidate download failed: {result.stderr.decode().strip()}"
            )
        archive.seek(0)
        if (
            "sha256:" + hashlib.file_digest(archive, "sha256").hexdigest()
            != artifact["digest"]
        ):
            raise RuntimeError(
                "downloaded candidate archive digest does not match GitHub"
            )
        archive.seek(0)
        with zipfile.ZipFile(archive) as zipped:
            names: set[str] = set()
            for item in zipped.infolist():
                # Producer uploads a flat bundle; never extract paths or symlinks.
                if (
                    not item.filename
                    or item.filename in {".", ".."}
                    or item.orig_filename != item.filename
                    or item.flag_bits & 1
                    or "/" in item.filename
                    or "\\" in item.filename
                    or item.filename in names
                    or item.is_dir()
                    or stat.S_IFMT(item.external_attr >> 16) not in (0, stat.S_IFREG)
                ):
                    raise RuntimeError(
                        f"unsafe candidate ZIP member: {item.filename!r}"
                    )
                names.add(item.filename)
                with (
                    zipped.open(item) as source,
                    (directory / item.filename).open("xb") as target,
                ):
                    shutil.copyfileobj(source, target)
    manifest = bundle(args, directory)
    if manifest["run_attempt"] != run["run_attempt"]:
        raise RuntimeError("candidate was produced by a stale preparation attempt")
    return manifest, artifact


def pypi_version(version: str) -> dict | None:
    request = urllib.request.Request(
        f"https://pypi.org/pypi/daml-config/{version}/json",
        headers={
            "Accept": "application/json",
            "User-Agent": "daml-release-publication",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)
    except urllib.error.HTTPError as exc:
        if exc.code == 404:
            return None
        raise


def wheel_hashes(manifest: dict) -> dict[str, str]:
    return {
        asset["filename"]: asset["sha256"]
        for asset in manifest["assets"]
        if asset["filename"].endswith(".whl")
    }


def observe_pypi(manifest: dict, observed: dict | None) -> None:
    if observed is None:
        raise RuntimeError(
            "PyPI version not publicly observable yet; stop and inspect, do not retry upload"
        )
    files = observed["urls"]
    hashes = {item["filename"]: item["digests"]["sha256"] for item in files}
    if (
        len(files) != len(hashes)
        or hashes != wheel_hashes(manifest)
        or any(
            item.get("yanked") is not False or item.get("packagetype") != "bdist_wheel"
            for item in files
        )
    ):
        raise RuntimeError(
            "public PyPI version does not contain exactly the three approved wheel hashes"
        )


def github_state(version: str) -> tuple[dict | None, dict | None]:
    tag = f"v{version}"
    reference = github(f"git/ref/tags/{tag}", missing_ok=True)
    release = github(f"releases/tags/{tag}", missing_ok=True)
    # Include drafts even when lookup-by-tag only exposes a published release.
    page = 1
    while release is None:
        releases = github(f"releases?per_page=100&page={page}")
        found = [item for item in releases if item["tag_name"] == tag]
        if found:
            release = found[0]
            break
        if len(releases) < 100:
            break
        page += 1
    return reference, release


def require_absent_github(version: str) -> None:
    reference, release = github_state(version)
    if reference is not None or release is not None:
        raise RuntimeError(
            f"GitHub v{version} tag/release already exists; inspect manually, never overwrite"
        )


def require_publish_context(publication_target: str) -> None:
    if (
        os.environ.get("GITHUB_ACTIONS") != "true"
        or os.environ.get("GITHUB_EVENT_NAME") != "workflow_dispatch"
        or os.environ.get("GITHUB_REPOSITORY") != REPOSITORY
        or os.environ.get("GITHUB_REF") != "refs/heads/main"
        or os.environ.get("GITHUB_WORKFLOW_REF")
        != f"{REPOSITORY}/{PUBLISH_WORKFLOW}@refs/heads/main"
    ):
        raise RuntimeError(
            "GitHub promotion is restricted to the protected main publication workflow"
        )
    if (
        publication_target not in PUBLICATION_TARGETS
        or os.environ.get("RELEASE_TARGET") != publication_target
    ):
        raise RuntimeError(
            "publication target must match the protected workflow RELEASE_TARGET"
        )


def release_files(directory: Path) -> dict[str, tuple[int, str]]:
    """Bind GitHub sidecars and manifest as well as the eight payloads."""
    files = {}
    for path in directory.iterdir():
        with path.open("rb") as stream:
            files[path.name] = (
                path.stat().st_size,
                hashlib.file_digest(stream, "sha256").hexdigest(),
            )
    return files


def validate_draft(release: dict, args: argparse.Namespace) -> None:
    if (
        release.get("id") != args.release_id
        or release.get("draft") is not True
        or release.get("tag_name") != f"v{args.version}"
        or release.get("target_commitish") != args.source_sha
    ):
        raise RuntimeError("staged draft no longer matches this publication attempt")
    assets = release["assets"]
    actual = {
        item["name"]: (item["size"], item.get("digest"))
        for item in assets
        if item.get("state") == "uploaded"
    }
    expected = {
        name: (size, f"sha256:{digest}")
        for name, (size, digest) in release_files(args.bundle).items()
    }
    if len(assets) != len(actual) or actual != expected:
        raise RuntimeError("draft assets differ from the unchanged approved bundle")
    reference = github(f"git/ref/tags/v{args.version}", missing_ok=True)
    if reference is None or (
        reference["object"].get("type") != "commit"
        or reference["object"].get("sha") != args.source_sha
    ):
        raise RuntimeError("release tag is missing or points to a different source")


def stage_github(args: argparse.Namespace) -> None:
    require_publish_context(args.publication_target)
    check_environment()
    run, _ = preparation(args)
    manifest = bundle(args, args.bundle)
    if manifest["run_attempt"] != run["run_attempt"]:
        raise RuntimeError("preparation was rerun after approval")
    if (
        args.publication_target == "github-and-pypi"
        and pypi_version(args.version) is not None
    ):
        raise RuntimeError(
            "PyPI version already exists; inspect manually before recovery"
        )
    require_absent_github(args.version)
    # Last admission before the first external effect. Older-main candidates
    # can need workflows:write, which GITHUB_TOKEN cannot grant; never bypass it.
    require_current_main(args)
    tag = f"v{args.version}"
    # Draft releases need not materialize a new Git tag. Exercise tag creation
    # permissions and rulesets before any irreversible registry upload.
    github(
        "git/refs",
        method="POST",
        data={"ref": f"refs/tags/{tag}", "sha": args.source_sha},
    )
    reference = github(f"git/ref/tags/{tag}")
    if (
        reference["object"].get("type") != "commit"
        or reference["object"].get("sha") != args.source_sha
    ):
        raise RuntimeError("new release tag does not match the approved source")
    subprocess.run(
        [
            "gh",
            "release",
            "create",
            tag,
            *(str(path) for path in sorted(args.bundle.iterdir())),
            "--repo",
            f"github.com/{REPOSITORY}",
            "--draft",
            "--verify-tag",
            "--target",
            args.source_sha,
            "--generate-notes",
            *(
                ["--notes", GITHUB_ONLY_NOTES]
                if args.publication_target == "github-only"
                else []
            ),
            "--title",
            tag,
        ],
        check=True,
    )
    _, release = github_state(args.version)
    if release is None:
        raise RuntimeError("new GitHub draft is not observable")
    args.release_id = release["id"]
    validate_draft(release, args)
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as stream:
        stream.write(f"release_id={args.release_id}\n")
    print(f"Staged GitHub draft {args.release_id}; no public GitHub release yet")


def promote(args: argparse.Namespace) -> None:
    require_publish_context(args.publication_target)
    check_environment()
    run, _ = preparation(args)
    manifest = bundle(args, args.bundle)
    if manifest["run_attempt"] != run["run_attempt"]:
        raise RuntimeError("preparation was rerun after approval")
    if args.publication_target == "github-and-pypi":
        observe_pypi(manifest, pypi_version(args.version))
    validate_draft(github(f"releases/{args.release_id}"), args)
    require_current_main(args)
    published = github(
        f"releases/{args.release_id}", method="PATCH", data={"draft": False}
    )
    reference = github(f"git/ref/tags/v{args.version}")
    if (
        published.get("draft") is not False
        or reference["object"].get("type") != "commit"
        or reference["object"].get("sha") != args.source_sha
    ):
        raise RuntimeError(
            "public GitHub release/tag does not match the approved source"
        )
    print(f"Published https://github.com/{REPOSITORY}/releases/tag/v{args.version}")


def plan(args: argparse.Namespace, manifest: dict, artifact: dict) -> dict:
    if args.publication_target == "github-and-pypi":
        observed = pypi_version(args.version)
        files = [] if observed is None else observed["urls"]
        expected = wheel_hashes(manifest)
        pypi = {
            "pypi_version_exists": observed is not None,
            "pypi_files": [
                {
                    "filename": item["filename"],
                    "sha256": item["digests"]["sha256"],
                    "approved": expected.get(item["filename"])
                    == item["digests"]["sha256"],
                    "yanked": item.get("yanked"),
                }
                for item in files
            ],
            "pypi_missing_files": sorted(
                set(expected) - {item["filename"] for item in files}
            ),
        }
    else:
        # Not observed or applicable: null must not imply registry absence.
        pypi = {
            "pypi_version_exists": None,
            "pypi_files": None,
            "pypi_missing_files": None,
        }
    reference, release = github_state(args.version)
    return {
        "repository": REPOSITORY,
        "version": args.version,
        "publication_target": args.publication_target,
        "source_sha": args.source_sha,
        "prepare_run_id": args.prepare_run_id,
        "manifest_sha256": args.manifest_sha256,
        "artifact_id": artifact["id"],
        "artifact_digest": artifact["digest"],
        **pypi,
        "github_tag": reference,
        "github_release": release,
        "approved_github_files": {
            name: {"size": size, "sha256": digest}
            for name, (size, digest) in release_files(args.bundle).items()
        },
        "remaining_effects": "No effects authorized by this plan. If all selected targets are absent, "
        "approve the exact candidate and publication target in release environment. Otherwise stop: compare remote "
        "tag/source and release asset digests against this bundle; separately authorize only "
        "remaining effects. No automatic recovery, overwrite, rebuild or package retry.",
    }


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser(
        "check-environment", help="read-only check of existing release protection"
    )
    for name in ("plan", "preflight", "stage-github", "promote"):
        command = commands.add_parser(name)
        command.add_argument(
            "--publication-target", required=True, choices=PUBLICATION_TARGETS
        )
        command.add_argument("--version", required=True)
        command.add_argument("--source-sha", required=True)
        command.add_argument("--prepare-run-id", required=True, type=int)
        command.add_argument("--manifest-sha256", required=True)
        command.add_argument("--artifact-id", type=int)
        if name == "preflight":
            command.add_argument("--out", required=True, type=Path)
            command.add_argument("--wheel-dir", type=Path)
        elif name in {"stage-github", "promote"}:
            command.add_argument("--bundle", required=True, type=Path)
            if name == "promote":
                command.add_argument("--release-id", required=True, type=int)
    args = parser.parse_args()
    if args.command == "check-environment":
        check_environment()
        print(
            "release environment requires dprbook, prevents self-review and allows only main"
        )
        return
    # Validate all interpolated path/URL/command inputs before networking.
    if not re.fullmatch(
        r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", args.version
    ):
        parser.error("version must be a canonical three-part release version")
    if not re.fullmatch(r"[0-9a-f]{40}", args.source_sha):
        parser.error("source-sha must be full lowercase 40-hex")
    if not re.fullmatch(r"[0-9a-f]{64}", args.manifest_sha256):
        parser.error("manifest-sha256 must be lowercase 64-hex")
    if args.prepare_run_id < 1 or (
        args.artifact_id is not None and args.artifact_id < 1
    ):
        parser.error("run and artifact IDs must be positive")
    if args.command == "promote" and args.release_id < 1:
        parser.error("release-id must be positive")
    if (
        args.command == "preflight"
        and args.publication_target == "github-only"
        and args.wheel_dir is not None
    ):
        parser.error("github-only does not prepare a registry wheel directory")
    if args.command == "stage-github":
        stage_github(args)
        return
    if args.command == "promote":
        promote(args)
        return
    if args.command == "plan":
        with tempfile.TemporaryDirectory() as temporary:
            args.bundle = Path(temporary) / "bundle"
            manifest, artifact = download_candidate(args, args.bundle)
            print(json.dumps(plan(args, manifest, artifact), indent=2))
        return
    check_environment()
    require_current_main(args)
    manifest, artifact = download_candidate(args, args.out)
    if (
        args.publication_target == "github-and-pypi"
        and pypi_version(args.version) is not None
    ):
        raise RuntimeError(
            "PyPI version already exists; use plan, never blanket skip-existing"
        )
    require_absent_github(args.version)
    if args.wheel_dir is not None:
        args.wheel_dir.mkdir(parents=True, exist_ok=False)
        wheels = wheel_hashes(manifest)
        if len(wheels) != 3:
            raise RuntimeError("approved bundle must supply exactly three wheels")
        for filename in wheels:
            shutil.copyfile(args.out / filename, args.wheel_dir / filename)
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with Path(output).open("a", encoding="utf-8") as stream:
            stream.write(f"artifact_id={artifact['id']}\n")
    summary = (
        f"Publication target: {args.publication_target}\n"
        f"Approved candidate: {REPOSITORY} {args.version}\n"
        f"Source: {args.source_sha}\nPreparation: {args.prepare_run_id}\n"
        f"Manifest SHA-256: {args.manifest_sha256}\n"
        f"Immutable artifact: {artifact['id']} ({artifact['digest']})\n"
        + (
            "Approval authorizes GitHub draft staging, PyPI wheels, then making the unchanged GitHub release public.\n"
            if args.publication_target == "github-and-pypi"
            else "Approval authorizes GitHub draft staging and making the unchanged GitHub release public; no PyPI publication.\n"
        )
        + RECOVERY
        + "\n"
    )
    print(summary)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with Path(os.environ["GITHUB_STEP_SUMMARY"]).open(
            "a", encoding="utf-8"
        ) as stream:
            stream.write(summary)


if __name__ == "__main__":
    try:
        main()
    except (
        RuntimeError,
        ValueError,
        OSError,
        subprocess.CalledProcessError,
        zipfile.BadZipFile,
    ) as exc:
        print(f"release-publish: {exc}\n{RECOVERY}", file=sys.stderr)
        raise SystemExit(1) from exc
