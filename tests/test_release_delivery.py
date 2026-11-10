#!/usr/bin/env python3
"""Independent delivery policy checks; no builds, credentials, or publication.

Run: python3 -B -m unittest discover -s tests -p test_release_delivery.py
Real release bytes are checked separately by tests/release_candidate_check.py.
"""

import argparse
import copy
import io
import importlib.util
import hashlib
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from contextlib import redirect_stderr
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def load_policy(name: str):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    scripts = str(ROOT / "scripts")
    with patch.dict(sys.modules, {name: module}), patch.object(
        sys, "path", [scripts, *sys.path]
    ):
        spec.loader.exec_module(module)
    return module


SHIPPING_MANIFESTS = {
    "Cargo.toml": "daml",
    "bindings/python/Cargo.toml": "daml-python",
    "bindings/browser/Cargo.toml": "daml-browser",
    "bindings/browser/package.json": "@dprb-work/daml-browser",
    "integrations/vscode/package.json": "definitively-asserted-daml",
}


def write_versions(root: Path, version: str) -> None:
    for relative, name in SHIPPING_MANIFESTS.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        if path.suffix == ".toml":
            path.write_text(
                f'[package]\nname = "{name}"\nversion = "{version}"\n',
                encoding="utf-8",
            )
        else:
            path.write_text(json.dumps({"name": name, "version": version}), encoding="utf-8")


class SourceVersionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.common = load_policy("release_common")

    def test_current_checkout_shipping_versions_agree(self) -> None:
        # The shipping set is fixed, but the release version must not be frozen at 0.2.
        self.assertRegex(self.common.check_versions(ROOT), r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")

    def test_coordinated_versions_accept_future_releases_and_ignore_private_packages(self) -> None:
        with tempfile.TemporaryDirectory(prefix="daml-version-policy-") as temporary:
            root = Path(temporary)
            for version in ("0.2.0", "1.7.12"):
                with self.subTest(version=version):
                    write_versions(root, version)
                    for private in ("fuzz/Cargo.toml", "integrations/verify/package.json"):
                        path = root / private
                        path.parent.mkdir(parents=True, exist_ok=True)
                        path.write_text(
                            '[package]\nname = "daml-fuzz"\nversion = "0.0.0"\n'
                            if path.suffix == ".toml"
                            else '{"name":"daml-integration-verify","version":"0.0.0"}',
                            encoding="utf-8",
                        )
                    self.assertEqual(self.common.check_versions(root), version)

    def test_each_shipping_manifest_drift_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="daml-version-drift-") as temporary:
            root = Path(temporary)
            for relative, name in SHIPPING_MANIFESTS.items():
                with self.subTest(manifest=relative):
                    write_versions(root, "0.2.0")
                    path = root / relative
                    path.write_text(
                        f'[package]\nname = "{name}"\nversion = "0.1.0"\n'
                        if path.suffix == ".toml"
                        else json.dumps({"name": name, "version": "0.1.0"}),
                        encoding="utf-8",
                    )
                    with self.assertRaises(ValueError):
                        self.common.check_versions(root)

    def test_noncanonical_coordinated_versions_are_not_releases(self) -> None:
        with tempfile.TemporaryDirectory(prefix="daml-version-format-") as temporary:
            root = Path(temporary)
            for version in ("v0.2.0", "01.2.0", "0.02.0", "0.2.00", "0.2", "0.2.0-rc.1", "0.2.0+build", ""):
                with self.subTest(version=version):
                    write_versions(root, version)
                    with self.assertRaises(ValueError):
                        self.common.check_versions(root)

    def test_missing_shipping_manifest_is_not_silently_ignored(self) -> None:
        with tempfile.TemporaryDirectory(prefix="daml-version-missing-") as temporary:
            root = Path(temporary)
            for relative in SHIPPING_MANIFESTS:
                with self.subTest(manifest=relative):
                    write_versions(root, "0.2.0")
                    (root / relative).unlink()
                    with self.assertRaises((ValueError, FileNotFoundError)):
                        self.common.check_versions(root)

