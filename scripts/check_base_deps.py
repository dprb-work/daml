#!/usr/bin/env python3
"""Verify base-dependency and shared-conventions alignment across repositories.

Stdlib only. Reads one declared Node.js version owner (no parallel registry)
and checks .node-version, package manifests, GitHub Actions setup-node steps
(resolving node-version-file targets), Node Dockerfile FROM lines, and
optionally project-template's generated react-vite starter. With --template,
each repository must also carry a byte-identical copy of the template's
CONVENTIONS.md at docs/conventions.md.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import re
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

EXACT_SEMVER_RE = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
TAG_SEMVER_RE = re.compile(r"^(\d+\.\d+\.\d+)(?:-|$)")
DIGEST_RE = re.compile(r"sha256:[0-9a-f]{64}")
RANGE_OPERATOR_RE = re.compile(r"(?:>=|<=|>|<|\^|~|\|\||\*|\bx\b|\bX\b)")
COMPARATOR_RE = re.compile(r"^(>=|<=|>|<|\^|~|=)?v?(\d+)(?:\.(\d+))?(?:\.(\d+))?$")
FROM_RE = re.compile(r"^\s*FROM\s+(.+?)\s*$", re.IGNORECASE)
SETUP_NODE_RE = re.compile(
    r"^(?P<indent>[ \t]*)(?:-[ \t]+)?uses:[ \t]*['\"]?actions/setup-node@[^\s'\"#]+"
)
SKIP_DIRS = frozenset(
    (
        ".agent .git .mypy_cache .pytest_cache .ruff_cache .scratch .storybook .venv "
        + "__pycache__ build dist evals fixtures node_modules openspec "
        + "storybook-static target venv"
    ).split()
)
CONVENTIONS_SOURCE = "CONVENTIONS.md"
CONVENTIONS_COPY = "docs/conventions.md"
# Declared value of a finding whose file does not exist.
MISSING = "<missing>"


@dataclass(frozen=True)
class DriftFinding:
    """One mismatched base-dependency selector."""

    repo: str
    path: str
    declared: str
    expected: str

    def format_line(self) -> str:
        return (
            f"DRIFT repo={self.repo} path={self.path} "
            f"declared={self.declared} expected={self.expected}"
        )


def read_declared_node(repo: Path, source: str | None = None) -> str:
    """Read the exact X.Y.Z Node.js version from one declared repository source."""
    if source is not None:
        rel = source
    elif (repo / ".node-version").is_file():
        rel = ".node-version"
    elif (repo / "package.json").is_file():
        rel = "package.json"
    else:
        raise ValueError(
            f"No declared Node.js source in {repo.name} (.node-version or package.json)"
        )
    target = repo / rel
    if not target.is_file():
        raise ValueError(f"Declared Node.js source missing in {repo.name}: {rel}")
    value = _read_version_target(target)
    if value is None or not EXACT_SEMVER_RE.fullmatch(value):
        raise ValueError(
            f"Declared Node.js source {repo.name}/{rel} must pin X.Y.Z; got {value!r}"
        )
    return value


def semver_satisfies(version: str, range_expr: str) -> bool:
    """Return whether an exact X.Y.Z version satisfies a Node semver range."""
    match = EXACT_SEMVER_RE.fullmatch(version)
    if not match:
        return False
    target = (int(match.group(1)), int(match.group(2)), int(match.group(3)))
    alts = [part.strip() for part in range_expr.split("||")]
    return bool(alts) and all(alts) and any(_matches_set(target, a) for a in alts)


def read_conventions(template: Path) -> bytes:
    """Read the template's generated conventions file that every consumer copies."""
    path = template / CONVENTIONS_SOURCE
    if not path.is_file():
        raise ValueError(f"Template {template.name} has no {CONVENTIONS_SOURCE}")
    return path.read_bytes()


