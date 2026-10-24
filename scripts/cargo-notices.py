#!/usr/bin/env python3
"""Render Cargo dependency notices using DAML's canonical license policy."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ABOUT_VERSION = "cargo-about 0.9.2"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest-path", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--all-features", action="store_true")
    parser.add_argument("--cargo", default="cargo")
    args = parser.parse_args()
    try:
        version = subprocess.run(
            [args.cargo, "about", "--version"],
            check=True,
            capture_output=True,
            text=True,
            encoding="utf-8",
            cwd=ROOT,
        ).stdout.strip()
        if version != ABOUT_VERSION:
            raise ValueError(f"install {ABOUT_VERSION} before generating notices")
        policy = tomllib.loads((ROOT / "deny.toml").read_text(encoding="utf-8"))
        allowed = policy.get("licenses", {}).get("allow")
        if not isinstance(allowed, list) or not all(
            isinstance(item, str) for item in allowed
        ):
            raise ValueError(
                "deny.toml [licenses].allow must be a list of license identifiers"
            )
        with tempfile.TemporaryDirectory(prefix="daml-notices-") as temporary:
            config = Path(temporary) / "about.toml"
            config.write_text(
                "\n".join(
                    (
                        "accepted = " + json.dumps(allowed),
                        "targets = " + json.dumps([args.target]),
                        "ignore-build-dependencies = false",
                        "ignore-dev-dependencies = true",
                        "ignore-transitive-dependencies = false",
                        "private = { ignore = true }",
                        "",
                    )
                ),
                encoding="utf-8",
            )
            command = [
                args.cargo,
                "about",
                "generate",
                "--manifest-path",
                str(args.manifest_path.resolve()),
                "--config",
                str(config),
                "--locked",
                "--target",
                args.target,
                "--fail",
                str(ROOT / "scripts" / "third-party-notices.hbs"),
            ]
            if args.all_features:
                command.append("--all-features")
            notices = subprocess.run(
                command,
                check=True,
                capture_output=True,
                cwd=ROOT,
            ).stdout
        sys.stdout.buffer.write(notices)
    except subprocess.CalledProcessError as error:
        detail = error.stderr or error.stdout
        if isinstance(detail, bytes):
            detail = detail.decode("utf-8", errors="replace")
        print(f"cargo-notices: {detail.strip()}", file=sys.stderr)
        return 1
    except (OSError, ValueError) as error:
        print(f"cargo-notices: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
