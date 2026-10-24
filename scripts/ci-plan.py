#!/usr/bin/env python3
"""Plan the repository-owned DAML CI checks for an explicit changed-path set."""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Mapping, Sequence
from pathlib import PurePosixPath

CHECKS = ("docs", "core", "dependencies", "fuzz", "editors", "binaries")
DOCS = {
    "README.md",
    "PLAN.md",
    "docs/ci.md",
    "docs/distribution.md",
    "docs/edge-cases.md",
    "docs/formatter-reuse.md",
    "docs/formatting.md",
    "docs/hardening-baseline.md",
    "docs/hardening.md",
    "docs/json-schema-2020-12-audit.md",
    "docs/language-walkthrough.md",
    "docs/lsp.md",
    "docs/native-authoring.md",
    "docs/parser-reuse.md",
    "docs/python.md",
    "docs/schemas.md",
    "docs/syntax.md",
    "integrations/README.md",
    "integrations/dsh/README.md",
    "integrations/omp/README.md",
    "integrations/vscode/README.md",
    "integrations/verify/README.md",
    "bindings/python/README.md",
}


def plan(mode: str, paths: Sequence[str]) -> dict[str, object]:
    """Return the tier and stable check identities for repo-relative paths."""
    if mode not in {"draft", "ready", "push", "manual-full"}:
        raise ValueError(f"unknown mode: {mode}")
    if not paths:
        raise ValueError("changed paths must not be empty")
    for path in paths:
        if not path or "\\" in path or "\x00" in path:
            raise ValueError(f"malformed changed path: {path!r}")
        parsed = PurePosixPath(path)
        if parsed.is_absolute() or ".." in parsed.parts or path in {".", ".."}:
            raise ValueError(f"changed path is not repository-relative: {path!r}")

    if mode == "draft":
        return {"tier": "draft", "checks": []}
    if mode == "manual-full":
        return {"tier": "full", "checks": list(CHECKS)}
    if any(path in {"*", "**"} for path in paths):
        return {"tier": "full", "checks": list(CHECKS)}

    changed = set(paths)
    if changed <= DOCS:
        return {"tier": "docs", "checks": ["docs"]}

    if changed <= DOCS | {"SOURCES.md"} and "SOURCES.md" in changed:
        checks = ["docs"] if changed & DOCS else []
        checks.append("binaries")
        return {"tier": "targeted", "checks": checks}
    selected = {"core"}
    for path in changed:
        if path.startswith(".github/workflows/") or path.startswith(
            ("scripts/check", "scripts/ci-plan")
        ):
            return {"tier": "full", "checks": list(CHECKS)}
        if (
            path in {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "deny.toml"}
            or path.startswith(("bindings/python/", "fuzz/Cargo."))
            or path == ".cargo/config.toml"
        ):
            selected.update(("dependencies", "fuzz", "binaries", "editors"))
        if (
            path.startswith(("fuzz/", "tests/fixtures/", "src/"))
            or path in {"Cargo.toml", "Cargo.lock", "deny.toml"}
        ):
            selected.add("fuzz")
        if path.startswith(("integrations/", "src/")):
            selected.add("editors")
        if path.startswith("src/") or path in {
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
        }:
            selected.add("binaries")

        if path.startswith(("src/", "tests/", "examples/", "integrations/", "fuzz/")):
            continue
        if path in DOCS or path.startswith("bindings/python/"):
            continue
        if path in {
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "deny.toml",
            ".cargo/config.toml",
        }:
            continue
        # Unknown files, executable documentation, workflows, and new trees fail closed.
        return {"tier": "full", "checks": list(CHECKS)}

    return {
        "tier": "targeted",
        "checks": [check for check in CHECKS if check in selected],
    }


def required_result(
    selected: Sequence[str],
    results: Mapping[str, str],
    planned_head: str,
    planned_base: str,
    current_head: str,
    current_base: str,
) -> bool:
    """Return whether selected checks passed for the still-current candidate/base."""
    if (
        not selected
        or not planned_head
        or not planned_base
        or planned_head != current_head
        or planned_base != current_base
        or len(set(selected)) != len(selected)
    ):
        return False
    return all(check in CHECKS and results.get(check) == "success" for check in selected)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--mode", required=True, choices=("draft", "ready", "push", "manual-full")
    )
    parser.add_argument(
        "--paths-file", required=True, help="newline-separated repository-relative changed paths"
    )
    args = parser.parse_args()
    try:
        with open(args.paths_file, encoding="utf-8") as stream:
            paths = [line.rstrip("\n") for line in stream]
        result = plan(args.mode, paths)
    except (OSError, UnicodeError, ValueError) as error:
        print(f"ci-plan: {error}", file=sys.stderr)
        return 2
    print(json.dumps(result, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
