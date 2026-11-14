#!/usr/bin/env python3
"""Behavioral contract tests for the DAML CI planner and required aggregate."""

import importlib.util
import json
import os
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
    def test_nightly_runs_all_source_groups_without_release_qualification(self) -> None:
        for path in ("README.md", "src/lsp.rs", "*"):
            self.assertEqual(
                plan("nightly", path),
                {"tier": "full", "checks": ["docs", "core", "dependencies", "fuzz", "editors"]},
            )

    def test_nightly_skips_only_when_a_successful_scheduled_head_matches(self) -> None:
        for current, previous, expected in (
            ("a" * 40, "a" * 40, "unchanged"),
            ("b" * 40, "a" * 40, "full"),
            ("a" * 40, "", "full"),
            ("", "", "full"),
        ):
            with self.subTest(current=current, previous=previous):
                with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8") as paths:
                    paths.write("*\n")
                    paths.flush()
                    result = subprocess.run(
                        ["python3", str(PLANNER), "--mode", "nightly",
                         "--paths-file", paths.name, "--head", current,
                         "--last-success-head", previous],
                        cwd=ROOT, capture_output=True, text=True, check=True,
                    )
                selected = json.loads(result.stdout)
                self.assertEqual(selected["tier"], expected)
                self.assertEqual(bool(selected["checks"]), expected == "full")

    def test_ready_documentation_selects_only_docs(self) -> None:
        self.assertEqual(plan("ready", "README.md"), {"tier": "docs", "checks": ["docs"]})

    def test_owned_prose_and_openspec_metadata_are_docs_but_runtime_markdown_is_not(self) -> None:
        for path in (
            "docs/README.md", "docs/backlog.md", "docs/development/ci.md",
            "openspec/changes/archive/change/.openspec.yaml",
            "openspec/changes/archive/change/specs/contract/spec.md",
        ):
            self.assertEqual(plan("ready", path), {"tier": "docs", "checks": ["docs"]})
        for path in ("docs/executable.yaml", "openspec/runner.yml", "skills/runtime/SKILL.md"):
            self.assertEqual(plan("ready", path)["tier"], "full")

    def test_push_never_selects_heavy_work_even_for_unknown_or_shared_changes(self) -> None:
        for path in ("src/parser.rs", "Cargo.lock", ".github/workflows/check.yml", "*", "unknown"):
            self.assertEqual(plan("push", path), {"tier": "push", "checks": ["docs"]})

    def test_mixed_or_renamed_documentation_cannot_hide_a_runtime_input(self) -> None:
        self.assertEqual(
            plan("ready", "docs/old.md", "src/new.rs"),
            {"tier": "cheap", "checks": ["core"]},
        )
        self.assertEqual(plan("ready", "docs/old.md", "unknown/new.md")["tier"], "full")

    def test_risky_shared_and_setup_inputs_automatically_run_all_source_groups(self) -> None:
        expected = {"docs", "core", "dependencies", "fuzz", "editors"}
        for path in (
            "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml",
            "bindings/python/uv.lock", "integrations/vscode/package-lock.json",
            "scripts/check-source", "tests/ci_plan_test.py", "fuzz/fuzz_targets/parse.rs",
        ):
            with self.subTest(path=path):
                result = plan("ready", path)
                self.assertEqual(result["tier"], "full")
                self.assertEqual(set(result["checks"]), expected)

    def test_nested_binding_setup_requires_full_source(self) -> None:
        for path in (
            "bindings/python/rust-toolchain.toml", "bindings/python/uv.toml",
            "bindings/python/new-setup.conf", "integrations/verify/rust-toolchain.toml",
        ):
            with self.subTest(path=path):
                self.assertEqual(
                    plan("ready", path),
                    {"tier": "full", "checks": ["docs", "core", "dependencies", "fuzz", "editors"]},
                )


    def test_malformed_path_sets_fail_rather_than_selecting_a_cheap_tier(self) -> None:
        for path in ("../escape.rs", "/absolute.rs", "src\\file.rs", "src/\rfile.rs"):
            self.assertNotEqual(plan_result("ready", path).returncode, 0)

    def test_draft_with_new_heavy_workflow_path_never_selects_automatic_heavy_checks(self) -> None:
        self.assertEqual(
            plan("draft", ".github/workflows/new-heavy-job.yml"),
            {"tier": "draft", "checks": []},
        )

    def test_release_preview_always_selects_every_qualification_group(self) -> None:
        for path in ("README.md", "src/format.rs", "SOURCES.md", "unmapped/input"):
            with self.subTest(path=path):
                result = plan("manual-full", path)
                self.assertEqual(result["tier"], "full")
                self.assertEqual(
                    set(result["checks"]),
                    {"docs", "core", "dependencies", "fuzz", "editors", "binaries"},
                )

    def test_automatic_plans_never_select_the_cross_platform_binary_group(self) -> None:
        for mode in ("draft", "ready", "push"):
            for path in (
                "README.md", "src/bin/daml-fmt.rs", "src/bin/daml-lsp.rs",
                "Cargo.toml", "Cargo.lock", "rust-toolchain.toml",
                "bindings/python/Cargo.toml", "SOURCES.md",
                ".github/workflows/check.yml", "scripts/check", "scripts/ci-plan.py",
                "unmapped/input", "*", "**",
            ):
                with self.subTest(mode=mode, path=path):
                    self.assertNotIn("binaries", plan(mode, path)["checks"])

    def test_runtime_skill_markdown_is_not_documentation_only(self) -> None:
        result = plan("ready", "skills/verify-daml-distribution/SKILL.md")
        self.assertNotEqual(result["tier"], "docs")
        self.assertTrue(result["checks"])

    def test_examples_yaml_selects_affected_core_boundary_without_docs_only_routing(self) -> None:
        result = plan("ready", "examples/config.yml")
        self.assertNotEqual(result["tier"], "docs")
        assert_selected(result["checks"], "core")

    def test_archive_manifest_keeps_native_compilation_without_platform_packaging(self) -> None:
        for mode in ("ready",):
            for paths in (("SOURCES.md",), ("README.md", "SOURCES.md")):
                with self.subTest(mode=mode, paths=paths):
                    result = plan(mode, *paths)
                    self.assertNotEqual(result["tier"], "docs")
                    self.assertEqual(
                        set(result["checks"]),
                        {"core", "docs"} if len(paths) == 2 else {"core"},
                    )

    def test_code_tests_and_fixture_inputs_select_required_behavioral_checks(self) -> None:
        self.assertEqual(plan("ready", "src/resolve.rs")["checks"], ["core"])
        self.assertEqual(set(plan("ready", "tests/validation.rs")["checks"]), {"core"})
        self.assertEqual(
            set(plan("ready", "tests/fixtures/schema.json")["checks"]),
            {"core"},
        )

    def test_semantic_editor_fixture_changes_select_real_editor_checks(self) -> None:
        for mode in ("ready",):
            for path in (
                "tests/fixtures/semantic-editor/workspace/daml.bundle",
                "tests/fixtures/semantic-editor/providers/Http.daml",
                "tests/fixtures/semantic-editor/contracts/Adapter.daml",
            ):
                with self.subTest(mode=mode, path=path):
                    result = plan(mode, path)
                    self.assertEqual(result["tier"], "cheap")
                    self.assertEqual(set(result["checks"]), {"core", "editors"})

    def test_parser_and_native_formatter_use_core_while_nightly_covers_editor_and_fuzz(self) -> None:
        required = {"core"}
        for path in (
            "src/parser.rs",
            "src/parser/native.rs",
            "src/native/format.rs",
            "src/format.rs",
        ):
            result = plan("ready", path)
            self.assertEqual(result["tier"], "cheap")
            self.assertEqual(required, set(result["checks"]))


    def test_lock_workflow_and_protocol_changes_cannot_select_docs_only(self) -> None:
        self.assertTrue({"core", "dependencies"} <= set(plan("ready", "Cargo.lock")["checks"]))
        workflow = plan("ready", ".github/workflows/check.yml")
        self.assertEqual(workflow["tier"], "full")
        assert_selected(plan("ready", "src/lsp/adapter.rs")["checks"], "core")

    def test_syntect_consumer_dependency_inputs_require_automatic_full_source(self) -> None:
        for mode in ("ready",):
            for filename in ("Cargo.toml", "Cargo.lock", "deny.toml"):
                with self.subTest(mode=mode, filename=filename):
                    result = plan(mode, f"integrations/verify/syntect/{filename}")
                    self.assertEqual(result["tier"], "full")
                    self.assertEqual(
                        set(result["checks"]), {"docs", "core", "dependencies", "fuzz", "editors"}
                    )

    def test_syntect_grammar_and_checks_keep_existing_integration_boundaries(self) -> None:
        for path in (
            "integrations/syntect/daml.sublime-syntax",
            "integrations/verify/syntect/tests/grammar.rs",
        ):
            with self.subTest(path=path):
                result = plan("ready", path)
                self.assertEqual(result["tier"], "cheap")
                self.assertEqual(set(result["checks"]), {"core", "editors"})

    def test_unknown_and_broad_paths_fail_closed(self) -> None:
        all_checks = {"docs", "core", "dependencies", "fuzz", "editors"}
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
        selected = plan("ready", ".github/workflows/check.yml")["checks"]
        success = {
            "docs": "success", "core": "success", "dependencies": "success",
            "fuzz": "success", "editors": "success",
        }
        current = dict(
            planned_head="head-1",
            planned_base="base-1",
            current_head="head-1",
            current_base="base-1",
        )
        self.assertTrue(required_result(selected, success, **current))
        for check in selected:
            for bad in ("failure", "cancelled", "skipped", "unknown"):
                self.assertFalse(
                    required_result(selected, {**success, check: bad}, **current),
                    (check, bad),
                )
            self.assertFalse(
                required_result(
                    selected,
                    {name: result for name, result in success.items() if name != check},
                    **current,
                )
            )
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