class ExpectedAssetTests(unittest.TestCase):
    def test_exact_shipping_payload_names_derive_from_version(self) -> None:
        common = load_policy("release_common")
        for version in ("0.2.0", "1.7.12"):
            with self.subTest(version=version):
                expected = {
                    f"daml-{version}-x86_64-unknown-linux-gnu.tar.gz": {"kind": "native", "target": "x86_64-unknown-linux-gnu"},
                    f"daml-{version}-aarch64-apple-darwin.tar.gz": {"kind": "native", "target": "aarch64-apple-darwin"},
                    f"daml-{version}-x86_64-pc-windows-msvc.zip": {"kind": "native", "target": "x86_64-pc-windows-msvc"},
                    f"daml_config-{version}-cp312-abi3-manylinux_2_35_x86_64.whl": {"kind": "python", "target": "x86_64-unknown-linux-gnu"},
                    f"daml_config-{version}-cp312-abi3-macosx_11_0_arm64.whl": {"kind": "python", "target": "aarch64-apple-darwin"},
                    f"daml_config-{version}-cp312-abi3-win_amd64.whl": {"kind": "python", "target": "x86_64-pc-windows-msvc"},
                    f"dprb-work-daml-browser-{version}.tgz": {"kind": "browser", "target": "wasm32-unknown-unknown"},
                    f"definitively-asserted-daml-{version}.vsix": {"kind": "vsix", "target": "universal"},
                }
                self.assertEqual(common.expected_assets(version), expected)


SOURCE_SHA = "0123456789abcdef0123456789abcdef01234567"
RUN_ID = 123456789
WORKFLOW_ID = 7654321
PREPARE_PATH = ".github/workflows/prepare-release.yml"
PUBLICATION_TARGETS = ("github-only", "github-and-pypi")


def prepare_origin() -> tuple[dict, dict]:
    """Reduced API-shaped responses, not network or producer substitutes."""
    workflow = {
        "id": WORKFLOW_ID,
        "node_id": "W_kwDOExample",
        "name": "Prepare release",
        "path": PREPARE_PATH,
        "state": "active",
        "url": f"https://api.github.com/repos/dprb-work/daml/actions/workflows/{WORKFLOW_ID}",
    }
    run = {
        "id": RUN_ID,
        "name": "Prepare release",
        "workflow_id": WORKFLOW_ID,
        "path": PREPARE_PATH,
        "event": "workflow_dispatch",
        "status": "completed",
        "conclusion": "success",
        "head_branch": "main",
        "head_sha": SOURCE_SHA,
        "run_attempt": 1,
        "run_number": 42,
        "repository": {"full_name": "dprb-work/daml", "fork": False},
        "head_repository": {"full_name": "dprb-work/daml", "fork": False},
        "html_url": f"https://github.com/dprb-work/daml/actions/runs/{RUN_ID}",
    }
    return run, workflow


def release_environment() -> tuple[dict, dict]:
    environment = {
        "id": 23456,
        "node_id": "EN_kwDOExample",
        "name": "release",
        "protection_rules": [{
            "id": 34567,
            "node_id": "PR_kwDOExample",
            "type": "required_reviewers",
            "prevent_self_review": True,
            "reviewers": [{
                "type": "User",
                "reviewer": {"login": "dprbook", "id": 45678, "type": "User"},
            }],
        }],
        "deployment_branch_policy": {
            "protected_branches": False,
            "custom_branch_policies": True,
        },
    }
    policies = {
        "total_count": 1,
        "branch_policies": [{
            "id": 56789, "node_id": "DBP_kwDOExample", "name": "main", "type": "branch",
        }],
    }
    return environment, policies


class PublicationPolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        # Restore import state after loading; no test-wide path or credential changes.
        with patch.dict(sys.modules, {"release_common": load_policy("release_common")}):
            cls.publisher = load_policy("release_publish")

    def test_canonical_successful_main_preparation_is_admitted(self) -> None:
        run, workflow = prepare_origin()
        self.publisher.validate_prepare_run(
            run, workflow, source_sha=SOURCE_SHA, prepare_run_id=RUN_ID,
        )

    def test_incomplete_failed_or_noncanonical_preparations_are_rejected(self) -> None:
        cases = (
            ("id", RUN_ID + 1),
            ("workflow_id", WORKFLOW_ID + 1),
            ("path", ".github/workflows/check.yml"),
            ("path", ".github/workflows/ci-heavy.yml"),
            ("event", "pull_request"),
            ("event", "push"),
            ("status", "in_progress"),
            ("status", "queued"),
            ("conclusion", "failure"),
            ("conclusion", "cancelled"),
            ("conclusion", "skipped"),
            ("conclusion", "neutral"),
            ("conclusion", None),
            ("head_branch", "build/release-pipeline"),
            ("head_branch", None),
            ("head_sha", "f" * 40),
            ("repository", {"full_name": "untrusted/daml", "fork": True}),
            ("head_repository", {"full_name": "untrusted/daml", "fork": True}),
            ("run_attempt", 0),
            ("run_attempt", True),
            ("run_attempt", None),
            ("head_repository", None),
        )
        for key, value in cases:
            with self.subTest(field=key, value=value):
                run, workflow = prepare_origin()
                run[key] = value
                with self.assertRaises(RuntimeError):
                    self.publisher.validate_prepare_run(
                        run, workflow, source_sha=SOURCE_SHA, prepare_run_id=RUN_ID,
                    )

    def test_workflow_identity_is_not_inferred_from_run_display_name(self) -> None:
        for key, value in (("id", WORKFLOW_ID + 1), ("path", ".github/workflows/check.yml")):
            with self.subTest(field=key):
                run, workflow = prepare_origin()
                workflow[key] = value
                with self.assertRaises(RuntimeError):
                    self.publisher.validate_prepare_run(
                        run, workflow, source_sha=SOURCE_SHA, prepare_run_id=RUN_ID,
                    )

    def preparation_responses(self) -> dict:
        run, workflow = prepare_origin()
        return {
            "actions/workflows/prepare-release.yml": workflow,
            f"actions/runs/{RUN_ID}": run,
            f"compare/{SOURCE_SHA}...main": {
                "status": "identical", "ahead_by": 0, "behind_by": 0,
                "merge_base_commit": {"sha": SOURCE_SHA},
            },
            f"actions/runs/{RUN_ID}/artifacts?per_page=100&page=1": {
                "total_count": 1,
                "artifacts": [{
                    "id": 87654321,
                    "name": "daml-release-candidate",
                    "expired": False,
                    "digest": "sha256:" + "a" * 64,
                    "workflow_run": {"id": RUN_ID, "head_sha": SOURCE_SHA, "head_branch": "main"},
                }],
            },
        }

    def admit_preparation(self, responses: dict, artifact_id=None):
        args = argparse.Namespace(
            source_sha=SOURCE_SHA, prepare_run_id=RUN_ID, artifact_id=artifact_id,
            publication_target="github-and-pypi",
        )
        # Only external GET responses are replaced, never the policy under test.
        with patch.object(self.publisher, "github", side_effect=responses.__getitem__):
            return self.publisher.preparation(args)

    def test_canonical_candidate_metadata_is_admitted(self) -> None:
        run, artifact = self.admit_preparation(self.preparation_responses())
        self.assertEqual(run["id"], RUN_ID)
        self.assertEqual(artifact["id"], 87654321)

    def test_publication_requires_current_default_main_and_matching_dispatch_source(self) -> None:
        responses = {
            "": {"default_branch": "main"},
            "git/ref/heads/main": {"ref": "refs/heads/main", "object": {"type": "commit", "sha": SOURCE_SHA}},
        }
        context = {
            "GITHUB_ACTIONS": "true",
            "GITHUB_REF": "refs/heads/main",
            "GITHUB_SHA": SOURCE_SHA,
            "GITHUB_REPOSITORY": "dprb-work/daml",
        }
        args = argparse.Namespace(
            source_sha=SOURCE_SHA, publication_target="github-and-pypi",
        )
        with patch.dict(os.environ, context), patch.object(self.publisher, "github", side_effect=responses.__getitem__):
            self.publisher.require_current_main(args)
        changed_main = copy.deepcopy(responses)
        changed_main["git/ref/heads/main"]["object"]["sha"] = "f" * 40
        changed_default = copy.deepcopy(responses)
        changed_default[""]["default_branch"] = "release"
        for changed in (changed_main, changed_default):
            with self.subTest(response=changed):
                with patch.dict(os.environ, context), patch.object(self.publisher, "github", side_effect=changed.__getitem__):
                    with self.assertRaises(RuntimeError):
                        self.publisher.require_current_main(args)
        for key, value in (
            ("GITHUB_SHA", "f" * 40),
            ("GITHUB_REF", "refs/heads/build/release-pipeline"),
            ("GITHUB_REPOSITORY", "untrusted/daml"),
        ):
            with self.subTest(dispatch_field=key):
                with patch.dict(os.environ, {**context, key: value}), patch.object(self.publisher, "github", side_effect=responses.__getitem__):
                    with self.assertRaises(RuntimeError):
                        self.publisher.require_current_main(args)

    def test_nonancestor_source_and_wrong_merge_base_are_rejected(self) -> None:
        for status, merge_base in (
            ("behind", SOURCE_SHA), ("diverged", SOURCE_SHA), ("ahead", "f" * 40),
        ):
            with self.subTest(status=status, merge_base=merge_base):
                responses = self.preparation_responses()
                responses[f"compare/{SOURCE_SHA}...main"] = {
                    "status": status, "merge_base_commit": {"sha": merge_base},
                }
                with self.assertRaises(RuntimeError):
                    self.admit_preparation(responses)

    def test_expired_replaced_or_wrong_origin_candidate_is_rejected(self) -> None:
        cases = (
            ("expired", True),
            ("digest", None),
            ("id", 0),
            ("workflow_run", {"id": RUN_ID + 1, "head_sha": SOURCE_SHA}),
            ("workflow_run", {"id": RUN_ID, "head_sha": "f" * 40}),
        )
        path = f"actions/runs/{RUN_ID}/artifacts?per_page=100&page=1"
        for field, value in cases:
            with self.subTest(field=field, value=value):
                responses = self.preparation_responses()
                responses[path]["artifacts"][0][field] = value
                with self.assertRaises(RuntimeError):
                    self.admit_preparation(responses)
        with self.assertRaises(RuntimeError):
            self.admit_preparation(self.preparation_responses(), artifact_id=99999999)
        for count in (0, 2):
            with self.subTest(candidate_count=count):
                responses = self.preparation_responses()
                artifact = responses[path]["artifacts"][0]
                responses[path]["artifacts"] = [copy.deepcopy(artifact) for _ in range(count)]
                responses[path]["total_count"] = count
                with self.assertRaises(RuntimeError):
                    self.admit_preparation(responses)

    def test_staged_draft_requires_an_existing_tag_at_the_exact_approved_commit(self) -> None:
        # Ordinary JSON bytes suffice for this direct remote draft/tag policy.
        # This fixture does not claim to be a compiled release or pass bundle admission.
        with tempfile.TemporaryDirectory(prefix="daml-draft-tag-policy-") as temporary:
            directory = Path(temporary)
            contents = b'{"fixture":"remote draft asset binding"}\n'
            (directory / "RELEASE.json").write_bytes(contents)
            args = argparse.Namespace(
                bundle=directory, version="0.2.0", source_sha=SOURCE_SHA, release_id=23456789,
                publication_target="github-and-pypi",
            )
            release = {
                "id": 23456789,
                "draft": True,
                "tag_name": "v0.2.0",
                "target_commitish": SOURCE_SHA,
                "assets": [{
                    "name": "RELEASE.json",
                    "state": "uploaded",
                    "size": len(contents),
                    "digest": "sha256:" + hashlib.sha256(contents).hexdigest(),
                }],
            }
            exact = {"ref": "refs/tags/v0.2.0", "object": {"type": "commit", "sha": SOURCE_SHA}}
            responses = {"git/ref/tags/v0.2.0": exact}
            with patch.object(self.publisher, "github", side_effect=lambda path, **kwargs: responses[path]):
                self.publisher.validate_draft(release, args)
            for reference in (
                None,
                {"ref": "refs/tags/v0.2.0", "object": {"type": "commit", "sha": "f" * 40}},
                {"ref": "refs/tags/v0.2.0", "object": {"type": "tag", "sha": SOURCE_SHA}},
            ):
                with self.subTest(reference=reference):
                    responses["git/ref/tags/v0.2.0"] = reference
                    with patch.object(self.publisher, "github", side_effect=lambda path, **kwargs: responses[path]):
                        with self.assertRaises(RuntimeError):
                            self.publisher.validate_draft(release, args)

    def test_main_only_release_with_required_human_and_no_self_review_is_admitted(self) -> None:
        self.publisher.validate_environment(*release_environment())

    def test_absent_or_weakened_environment_gate_is_rejected(self) -> None:
        environment, policies = release_environment()
        cases = [
            {},
            {**environment, "name": "unprotected"},
            {**environment, "protection_rules": []},
            {**environment, "deployment_branch_policy": None},
            {**environment, "deployment_branch_policy": {
                "protected_branches": True, "custom_branch_policies": False,
            }},
        ]
        for replacement in cases:
            with self.subTest(environment=replacement):
                with self.assertRaises(RuntimeError):
                    self.publisher.validate_environment(replacement, policies)

    def test_automation_review_or_self_review_does_not_replace_human_approval(self) -> None:
        for change in ("self-review", "no-reviewer", "automation-reviewer", "team-reviewer", "extra-reviewer"):
            with self.subTest(change=change):
                environment, policies = release_environment()
                rule = environment["protection_rules"][0]
                if change == "self-review":
                    rule["prevent_self_review"] = False
                elif change == "no-reviewer":
                    rule["reviewers"] = []
                elif change == "automation-reviewer":
                    rule["reviewers"][0]["reviewer"]["login"] = "dprRobo"
                elif change == "team-reviewer":
                    rule["reviewers"][0]["type"] = "Team"
                else:
                    extra = copy.deepcopy(rule["reviewers"][0])
                    extra["reviewer"]["login"] = "dprRobo"
                    rule["reviewers"].append(extra)
                with self.assertRaises(RuntimeError):
                    self.publisher.validate_environment(environment, policies)

    def test_wildcard_tag_additional_or_incompletely_enumerated_policies_are_rejected(self) -> None:
        environment, policies = release_environment()
        wildcard = copy.deepcopy(policies)
        wildcard["branch_policies"][0]["name"] = "*"
        tag = copy.deepcopy(policies)
        tag["branch_policies"][0]["type"] = "tag"
        extra = copy.deepcopy(policies)
        extra["branch_policies"].append({"id": 67890, "name": "release/*", "type": "branch"})
        extra["total_count"] = 2
        cases = ({}, {"total_count": 0, "branch_policies": []}, wildcard, tag, extra, {**policies, "total_count": 2})
        for replacement in cases:
            with self.subTest(policies=replacement):
                with self.assertRaises(RuntimeError):
                    self.publisher.validate_environment(environment, replacement)

    def test_every_publication_command_requires_a_known_explicit_target(self) -> None:
        for command in ("plan", "preflight", "stage-github", "promote"):
            for target in (None, "", "pypi", "github", "GITHUB-ONLY"):
                with self.subTest(command=command, target=target):
                    argv = [
                        "release_publish.py", command,
                        "--version", "0.2.1", "--source-sha", SOURCE_SHA,
                        "--prepare-run-id", str(RUN_ID),
                        "--manifest-sha256", "a" * 64,
                    ]
                    if command == "preflight":
                        argv.extend(["--out", "unused-candidate"])
                    elif command in {"stage-github", "promote"}:
                        argv.extend(["--bundle", "unused-candidate"])
                    if command == "promote":
                        argv.extend(["--release-id", "23456789"])
                    if target is not None:
                        argv.extend(["--publication-target", target])
                    with (
                        patch.object(sys, "argv", argv),
                        patch.object(self.publisher, "github", side_effect=AssertionError("network before target admission")),
                        patch.object(self.publisher, "pypi_version", side_effect=AssertionError("PyPI before target admission")),
                        patch.object(self.publisher.subprocess, "run", side_effect=AssertionError("subprocess before target admission")),
                        redirect_stderr(io.StringIO()),
                    ):
                        with self.assertRaises(SystemExit) as failure:
                            self.publisher.main()
                    self.assertEqual(failure.exception.code, 2)

    def test_write_commands_reject_missing_or_different_workflow_target_before_effects(self) -> None:
        for command in (self.publisher.stage_github, self.publisher.promote):
            for target in PUBLICATION_TARGETS:
                other = next(mode for mode in PUBLICATION_TARGETS if mode != target)
                for workflow_target in (None, "", "unknown", other):
                    with self.subTest(command=command, target=target, workflow_target=workflow_target):
                        context = {
                            "GITHUB_ACTIONS": "true",
                            "GITHUB_EVENT_NAME": "workflow_dispatch",
                            "GITHUB_REPOSITORY": "dprb-work/daml",
                            "GITHUB_REF": "refs/heads/main",
                            "GITHUB_SHA": SOURCE_SHA,
                            "GITHUB_WORKFLOW_REF": "dprb-work/daml/.github/workflows/publish-release.yml@refs/heads/main",
                        }
                        if workflow_target is not None:
                            context["RELEASE_TARGET"] = workflow_target
                        args = argparse.Namespace(
                            version="0.2.1", source_sha=SOURCE_SHA,
                            prepare_run_id=RUN_ID, manifest_sha256="a" * 64,
                            artifact_id=87654321, bundle=Path("unused-candidate"),
                            release_id=23456789, publication_target=target,
                        )
                        with (
                            patch.dict(os.environ, context, clear=True),
                            patch.object(self.publisher, "github", side_effect=AssertionError("GitHub before target admission")),
                            patch.object(self.publisher, "pypi_version", side_effect=AssertionError("PyPI before target admission")),
                            patch.object(self.publisher.subprocess, "run", side_effect=AssertionError("external write before target admission")),
                        ):
                            with self.assertRaises(RuntimeError):
                                command(args)

    def test_matching_target_does_not_authorize_noncanonical_write_context(self) -> None:
        cases = (
            ("GITHUB_ACTIONS", "false"),
            ("GITHUB_EVENT_NAME", "push"),
            ("GITHUB_REPOSITORY", "untrusted/daml"),
            ("GITHUB_REF", "refs/tags/v0.2.1"),
            ("GITHUB_WORKFLOW_REF", "dprb-work/daml/.github/workflows/check.yml@refs/heads/main"),
            ("GITHUB_WORKFLOW_REF", "dprb-work/daml/.github/workflows/publish-release.yml@refs/heads/release"),
        )
        for command in (self.publisher.stage_github, self.publisher.promote):
            for target in PUBLICATION_TARGETS:
                for field, value in cases:
                    with self.subTest(command=command, target=target, field=field, value=value):
                        context = {
                            "GITHUB_ACTIONS": "true",
                            "GITHUB_EVENT_NAME": "workflow_dispatch",
                            "GITHUB_REPOSITORY": "dprb-work/daml",
                            "GITHUB_REF": "refs/heads/main",
                            "GITHUB_SHA": SOURCE_SHA,
                            "GITHUB_WORKFLOW_REF": "dprb-work/daml/.github/workflows/publish-release.yml@refs/heads/main",
                            "RELEASE_TARGET": target,
                            field: value,
                        }
                        args = argparse.Namespace(publication_target=target)
                        with (
                            patch.dict(os.environ, context, clear=True),
                            patch.object(self.publisher, "github", side_effect=AssertionError("GitHub before context admission")),
                            patch.object(self.publisher, "pypi_version", side_effect=AssertionError("PyPI before context admission")),
                            patch.object(self.publisher.subprocess, "run", side_effect=AssertionError("write before context admission")),
                        ):
                            with self.assertRaises(RuntimeError):
                                command(args)

    def test_registry_observation_requires_exact_unyanked_wheel_hashes(self) -> None:
        manifest = {"assets": [
            {"filename": f"daml_config-0.2.1-{platform}.whl", "sha256": digit * 64}
            for platform, digit in (("linux", "a"), ("macos", "b"), ("windows", "c"))
        ]}
        exact = {"urls": [
            {
                "filename": asset["filename"],
                "digests": {"sha256": asset["sha256"]},
                "yanked": False,
                "packagetype": "bdist_wheel",
            }
            for asset in manifest["assets"]
        ]}
        self.publisher.observe_pypi(manifest, exact)
        absent_file = copy.deepcopy(exact)
        absent_file["urls"].pop()
        wrong_hash = copy.deepcopy(exact)
        wrong_hash["urls"][0]["digests"]["sha256"] = "f" * 64
        duplicate = copy.deepcopy(exact)
        duplicate["urls"].append(copy.deepcopy(duplicate["urls"][0]))
        extra = copy.deepcopy(exact)
        extra["urls"].append({
            "filename": "daml_config-0.2.1.tar.gz", "digests": {"sha256": "d" * 64},
            "yanked": False, "packagetype": "sdist",
        })
        yanked = copy.deepcopy(exact)
        yanked["urls"][0]["yanked"] = True
        wrong_kind = copy.deepcopy(exact)
        wrong_kind["urls"][0]["packagetype"] = "sdist"
        for observed in (None, {"urls": []}, absent_file, wrong_hash, duplicate, extra, yanked, wrong_kind):
            with self.subTest(observed=observed):
                with self.assertRaises(RuntimeError):
                    self.publisher.observe_pypi(manifest, observed)

    def test_github_only_rejects_registry_wheel_directory_without_preparing_files(self) -> None:
        with tempfile.TemporaryDirectory(prefix="daml-no-registry-wheels-") as temporary:
            root = Path(temporary)
            candidate = root / "candidate"
            registry = root / "pypi"
            argv = [
                "release_publish.py", "preflight", "--publication-target", "github-only",
                "--version", "0.2.1", "--source-sha", SOURCE_SHA,
                "--prepare-run-id", str(RUN_ID), "--manifest-sha256", "a" * 64,
                "--out", str(candidate), "--wheel-dir", str(registry),
            ]
            with (
                patch.object(sys, "argv", argv),
                patch.object(self.publisher, "github", side_effect=AssertionError("network before wheel-dir admission")),
                patch.object(self.publisher, "pypi_version", side_effect=AssertionError("GitHub-only contacted PyPI")),
                patch.object(self.publisher.subprocess, "run", side_effect=AssertionError("external effect before wheel-dir admission")),
                redirect_stderr(io.StringIO()),
            ):
                with self.assertRaises(SystemExit) as failure:
                    self.publisher.main()
            self.assertEqual(failure.exception.code, 2)
            self.assertFalse(registry.exists())
            self.assertFalse(candidate.exists())



if __name__ == "__main__":
    unittest.main()
