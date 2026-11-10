#!/usr/bin/env python3
"""Pure security-proof metadata checks; no scanner or compiled candidate proof."""

import copy
import hashlib
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from datetime import UTC, datetime, timedelta
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
REQUIRED_CHECKS = ("dependencies", "secrets", "sast", "malware")
SOURCE_SHA = "0123456789abcdef0123456789abcdef01234567"


def load_module():
    name = "release_security"
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / f"{name}.py")
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load release_security")
    module = importlib.util.module_from_spec(spec)
    with patch.dict(sys.modules, {name: module}):
        spec.loader.exec_module(module)
    return module


class SecurityEvidenceAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        security = self.root / "security"
        security.mkdir()
        policy = {
            "schema": 1,
            "max_evidence_age_hours": 24,
            "max_database_age_hours": 24,
            "tools": {
                "trivy": {
                    "version": "0.76.2",
                    "image": "trivy@sha256:" + "1" * 64,
                },
                "semgrep": {
                    "version": "1.184.0",
                    "image": "semgrep@sha256:" + "2" * 64,
                },
                "clamav": {
                    "version": "1.5.4",
                    "image": "clamav@sha256:" + "3" * 64,
                },
            },
            "dependency_files": ["Cargo.lock"],
            "sast_rule_ids": ["local-rule"],
            "sast_extensions": [".py", ".ts", ".js", ".jsx", ".rs"],
        }
        (security / "policy.json").write_text(
            json.dumps(policy) + "\n", encoding="utf-8"
        )
        self.policy_bytes = (security / "policy.json").read_bytes()
        (security / "semgrep.yml").write_text(
            "rules:\n"
            "  - id: local-rule\n"
            "    languages: [python]\n"
            "    message: Avoid dynamic evaluation.\n"
            "    severity: ERROR\n"
            "    pattern: eval($VALUE)\n",
            encoding="utf-8",
        )
        self.rules_bytes = (security / "semgrep.yml").read_bytes()
        self.policy_sha = hashlib.sha256(
            (security / "policy.json").read_bytes()
        ).hexdigest()
        self.rules_sha = hashlib.sha256(
            (security / "semgrep.yml").read_bytes()
        ).hexdigest()
        self.tools = policy["tools"]
        self.now = datetime.now(UTC)
        self.database = {
            "updated_at": self.now.isoformat(),
            "downloaded_at": self.now.isoformat(),
            "sha256": "c" * 64,
        }
        self.malware_database = {
            "refreshed_at": self.now.isoformat(),
            **{
                name: {
                    "version": 1,
                    "built_at": self.now.isoformat(),
                    "sha256": "d" * 64,
                }
                for name in ("daily", "main", "bytecode")
            },
        }
        self.assets = [
            {
                "filename": "daml.tar.gz",
                "sha256": "a" * 64,
                "size": 123,
                "kind": "archive",
                "target": "release/daml.tar.gz",
            }
        ]
        self.module = load_module()

    def tearDown(self):
        self.temp.cleanup()

    def evidence(self):
        checks = []
        for check in REQUIRED_CHECKS:
            tool_name = (
                "trivy"
                if check in ("dependencies", "secrets")
                else ("clamav" if check == "malware" else "semgrep")
            )
            row = {
                "id": check,
                "status": "passed",
                "tool": {
                    "name": tool_name,
                    "version": self.tools[tool_name]["version"],
                    "image": self.tools[tool_name]["image"],
                    "image_id": "sha256:" + "e" * 64,
                },
                "findings": 0,
                "coverage": {"complete": True},
            }
            if check == "dependencies":
                row["database"] = copy.deepcopy(self.database)
            elif check == "malware":
                row["database"] = copy.deepcopy(self.malware_database)
            checks.append(row)
        return {
            "schema": 1,
            "source_sha": SOURCE_SHA,
            "assets": copy.deepcopy(self.assets),
            "policy_sha256": self.policy_sha,
            "rules_sha256": self.rules_sha,
            "scanned_at": self.now.isoformat(),
            "status": "passed",
            "checks": checks,
        }

    def assert_rejected(self, evidence, *, source_sha=SOURCE_SHA, assets=None):
        with self.assertRaises(ValueError):
            self.module.validate_evidence(
                self.root,
                evidence,
                source_sha=source_sha,
                assets=self.assets if assets is None else assets,
            )

    def test_exact_source_and_asset_identity_is_admitted(self):
        self.module.validate_evidence(
            self.root, self.evidence(), source_sha=SOURCE_SHA, assets=self.assets
        )

    def test_source_or_asset_identity_mismatch_is_rejected(self):
        self.assert_rejected(self.evidence(), source_sha="f" * 40)
        changed = copy.deepcopy(self.assets)
        changed[0]["sha256"] = "b" * 64
        self.assert_rejected(self.evidence(), assets=changed)

    def test_policy_and_rules_are_bound_to_current_bytes(self):
        evidence = self.evidence()
        (self.root / "security" / "policy.json").write_text(
            json.dumps(
                {
                    "schema": 1,
                    "max_evidence_age_hours": 12,
                    "max_database_age_hours": 24,
                    "tools": self.tools,
                    "dependency_files": ["Cargo.lock"],
                    "sast_rule_ids": ["local-rule"],
                    "sast_extensions": [".py", ".ts", ".js", ".jsx", ".rs"],
                }
            )
            + "\n",
            encoding="utf-8",
        )
        self.assert_rejected(evidence)
        (self.root / "security" / "policy.json").write_bytes(self.policy_bytes)
        evidence = self.evidence()
        changed_rules = self.rules_bytes.replace(
            b"dynamic evaluation", b"changed evaluation"
        )
        (self.root / "security" / "semgrep.yml").write_bytes(changed_rules)
        self.assert_rejected(evidence)

    def test_missing_duplicate_extra_or_failed_required_check_is_rejected(self):
        evidence = self.evidence()
        evidence["checks"].pop()
        self.assert_rejected(evidence)
        evidence = self.evidence()
        evidence["checks"].append(copy.deepcopy(evidence["checks"][0]))
        self.assert_rejected(evidence)
        evidence = self.evidence()
        extra = copy.deepcopy(evidence["checks"][0])
        extra["id"] = "unconfigured"
        evidence["checks"].append(extra)
        self.assert_rejected(evidence)
        for row_status in ("findings", "error"):
            evidence = self.evidence()
            evidence["checks"][0]["status"] = row_status
            evidence["status"] = "failed" if row_status == "findings" else "incomplete"
            self.assert_rejected(evidence)

    def test_overall_pass_cannot_hide_missing_or_failed_scanner(self):
        evidence = self.evidence()
        evidence["checks"] = [row for row in evidence["checks"] if row["id"] != "sast"]
        self.assert_rejected(evidence)
        evidence = self.evidence()
        evidence["checks"][2]["status"] = "error"
        self.assert_rejected(evidence)

    def test_stale_future_and_invalid_timestamps_are_rejected(self):
        for timestamp in (
            (self.now - timedelta(days=3)).isoformat(),
            (self.now + timedelta(days=3)).isoformat(),
            "not-a-timestamp",
        ):
            evidence = self.evidence()
            evidence["scanned_at"] = timestamp
            self.assert_rejected(evidence)


    def test_passed_label_cannot_hide_findings_or_incomplete_coverage(self):
        for index, check in enumerate(REQUIRED_CHECKS):
            for field, value in (
                ("findings", 1),
                ("findings", False),
                ("coverage", {"complete": False}),
                ("coverage", {}),
            ):
                with self.subTest(check=check, field=field, value=value):
                    evidence = self.evidence()
                    evidence["checks"][index][field] = value
                    self.assert_rejected(evidence)

    def test_scanner_tool_identity_is_bound_to_policy_for_every_check(self):
        for index, check in enumerate(REQUIRED_CHECKS):
            for field, value in (
                ("name", "unconfigured-scanner"),
                ("version", "999.0.0"),
                ("image", "unconfigured@sha256:" + "f" * 64),
                ("image_id", "floating-image-tag"),
            ):
                with self.subTest(check=check, field=field):
                    evidence = self.evidence()
                    evidence["checks"][index]["tool"][field] = value
                    self.assert_rejected(evidence)

    def test_evidence_asset_binding_includes_filename_size_kind_and_target(self):
        for field, value in (
            ("filename", "different.tar.gz"),
            ("size", 124),
            ("kind", "different"),
            ("target", "different-target"),
        ):
            with self.subTest(field=field):
                evidence = self.evidence()
                evidence["assets"][0][field] = value
                self.assert_rejected(evidence)
        evidence = self.evidence()
        evidence["assets"].append(copy.deepcopy(evidence["assets"][0]))
        self.assert_rejected(evidence)

    def test_missing_stale_or_future_database_proof_is_rejected(self):
        old = (self.now - timedelta(days=3)).isoformat()
        future = (self.now + timedelta(days=3)).isoformat()
        for index in (0, 3):
            with self.subTest(check=REQUIRED_CHECKS[index], missing=True):
                evidence = self.evidence()
                del evidence["checks"][index]["database"]
                self.assert_rejected(evidence)
        for field in ("updated_at", "downloaded_at"):
            for timestamp in (old, future):
                with self.subTest(check="dependencies", field=field, timestamp=timestamp):
                    evidence = self.evidence()
                    evidence["checks"][0]["database"][field] = timestamp
                    self.assert_rejected(evidence)
        for timestamp in (old, future):
            with self.subTest(check="malware", field="refreshed_at", timestamp=timestamp):
                evidence = self.evidence()
                evidence["checks"][3]["database"]["refreshed_at"] = timestamp
                self.assert_rejected(evidence)
            with self.subTest(check="malware", field="daily-built-at", timestamp=timestamp):
                evidence = self.evidence()
                evidence["checks"][3]["database"]["daily"]["built_at"] = timestamp
                self.assert_rejected(evidence)

    def test_malformed_evidence_and_passing_finding_summaries_are_rejected(self):
        for schema in (2, True):
            with self.subTest(schema=schema):
                evidence = self.evidence()
                evidence["schema"] = schema
                self.assert_rejected(evidence)
        evidence = self.evidence()
        evidence["unapproved_field"] = True
        self.assert_rejected(evidence)
        evidence = self.evidence()
        evidence["checks"][0]["summaries"] = [{"identifier": "finding"}]
        evidence["checks"][0]["summaries_truncated"] = False
        self.assert_rejected(evidence)
        evidence = self.evidence()
        evidence["checks"][0]["summaries"] = []
        evidence["checks"][0]["summaries_truncated"] = True
        self.assert_rejected(evidence)


class DependencyInventoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.module = load_module()

    def tearDown(self):
        self.temp.cleanup()

    def repository_inventory(self):
        names = [
            name.decode("utf-8")
            for name in self.module._git(ROOT, "ls-files", "-z").split(b"\0")
            if name
        ]
        policy = json.loads(
            (ROOT / "security" / "policy.json").read_text(encoding="utf-8")
        )
        return names, policy

    def test_repository_tracked_locks_are_admitted_before_scanner_execution(self):
        names, policy = self.repository_inventory()
        stopped = RuntimeError("inventory admitted; scanner intentionally not executed")
        # Exercise the real inventory gate, then stop at the scanner boundary.
        # This is not evidence that a scanner covered these dependencies.
        with (
            patch.object(self.module, "_trivy", side_effect=stopped),
            self.assertRaises(RuntimeError) as caught,
        ):
            self.module._dependencies(
                [],
                {"tool": {"image": "not-executed"}},
                ROOT,
                self.root,
                policy,
                names,
            )
        self.assertIs(caught.exception, stopped)

    def test_unconfigured_tracked_syntect_lock_is_rejected_without_scanning(self):
        names, policy = self.repository_inventory()
        lock = "integrations/verify/syntect/Cargo.lock"
        self.assertIn(lock, names)
        self.assertIn(lock, policy["dependency_files"])
        policy["dependency_files"].remove(lock)
        with (
            patch.object(
                self.module,
                "_trivy",
                side_effect=AssertionError("inventory drift must fail before scanning"),
            ),
            self.assertRaises(self.module._DependencyFailure) as caught,
        ):
            self.module._dependencies([], {}, ROOT, self.root, policy, names)
        self.assertEqual(
            caught.exception.detail,
            {
                "failure_count": 1,
                "failures": [{"code": "unconfigured-lock", "path": lock}],
                "failures_truncated": False,
            },
        )

    def test_empty_npm_lock_requires_matching_dependency_free_manifest(self):
        directory = self.root / "npm"
        directory.mkdir()
        lock = directory / "package-lock.json"
        lock.write_text(
            json.dumps({"lockfileVersion": 3, "packages": {"": {}}}),
            encoding="utf-8",
        )
        (directory / "package.json").write_text(
            json.dumps({"name": "empty-project"}), encoding="utf-8"
        )
        self.assertEqual(self.module._expected_packages(lock), set())

        (directory / "package.json").write_text(
            json.dumps(
                {"name": "empty-project", "dependencies": {"declared": "^1.0.0"}}
            ),
            encoding="utf-8",
        )
        with self.assertRaises(ValueError):
            self.module._expected_packages(lock)

    def test_uv_report_excludes_only_lock_proven_local_editable_root(self):
        lock = self.root / "uv.lock"
        lock.write_text(
            "version = 1\n"
            '[[package]]\nname = "daml-config"\nsource = { editable = "." }\n'
            '[[package]]\nname = "registry-dep"\nversion = "1.2.3"\n'
            'source = { registry = "https://pypi.org/simple" }\n',
            encoding="utf-8",
        )
        rows = [
            {
                "Packages": [
                    {
                        "Name": "daml-config",
                        "ID": "daml-config",
                        "Relationship": "root",
                        "AnalyzedBy": "uv",
                    },
                    {"Name": "registry-dep", "Version": "1.2.3"},
                ]
            }
        ]
        self.assertEqual(
            self.module._reported_packages(lock, rows),
            ({("registry-dep", "1.2.3")}, ["daml-config"]),
        )

    def test_uv_registry_package_without_version_is_rejected(self):
        lock = self.root / "uv.lock"
        lock.write_text(
            "version = 1\n"
            '[[package]]\nname = "registry-dep"\nversion = "1.2.3"\n'
            'source = { registry = "https://pypi.org/simple" }\n',
            encoding="utf-8",
        )
        with self.assertRaises(ValueError):
            self.module._reported_packages(
                lock,
                [
                    {
                        "Packages": [
                            {
                                "Name": "registry-dep",
                                "ID": "registry-dep",
                                "Relationship": "root",
                                "AnalyzedBy": "uv",
                            }
                        ]
                    }
                ],
            )

    def test_pnpm_v9_peer_snapshots_count_registry_packages_once(self):
        lock = self.root / "pnpm-lock.yaml"
        lock.write_text(
            "lockfileVersion: '9.0'\n"
            "settings:\n  autoInstallPeers: true\n"
            "importers:\n  .:\n    dependencies: {}\n"
            "packages:\n"
            "  plugin@1.0.0:\n    resolution: {integrity: sha512-example}\n"
            "  peer@2.0.0: {}\n"
            "  peer@3.0.0: {}\n"
            "snapshots:\n"
            "  plugin@1.0.0(peer@2.0.0):\n    dependencies: {}\n"
            "  plugin@1.0.0(peer@3.0.0):\n    dependencies: {}\n"
            "  peer@2.0.0: {}\n"
            "  peer@3.0.0: {}\n",
            encoding="utf-8",
        )
        self.assertEqual(
            self.module._expected_packages(lock),
            {("plugin", "1.0.0"), ("peer", "2.0.0"), ("peer", "3.0.0")},
        )

    def test_pnpm_unsupported_lockfile_version_is_rejected(self):
        lock = self.root / "pnpm-lock.yaml"
        lock.write_text(
            "lockfileVersion: '8.0'\npackages: {}\nsnapshots: {}\n",
            encoding="utf-8",
        )
        with self.assertRaises(ValueError):
            self.module._expected_packages(lock)


