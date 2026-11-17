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

    def test_required_aggregate_rejects_failures_and_stale_head_but_ignores_base_tip(self) -> None:
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
        planned_head = "head-1"
        current_head = "head-1"
        self.assertTrue(required_result(selected, success, planned_head, current_head))
        for check in selected:
            for bad in ("failure", "cancelled", "skipped", "unknown"):
                self.assertFalse(
                    required_result(selected, {**success, check: bad}, planned_head, current_head),
                    (check, bad),
                )
            self.assertFalse(
                required_result(
                    selected,
                    {name: result for name, result in success.items() if name != check},
                    planned_head,
                    current_head,
                )
            )
        self.assertFalse(
            required_result(["unknown-check"], {"unknown-check": "success"}, planned_head, current_head)
        )
        self.assertFalse(required_result(selected, success, planned_head, "head-2"))
        self.assertFalse(required_result(selected, success, "", current_head))
        self.assertFalse(required_result([], success, planned_head, current_head))
class WorkflowBoundaryTests(unittest.TestCase):
    """Execute the planning, live-read, and aggregate steps from check.yml."""

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
        self.output = self.root / "step-output"
        self.summary = self.root / "step-summary"
        self.runner_temp = self.root / "runner-temp"
        self.runner_temp.mkdir()
        self.output.touch()
        self.summary.touch()
        self._init_repository()
        self._install_python_stub()

    def _init_repository(self) -> None:
        self.git("init", "-b", "main")
        self.git("config", "user.email", "ci@example.invalid")
        self.git("config", "user.name", "CI boundary test")
        self.git("config", "core.hooksPath", str(self.root / "empty-hooks"))
        planner = self.root / "scripts" / "ci-plan.py"
        planner.parent.mkdir(parents=True)
        planner.write_bytes(PLANNER.read_bytes())
        (self.root / "src").mkdir()
        (self.root / "src" / "base.rs").write_text("base\n")
        self.git("add", ".")
        self.git("commit", "-m", "base A")
        self.base_a = self.git("rev-parse", "HEAD")
        self.git("checkout", "-b", "head")
        (self.root / "src" / "head.rs").write_text("head change\n")
        self.git("add", ".")
        self.git("commit", "-m", "head H")
        self.head = self.git("rev-parse", "HEAD")
        self.git("checkout", "main")
        (self.root / "docs").mkdir()
        (self.root / "docs" / "base-advance.md").write_text("base-only change\n")
        self.git("add", ".")
        self.git("commit", "-m", "advance base B")
        self.base_b = self.git("rev-parse", "HEAD")
        tree = self.git("rev-parse", f"{self.head}^{{tree}}")
        self.merge = self.git("commit-tree", tree, "-p", self.base_a, "-p", self.head, "-m", "event merge M")
        self.git("checkout", "--detach", self.merge)

    def _install_python_stub(self) -> None:
        stub = self.root / "bin"
        stub.mkdir()
        executable = stub / "python3"
        executable.write_text(
            f"""#!{os.sys.executable}
import json, os, sys
if sys.argv[-1:] == ["-"]:
    source = sys.stdin.read()
    import urllib.request
    class Response:
        def __init__(self, value):
            self.value = value
        def read(self, *args):
            return json.dumps(self.value).encode()
        def __enter__(self):
            return self
        def __exit__(self, *args):
            return False
    def urlopen(request, *args, **kwargs):
        pull = json.loads(os.environ["PR_RESPONSE"])
        if "/git/ref/" in request.full_url:
            return Response(dict(object=dict(sha=pull["base"]["sha"])))
        return Response(pull)
    urllib.request.urlopen = urlopen
    exec(compile(source, "<workflow-step>", "exec"), {{"__name__": "__main__"}})
else:
    os.execv(os.environ["REAL_PYTHON"], [os.environ["REAL_PYTHON"], *sys.argv[1:]])
"""
        )
        executable.chmod(0o755)
        self.environment.update({
            "PATH": f"{stub}:{os.environ['PATH']}",
            "REAL_PYTHON": os.sys.executable,
            "GITHUB_OUTPUT": str(self.output),
            "GITHUB_STEP_SUMMARY": str(self.summary),
            "RUNNER_TEMP": str(self.runner_temp),
            "GH_TOKEN": "test-token",
            "REPOSITORY": "owner/repo",
            "PR_NUMBER": "72",
            "EVENT_NAME": "pull_request",
            "EVENT_PR_HEAD": self.head,
            "EVENT_HEAD_REF": "feature",
            "EVENT_HEAD_REPO": "fork/source",
            "EVENT_BASE_REF": "main",
            "EVENT_BASE_REPO": "owner/repo",
            "PR_DRAFT": "false",
            "PR_BASE": self.base_b,
            "PR_HEAD": self.head,
            "PR_BASE_REF": "main",
            "PUSH_BASE": "",
            "PUSH_HEAD": "",
            "EVENT_HEAD": self.merge,
            "REQUEST_MODE": "",
            "LAST_SUCCESS_HEAD": "",
            "REF": "refs/pull/72/merge",
        })

    def git(self, *args: str) -> str:
        return subprocess.check_output(
            ["git", *args], cwd=self.root, env=self.environment,
            stderr=subprocess.DEVNULL, text=True,
        ).strip()

    @staticmethod
    def workflow_run(marker: str) -> str:
        lines = (ROOT / ".github/workflows/check.yml").read_text().splitlines()
        step = lines.index(marker)
        run = next(index for index in range(step, len(lines)) if lines[index] == "        run: |")
        commands = []
        for line in lines[run + 1:]:
            if line and not line.startswith("          "):
                break
            commands.append(line[10:])
        return "\n".join(commands)

    def run_step(self, marker: str, environment: dict[str, str]) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", "-e", "-o", "pipefail", "-c", self.workflow_run(marker)],
            cwd=self.root, env={**self.environment, **environment},
            capture_output=True, text=True,
        )

    def run_plan(self, base: str | None = None) -> subprocess.CompletedProcess[str]:
        response = json.dumps(self._pr_response(base_sha=self.base_b))
        context = self.run_step("      - id: pr-context", {"PR_RESPONSE": response})
        if context.returncode != 0:
            return context
        context_outputs = dict(line.split("=", 1) for line in self.output.read_text().splitlines())
        return self.run_step("      - id: plan", {
            "PR_RESPONSE": response,
            "PR_BASE": base or context_outputs["base"],
            "PR_HEAD": context_outputs["head"],
            "PR_BASE_REF": context_outputs["base_ref"],
            "PR_DRAFT": context_outputs["draft"],
        })

    def test_admission_uses_event_merge_first_parent_when_live_base_advanced(self) -> None:
        result = self.run_plan()
        self.assertEqual(result.returncode, 0, result.stderr)
        outputs = dict(line.split("=", 1) for line in self.output.read_text().splitlines())
        self.assertEqual(outputs["base"], self.base_a)
        self.assertEqual(json.loads(outputs["checks"]), ["core"])

    def test_admission_refuses_merge_with_wrong_head_parent(self) -> None:
        wrong_merge = self.git(
            "commit-tree", self.git("rev-parse", f"{self.merge}^{{tree}}"),
            "-p", self.base_a, "-p", self.base_b, "-m", "wrong head parent",
        )
        self.git("checkout", "--detach", wrong_merge)
        result = self.run_plan(self.base_a)
        self.assertNotEqual(result.returncode, 0)

    def _pr_response(self, **changes: object) -> dict[str, object]:
        head: dict[str, object] = {"sha": self.head, "ref": "feature", "repo": {"full_name": "fork/source"}}
        base: dict[str, object] = {"sha": self.base_a, "ref": "main", "repo": {"full_name": "owner/repo"}}
        pull: dict[str, object] = {"state": "open", "draft": False, "head": head, "base": base}
        for field, value in changes.items():
            if field == "head_sha":
                head["sha"] = value
            elif field == "head_ref":
                head["ref"] = value
            elif field == "head_repo":
                head["repo"] = {"full_name": value}
            elif field == "base_sha":
                base["sha"] = value
            elif field == "base_ref":
                base["ref"] = value
            elif field == "base_repo":
                base["repo"] = {"full_name": value}
            else:
                pull[field] = value
        return pull

    def _run_current_and_decision(self, response: dict[str, object]) -> subprocess.CompletedProcess[str]:
        env = {"PR_RESPONSE": json.dumps(response)}
        current = self.run_step("      - id: current", env)
        if current.returncode != 0:
            return current
        current_outputs = dict(line.split("=", 1) for line in self.output.read_text().splitlines())
        return self.run_step("      - name: Require all selected checks for the current candidate", {
            **env,
            "PLAN_TIER": "full",
            "PLAN_CHECKS": '["docs"]',
            "PLAN_HEAD": self.head,
            "PLAN_BASE_REF": "main",
            "CURRENT_STATE": response["state"],
            "CURRENT_HEAD_REF": response["head"]["ref"],
            "CURRENT_HEAD_REPO": response["head"]["repo"]["full_name"],
            "CURRENT_BASE_REPO": response["base"]["repo"]["full_name"],
            "PLAN_DRAFT": "false",
            "CURRENT_BASE_REF": current_outputs.get("base_ref", ""),
            "CURRENT_HEAD": current_outputs.get("head", ""),
            "CURRENT_DRAFT": current_outputs.get("draft", ""),
            "PLAN_RESULT": "success",
            "HEAVY_RESULT": "success",
            "PLAN_MODE": "ready",
        })

    def test_final_live_read_refuses_closed_draft_and_changed_pr_identity(self) -> None:
        accepted = self._run_current_and_decision(self._pr_response())
        self.assertEqual(accepted.returncode, 0, accepted.stderr)
        refusals = {
            "closed": {"state": "closed"},
            "draft": {"draft": True},
            "base ref": {"base_ref": "release"},
            "base repository": {"base_repo": "other/repo"},
            "head ref": {"head_ref": "other-branch"},
            "head repository": {"head_repo": "other/source"},
            "head SHA": {"head_sha": "f" * 40},
        }
        for field, change in refusals.items():
            with self.subTest(field=field):
                result = self._run_current_and_decision(self._pr_response(**change))
                self.assertNotEqual(result.returncode, 0)

    def test_final_live_read_accepts_base_tip_movement_alone(self) -> None:
        accepted = self._run_current_and_decision(self._pr_response(base_sha=self.base_b))
        self.assertEqual(accepted.returncode, 0, accepted.stderr)


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
