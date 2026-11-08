"""Replay recorded Spring parser decisions, without adding Java to repository gates.

Acquire the pinned JDK/jars described in tests/fixtures/spring-cron.json separately.
Run from any directory:
  python replay.py --java-home /path/to/jdk --jars /path/to/jars --output /path/to/receipt.json
The class output and receipt remain under the explicitly supplied output directory.
This helper never changes expected fixtures or downloads dependencies.
"""

import argparse
import base64
import hashlib
import json
import subprocess
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--java-home", required=True, type=Path)
    parser.add_argument("--jars", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    source = Path(__file__).resolve().with_name("CronOracle.java")
    fixture_path = source.parents[2] / "fixtures" / "spring-cron.json"
    fixture = json.loads(fixture_path.read_text())
    jars = [args.jars / f"spring-{name}-7.0.10.jar" for name in ("context", "core")]
    for jar, key in zip(jars, ("context_sha256", "core_sha256"), strict=True):
        actual = hashlib.sha256(jar.read_bytes()).hexdigest()
        if actual != fixture["reference"][key]:
            raise ValueError(f"Wrong pinned reference jar: {jar}: {actual}")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    java = str(args.java_home / "bin" / "java")
    version = subprocess.run(
        [java, "-version"], capture_output=True, text=True, check=True
    ).stderr
    classpath = ":".join(str(jar.resolve()) for jar in jars)
    subprocess.run(
        [
            str(args.java_home / "bin" / "javac"),
            "-cp",
            classpath,
            "-d",
            str(args.output.parent),
            str(source),
        ],
        check=True,
    )
    reference = subprocess.run(
        [
            java,
            "-XX:ActiveProcessorCount=1",
            "-cp",
            f"{args.output.parent.resolve()}:{classpath}",
            "CronOracle",
        ],
        input="".join(
            base64.b64encode(case["expression"].encode()).decode() + "\n"
            for case in fixture["cases"]
        ),
        capture_output=True,
        text=True,
        check=True,
    )
    lines = reference.stdout.splitlines()
    if len(lines) != len(fixture["cases"]):
        raise ValueError(f"Oracle response count mismatch: {len(lines)}")
    observations = []
    differences = []
    for case, line in zip(fixture["cases"], lines, strict=True):
        status, payload = line.split("\t", 1)
        if status not in ("OK", "ERR"):
            raise ValueError(f"Unknown oracle response: {status}")
        text = base64.b64decode(payload).decode()
        actual = {"expression": case["expression"], "accepted": status == "OK"}
        actual["retained" if status == "OK" else "error"] = text
        observations.append(actual)
        if actual["accepted"] != case["accepted"] or (
            actual["accepted"] and actual["retained"] != case["retained"]
        ):
            differences.append({"expected": case, "actual": actual})
    args.output.write_text(
        json.dumps(
            {
                "java_version": version,
                "fixture_sha256": hashlib.sha256(fixture_path.read_bytes()).hexdigest(),
                "oracle_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
                "observations": observations,
                "differences": differences,
            },
            ensure_ascii=True,
            indent=2,
        )
        + "\n"
    )
    print(
        f"{len(observations)} reference observations; {len(differences)} differences; {args.output}"
    )
    if differences:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