class ContainerOwnershipTests(unittest.TestCase):
    def setUp(self):
        self.module = load_module()

    def test_rootless_docker_uses_root_inside_user_namespace(self):
        result = subprocess.CompletedProcess(
            ["docker", "info"], 0, stdout=b'["name=rootless","name=seccomp"]'
        )
        with patch.object(self.module, "_command", return_value=result):
            self.assertEqual(self.module._container_user(["docker"]), "0:0")

    def test_rootful_docker_uses_effective_host_ids(self):
        result = subprocess.CompletedProcess(
            ["docker", "info"], 0, stdout=b'["name=seccomp,profile=default"]'
        )
        with (
            patch.object(self.module, "_command", return_value=result),
            patch.object(self.module.os, "geteuid", return_value=1001),
            patch.object(self.module.os, "getegid", return_value=1002),
        ):
            self.assertEqual(self.module._container_user(["docker"]), "1001:1002")

    def test_invalid_docker_security_options_are_rejected(self):
        for output in (b"{}", b'["name=seccomp", 1]', b"not json"):
            result = subprocess.CompletedProcess(["docker", "info"], 0, stdout=output)
            with (
                self.subTest(output=output),
                patch.object(self.module, "_command", return_value=result),
                self.assertRaises(ValueError),
            ):
                self.module._container_user(["docker"])


if __name__ == "__main__":
    unittest.main()
