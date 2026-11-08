#!/usr/bin/env python3
"""Independent delivery policy checks; no builds, credentials, or publication.

Run: python3 -B -m unittest discover -s tests -p test_release_delivery.py
Real release bytes are checked separately by tests/release_candidate_check.py.
"""

import argparse
import copy
import importlib.util
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def load_policy(name: str):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    with patch.dict(sys.modules, {name: module}):
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
        args = argparse.Namespace(source_sha=SOURCE_SHA)
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



if __name__ == "__main__":
    unittest.main()
