#!/usr/bin/env python3
"""Exercise routed local admission using isolated Git repositories."""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class LocalGateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for relative in ("scripts/ci-plan.py", "scripts/check-changes", "scripts/check-tier", "tests/ci_plan_test.py"):
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / relative, target)
        self.git("init", "-b", "main")
        self.git("config", "user.email", "gate@example.invalid")
        self.git("config", "user.name", "Gate test")
        self.git("config", "core.hooksPath", str(self.root / "empty-hooks"))
        (self.root / "README.md").write_text("baseline\n")
        self.git("add", ".")
        self.git("commit", "-m", "baseline")
        self.git("remote", "add", "origin", str(self.root))
        self.git("checkout", "-b", "candidate")

    def git(self, *args: str) -> str:
        return subprocess.check_output(["git", *args], cwd=self.root, env=self.environment, stderr=subprocess.DEVNULL, text=True).strip()

    def gate(self, *args: str, incoming: str = "") -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", "scripts/check-changes", *args], cwd=self.root,
            input=incoming, capture_output=True, text=True, env=self.environment,
        )

    def test_dirty_checkout_refuses_before_checks(self) -> None:
        (self.root / "README.md").write_text("uncommitted\n")
        result = self.gate()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("clean checkout", result.stderr)

    def test_noncurrent_outgoing_tip_is_not_silently_ignored(self) -> None:
        result = self.gate("pre-push", incoming=f"refs/heads/other {'f' * 40} refs/heads/other {'0' * 40}\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("outgoing tip", result.stderr)

    def test_no_bypass_argument_is_admitted(self) -> None:
        result = self.gate("--no-verify")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("usage:", result.stderr)

    def test_documentation_route_still_rejects_whitespace_defects(self) -> None:
        (self.root / "README.md").write_text("bad whitespace \n")
        self.git("add", "README.md")
        self.git("commit", "-m", "docs")
        result = self.gate()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("trailing whitespace", result.stdout)

    def test_selected_failure_stops_full_source_and_prevents_success(self) -> None:
        runner = self.root / "scripts/check-tier"
        runner.write_text("#!/bin/sh\ncase \"$1\" in\ncore) exit 9;;\nfuzz) touch unexpected-fuzz-effect;;\nesac\n")
        self.git("add", "scripts/check-tier")
        self.git("commit", "-m", "failing core fixture")
        result = self.gate("full")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("exit status 9", result.stderr)
        self.assertFalse((self.root / "unexpected-fuzz-effect").exists())


if __name__ == "__main__":
    unittest.main()
