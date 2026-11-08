#!/usr/bin/env python3
"""Release-only security preparation, isolated scanning and evidence admission.

Stdlib only. No scanner is installed on the host. Raw scanner output is never
printed or retained: in particular secret matches cannot enter release evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib
import uuid
from datetime import UTC, datetime
from pathlib import Path, PurePosixPath

CHECK_TOOLS = {
    "dependencies": "trivy",
    "secrets": "trivy",
    "sast": "semgrep",
    "malware": "clamav",
}
HEX = re.compile(r"[0-9a-f]{64}\Z")
SUMMARY_LIMIT = 100
ROOT = Path(__file__).resolve().parent.parent


def _require(condition: object, message: str) -> None:
    if not condition:
        raise ValueError(message)


def _json(data: bytes | str) -> dict:
    def pairs(items: list[tuple[str, object]]) -> dict:
        result = {}
        for key, value in items:
            _require(key not in result, "duplicate JSON field")
            result[key] = value
        return result

    value = json.loads(data, object_pairs_hook=pairs)
    _require(isinstance(value, dict), "expected JSON object")
    return value


def _digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def _now() -> str:
    return datetime.now(UTC).isoformat().replace("+00:00", "Z")


def _timestamp(value: object) -> datetime:
    _require(isinstance(value, str), "missing UTC timestamp")
    try:
        result = datetime.fromisoformat(value)
    except ValueError as error:
        raise ValueError("invalid UTC timestamp") from error
    _require(
        result.utcoffset() is not None and result.utcoffset().total_seconds() == 0,
        "timestamp must be UTC",
    )
    return result


def _fresh(value: object, hours: int, *, at: datetime | None = None) -> None:
    age = ((at or datetime.now(UTC)) - _timestamp(value)).total_seconds()
    _require(0 <= age <= hours * 3600, "stale or future security timestamp")


def _relative(value: object) -> str:
    _require(
        isinstance(value, str)
        and value
        and "\\" not in value
        and not any(ord(character) < 32 for character in value),
        "unsafe relative path",
    )
    path = PurePosixPath(value)
    _require(
        not path.is_absolute()
        and all(part not in ("", ".", "..") for part in value.split("/")),
        "unsafe relative path",
    )
    return value


def _regular(root: Path, name: str) -> Path:
    _require(
        not root.is_symlink()
        and not any(parent.is_symlink() for parent in root.parents),
        "symlink in security input root",
    )
    path = root
    for part in _relative(name).split("/"):
        path = path / part
        _require(not path.is_symlink(), "symlink in security input")
    _require(
        path.is_file() and stat.S_ISREG(path.stat().st_mode),
        "missing regular security input",
    )
    _require(
        path.resolve().is_relative_to(root.resolve()), "security input escapes root"
    )
    return path


def _assets(assets: object) -> list[dict]:
    _require(isinstance(assets, list) and assets, "assets must be a nonempty list")
    names = set()
    for asset in assets:
        _require(
            isinstance(asset, dict)
            and set(asset) == {"filename", "sha256", "size", "kind", "target"},
            "malformed security asset",
        )
        name = _relative(asset["filename"])
        _require(
            "/" not in name and name not in names, "duplicate or nested asset filename"
        )
        names.add(name)
        _require(
            isinstance(asset["sha256"], str) and HEX.fullmatch(asset["sha256"]),
            "invalid asset hash",
        )
        _require(type(asset["size"]) is int and asset["size"] > 0, "invalid asset size")
        _require(
            all(
                isinstance(asset[key], str) and asset[key] for key in ("kind", "target")
            ),
            "invalid asset identity",
        )
    return sorted(
        (dict(asset) for asset in assets), key=lambda asset: asset["filename"]
    )


def _policy(root: Path) -> tuple[dict, str, str]:
    policy_path = _regular(root, "security/policy.json")
    rules_path = _regular(root, "security/semgrep.yml")
    policy = _json(policy_path.read_bytes())
    _require(
        type(policy.get("schema")) is int and policy["schema"] == 1,
        "unsupported security policy",
    )
    for key in ("max_evidence_age_hours", "max_database_age_hours"):
        _require(
            type(policy.get(key)) is int and 0 < policy[key] <= 24,
            "security maximum age exceeds 24 hours",
        )
    tools = policy.get("tools")
    _require(
        isinstance(tools, dict) and set(tools) == set(CHECK_TOOLS.values()),
        "security engine set differs",
    )
    for tool in tools.values():
        _require(
            isinstance(tool, dict) and set(tool) == {"version", "image"},
            "malformed tool policy",
        )
        _require(
            isinstance(tool["version"], str)
            and re.fullmatch(r"\d+\.\d+\.\d+", tool["version"]),
            "invalid tool version",
        )
        _require(
            isinstance(tool["image"], str)
            and re.fullmatch(r"[a-zA-Z0-9./:_-]+@sha256:[0-9a-f]{64}", tool["image"]),
            "tool image must be immutable",
        )
    for key in ("dependency_files", "sast_rule_ids", "sast_extensions"):
        value = policy.get(key)
        _require(
            isinstance(value, list)
            and value
            and all(isinstance(item, str) and item for item in value)
            and len(set(value)) == len(value),
            "invalid security coverage policy",
        )
    for name in policy["dependency_files"]:
        _relative(name)
    ids = re.findall(
        r"^  - id: ([a-z0-9-]+)$", rules_path.read_text(encoding="utf-8"), re.MULTILINE
    )
    _require(ids == policy["sast_rule_ids"], "local SAST rule set differs from policy")
    return policy, _digest(policy_path), _digest(rules_path)


def _database(check: dict, policy: dict, at: datetime) -> None:
    database = check.get("database")
    _require(isinstance(database, dict), "missing database proof")
    hours = policy["max_database_age_hours"]
    if check["id"] == "dependencies":
        _require(
            set(database) == {"updated_at", "downloaded_at", "sha256"},
            "malformed advisory database proof",
        )
        for key in ("updated_at", "downloaded_at"):
            _fresh(database[key], hours, at=at)
        _require(
            isinstance(database["sha256"], str) and HEX.fullmatch(database["sha256"]),
            "invalid advisory database identity",
        )
        return
    _require(
        set(database) == {"refreshed_at", "daily", "main", "bytecode"},
        "malformed signature database proof",
    )
    _fresh(database["refreshed_at"], hours, at=at)
    for name in ("daily", "main", "bytecode"):
        row = database[name]
        _require(
            isinstance(row, dict) and set(row) == {"version", "built_at", "sha256"},
            "malformed signature identity",
        )
        _require(
            type(row["version"]) is int
            and row["version"] > 0
            and isinstance(row["sha256"], str)
            and HEX.fullmatch(row["sha256"]),
            "invalid signature identity",
        )
        _require(_timestamp(row["built_at"]) <= at, "future signature database")
    # Main and bytecode change less frequently. Daily must itself be current;
    # merely running freshclam against an obsolete mirror cannot refresh proof.
    _fresh(database["daily"]["built_at"], hours, at=at)


def validate_evidence(
    root: Path, evidence: object, *, source_sha: str, assets: list[dict]
) -> None:
    """Admit only complete current proof for the exact policy, source and assets.

    Does not run git/Docker, write files, refresh databases or read a manifest.
    Publication trust is the approved enclosing manifest digest, not this JSON.
    """
    policy, policy_hash, rules_hash = _policy(Path(root))
    expected = _assets(assets)
    _require(
        isinstance(source_sha, str) and re.fullmatch(r"[0-9a-f]{40}", source_sha),
        "invalid security source SHA",
    )
    _require(
        isinstance(evidence, dict)
        and set(evidence)
        == {
            "schema",
            "source_sha",
            "assets",
            "policy_sha256",
            "rules_sha256",
            "scanned_at",
            "status",
            "checks",
        },
        "malformed security evidence",
    )
    _require(
        type(evidence["schema"]) is int and evidence["schema"] == 1,
        "unsupported security evidence",
    )
    _require(_assets(evidence["assets"]) == expected, "malformed evidence assets")
    _require(
        evidence["source_sha"] == source_sha and evidence["assets"] == expected,
        "security source or assets differ",
    )
    _require(
        evidence["policy_sha256"] == policy_hash
        and evidence["rules_sha256"] == rules_hash,
        "security policy or rules changed",
    )
    _fresh(evidence["scanned_at"], policy["max_evidence_age_hours"])
    at = _timestamp(evidence["scanned_at"])
    _require(evidence["status"] == "passed", "security suite is not passed")
    checks = evidence["checks"]
    _require(
        isinstance(checks, list) and len(checks) == len(CHECK_TOOLS),
        "incomplete security check set",
    )
    seen = set()
    for check in checks:
        _require(isinstance(check, dict), "malformed security check")
        required = {"id", "status", "tool", "findings", "coverage"}
        allowed = (
            required
            | {"summaries", "summaries_truncated"}
            | (
                {"database"}
                if check.get("id") in ("dependencies", "malware")
                else set()
            )
        )
        _require(required <= set(check) <= allowed, "malformed security check fields")
        identity = check["id"]
        _require(
            isinstance(identity, str)
            and identity in CHECK_TOOLS
            and identity not in seen,
            "duplicate or extra security check",
        )
        seen.add(identity)
        _require(
            check["status"] == "passed"
            and type(check["findings"]) is int
            and check["findings"] == 0,
            "security check not passed",
        )
        if "summaries" in check or "summaries_truncated" in check:
            _require(
                check.get("summaries") == []
                and check.get("summaries_truncated") is False,
                "passing proof contains finding summaries or truncation",
            )
        tool = check["tool"]
        name = CHECK_TOOLS[identity]
        _require(
            isinstance(tool, dict)
            and set(tool) == {"name", "version", "image", "image_id"},
            "missing tool identity",
        )
        _require(
            tool["name"] == name
            and tool["version"] == policy["tools"][name]["version"]
            and tool["image"] == policy["tools"][name]["image"],
            "scanner tool differs from policy",
        )
        _require(
            isinstance(tool["image_id"], str)
            and re.fullmatch(r"sha256:[0-9a-f]{64}", tool["image_id"]),
            "invalid actual image identity",
        )
        _require(
            isinstance(check["coverage"], dict)
            and check["coverage"].get("complete") is True,
            "missing or incomplete scanner coverage",
        )
        if identity in ("dependencies", "malware"):
            _database(check, policy, at)
            _database(check, policy, datetime.now(UTC))


def _command(
    args: list[str],
    *,
    cwd: Path | None = None,
    allowed: tuple[int, ...] = (0,),
    timeout: int = 1800,
) -> subprocess.CompletedProcess:
    # Capture both streams: scanner diagnostics may contain source/secret data.
    result = subprocess.run(
        args, cwd=cwd, capture_output=True, timeout=timeout, check=False
    )
    _require(result.returncode in allowed, "security command failed")
    return result


def _docker() -> list[str]:
    context = os.environ.get("SECURITY_DOCKER_CONTEXT") or os.environ.get(
        "DOCKER_CONTEXT"
    )
    if not context:
        context = _command(["docker", "context", "show"]).stdout.decode().strip()
    return ["docker", "--context", context]


def _container_user(docker: list[str]) -> str:
    """Use the host input owner without assuming root has bind-mount access."""
    options = json.loads(
        _command(docker + ["info", "--format", "{{json .SecurityOptions}}"]).stdout
    )
    _require(
        isinstance(options, list)
        and all(isinstance(option, str) for option in options),
        "invalid Docker security options",
    )
    if any(option.split(",", 1)[0] == "name=rootless" for option in options):
        return "0:0"
    return f"{os.geteuid()}:{os.getegid()}"


def _container(
    docker: list[str],
    image: str,
    entrypoint: str,
    args: list[str],
    mounts: list[tuple[Path, str, bool]],
    *,
    network: bool = False,
    timeout: int = 1800,
    user: str | None = None,
) -> subprocess.CompletedProcess:
    if user is None:
        user = _container_user(docker)
    name = "release-security-" + uuid.uuid4().hex
    command = docker + [
        "run",
        "--rm",
        "--pull=never",
        "--name",
        name,
        "--label",
        "org.release-security.owner=" + name,
        "--network=" + ("bridge" if network else "none"),
        "--cpus=2",
        "--memory=4g",
        "--pids-limit=256",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges",
        "--read-only",
        "--user=" + user,
        "--tmpfs",
        "/tmp:rw,nosuid,nodev,size=1g",
        "--env",
        "HOME=/tmp",
        "--env",
        "LC_ALL=C",
        "--env",
        "SEMGREP_SEND_METRICS=off",
        "--env",
        "SEMGREP_ENABLE_VERSION_CHECK=0",
        "--workdir",
        "/tmp",
    ]
    for source, target, readonly in mounts:
        _require("," not in str(source), "unsupported Docker bind path")
        command += [
            "--mount",
            f"type=bind,source={source.resolve()},target={target}"
            + (",readonly" if readonly else ""),
        ]
    command += ["--entrypoint", entrypoint, image] + args
    try:
        return _command(command, allowed=(0, 1, 2), timeout=timeout)
    except (subprocess.TimeoutExpired, KeyboardInterrupt) as error:
        # Only our randomly named container; Docker CLI cancellation alone need
        # not terminate its daemon-side workload.
        try:
            subprocess.run(
                docker + ["rm", "--force", name],
                capture_output=True,
                check=False,
                timeout=30,
            )
        except (OSError, subprocess.SubprocessError):
            error.add_note(
                "Task-owned container cleanup failed; "
                "inspect release-security ownership labels."
            )
        raise


def _success(result: subprocess.CompletedProcess) -> bytes:
    _require(result.returncode == 0, "security preparation failed")
    return result.stdout


def _clam_database(directory: Path, refreshed_at: str) -> dict:
    result = {"refreshed_at": refreshed_at}
    for name in ("daily", "main", "bytecode"):
        candidates = [
            path
            for suffix in ("cvd", "cld")
            if (path := directory / f"{name}.{suffix}").is_file()
        ]
        _require(
            len(candidates) == 1 and not candidates[0].is_symlink(),
            "missing or ambiguous signature database",
        )
        path = candidates[0]
        with path.open("rb") as stream:
            header = stream.read(512).decode("ascii").strip().split(":")
        _require(
            header[0] == "ClamAV-VDB" and len(header) >= 9, "invalid signature header"
        )
        result[name] = {
            "version": int(header[2]),
            "built_at": datetime.fromtimestamp(int(header[8]), UTC)
            .isoformat()
            .replace("+00:00", "Z"),
            "sha256": _digest(path),
        }
    return result


def _preparation_output(result: subprocess.CompletedProcess) -> str:
    """Bound diagnostics from public DB preparation, never source scanners."""
    text = (result.stdout + result.stderr).decode("utf-8", errors="replace")
    return "".join(
        character if character.isprintable() or character in "\n\t" else "?"
        for character in text
    )[:4096]


def _prepare_engine(docker: list[str], name: str, spec: dict, cache: Path) -> dict:
    _command(docker + ["pull", spec["image"]])
    inspection = _json(
        _command(
            docker + ["image", "inspect", "--format", "{{json .}}", spec["image"]]
        ).stdout
    )
    _require(
        spec["image"] in inspection["RepoDigests"]
        or spec["image"].split("@", 1)[0].split(":", 1)[0]
        + "@"
        + spec["image"].split("@", 1)[1]
        in inspection["RepoDigests"],
        "pulled image digest differs",
    )
    tool = {"name": name, **spec, "image_id": inspection["Id"]}
    entrypoint = {"trivy": "trivy", "semgrep": "semgrep", "clamav": "clamscan"}[name]
    version_args = (
        ["--version", "--disable-version-check", "--metrics=off"]
        if name == "semgrep"
        else ["--version"]
    )
    output = _success(
        _container(docker, spec["image"], entrypoint, version_args, [], timeout=120)
    ).decode()
    _require(
        re.search(r"(?<![0-9.])" + re.escape(spec["version"]) + r"(?![0-9.])", output),
        "scanner version differs",
    )
    result = {"tool": tool}
    try:
        if name == "trivy":
            directory = cache / "trivy"
            directory.mkdir(parents=True, exist_ok=True)
            update = _container(
                docker,
                spec["image"],
                "trivy",
                [
                    "--cache-dir",
                    "/cache",
                    "--config",
                    "",
                    "fs",
                    "--download-db-only",
                    "--no-progress",
                    "--disable-telemetry",
                    "--skip-version-check",
                ],
                [(directory, "/cache", False)],
                network=True,
            )
            if update.returncode:
                result["preparation_diagnostic"] = _preparation_output(update)
            _success(update)
            metadata = _json(_regular(directory, "db/metadata.json").read_bytes())
            result["database"] = {
                "updated_at": metadata["UpdatedAt"],
                "downloaded_at": metadata["DownloadedAt"],
                "sha256": _digest(_regular(directory, "db/trivy.db")),
            }
        elif name == "clamav":
            directory = cache / "clamav"
            directory.mkdir(parents=True, exist_ok=True)
            config = cache / "freshclam.conf"
            config.write_text(
                "DatabaseDirectory /database\n"
                "DatabaseOwner clamav\n"
                "DatabaseMirror database.clamav.net\n"
                "TestDatabases yes\n"
                "Bytecode yes\n",
                encoding="ascii",
            )
            # The image's observed uid100/gid101 must not call initgroups under
            # cap-drop ALL. Only this public-DB leaf is writable to mapped uid100;
            # its private parent remains mode0700 throughout preparation.
            directory.chmod(0o777)
            try:
                update = _container(
                    docker,
                    spec["image"],
                    "/bin/sh",
                    [
                        "-c",
                        (
                            "umask 022; "
                            "freshclam --config-file=/config/freshclam.conf --stdout; "
                            "result=$?; "
                            "find /database -mindepth 1 "
                            "-exec chmod a+rwX {} + || exit 2; "
                            'exit "$result"'
                        ),
                    ],
                    [
                        (directory, "/database", False),
                        (config, "/config/freshclam.conf", True),
                    ],
                    network=True,
                    user="100:101",
                )
                if update.returncode:
                    result["preparation_diagnostic"] = _preparation_output(update)
                _success(update)
            finally:
                directory.chmod(0o700)
            result["database"] = _clam_database(directory, _now())
    except (ValueError, OSError, KeyError, UnicodeError, subprocess.SubprocessError):
        result["database_error"] = "database-preparation-failed"
    return result


def _prepare(policy: dict, cache: Path) -> tuple[list[str], dict]:
    cache.chmod(0o700)
    docker = _docker()
    # The selected daemon must be local: bind paths belong to its host.
    info = _json(
        _command(
            docker + ["context", "inspect", "--format", "{{json .}}", docker[-1]]
        ).stdout
    )
    _require(
        info["Endpoints"]["docker"]["Host"].startswith("unix://"),
        "release scanners require a local Docker daemon",
    )
    results = {}
    for name, spec in policy["tools"].items():
        try:
            results[name] = _prepare_engine(docker, name, spec, cache)
            if "database" in results[name]:
                try:
                    _database(
                        {
                            "id": "dependencies" if name == "trivy" else "malware",
                            "database": results[name]["database"],
                        },
                        policy,
                        datetime.now(UTC),
                    )
                except ValueError:
                    results[name]["database_error"] = "database-stale-or-malformed"
        except (
            ValueError,
            OSError,
            KeyError,
            UnicodeError,
            subprocess.SubprocessError,
        ):
            results[name] = {"error": "engine-preparation-failed"}
    return docker, results


def _git(root: Path, *args: str) -> bytes:
    return _command(["git", "--no-optional-locks", "-C", str(root), *args]).stdout


def _clean_source(root: Path, source_sha: str) -> None:
    _require(
        _git(root, "rev-parse", "HEAD").decode().strip() == source_sha,
        "security source HEAD differs",
    )
    _require(
        not _git(root, "status", "--porcelain", "--untracked-files=all"),
        "security source must be clean",
    )


def _snapshot(root: Path, destination: Path, source_sha: str) -> list[str]:
    _clean_source(root, source_sha)
    names = []
    for entry in _git(root, "ls-tree", "-rz", "--full-tree", source_sha).split(b"\0"):
        if not entry:
            continue
        metadata, raw_name = entry.split(b"\t", 1)
        mode, kind, identity = metadata.split()
        _require(
            mode in (b"100644", b"100755") and kind == b"blob",
            "unsupported source symlink or submodule",
        )
        name = _relative(raw_name.decode("utf-8"))
        path = _regular(root, name)
        content = path.read_bytes()
        actual = hashlib.sha1(
            b"blob " + str(len(content)).encode("ascii") + b"\0" + content,
            usedforsecurity=False,
        ).hexdigest()
        _require(
            actual == identity.decode("ascii"),
            "source bytes differ from qualified commit",
        )
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)
        names.append(name)
    _require(names, "empty source snapshot")
    _clean_source(root, source_sha)
    return names


def _snapshot_assets(directory: Path, destination: Path, assets: list[dict]) -> None:
    _require(
        directory.is_dir() and not directory.is_symlink(), "unsafe candidate directory"
    )
    for asset in assets:
        source = _regular(directory, asset["filename"])
        target = destination / asset["filename"]
        with source.open("rb") as reader, target.open("xb") as writer:
            shutil.copyfileobj(reader, writer)
        _require(
            target.stat().st_size == asset["size"]
            and _digest(target) == asset["sha256"],
            "candidate bytes differ from admitted asset",
        )


def _pnpm_inventory(path: Path) -> set[tuple[str, str]]:
    """Read only pnpm v9 registry package keys, not arbitrary YAML.

    Trivy remains the YAML/lock parser. This independent inventory requires
    canonical block maps, checks every package and snapshot key, and rejects
    alternate versions, merge/alias/tag syntax and non-semver package keys.
    Metadata below each entry is not interpreted as dependency identity.
    """
    package_key = re.compile(
        r"(?P<name>(?:@[A-Za-z0-9_.~-]+/)?[A-Za-z0-9_.~-]+)"
        r"@(?P<version>(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?)"
        r"(?P<peers>[()A-Za-z0-9@/_.+~=-]*)"
    )
    sections = set()
    packages = set()
    snapshots = set()
    snapshot_keys = set()
    section = None
    current_entry = False
    for line in path.read_text(encoding="utf-8").splitlines():
        _require("\t" not in line, "pnpm inventory does not support tabs")
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        _require(
            not re.search(r"(?:^|[\s,{])(?:[&*!]|<<\s*:|[|>](?:[-+\s]|$))", line),
            "unsupported pnpm YAML alias, tag, merge or block scalar",
        )
        if not line.startswith(" "):
            header = re.fullmatch(r"([A-Za-z][A-Za-z0-9]*):(?: (.+))?", line)
            _require(header is not None, "unsupported pnpm top-level syntax")
            section, value = header.groups()
            _require(
                section
                in {"lockfileVersion", "settings", "importers", "packages", "snapshots"}
                and section not in sections,
                "unsupported or duplicate pnpm section",
            )
            sections.add(section)
            current_entry = False
            if section == "lockfileVersion":
                _require(
                    value in {"'9.0'", '"9.0"', "9.0"},
                    "only pnpm lockfile version 9.0 is supported",
                )
            else:
                _require(value is None, "pnpm inventory requires block mappings")
            continue
        _require(
            section is not None and section != "lockfileVersion",
            "malformed pnpm document",
        )
        if section not in {"packages", "snapshots"}:
            continue
        indentation = len(line) - len(line.lstrip(" "))
        if indentation != 2:
            _require(
                indentation >= 4 and current_entry, "malformed pnpm package indentation"
            )
            continue
        header = re.fullmatch(r"  (.+):(?: \{\})?", line)
        _require(header is not None, "unsupported pnpm package entry")
        key = header.group(1)
        _require(
            len(key) <= 4096, "pnpm package key exceeds supported peer-context bound"
        )
        if key.startswith(("'", '"')):
            _require(len(key) >= 2 and key[-1] == key[0], "unsupported pnpm quoted key")
            key = key[1:-1]
        match = package_key.fullmatch(key)
        _require(match is not None, "unsupported pnpm non-registry package identity")
        identity = (match["name"], match["version"])
        base, _, build = match["version"].partition("+")
        _, separator, prerelease = base.partition("-")
        if separator:
            _require(
                all(
                    part
                    and (
                        not part.isdigit() or len(part) == 1 or not part.startswith("0")
                    )
                    for part in prerelease.split(".")
                ),
                "unsupported pnpm prerelease version",
            )
        if build:
            _require(all(build.split(".")), "unsupported pnpm build version")
        peers = match["peers"]
        depth = 0
        for character in peers:
            if character == "(":
                depth += 1
            elif character == ")":
                depth -= 1
            else:
                _require(depth > 0, "malformed pnpm peer context")
            _require(depth >= 0, "malformed pnpm peer context")
        _require(
            depth == 0 and (not peers or peers.startswith("(") and peers.endswith(")")),
            "malformed pnpm peer context",
        )
        if section == "packages":
            _require(
                not peers and identity not in packages,
                "unsupported or duplicate pnpm package key",
            )
            packages.add(identity)
        else:
            _require(key not in snapshot_keys, "duplicate pnpm snapshot key")
            snapshot_keys.add(key)
            snapshots.add(identity)
        current_entry = True
    _require(
        {"lockfileVersion", "packages", "snapshots"} <= sections
        and packages
        and snapshots == packages,
        "pnpm package and snapshot inventories differ",
    )
    return packages


def _expected_packages(path: Path) -> set[tuple[str, str]]:
    if path.name == "package-lock.json":
        document = _json(path.read_bytes())
        _require(
            document.get("lockfileVersion") in (2, 3), "unsupported npm lock version"
        )
        packages = document["packages"]
        _require(isinstance(packages, dict), "invalid npm package inventory")
        expected = {
            (row.get("name") or name.rsplit("node_modules/", 1)[-1], row["version"])
            for name, row in packages.items()
            if name and "version" in row and not row.get("link")
        }
        if not expected:
            _require(
                set(packages) == {""} and isinstance(packages[""], dict),
                "npm lock lacks an external package inventory",
            )
            manifest = _json(_regular(path.parent, "package.json").read_bytes())
            declarations = (
                "dependencies",
                "devDependencies",
                "optionalDependencies",
                "peerDependencies",
            )
            _require(
                all(
                    packages[""].get(field, {}) == {} and manifest.get(field, {}) == {}
                    for field in declarations
                )
                and all(
                    packages[""].get(field, []) == [] and manifest.get(field, []) == []
                    for field in ("bundleDependencies", "bundledDependencies")
                ),
                "empty npm lock contradicts dependency declarations",
            )
        return expected
    if path.name == "pnpm-lock.yaml":
        return _pnpm_inventory(path)
    document = tomllib.loads(path.read_text(encoding="utf-8"))
    if path.name == "Cargo.lock":
        return {
            (row["name"], row["version"])
            for row in document["package"]
            if "source" in row
        }
    _require(path.name == "uv.lock", "unsupported required dependency lock")
    return {
        (row["name"], row["version"])
        for row in document["package"]
        if "registry" in row.get("source", {})
    }


def _trivy(
    docker: list[str], image: str, source: Path, cache: Path, scanner: str, target: str
) -> dict:
    result = _container(
        docker,
        image,
        "trivy",
        [
            "--cache-dir",
            "/cache",
            "--config",
            "",
            "fs",
            "--scanners",
            scanner,
            "--format",
            "json",
            "--exit-code",
            "0",
            "--no-progress",
            "--disable-telemetry",
            "--skip-version-check",
            "--offline-scan",
            "--skip-db-update",
            "--skip-java-db-update",
            "--skip-check-update",
            "--ignorefile",
            "",
            "--secret-config",
            "",
            "--include-dev-deps",
            "--list-all-pkgs",
            "--parallel",
            "2",
            target,
        ],
        [(source, "/source", True), (cache / "trivy", "/cache", True)],
    )
    _require(result.returncode == 0, "Trivy execution failed")
    document = _json(result.stdout)
    _require(
        document.get("SchemaVersion") == 2
        and isinstance(document.get("Results", []), list),
        "malformed Trivy report",
    )
    return document


def _finding_identifier(value: object) -> str:
    _require(
        isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9_@./:+!~=-]{1,200}", value),
        "malformed finding identifier",
    )
    return value


def _finding_location(value: object, line: object, names: list[str]) -> tuple[str, int]:
    _require(isinstance(value, str), "missing finding path")
    path = value.removeprefix("/source/").removeprefix("./")
    _require(
        path in names and _relative(path) == path and len(path) <= 1024,
        "finding path outside scanned inventory",
    )
    _require(type(line) is int and line > 0, "missing finding line")
    return path, line


def _reported_packages(
    path: Path, rows: list[dict]
) -> tuple[set[tuple[str, str]], list[str]]:
    """Exclude only a lock-proven, unversioned uv editable root."""
    editable_roots = set()
    registry_names = set()
    if path.name == "uv.lock":
        locked = tomllib.loads(path.read_text(encoding="utf-8"))["package"]
        editable_roots = {
            row["name"]
            for row in locked
            if row.get("source") == {"editable": "."} and "version" not in row
        }
        registry_names = {
            row["name"] for row in locked if "registry" in row.get("source", {})
        }
    observed = set()
    excluded = set()
    for row in rows:
        for package in row.get("Packages", []):
            name = package["Name"]
            _require(isinstance(name, str) and name, "invalid reported package name")
            if "Version" not in package:
                _require(
                    name in editable_roots
                    and name not in registry_names
                    and package.get("ID") == name
                    and package.get("Relationship") == "root"
                    and package.get("AnalyzedBy") == "uv",
                    "unversioned package is not an admitted uv editable root",
                )
                excluded.add(name)
                continue
            version = package["Version"]
            _require(
                isinstance(version, str) and version, "invalid reported package version"
            )
            observed.add((name, version))
    return observed, sorted(excluded)


class _DependencyFailure(ValueError):
    """Carry bounded failure details and results from completed independent locks."""

    def __init__(
        self, detail: dict, findings: int, coverage: dict, summaries: list[dict]
    ):
        super().__init__("dependency-scan-incomplete")
        self.detail = detail
        self.findings = findings
        self.coverage = coverage
        self.summaries = summaries


def _dependencies(
    docker: list[str],
    prepared: dict,
    source: Path,
    cache: Path,
    policy: dict,
    names: list[str],
) -> tuple[int, dict, list[dict]]:
    locks = sorted(
        name
        for name in names
        if Path(name).name
        in (
            "Cargo.lock",
            "package-lock.json",
            "uv.lock",
            "yarn.lock",
            "pnpm-lock.yaml",
            "poetry.lock",
            "Pipfile.lock",
            "pylock.toml",
        )
        or Path(name).name.startswith("requirements")
        and name.endswith(".txt")
    )
    if locks != sorted(policy["dependency_files"]):
        differences = [
            {"code": "required-lock-missing", "path": name}
            for name in sorted(set(policy["dependency_files"]) - set(locks))
        ] + [
            {"code": "unconfigured-lock", "path": name}
            for name in sorted(set(locks) - set(policy["dependency_files"]))
        ]
        raise _DependencyFailure(
            {
                "failure_count": len(differences),
                "failures": differences[:20],
                "failures_truncated": len(differences) > 20,
            },
            0,
            {},
            [],
        )
    iac = [
        name
        for name in names
        if Path(name).name == "Dockerfile" or Path(name).suffix in (".tf", ".tfplan")
    ]
    if iac:
        raise _DependencyFailure(
            {
                "failure_count": len(iac),
                "failures": [
                    {"code": "unconfigured-iac-input", "path": name}
                    for name in iac[:20]
                ],
                "failures_truncated": len(iac) > 20,
            },
            0,
            {},
            [],
        )
    count = 0
    coverage = {}
    summaries = []
    failures = []
    failure_count = 0
    for name in locks:
        detail = {"code": "lock-inventory-invalid", "path": name}
        try:
            path = source / name
            expected = _expected_packages(path)
            _require(
                expected or path.name == "package-lock.json",
                "unsupported empty dependency inventory",
            )
            detail["code"] = "scanner-execution-or-report-failed"
            report = _trivy(
                docker,
                prepared["tool"]["image"],
                source,
                cache,
                "vuln",
                "/source/" + name,
            )
            rows = report.get("Results", [])
            detail["code"] = "scanner-package-inventory-invalid"
            observed, local_roots = _reported_packages(path, rows)
            missing = expected - observed
            if missing:
                detail.update(
                    code="required-packages-omitted",
                    expected_packages=len(expected),
                    reported_packages=len(observed),
                    missing_count=len(missing),
                    missing_packages=[
                        {
                            "package": _finding_identifier(package),
                            "version": _finding_identifier(version),
                        }
                        for package, version in sorted(missing)[:20]
                    ],
                    missing_truncated=len(missing) > 20,
                )
                raise ValueError("required dependency packages omitted")
            detail["code"] = "lock-input-unreadable"
            coverage[name] = {
                "packages": len(observed),
                "required_packages": len(expected),
                "sha256": _digest(path),
                "complete": False,
                "local_unversioned_packages": local_roots,
                "empty_dependency_lock": not expected,
            }
            detail["code"] = "advisory-record-invalid"
            for row in rows:
                for finding in row.get("Vulnerabilities", []):
                    count += 1
                    if len(summaries) < SUMMARY_LIMIT:
                        severity = finding["Severity"]
                        _require(
                            severity
                            in {"UNKNOWN", "LOW", "MEDIUM", "HIGH", "CRITICAL"},
                            "malformed advisory severity",
                        )
                        summaries.append(
                            {
                                "id": _finding_identifier(finding["VulnerabilityID"]),
                                "path": name,
                                "package": _finding_identifier(finding["PkgName"]),
                                "version": _finding_identifier(
                                    finding["InstalledVersion"]
                                ),
                                "severity": severity,
                            }
                        )
            coverage[name]["complete"] = True
        except (
            ValueError,
            OSError,
            KeyError,
            TypeError,
            UnicodeError,
            subprocess.SubprocessError,
        ) as error:
            if detail["code"] == "scanner-execution-or-report-failed":
                if isinstance(error, subprocess.TimeoutExpired):
                    detail["code"] = "scanner-timeout"
                elif isinstance(error, json.JSONDecodeError):
                    detail["code"] = "scanner-json-invalid"
                else:
                    detail["code"] = {
                        "Trivy execution failed": "scanner-exit-nonzero",
                        "malformed Trivy report": "scanner-report-invalid",
                        "security command failed": "container-command-failed",
                    }.get(str(error), detail["code"])
            elif detail["code"] == "scanner-package-inventory-invalid":
                detail["code"] = {
                    "unversioned package is not an admitted uv editable root": (
                        "unclassified-unversioned-package"
                    ),
                    "invalid reported package name": "reported-package-name-invalid",
                    "invalid reported package version": (
                        "reported-package-version-invalid"
                    ),
                }.get(str(error), detail["code"])
            detail["cause"] = type(error).__name__
            failure_count += 1
            if len(failures) < 20:
                failures.append(detail)
    if failure_count:
        raise _DependencyFailure(
            {
                "failure_count": failure_count,
                "failures": failures,
                "failures_truncated": failure_count > len(failures),
            },
            count,
            coverage,
            summaries,
        )
    return (
        count,
        {
            "locks": coverage,
            "dev_dependencies": "included; Cargo locks scanned without manifests",
            "iac": "not applicable: no configured deployable IaC inputs",
        },
        summaries,
    )


def _secrets(
    docker: list[str], prepared: dict, source: Path, cache: Path, names: list[str]
) -> tuple[int, dict, list[dict]]:
    report = _trivy(
        docker, prepared["tool"]["image"], source, cache, "secret", "/source"
    )
    count = 0
    summaries = []
    for row in report.get("Results", []):
        for finding in row.get("Secrets", []):
            count += 1
            if len(summaries) < SUMMARY_LIMIT:
                path, line = _finding_location(
                    row["Target"], finding["StartLine"], names
                )
                summaries.append(
                    {
                        "id": _finding_identifier(finding["RuleID"]),
                        "path": path,
                        "line": line,
                    }
                )
    return (
        count,
        {
            "scope": "tracked current-source snapshot; no history",
            "files": len(names),
            "rules": "Trivy built-in secret rules; format and size limitations apply",
        },
        summaries,
    )


def _applicable_rules(
    source: Path, policy: dict, targets: list[str]
) -> tuple[list[str], list[str]]:
    """Read language metadata from the local authored rule format, not YAML."""
    extensions_by_language = {
        "python": {".py"},
        "javascript": {".js", ".jsx", ".mjs", ".cjs"},
        "typescript": {".ts", ".tsx", ".mts", ".cts"},
        "rust": {".rs"},
    }
    extensions = {Path(target).suffix for target in targets}
    supported = set().union(*extensions_by_language.values())
    _require(extensions <= supported, "unsupported SAST target extension")
    languages = {
        language
        for language, suffixes in extensions_by_language.items()
        if extensions & suffixes
    }
    declarations = re.findall(
        r"^  - id: ([a-z0-9-]+)\n    languages: \[([a-z, ]+)\]$",
        (source / "security/semgrep.yml").read_text(encoding="utf-8"),
        re.MULTILINE,
    )
    _require(
        [identity for identity, _ in declarations] == policy["sast_rule_ids"],
        "unsupported local SAST language declaration",
    )
    rule_languages = {
        identity: {language.strip() for language in declared.split(",")}
        for identity, declared in declarations
    }
    _require(
        all(
            declared and declared <= extensions_by_language.keys()
            for declared in rule_languages.values()
        ),
        "unsupported local SAST language",
    )
    _require(
        all(
            any(language in declared for declared in rule_languages.values())
            for language in languages
        ),
        "SAST target language has no configured rules",
    )
    applicable = [
        identity
        for identity in policy["sast_rule_ids"]
        if rule_languages[identity] & languages
    ]
    _require(applicable, "zero applicable SAST rules")
    return applicable, sorted(languages)


def _sast(
    docker: list[str], prepared: dict, source: Path, policy: dict, names: list[str]
) -> tuple[int, dict, list[dict]]:
    targets = [
        "/source/" + name
        for name in names
        if Path(name).suffix in policy["sast_extensions"]
    ]
    _require(targets, "no SAST target files")
    applicable_rules, languages = _applicable_rules(source, policy, targets)
    result = _container(
        docker,
        prepared["tool"]["image"],
        "semgrep",
        [
            "scan",
            "--oss-only",
            "--config",
            "/source/security/semgrep.yml",
            "--metrics=off",
            "--disable-version-check",
            "--disable-nosem",
            "--no-git-ignore",
            "--no-rewrite-rule-ids",
            "--strict",
            "--json",
            "--time",
            "--jobs=2",
            "--max-target-bytes=0",
            "--timeout=60",
            "--timeout-threshold=0",
            *targets,
        ],
        [(source, "/source", True)],
    )
    _require(result.returncode == 0, "Semgrep execution failed")
    report = _json(result.stdout)
    _require(
        report.get("version") == prepared["tool"]["version"]
        and report.get("errors") == []
        and isinstance(report.get("results"), list),
        "Semgrep incomplete or malformed report",
    )
    scanned = report.get("paths", {}).get("scanned")
    _require(
        isinstance(scanned, list) and set(scanned) == set(targets),
        "Semgrep skipped source files",
    )
    rules = report.get("time", {}).get("rules")
    _require(
        isinstance(rules, list)
        and len(rules) == len(applicable_rules)
        and set(rules) == set(applicable_rules),
        "Semgrep executed zero or incomplete applicable rules",
    )
    _require(
        not report.get("skipped_rules")
        and not report.get("time", {}).get("fixpoint_timeouts"),
        "Semgrep skipped rules or timed out",
    )
    summaries = []
    for finding in report["results"][:SUMMARY_LIMIT]:
        rule = finding["check_id"]
        _require(
            rule in applicable_rules,
            "finding uses an inapplicable or unconfigured SAST rule",
        )
        path, line = _finding_location(finding["path"], finding["start"]["line"], names)
        summaries.append({"id": rule, "path": path, "line": line})
    return (
        len(report["results"]),
        {
            "files": len(targets),
            "rules": applicable_rules,
            "configured_rules": policy["sast_rule_ids"],
            "languages": languages,
            "engine": "CE OSS; local patterns only; no interfile analysis",
        },
        summaries,
    )


def _malware(
    docker: list[str], prepared: dict, artifacts: Path, cache: Path, assets: list[dict]
) -> tuple[int, dict, list[dict]]:
    _require(
        all(asset["size"] <= 1073741824 for asset in assets),
        "artifact exceeds complete malware scan limit",
    )
    result = _container(
        docker,
        prepared["tool"]["image"],
        "clamscan",
        [
            "--database=/database",
            "--official-db-only=yes",
            "--recursive=yes",
            "--follow-dir-symlinks=0",
            "--follow-file-symlinks=0",
            "--scan-archive=yes",
            "--alert-exceeds-max=yes",
            "--alert-encrypted=yes",
            "--alert-broken=yes",
            "--alert-broken-media=yes",
            "--max-filesize=1024M",
            "--max-scansize=2048M",
            "--max-files=100000",
            "--max-recursion=64",
            "--max-scantime=300000",
            "--pcre-max-filesize=1024M",
            "--stdout",
            "/artifacts",
        ],
        [(artifacts, "/artifacts", True), (cache / "clamav", "/database", True)],
    )
    text = (result.stdout + result.stderr).decode("utf-8", errors="replace")
    _require(result.returncode in (0, 1), "ClamAV execution failed")
    _require(
        not re.search(
            r"(?:WARNING|ERROR|Heuristics\.Limits|Heuristics\.Encrypted|Heuristics\.Broken|\bskipp(?:ed|ing)\b)",
            text,
            re.IGNORECASE,
        ),
        "ClamAV scan incomplete",
    )
    summary = dict(
        re.findall(
            r"^(Known viruses|Scanned files|Infected files|Total errors):\s*(\d+)\s*$",
            text,
            re.MULTILINE,
        )
    )
    _require(
        int(summary.get("Known viruses", "0")) > 0
        and int(summary.get("Scanned files", "0")) >= len(assets)
        and int(summary.get("Total errors", "0")) == 0,
        "ClamAV incomplete scan summary",
    )
    for asset in assets:
        _require(
            re.search(
                r"^"
                + re.escape("/artifacts/" + asset["filename"])
                + r": (?:OK|[^\n]+ FOUND)$",
                text,
                re.MULTILINE,
            ),
            "ClamAV omitted an artifact",
        )
    count = int(summary["Infected files"])
    _require(
        (result.returncode == 1) == (count > 0), "ClamAV status disagrees with findings"
    )
    summaries = []
    for asset in assets:
        match = re.search(
            r"^"
            + re.escape("/artifacts/" + asset["filename"])
            + r": ([A-Za-z0-9_.-]{1,200}) FOUND$",
            text,
            re.MULTILINE,
        )
        if match and len(summaries) < SUMMARY_LIMIT:
            summaries.append({"id": match.group(1), "artifact": asset["filename"]})
    _require(
        len(summaries) == min(count, SUMMARY_LIMIT),
        "malformed or missing malware finding identity",
    )
    return (
        count,
        {
            "assets": [asset["filename"] for asset in assets],
            "scanned_files": int(summary["Scanned files"]),
            "archive_recursion": 64,
            "max_file_bytes": 1073741824,
            "max_scan_bytes": 2147483648,
            "limits_fail_closed": True,
        },
        summaries,
    )


def run_suite(
    root: Path, directory: Path, *, source_sha: str, assets: list[dict]
) -> dict:
    """Prepare all configured engines, scan immutable inputs, return redacted proof.

    Findings and scanner errors do not short-circuit independent categories.
    Invalid policy/identity is a boundary ValueError; operational failures produce
    all four error rows and an incomplete report suitable for embedding.
    """
    root, directory = Path(root).resolve(), Path(directory)
    policy, policy_hash, rules_hash = _policy(root)
    admitted = _assets(assets)
    _require(
        isinstance(source_sha, str) and re.fullmatch(r"[0-9a-f]{40}", source_sha),
        "invalid security source SHA",
    )
    checks = [
        {
            "id": identity,
            "status": "error",
            "tool": {"name": name, **policy["tools"][name], "image_id": ""},
            "findings": 0,
            "coverage": {"complete": False},
            "error": "not-executed",
        }
        for identity, name in CHECK_TOOLS.items()
    ]
    evidence = {
        "schema": 1,
        "source_sha": source_sha,
        "assets": admitted,
        "policy_sha256": policy_hash,
        "rules_sha256": rules_hash,
        "scanned_at": _now(),
        "status": "incomplete",
        "checks": checks,
    }
    cache_parent = os.environ.get("RELEASE_SECURITY_CACHE")
    if cache_parent:
        _require(
            not Path(cache_parent).resolve().is_relative_to(root),
            "security cache must be outside source checkout",
        )
        Path(cache_parent).mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix="release-security-", dir=cache_parent
    ) as temporary:
        workspace = Path(temporary)
        source, artifacts, cache = (
            workspace / "source",
            workspace / "artifacts",
            workspace / "cache",
        )
        for path in (source, artifacts, cache):
            path.mkdir()
        try:
            names = _snapshot(root, source, source_sha)
            _snapshot_assets(directory, artifacts, admitted)
            _require(
                _policy(source)[1:] == (policy_hash, rules_hash),
                "policy changed during source admission",
            )
        except (
            ValueError,
            OSError,
            KeyError,
            UnicodeError,
            subprocess.SubprocessError,
        ):
            for check in checks:
                check["error"] = "input-admission-failed"
            return evidence
        try:
            docker, prepared = _prepare(policy, cache)
        except (
            ValueError,
            OSError,
            KeyError,
            UnicodeError,
            subprocess.SubprocessError,
        ):
            for check in checks:
                check["error"] = "container-runtime-unavailable"
            return evidence
        for check in checks:
            identity = check["id"]
            engine = prepared[CHECK_TOOLS[identity]]
            if "error" in engine:
                check["error"] = engine["error"]
                continue
            check["tool"] = engine["tool"]
            if identity in ("dependencies", "malware"):
                if "database_error" in engine:
                    check["error"] = engine["database_error"]
                    if "preparation_diagnostic" in engine:
                        check["preparation_diagnostic"] = engine[
                            "preparation_diagnostic"
                        ]
                    continue
                check["database"] = engine["database"]
            try:
                if identity in ("dependencies", "malware"):
                    _database(check, policy, datetime.now(UTC))
                if identity == "dependencies":
                    count, coverage, summaries = _dependencies(
                        docker, engine, source, cache, policy, names
                    )
                elif identity == "secrets":
                    count, coverage, summaries = _secrets(
                        docker, engine, source, cache, names
                    )
                elif identity == "sast":
                    count, coverage, summaries = _sast(
                        docker, engine, source, policy, names
                    )
                else:
                    count, coverage, summaries = _malware(
                        docker, engine, artifacts, cache, admitted
                    )
                check.update(
                    status="findings" if count else "passed",
                    findings=count,
                    coverage={"complete": True, **coverage},
                    summaries=summaries,
                    summaries_truncated=count > len(summaries),
                )
                del check["error"]
            except _DependencyFailure as error:
                check.update(
                    status="error",
                    error="dependency-scan-incomplete",
                    failure_detail=error.detail,
                    findings=error.findings,
                    coverage={"complete": False, "locks": error.coverage},
                    summaries=error.summaries,
                    summaries_truncated=error.findings > len(error.summaries),
                )
            except (
                ValueError,
                OSError,
                KeyError,
                TypeError,
                UnicodeError,
                subprocess.SubprocessError,
            ):
                check["error"] = "scanner-failed-or-incomplete"
        try:
            # Scanners only saw the byte-verified frozen source. Also reject a
            # checkout or candidate changed while scans were running.
            _clean_source(root, source_sha)
            _require(
                _policy(root)[1:] == (policy_hash, rules_hash),
                "security configuration changed",
            )
            for name in names:
                _require(
                    _digest(_regular(root, name)) == _digest(source / name),
                    "source changed during scan",
                )
            for asset in admitted:
                path = _regular(directory, asset["filename"])
                _require(
                    path.stat().st_size == asset["size"]
                    and _digest(path) == asset["sha256"],
                    "candidate changed during scan",
                )
        except (ValueError, OSError, subprocess.SubprocessError):
            for check in checks:
                check.update(status="error", error="input-changed-during-scan")
        evidence["scanned_at"] = _now()
        evidence["status"] = (
            "incomplete"
            if any(check["status"] == "error" for check in checks)
            else "failed"
            if any(check["status"] == "findings" for check in checks)
            else "passed"
        )
        if evidence["status"] == "passed":
            try:
                validate_evidence(
                    root, evidence, source_sha=source_sha, assets=admitted
                )
            except ValueError:
                evidence["status"] = "incomplete"
                for check in checks:
                    check.update(
                        status="error",
                        error="evidence-freshness-or-completeness-failed",
                    )
    return evidence


def print_summary(evidence: dict) -> None:
    """Expose redacted results even when failed CI retains no bundle."""
    print(
        f"Release security: {evidence['status']} (source {evidence['source_sha']})",
        file=sys.stderr,
    )
    for check in evidence["checks"]:
        print(
            f"  {check['id']}: {check['status']}; findings={check['findings']}",
            file=sys.stderr,
        )
        if "error" in check:
            print(f"    error: {check['error']}", file=sys.stderr)
        for finding in check.get("summaries", []):
            print("    " + json.dumps(finding, sort_keys=True), file=sys.stderr)
        if "failure_detail" in check:
            print(
                "    " + json.dumps(check["failure_detail"], sort_keys=True),
                file=sys.stderr,
            )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("prepare", "scan", "verify"))
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--directory", type=Path)
    parser.add_argument("--source-sha")
    parser.add_argument(
        "--assets-json", type=Path, help="JSON array of admitted asset records"
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="new evidence output file (outside candidate directory)",
    )
    parser.add_argument(
        "--evidence-file", type=Path, help="existing evidence JSON (verify only)"
    )
    parser.add_argument(
        "--cache", type=Path, help="preparation workspace (prepare only)"
    )
    args = parser.parse_args()
    if args.action == "prepare":
        _require(
            args.cache is not None and not args.cache.exists(),
            "prepare requires a new --cache directory",
        )
        args.cache.mkdir(mode=0o700, parents=True)
        policy, _, _ = _policy(args.root)
        _, results = _prepare(policy, args.cache)
        print(json.dumps(results, indent=2, sort_keys=True))
        return int(
            any(
                "error" in result or "database_error" in result
                for result in results.values()
            )
        )
    _require(
        args.directory is not None
        and args.source_sha is not None
        and args.assets_json is not None,
        "scan/verify requires --directory, --source-sha and --assets-json",
    )
    assets = _assets(json.loads(args.assets_json.read_bytes()))
    if args.action == "verify":
        _require(args.evidence_file is not None, "verify requires --evidence-file")
        evidence = _json(args.evidence_file.read_bytes())
        validate_evidence(
            args.root, evidence, source_sha=args.source_sha, assets=assets
        )
        for asset in assets:
            path = _regular(args.directory, asset["filename"])
            _require(
                path.stat().st_size == asset["size"]
                and _digest(path) == asset["sha256"],
                "candidate identity differs",
            )
        return 0
    _require(
        args.output is not None
        and not args.output.resolve().is_relative_to(args.directory.resolve()),
        "scan requires --output outside candidate directory",
    )
    evidence = run_suite(
        args.root, args.directory, source_sha=args.source_sha, assets=assets
    )
    with args.output.open("x", encoding="utf-8") as stream:
        stream.write(json.dumps(evidence, indent=2, sort_keys=True) + "\n")
    print_summary(evidence)
    return int(evidence["status"] != "passed")


if __name__ == "__main__":
    raise SystemExit(main())
