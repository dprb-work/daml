#!/usr/bin/env python3
"""Collect eight unchanged producer payloads from one qualification run."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

from release_common import (
    REPOSITORY,
    ROOT,
    check_sidecar,
    check_versions,
    expected_assets,
    inspect_payload,
    require,
    sha256,
    source_identity,
    validate_bundle,
)


def prepare(
    artifacts: Path,
    out: Path,
    *,
    source_sha: str,
    prepare_run_id: int,
    run_attempt: int,
) -> dict:
    source_identity(source_sha)
    require(
        type(prepare_run_id) is int
        and prepare_run_id > 0
        and type(run_attempt) is int
        and run_attempt > 0,
        "invalid preparation run identity",
    )
    require(not out.exists() and not out.is_symlink(), f"output already exists: {out}")
    head = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()
    require(head == source_sha, "collector checkout differs from qualified source")
    dirty = subprocess.check_output(
        ["git", "status", "--porcelain", "--untracked-files=normal"],
        cwd=ROOT,
        text=True,
    ).strip()
    require(not dirty, "collector requires a clean source checkout")
    version = check_versions()
    expected = expected_assets(version)
    groups = {}
    for name, identity in expected.items():
        kind, target = identity["kind"], identity["target"]
        if kind == "native":
            group = f"daml-{target}"
        elif kind == "python":
            group = f"daml-config-{target}"
        elif kind == "browser":
            group = "daml-browser-sdk"
        else:
            group = "definitively-asserted-daml-vsix"
        groups[group] = name
    require(
        artifacts.is_dir() and not artifacts.is_symlink(),
        "artifacts must be grouped producer directories",
    )
    require(
        {path.name for path in artifacts.iterdir()} == set(groups),
        "producer artifact directory set differs",
    )
    assets = []
    sources = {}
    for group, name in groups.items():
        directory = artifacts / group
        require(
            directory.is_dir() and not directory.is_symlink(),
            f"unsafe artifact directory: {group}",
        )
        entries = list(directory.iterdir())
        names = {path.name for path in entries}
        pair = {name, name + ".sha256"}
        require(
            (names == pair or (expected[name]["kind"] == "vsix" and names == {name}))
            and all(path.is_file() and not path.is_symlink() for path in entries),
            f"producer file set differs: {group}",
        )
        path = directory / name
        digest = sha256(path)
        if name + ".sha256" in names:
            check_sidecar(path, digest)
        inspect_payload(path, version=version, source_sha=source_sha, **expected[name])
        assets.append(
            {
                "filename": name,
                "sha256": digest,
                "size": path.stat().st_size,
                **expected[name],
            }
        )
        sources[name] = path
    manifest = {
        "schema": 1,
        "repository": REPOSITORY,
        "source_sha": source_sha,
        "version": version,
        "prepare_run_id": prepare_run_id,
        "run_attempt": run_attempt,
        "assets": sorted(assets, key=lambda asset: asset["filename"]),
    }
    # Exclusive creation reserves a new candidate; a failed collection remains visible
    # and is never silently overwritten or removed on retry.
    out.mkdir(parents=True, exist_ok=False)
    for asset in manifest["assets"]:
        name = asset["filename"]
        with sources[name].open("rb") as source, (out / name).open("xb") as destination:
            shutil.copyfileobj(source, destination)
        with (out / (name + ".sha256")).open(
            "x", encoding="ascii", newline="\n"
        ) as sidecar:
            sidecar.write(f"{asset['sha256']}  {name}\n")
    with (out / "RELEASE.json").open("x", encoding="utf-8", newline="\n") as stream:
        stream.write(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    validate_bundle(
        out, version=version, source_sha=source_sha, prepare_run_id=prepare_run_id
    )
    digest = sha256(out / "RELEASE.json")
    packet = (
        "## DAML prepared candidate (not publication authorization)\n\n"
        f"- Repository: `{REPOSITORY}`\n- Source SHA: `{source_sha}`\n"
        f"- Version: `{version}`\n- Preparation run: `{prepare_run_id}` (attempt `{run_attempt}`)\n"
        "- Artifact: `daml-release-candidate` (8 payloads, 8 SHA-256 sidecars, RELEASE.json)\n"
        f"- Approval manifest SHA-256: `{digest}`\n\n"
        "Only a successful canonical `prepare-release.yml` run on main qualifies for promotion. "
        "A Check dispatch is a qualification preview. A rerun or changed manifest needs a new approved digest.\n"
    )
    print(packet)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with Path(summary).open("a", encoding="utf-8") as stream:
            stream.write(packet)
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with Path(output).open("a", encoding="utf-8") as stream:
            stream.write(f"version={version}\nmanifest_sha256={digest}\n")
    return manifest


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--artifacts",
        required=True,
        type=Path,
        help="current run's grouped downloaded artifact directories",
    )
    parser.add_argument("--out", type=Path, default=ROOT / "dist" / "release")
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--prepare-run-id", required=True, type=int)
    parser.add_argument("--run-attempt", required=True, type=int)
    args = parser.parse_args()
    prepare(
        args.artifacts,
        args.out,
        source_sha=args.source_sha,
        prepare_run_id=args.prepare_run_id,
        run_attempt=args.run_attempt,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
