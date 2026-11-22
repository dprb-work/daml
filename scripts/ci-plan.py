#!/usr/bin/env python3
"""Plan the repository-owned DAML CI checks for an explicit changed-path set."""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Mapping, Sequence
from pathlib import PurePosixPath

CHECKS = ("docs", "core", "dependencies", "fuzz", "editors", "binaries")
MODES = ("draft", "ready", "local", "push", "manual-full", "nightly")
# Release-only platform policy: see the header of .github/workflows/prepare-release.yml.
# Core already compiles native executables on Linux; do not duplicate packaging on PRs.
AUTOMATIC_CHECKS = tuple(check for check in CHECKS if check != "binaries")
# Hosted ready-PR runs keep only these; the local push gate and the nightly run the rest.
CHEAP_CHECKS = ("docs", "core")
BROAD_PATHS = {"*", "**", "."}
DOCS = {
    "README.md",
    "CONTRIBUTING.md",
    "docs/distribution.md",
    "docs/formatting.md",
    "docs/lsp.md",
    "docs/native-authoring.md",
    "docs/python.md",
    "docs/schemas.md",
    "docs/syntax.md",
    "integrations/README.md",
    "integrations/vscode/README.md",
    "integrations/verify/README.md",
}


def malformed(path: str) -> bool:
    """Return whether a changed path is not a plain repository-relative path."""
    parsed = PurePosixPath(path)
    return (
        not path
        or any(char in path for char in ("\\", "\x00", "\n", "\r"))
        or parsed.is_absolute()
        or ".." in parsed.parts
    )


def plan(
    mode: str, paths: Sequence[str], head: str = "", last_success_head: str = ""
) -> dict[str, object]:
    """Return the tier and stable check identities for repo-relative paths."""
    if mode not in MODES:
        raise ValueError(f"unknown mode: {mode}")
    invalid = [path for path in paths if malformed(path)]
    # The local push gate cannot classify these, so it runs every source group.
    if mode == "local" and (not paths or invalid):
        return {"tier": "full", "checks": list(AUTOMATIC_CHECKS)}
    if not paths:
        raise ValueError("changed paths must not be empty")
    if invalid:
        raise ValueError(f"malformed changed path: {invalid[0]!r}")

    if mode == "draft":
        return {"tier": "draft", "checks": []}
    if mode == "nightly":
        if head and head == last_success_head:
            return {"tier": "unchanged", "checks": []}
        return {"tier": "full", "checks": list(AUTOMATIC_CHECKS)}
    if mode == "manual-full":
        return {"tier": "full", "checks": list(CHECKS)}
    if mode == "push":
        return {"tier": "push", "checks": ["docs"]}
    routed = route(paths)
    if mode == "local" or routed["tier"] == "docs":
        return routed
    return {
        "tier": "cheap",
        "checks": [check for check in routed["checks"] if check in CHEAP_CHECKS],
    }


def route(paths: Sequence[str]) -> dict[str, list[str] | str]:
    """Return the local push gate's checks for valid, non-empty changed paths."""
    if any(path in BROAD_PATHS for path in paths):
        return {"tier": "full", "checks": list(AUTOMATIC_CHECKS)}

    changed = set(paths)
    docs = DOCS | {
        path for path in changed
        if (path.startswith("docs/") and path.endswith(".md"))
        or (path.startswith(("openspec/changes/", "openspec/specs/"))
            and (path.endswith(".md") or PurePosixPath(path).name == ".openspec.yaml"))
    }
    if changed <= docs:
        return {"tier": "docs", "checks": ["docs"]}

    if changed <= docs | {"SOURCES.md"} and "SOURCES.md" in changed:
        checks = ["docs"] if changed & docs else []
        checks.append("core")
        return {"tier": "cheap", "checks": checks}
    selected = {"core"}
    for path in changed:
        if (
            path.startswith((".github/", ".cargo/", ".githooks/", "scripts/"))
            or path == "tests/ci_plan_test.py"
            or PurePosixPath(path).name in {
                "Cargo.toml", "Cargo.lock", "deny.toml", "package.json",
                "package-lock.json", "pyproject.toml", "uv.lock", "uv.toml",
                "rust-toolchain.toml", "rust-toolchain", ".python-version", ".npmrc",
            }
            or path.startswith("fuzz/")
        ):
            return {"tier": "full", "checks": list(AUTOMATIC_CHECKS)}
        if path.startswith(
            ("integrations/", "tests/fixtures/semantic-editor/")
        ):
            selected.add("editors")

        if path.startswith(("src/", "tests/", "examples/", "integrations/", "fuzz/")):
            continue
        if path in docs or path.startswith(
            ("bindings/python/src/", "bindings/python/python/", "bindings/python/tests/")
        ):
            continue
        # Unknown files, executable documentation, workflows, and new trees fail closed.
        return {"tier": "full", "checks": list(AUTOMATIC_CHECKS)}

    return {
        "tier": "cheap",
        "checks": [check for check in CHECKS if check in selected],
    }


def required_result(
    selected: Sequence[str],
    results: Mapping[str, str],
    planned_head: str,
    current_head: str,
) -> bool:
    """Return whether selected checks passed for the still-current candidate."""
    if (
        not selected
        or not planned_head
        or planned_head != current_head
        or len(set(selected)) != len(selected)
    ):
        return False
    return all(check in CHECKS and results.get(check) == "success" for check in selected)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", required=True, choices=MODES)
    parser.add_argument(
        "--paths-file", required=True, help="newline-separated repository-relative changed paths"
    )
    parser.add_argument("--head", default="")
    parser.add_argument("--last-success-head", default="")
    args = parser.parse_args()
    try:
        with open(args.paths_file, encoding="utf-8", newline="") as stream:
            paths = [line.rstrip("\n") for line in stream]
        result = plan(args.mode, paths, args.head, args.last_success_head)
    except (OSError, UnicodeError, ValueError) as error:
        print(f"ci-plan: {error}", file=sys.stderr)
        return 2
    print(json.dumps(result, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
