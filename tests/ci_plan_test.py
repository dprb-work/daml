#!/usr/bin/env python3
"""Behavioral contract tests for the DAML CI planner and required aggregate."""

import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PLANNER = ROOT / "scripts" / "ci-plan.py"


def plan_result(mode: str, *paths: str) -> subprocess.CompletedProcess[str]:
    with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8") as changed:
        changed.write("\n".join(paths))
        changed.flush()
        return subprocess.run(
            ["python3", str(PLANNER), "--mode", mode, "--paths-file", changed.name],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )


def plan(mode: str, *paths: str) -> dict[str, object]:
    result = plan_result(mode, *paths)
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    return json.loads(result.stdout)


def assert_selected(actual: list[str], *required: str) -> None:
    if not set(required) <= set(actual):
        raise AssertionError(f"missing required checks {set(required) - set(actual)} in {actual}")


class CiPlanContractTests(unittest.TestCase):
    def test_ready_documentation_selects_only_docs(self) -> None:
        self.assertEqual(plan("ready", "README.md"), {"tier": "docs", "checks": ["docs"]})

    def test_draft_with_new_heavy_workflow_path_never_selects_automatic_heavy_checks(self) -> None:
        self.assertEqual(
            plan("draft", ".github/workflows/new-heavy-job.yml"),
            {"tier": "draft", "checks": []},
        )

    def test_manual_full_selects_every_required_boundary_even_for_docs(self) -> None:
        result = plan("manual-full", "README.md")
        self.assertEqual(result["tier"], "full")
        self.assertEqual(set(result["checks"]), {"docs", "core", "dependencies", "fuzz", "editors", "binaries"})

    def test_runtime_skill_markdown_is_not_documentation_only(self) -> None:
        result = plan("ready", "skills/verify-daml-distribution/SKILL.md")
        self.assertNotEqual(result["tier"], "docs")
        self.assertTrue(result["checks"])

    def test_examples_yaml_selects_affected_core_boundary_without_docs_only_routing(self) -> None:
        result = plan("ready", "examples/config.yml")
        self.assertNotEqual(result["tier"], "docs")
        assert_selected(result["checks"], "core")

    def test_archive_manifest_selects_distribution_proof_without_docs_only_routing(self) -> None:
        result = plan("ready", "SOURCES.md")
        self.assertNotEqual(result["tier"], "docs")
        assert_selected(result["checks"], "binaries")

    def test_code_tests_and_fixture_inputs_select_required_behavioral_checks(self) -> None:
        assert_selected(plan("ready", "src/resolve.rs")["checks"], "core", "binaries", "fuzz", "editors")
        self.assertEqual(set(plan("ready", "tests/validation.rs")["checks"]), {"core"})
        self.assertEqual(
            set(plan("ready", "tests/fixtures/schema.json")["checks"]),
            {"core", "fuzz"},
        )

    def test_parser_and_native_formatter_changes_require_editor_integration(self) -> None:
        required = {"core", "fuzz", "binaries", "editors"}
        for path in (
            "src/parser.rs",
            "src/parser/native.rs",
            "src/native/format.rs",
            "src/format.rs",
        ):
            result = plan("ready", path)
            self.assertIn(result["tier"], {"targeted", "full"})
            self.assertTrue(required <= set(result["checks"]))


    def test_lock_workflow_and_protocol_changes_cannot_select_docs_only(self) -> None:
        self.assertTrue({"core", "dependencies"} <= set(plan("ready", "Cargo.lock")["checks"]))
        workflow = plan("ready", ".github/workflows/check.yml")
        self.assertEqual(workflow["tier"], "full")
        assert_selected(plan("ready", "src/lsp/adapter.rs")["checks"], "core", "editors")

    def test_unknown_and_broad_paths_fail_closed(self) -> None:
        all_checks = {"docs", "core", "dependencies", "fuzz", "editors", "binaries"}
        for path in ("unmapped/new-input.bin", ".", "*.rs"):
            with self.subTest(path=path):
                result = plan_result("ready", path)
                if result.returncode == 0:
                    selected = json.loads(result.stdout)
                    self.assertEqual(selected["tier"], "full")
                    self.assertEqual(set(selected["checks"]), all_checks)

    def test_empty_path_file_is_not_a_successful_plan(self) -> None:
        result = plan_result("ready")
        self.assertNotEqual(result.returncode, 0)

    def test_required_aggregate_rejects_failures_cancellation_and_stale_head_or_base(self) -> None:
        planner_spec = importlib.util.spec_from_file_location("ci_plan", PLANNER)
        self.assertIsNotNone(planner_spec)
        self.assertIsNotNone(planner_spec.loader)
        planner = importlib.util.module_from_spec(planner_spec)
        planner_spec.loader.exec_module(planner)
        required_result = planner.required_result
        selected = ["core", "editors"]
        success = {"core": "success", "editors": "success"}
        current = dict(
            planned_head="head-1",
            planned_base="base-1",
            current_head="head-1",
            current_base="base-1",
        )
        self.assertTrue(required_result(selected, success, **current))
        for bad in ("failure", "cancelled", "skipped", "unknown"):
            self.assertFalse(
                required_result(selected, {**success, "editors": bad}, **current), bad
            )
        self.assertFalse(required_result(selected, {"core": "success"}, **current))
        self.assertFalse(
            required_result(["unknown-check"], {"unknown-check": "success"}, **current)
        )
        self.assertFalse(
            required_result(selected, success, **{**current, "current_head": "head-2"})
        )
        self.assertFalse(
            required_result(selected, success, **{**current, "current_base": "base-2"})
        )
        self.assertFalse(
            required_result(selected, success, **{**current, "planned_head": ""})
        )
        self.assertFalse(required_result([], success, **current))



if __name__ == "__main__":
    unittest.main()