class NightlyIssueContractTests(unittest.TestCase):
    """Execute the actual reporter shell against isolated issue state."""

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        directory = Path(self.temporary.name)
        self.store = directory / "issue.json"
        self.store.write_text(json.dumps({"state": "CLOSED", "comments": ["previous report"]}))
        gh = directory / "gh"
        gh.write_text("""#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
store = Path(os.environ["ISSUE_STORE"])
issue = json.loads(store.read_text())
args = sys.argv[1:]
action = args[1]
if action == "list":
    if args[args.index("--state") + 1] == "open" and issue["state"] != "OPEN":
        raise SystemExit(0)
    fields = args[args.index("--json") + 1].split(",")
    print("17\\t" + issue["state"] if "state" in fields else "17")
elif action in {"comment", "close", "reopen", "edit"}:
    if args[2] != "17":
        raise SystemExit("wrong issue identity")
    if action == "comment":
        issue["comments"].append(args[args.index("--body") + 1])
    elif action == "close":
        issue["state"] = "CLOSED"
    elif action == "reopen":
        issue["state"] = "OPEN"
    else:
        issue["body"] = args[args.index("--body") + 1]
    store.write_text(json.dumps(issue))
else:
    raise SystemExit("unexpected issue operation")
""")
        gh.chmod(0o755)
        lines = (ROOT / ".github/workflows/check.yml").read_text().splitlines()
        job = lines.index("  nightly-report:")
        run = next(index for index in range(job, len(lines)) if lines[index] == "        run: |")
        commands = []
        for line in lines[run + 1:]:
            if line and not line.startswith("          "):
                break
            commands.append(line[10:])
        self.reporter = "\n".join(commands)
        self.environment = {
            **os.environ, "PATH": f"{directory}:{os.environ['PATH']}",
            "ISSUE_STORE": str(self.store), "HEAD": "a" * 40,
            "RUN_URL": "https://example.invalid/run/1",
        }

    def run_reporter(self, result: str = "success") -> None:
        subprocess.run(
            ["bash", "-e", "-o", "pipefail", "-c", self.reporter], check=True,
            env={**self.environment, "RESULTS": json.dumps({"required": {"result": result}})},
            capture_output=True, text=True,
        )

    def test_green_does_not_comment_on_or_modify_a_closed_issue(self) -> None:
        before = json.loads(self.store.read_text())
        self.run_reporter()
        self.assertEqual(json.loads(self.store.read_text()), before)

    def test_recovery_closes_open_issue_without_repeated_green_comments(self) -> None:
        self.store.write_text(json.dumps({"state": "OPEN", "comments": []}))
        self.run_reporter()
        recovered = json.loads(self.store.read_text())
        self.assertEqual(recovered["state"], "CLOSED")
        self.assertIn(self.environment["RUN_URL"], recovered["comments"][0])
        self.run_reporter()
        self.assertEqual(json.loads(self.store.read_text()), recovered)

    def test_red_reopens_and_updates_the_existing_closed_issue(self) -> None:
        self.run_reporter("failure")
        issue = json.loads(self.store.read_text())
        self.assertEqual(issue["state"], "OPEN")
        self.assertIn(self.environment["RUN_URL"], issue["body"])



if __name__ == "__main__":
    unittest.main()