def check_repository(
    repo: Path,
    *,
    expected_node: str,
    expected_node_image: str | None = None,
    source: str | None = None,
    excludes: tuple[str, ...] = (),
    repo_label: str | None = None,
    conventions: bytes | None = None,
) -> list[DriftFinding]:
    """Check one repository's Node.js and Node-image selectors.

    With ``conventions``, also require ``docs/conventions.md`` to hold exactly
    those bytes.
    """
    resolved = repo.resolve(strict=True)
    label = repo_label or resolved.name
    findings: list[DriftFinding] = []
    expected_digest = _extract_digest(expected_node_image)

    if source is not None:
        src_path = resolved / source
        declared = _read_version_target(src_path) if src_path.is_file() else None
        if declared != expected_node:
            findings.append(
                DriftFinding(label, source, declared or MISSING, expected_node)
            )

    node_ver = resolved / ".node-version"
    if (
        node_ver.is_file()
        and source != ".node-version"
        and not _is_excluded(".node-version", excludes)
    ):
        declared = _read_version_target(node_ver) or "<invalid>"
        if declared != expected_node:
            findings.append(
                DriftFinding(label, ".node-version", declared, expected_node)
            )

    for rel, file_path in _walk_files(resolved, excludes):
        name = file_path.name
        if name == "package.json":
            _check_manifest(label, rel, file_path, expected_node, findings)
        elif _is_workflow(rel, name):
            _check_workflow(label, resolved, rel, file_path, expected_node, findings)
        elif _is_dockerfile(name):
            _check_dockerfile(
                label, rel, file_path, expected_node, expected_digest, findings
            )
    if conventions is not None:
        _check_conventions_copy(label, resolved, conventions, findings)
    return findings


def check_template_generator(
    template_root: Path, expected_node: str
) -> list[DriftFinding]:
    """Materialize the react-vite starter; check its Node pins and conventions copy."""
    creator = template_root / "scripts" / "create_project.py"
    if not creator.is_file():
        return [
            DriftFinding(
                template_root.name,
                "scripts/create_project.py",
                MISSING,
                expected_node,
            )
        ]
    spec = importlib.util.spec_from_file_location("_template_create_project", creator)
    if spec is None or spec.loader is None:
        return [
            DriftFinding(
                template_root.name,
                "scripts/create_project.py",
                "<unloadable>",
                expected_node,
            )
        ]
    scripts_dir = str(creator.parent)
    added = scripts_dir not in sys.path
    if added:
        sys.path.insert(0, scripts_dir)
    try:
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        with tempfile.TemporaryDirectory(prefix="base-deps-starter-") as tmp:
            generated = mod.create_project(
                "react-vite", Path(tmp) / "drift-check", "drift-check"
            )
            return check_repository(
                generated,
                expected_node=expected_node,
                source="package.json",
                repo_label=f"{template_root.name}:generated/react-vite",
                conventions=(template_root / CONVENTIONS_SOURCE).read_bytes(),
            )
    finally:
        if added and scripts_dir in sys.path:
            sys.path.remove(scripts_dir)


