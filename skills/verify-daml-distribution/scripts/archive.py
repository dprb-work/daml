#!/usr/bin/env python3
"""Verify/extract a DAML bundle and exercise its installed executable contracts."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import signal
import stat
import subprocess
import tarfile
import tempfile
from typing import IO, cast
import zipfile


def safe_path(name: str) -> PurePosixPath:
    path = PurePosixPath(name)
    if path.is_absolute() or ".." in path.parts or "\\" in name or ":" in name or not path.parts:
        raise ValueError(f"unsafe archive member: {name}")
    return path


def store(source: IO[bytes], path: PurePosixPath, destination: Path, files: set[PurePosixPath]) -> None:
    if path in files:
        raise ValueError(f"duplicate archive member: {path}")
    files.add(path)
    target = destination.joinpath(*path.parts)
    target.parent.mkdir(parents=True, exist_ok=True)
    with target.open("xb") as output:
        shutil.copyfileobj(source, output)
    target.chmod(0o755 if path.parent.name == "bin" else 0o644)


def unpack(archive: Path, destination: Path) -> tuple[Path, dict[str, object]]:
    """Verify the sidecar and fixed bundle layout, installing only to a new path."""
    archive = archive.resolve()
    expected = archive.with_name(archive.name + ".sha256").read_text(encoding="ascii").split()
    with archive.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    if expected != [digest, archive.name]:
        raise ValueError("archive checksum or sidecar filename does not match")
    if destination.exists():
        raise ValueError(f"installation destination already exists: {destination}")
    destination.mkdir(parents=True)
    files: set[PurePosixPath] = set()
    is_zip = archive.name.endswith(".zip")
    if is_zip:
        with zipfile.ZipFile(archive) as bundle:
            for member in bundle.infolist():
                path = safe_path(member.filename)
                if member.is_dir():
                    continue
                if stat.S_IFMT(member.external_attr >> 16) not in (0, stat.S_IFREG):
                    raise ValueError(f"non-regular archive member: {path}")
                with bundle.open(member) as source:
                    store(source, path, destination, files)
    else:
        with tarfile.open(archive, "r:gz") as bundle:
            for member in bundle.getmembers():
                path = safe_path(member.name)
                if member.isdir():
                    continue
                if not member.isfile():
                    raise ValueError(f"non-regular archive member: {path}")
                source = bundle.extractfile(member)
                if source is None:
                    raise ValueError(f"archive file has no content: {path}")
                with source:
                    store(source, path, destination, files)
    roots = {path.parts[0] for path in files}
    if len(roots) != 1:
        raise ValueError("bundle must contain one top-level directory")
    root = roots.pop()
    suffix = ".exe" if is_zip else ""
    required = {PurePosixPath(root) / name for name in (
        "BUILD.json", "SOURCES.md", "THIRD-PARTY-NOTICES.txt",
        f"bin/daml-lsp{suffix}", f"bin/daml-fmt{suffix}",
    )}
    if files != required:
        raise ValueError("bundle does not contain exactly the expected files")
    installed = destination / root
    loaded: object = json.loads((installed / "BUILD.json").read_text(encoding="utf-8"))
    if not isinstance(loaded, dict) or not all(isinstance(key, str) for key in loaded):
        raise ValueError("build manifest must be an object with string keys")
    build = cast(dict[str, object], loaded)
    for key in ("version", "target", "git_commit", "rustc"):
        if not isinstance(build.get(key), str) or not build[key]:
            raise ValueError(f"build manifest is missing {key}")
    if root != f"daml-{build['version']}-{build['target']}":
        raise ValueError("archive root does not match its build identity")
    return installed, {**build, "archive_sha256": digest}


def check_omp(archive: Path, omp: Path, evidence: Path) -> None:
    """Drive the installed OMP executable with isolated state and no model turn."""
    if os.name != "posix":
        raise RuntimeError("the isolated OMP driver currently requires a Unix host")
    evidence = evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    omp = omp.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="daml-isolated-omp-") as temporary:
        root = Path(temporary)
        installed, build = unpack(archive, root / "downloaded")
        prefix = root / "application"
        _ = shutil.move(str(installed), prefix)
        project = root / "project"
        (project / ".omp").mkdir(parents=True)
        (project / "daml-smoke.project").touch()
        _ = (project / "trial.daml").write_text('"😀é" ?', encoding="utf-8")
        _ = (project / ".omp" / "lsp.json").write_text(json.dumps({"servers": {"daml": {
            "command": str(prefix / "bin" / "daml-lsp"), "args": [], "fileTypes": [".daml"],
            "languageId": "daml", "rootMarkers": ["daml-smoke.project"],
        }}}), encoding="utf-8")
        overlay = root / "isolated.yml"
        _ = overlay.write_text("lsp:\n  enabled: true\n  shared: false\n", encoding="utf-8")
        for directory in ("home", "agent", "config", "dsh", "xdg-config", "xdg-cache", "tmp"):
            (root / directory).mkdir()
        env = {
            "PATH": f"{prefix / 'bin'}:/usr/bin:/bin", "LANG": "C.UTF-8",
            "HOME": str(root / "home"), "TMPDIR": str(root / "tmp"),
            "PI_CONFIG_DIR": str(root / "config"), "PI_CODING_AGENT_DIR": str(root / "agent"),
            "DSH_HOME": str(root / "dsh"), "XDG_CONFIG_HOME": str(root / "xdg-config"),
            "XDG_CACHE_HOME": str(root / "xdg-cache"), "PI_DISABLE_LSPMUX": "1",
            "DAML_PROBE_REPORT": str(root / "probe.json"),
            "DAML_PROBE_FORMATTER": str(prefix / "bin" / "daml-fmt"),
        }
        version = subprocess.run([str(omp), "--version"], env=env, check=True, capture_output=True, text=True, timeout=15).stdout.strip()
        # A selected catalog model satisfies startup, but no prompt is sent and
        # no real credential is provided or inherited.
        command = [
            str(omp), "--mode", "rpc", "--no-session", "--no-skills", "--no-rules",
            "--no-extensions", "-e", str(Path(__file__).with_name("omp-probe.mjs")),
            "--cwd", str(project), "--config", str(overlay), "--no-title", "--no-ui",
            "--model", "openai/gpt-4o", "--api-key", "verification-no-model-requests", "--max-time", "90s",
        ]
        timed_out = False
        with subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              text=True, env=env, cwd=project, start_new_session=True) as run:
            try:
                stdout, stderr = run.communicate("", timeout=120)
            except subprocess.TimeoutExpired:
                timed_out = True
                try:
                    os.killpg(run.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    stdout, stderr = run.communicate(timeout=10)
                except subprocess.TimeoutExpired:
                    try:
                        os.killpg(run.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    stdout, stderr = run.communicate()
        _ = (evidence / "omp.stdout.jsonl").write_text(stdout, encoding="utf-8")
        _ = (evidence / "omp.stderr.txt").write_text(stderr, encoding="utf-8")
        report_path = root / "probe.json"
        if report_path.exists():
            _ = shutil.copyfile(report_path, evidence / "probe.json")
        if timed_out:
            raise RuntimeError("isolated OMP exceeded its deadline; owned process group was stopped")
        frames = [json.loads(line) for line in stdout.splitlines() if line.strip()]
        if any(frame.get("type") == "agent_start" for frame in frames):
            raise RuntimeError("unexpected model-backed agent turn during the probe")
        if run.returncode != 0:
            raise RuntimeError(f"isolated OMP exited {run.returncode}; inspect retained stderr")
        report = json.loads(report_path.read_text(encoding="utf-8"))
        if not report.get("passed"):
            raise RuntimeError(f"isolated OMP verification failed: {report.get('error')}")
        summary = {"passed": True, "omp_version": version, "omp_exit": run.returncode,
                   "model_turns": 0, "build": build, "scope": "installed OMP RPC process and built-in LSP tool"}
    summary["isolated_state_removed"] = not root.exists()
    _ = (evidence / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(evidence / "summary.json")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    _ = parser.add_argument("archive", type=Path)
    _ = parser.add_argument("--omp", type=Path, help="installed OMP executable; select isolated OMP verification")
    _ = parser.add_argument("--evidence", type=Path, help="new evidence directory for OMP verification")
    args = parser.parse_args()
    if args.omp is not None:
        if args.evidence is None:
            parser.error("--omp requires --evidence")
        check_omp(Path(args.archive), Path(args.omp), Path(args.evidence))
        return
    if args.evidence is not None:
        parser.error("--evidence requires --omp")
    repo = Path(__file__).resolve().parents[3]
    with tempfile.TemporaryDirectory(prefix="daml-distribution-contracts-") as temporary:
        installed, build = unpack(args.archive, Path(temporary) / "install")
        env = os.environ.copy()
        env["DAML_TEST_BIN_DIR"] = str(installed / "bin")
        _ = subprocess.run([
            "cargo", "test", "--release", "--locked", "--features", "lsp",
            "--target", str(build["target"]), "--test", "lsp", "--test", "formatter_cli",
        ], cwd=repo, env=env, check=True)
        print(json.dumps({"installed_contracts": "passed", **build}, indent=2))


if __name__ == "__main__":
    main()
