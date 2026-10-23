#!/usr/bin/env python3
"""Run the packaged OMP registration against DAML's released language server."""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile


REPO = Path(__file__).resolve().parents[3]
PAYLOAD = REPO / "integrations" / "omp" / "lsp.json"
PROBE = REPO / "skills" / "verify-daml-distribution" / "scripts" / "omp-probe.mjs"


def required_executable(name: str) -> Path:
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(f"set {name} to an absolute executable path")
    path = Path(value).resolve(strict=True)
    if not path.is_file() or not os.access(path, os.X_OK):
        raise RuntimeError(f"{name} is not an executable file: {path}")
    return path


def main() -> None:
    omp = required_executable("OMP_BINARY")
    daml_lsp = required_executable("DAML_LSP_PATH")
    evidence = Path(os.environ.get("DAML_OMP_EVIDENCE", REPO / ".scratch" / "omp-integration-verify")).resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    payload = json.loads(PAYLOAD.read_text(encoding="utf-8"))
    if not isinstance(payload, dict) or not isinstance(payload.get("servers"), dict) or "daml" not in payload["servers"]:
        raise RuntimeError("packaged OMP payload must define a daml server")

    with tempfile.TemporaryDirectory(prefix="daml-omp-integration-") as temporary:
        root = Path(temporary)
        project = root / "project #?%20"
        (project / ".omp").mkdir(parents=True)
        (project / "daml.bundle").write_text("{}\n", encoding="utf-8")
        (project / "trial.daml").write_text('"😀é" ?', encoding="utf-8")

        merged = {"servers": {"existing-probe": {
            "command": str(omp), "args": ["--version"], "fileTypes": [".probe"],
            "languageId": "probe", "rootMarkers": ["daml.bundle"],
        }}}
        daml_registration = dict(payload["servers"]["daml"])
        daml_registration["command"] = str(daml_lsp)
        merged["servers"].update(payload["servers"])
        merged["servers"]["daml"] = daml_registration
        (project / ".omp" / "lsp.json").write_text(json.dumps(merged, indent=2) + "\n", encoding="utf-8")

        for name in ("home", "agent", "config", "dsh", "xdg-config", "xdg-cache", "tmp"):
            (root / name).mkdir()
        report_path = root / "probe.json"
        overlay = root / "isolated.yml"
        overlay.write_text("lsp:\n  enabled: true\n  shared: false\n", encoding="utf-8")
        env = {
            "PATH": f"{daml_lsp.parent}:/usr/bin:/bin", "LANG": "C.UTF-8",
            "HOME": str(root / "home"), "TMPDIR": str(root / "tmp"),
            "PI_CONFIG_DIR": str(root / "config"), "PI_CODING_AGENT_DIR": str(root / "agent"),
            "DSH_HOME": str(root / "dsh"), "XDG_CONFIG_HOME": str(root / "xdg-config"),
            "XDG_CACHE_HOME": str(root / "xdg-cache"), "PI_DISABLE_LSPMUX": "1",
            "DAML_PROBE_REPORT": str(report_path), "DAML_PROBE_FORMATTER": str(daml_lsp.with_name("daml-fmt")),
        }
        version = subprocess.run([str(omp), "--version"], env=env, cwd=project, check=True,
                                 capture_output=True, text=True, encoding="utf-8", timeout=15).stdout.strip()
        command = [
            str(omp), "--mode", "rpc", "--no-session", "--no-skills", "--no-rules",
            "--no-extensions", "-e", str(PROBE), "--cwd", str(project), "--config", str(overlay),
            "--no-title", "--no-ui", "--model", "openai/gpt-4o",
            "--api-key", "verification-no-model-requests", "--max-time", "90s",
        ]
        failure = None
        with subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              text=True, encoding="utf-8", env=env, cwd=project, start_new_session=True) as run:
            try:
                stdout, stderr = run.communicate("", timeout=120)
            except BaseException as error:
                failure = error
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
                    stdout, stderr = run.communicate(timeout=10)
        (evidence / "omp.stdout.jsonl").write_text(stdout, encoding="utf-8")
        (evidence / "omp.stderr.txt").write_text(stderr, encoding="utf-8")
        if report_path.exists():
            shutil.copyfile(report_path, evidence / "probe.json")
        if failure is not None:
            raise failure
        frames = [json.loads(line) for line in stdout.splitlines() if line.strip()]
        if any(frame.get("type") == "agent_start" for frame in frames):
            raise RuntimeError("unexpected model-backed agent turn during the probe")
        if run.returncode != 0:
            raise RuntimeError(f"OMP exited {run.returncode}; inspect retained stderr")
        report = json.loads(report_path.read_text(encoding="utf-8"))
        if not report.get("passed"):
            raise RuntimeError(f"OMP DAML integration failed: {report.get('error')}")
        if report.get("cwd") != str(project):
            raise RuntimeError("OMP probe ran outside its isolated project")
        summary = {
            "passed": True, "omp_version": version, "omp_exit": run.returncode,
            "model_turns": 0, "scope": "actual OMP RPC process and built-in LSP client using integrations/omp/lsp.json",
        }

    summary["isolated_state_removed"] = not root.exists()
    if not summary["isolated_state_removed"]:
        raise RuntimeError("temporary isolated OMP state remains")
    (evidence / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(evidence / "summary.json")


if __name__ == "__main__":
    main()