def main(argv: list[str] | None = None) -> int:
    """CLI entry point for single-repo or cross-repo base-dependency drift checks."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("repos", nargs="*", type=Path, default=[Path(".")])
    parser.add_argument("--template", type=Path)
    parser.add_argument("--expected-node")
    parser.add_argument("--expected-node-image")
    parser.add_argument("--source")
    parser.add_argument("--exclude", action="append", default=[])
    parser.add_argument("--check-template", action="store_true")
    args = parser.parse_args(argv)
    repos: list[Path] = list(args.repos)
    template: Path | None = args.template
    expected_arg: str | None = args.expected_node
    expected_image: str | None = args.expected_node_image
    source: str | None = args.source
    excludes = tuple(item.strip("/") for item in list(args.exclude) if item.strip("/"))
    check_template: bool = bool(args.check_template)

    try:
        if expected_arg is not None:
            if not EXACT_SEMVER_RE.fullmatch(expected_arg):
                raise ValueError(f"--expected-node must be X.Y.Z; got {expected_arg!r}")
            expected_node = expected_arg
        elif template is not None:
            expected_node = read_declared_node(template.resolve(strict=True))
        elif len(repos) == 1:
            expected_node = read_declared_node(repos[0].resolve(strict=True), source)
        else:
            raise ValueError("Provide --template or --expected-node for multiple repos")
        _extract_digest(expected_image)
        conventions = (
            read_conventions(template.resolve(strict=True))
            if template is not None
            else None
        )
    except (OSError, ValueError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1

    findings: list[DriftFinding] = []
    for repo in repos:
        findings.extend(
            check_repository(
                repo,
                expected_node=expected_node,
                expected_node_image=expected_image,
                source=source,
                excludes=excludes,
                conventions=conventions,
            )
        )
    if check_template:
        tpl_root = (template or repos[0]).resolve(strict=True)
        findings.extend(check_template_generator(tpl_root, expected_node))

    if findings:
        for finding in findings:
            print(finding.format_line(), file=sys.stderr)
        return 1
    checked = ", ".join(repo.resolve().name for repo in repos)
    print(f"Base dependency check passed: Node {expected_node} ({checked})")
    return 0


def _read_version_target(path: Path) -> str | None:
    if path.suffix == ".json":
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError):
            return None
        engines = payload.get("engines") if isinstance(payload, dict) else None
        node_val = engines.get("node") if isinstance(engines, dict) else None
        return node_val.strip() if isinstance(node_val, str) else None
    try:
        lines = [
            ln.strip()
            for ln in path.read_text(encoding="utf-8").splitlines()
            if ln.strip() and not ln.strip().startswith("#")
        ]
    except (OSError, UnicodeDecodeError):
        return None
    return lines[0] if len(lines) == 1 else None


def _check_conventions_copy(
    label: str, repo: Path, conventions: bytes, findings: list[DriftFinding]
) -> None:
    copy = repo / CONVENTIONS_COPY
    expected = _sha256_ref(conventions)
    if not copy.is_file():
        findings.append(DriftFinding(label, CONVENTIONS_COPY, MISSING, expected))
    elif (declared := copy.read_bytes()) != conventions:
        findings.append(
            DriftFinding(label, CONVENTIONS_COPY, _sha256_ref(declared), expected)
        )


def _sha256_ref(content: bytes) -> str:
    return f"sha256:{hashlib.sha256(content).hexdigest()}"


def _matches_set(target: tuple[int, int, int], expr: str) -> bool:
    tokens = expr.split()
    if not tokens:
        return False
    for tok in tokens:
        if tok in {"*", "x", "X"}:
            continue
        m = COMPARATOR_RE.fullmatch(tok)
        if not m:
            return False
        op = m.group(1) or "="
        maj = int(m.group(2))
        mn = int(m.group(3)) if m.group(3) is not None else None
        pt = int(m.group(4)) if m.group(4) is not None else None
        low = (maj, mn or 0, pt or 0)
        if op == ">=" and target < low:
            return False
        if op == ">" and target <= low:
            return False
        if op == "<=":
            hi = (
                (maj, mn, pt)
                if mn is not None and pt is not None
                else (maj, mn, sys.maxsize)
                if mn is not None
                else (maj, sys.maxsize, sys.maxsize)
            )
            if target > hi:
                return False
        if op == "<" and target >= low:
            return False
        if op == "^":
            hi = (
                (maj + 1, 0, 0)
                if maj > 0
                else (0, mn + 1, 0)
                if mn is not None and mn > 0
                else (0, 0, (pt or 0) + 1)
            )
            if not (low <= target < hi):
                return False
        if op == "~":
            hi = (maj, mn + 1, 0) if mn is not None else (maj + 1, 0, 0)
            if not (low <= target < hi):
                return False
        if op == "=":
            if (
                (mn is None and target[0] != maj)
                or (mn is not None and pt is None and target[:2] != (maj, mn))
                or (pt is not None and target != (maj, mn, pt))
            ):
                return False
    return True


def _extract_digest(expected_image: str | None) -> str | None:
    if expected_image is None:
        return None
    match = DIGEST_RE.search(expected_image)
    if not match:
        raise ValueError(
            f"--expected-node-image needs a sha256 digest; got {expected_image!r}"
        )
    return match.group(0)


def _is_excluded(rel: str, excludes: tuple[str, ...]) -> bool:
    return any(rel == p or rel.startswith(f"{p}/") for p in excludes)


def _walk_files(repo: Path, excludes: tuple[str, ...]) -> list[tuple[str, Path]]:
    found: list[tuple[str, Path]] = []
    for root, dirs, files in os.walk(repo):
        rp = Path(root)
        rdir = rp.relative_to(repo).as_posix()
        dirs[:] = [
            d
            for d in sorted(dirs)
            if d not in SKIP_DIRS
            and not _is_excluded(d if rdir == "." else f"{rdir}/{d}", excludes)
        ]
        for fname in sorted(files):
            rfile = fname if rdir == "." else f"{rdir}/{fname}"
            if not _is_excluded(rfile, excludes):
                found.append((rfile, rp / fname))
    return found


def _is_workflow(rel: str, name: str) -> bool:
    return name == "ci-steps.yml" or (
        rel.startswith(".github/workflows/") and name.endswith((".yml", ".yaml"))
    )


def _is_dockerfile(name: str) -> bool:
    return (
        name == "Dockerfile"
        or name.startswith("Dockerfile.")
        or name.endswith(".Dockerfile")
    )


def _check_manifest(
    label: str, rel: str, path: Path, expected: str, findings: list[DriftFinding]
) -> None:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        return
    engines = payload.get("engines") if isinstance(payload, dict) else None
    raw = engines.get("node") if isinstance(engines, dict) else None
    if not isinstance(raw, str) or not raw.strip():
        return
    val = raw.strip()
    if RANGE_OPERATOR_RE.search(val):
        if not semver_satisfies(expected, val):
            findings.append(DriftFinding(label, rel, val, expected))
    elif val.removeprefix("v") != expected:
        findings.append(DriftFinding(label, rel, val, expected))


def _check_workflow(
    label: str,
    repo: Path,
    rel: str,
    path: Path,
    expected: str,
    findings: list[DriftFinding],
) -> None:
    lines = path.read_text(encoding="utf-8").splitlines()
    owner = (
        path.parent
        if rel.startswith("templates/") and path.name == "ci-steps.yml"
        else repo
    )
    idx = 0
    while idx < len(lines):
        m = SETUP_NODE_RE.match(lines[idx])
        if not m:
            idx += 1
            continue
        step_ref = f"{rel}:{idx + 1}"
        base = len(m.group("indent"))
        # A list item "- uses:" aligns sibling keys at indent + 2.
        item = base + 2 if lines[idx].lstrip(" \t").startswith("-") else base
        with_map, idx = _read_with(lines, idx + 1, base, item)
        vfile = with_map.get("node-version-file")
        vlit = with_map.get("node-version")
        if vfile is not None:
            target = (owner / vfile).resolve()
            if not target.is_file():
                findings.append(
                    DriftFinding(
                        label,
                        step_ref,
                        f"node-version-file:{vfile} (<missing>)",
                        expected,
                    )
                )
            else:
                eff = _read_version_target(target) or "<invalid>"
                if eff != expected:
                    findings.append(
                        DriftFinding(
                            label,
                            step_ref,
                            f"node-version-file:{vfile}->{eff}",
                            expected,
                        )
                    )
        elif vlit is not None:
            if vlit.removeprefix("v") != expected:
                findings.append(DriftFinding(label, step_ref, vlit, expected))
        else:
            findings.append(
                DriftFinding(label, step_ref, "<missing node-version>", expected)
            )


def _read_with(
    lines: list[str], start: int, base: int, item: int
) -> tuple[dict[str, str], int]:
    values: dict[str, str] = {}
    i = start
    while i < len(lines):
        raw = lines[i]
        s = raw.strip()
        if not s or s.startswith("#"):
            i += 1
            continue
        ind = len(raw) - len(raw.lstrip(" \t"))
        if ind < item or (ind == base and s.startswith("-")):
            break
        if s.startswith("with:"):
            inline = s[5:].strip()
            if inline.startswith("{"):
                body = inline[1 : inline.find("}")] if "}" in inline else inline[1:]
                for entry in body.split(","):
                    if ":" in entry:
                        k, v = entry.split(":", 1)
                        values[k.strip()] = _clean_scalar(v)
                i += 1
            else:
                w_ind = ind
                i += 1
                while i < len(lines):
                    sub = lines[i]
                    ss = sub.strip()
                    if not ss or ss.startswith("#"):
                        i += 1
                        continue
                    if len(sub) - len(sub.lstrip(" \t")) <= w_ind:
                        break
                    if ":" in ss:
                        k, v = ss.split(":", 1)
                        values[k.strip()] = _clean_scalar(v)
                    i += 1
            continue
        i += 1
    return values, i


def _clean_scalar(raw: str) -> str:
    val = raw.strip()
    if val and val[0] in {"'", '"'}:
        end = val.find(val[0], 1)
        if end > 0:
            return val[1:end].strip()
    return val.split(" #", 1)[0].strip().strip("'\"")


def _check_dockerfile(
    label: str,
    rel: str,
    path: Path,
    expected: str,
    digest: str | None,
    findings: list[DriftFinding],
) -> None:
    for line_no, raw in enumerate(
        path.read_text(encoding="utf-8").splitlines(), start=1
    ):
        s = raw.strip()
        m = FROM_RE.match(s) if s and not s.startswith("#") else None
        if not m:
            continue
        tok = next((t for t in m.group(1).split() if not t.startswith("--")), None)
        if tok is None or tok.startswith("$"):
            continue
        img, _, dpart = tok.partition("@")
        # Separate registry port from tag by looking after the last slash.
        colon = img.find(":", img.rfind("/") + 1)
        repo_name, tag = (img[:colon], img[colon + 1 :]) if colon >= 0 else (img, None)
        if repo_name != "node" and not repo_name.endswith("/node"):
            continue
        ref = f"{rel}:{line_no}"
        if tag is not None:
            tm = TAG_SEMVER_RE.match(tag)
            if (tm.group(1) if tm else tag) != expected:
                findings.append(DriftFinding(label, ref, tok, expected))
        if digest is not None and dpart != digest:
            findings.append(
                DriftFinding(label, ref, dpart or "<unpinned-digest>", digest)
            )


if __name__ == "__main__":
    raise SystemExit(main())
