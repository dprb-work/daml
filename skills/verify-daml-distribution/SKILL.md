---
name: verify-daml-distribution
description: Verify a DAML native archive or Python wheel, install it away from the build tree, and exercise its actual installed consumer contracts.
---

# Verify DAML distributions

Use the packaged artifact and its matching checksum sidecar. The
[feature map](features/README.md) separates native executable and Python library
coverage. Choose one recipe; Python wheel verification never invokes Cargo.

## Run

Start in this skill directory. Supply an absolute archive path and its matching
`.sha256` sidecar. Python 3.11 or later is required:

```sh
python3 scripts/archive.py /absolute/path/to/daml-VERSION-TARGET.tar.gz
```

The archive target must match the host. The helper needs Rust/Cargo and the
repository's pinned toolchain to build the existing process-contract tests.

## Archive checks

Confirm that the archive and sidecar come from the intended repository workflow
run and target. The helper verifies the SHA-256, rejects unsafe, special, or
duplicate members, requires the fixed six-file payload, and checks the archive
root against `BUILD.json`. The payload includes DAML's MIT `LICENSE`, generated
third-party notices, and the source inventory. A matching checksum establishes
integrity, not trust in an unknown publisher.

The helper refuses an existing installation destination. It extracts the
archive to a new temporary path and runs the existing formatter and LSP process
contracts with `DAML_TEST_BIN_DIR` pointing to the archive's executables. Cargo
may build the test harness, but the subprocesses under test come from the
archive. A successful build alone does not prove the installed contracts.

## Python wheel recipe

From the repository root, with standard CPython 3.12+ and `uv` on PATH:

```sh
python skills/verify-daml-distribution/scripts/wheel.py /absolute/path/to/daml_config-VERSION-cp312-abi3-PLATFORM.whl
```

Relative paths and `~` are resolved once at CLI admission, before any external
working-directory change; the receipt records the resolved exact candidate.

Use `--python /absolute/path/to/python` to select another installed standard
CPython 3.12+ interpreter. PyPy and free-threaded builds are outside the supported
contract. No Rust/Cargo, maturin, checkout-built extension, or project virtual
environment is needed by the installation/consumer stages. The verifier itself
reads this checkout's manifests, Python package files, canonical legal files,
lockfile, and Git commit to identify the intended candidate. Run it against the
source revision that produced the wheel.

The [Python wheel map](features/python-wheel.md) specifies the pre-install
doctor and the real base/types workflows. Checksums and RECORD hashes establish
consistency, not publisher trust. Native header, GLIBC-reference, and deployment
checks reject contradictory platform claims; they do not replace the producer's
successful maturin audit or prove the truth of self-reported build history.
Dirty local qualification is allowed and reported; a clean candidate is a
separate handoff requirement.

Verification inputs belong to an independent check owner. Route a mismatch with
the exact command, candidate hash, receipt, and failed assumption to that owner
before changing assertions or expected results. Record amendments and new input
identities with the owning review's evidence, and invalidate affected prior runs.

## Evidence and cleanup

The command prints the build identity, archive hash, and passing process-contract
result. Retain that output in CI or the owning review.

The helper owns the temporary installation through a temporary-directory
context, including failure paths. Keep command or CI evidence outside the
deleted tree.

Wheel verification writes one JSON receipt to stdout on success or failure;
subprocess diagnostics go to stderr. Capture stdout in CI or an evidence file
outside the scratch tree. The receipt identifies the exact candidate hash, build,
interpreter, installed module origins, workflow results, and cleanup state.
Each invocation creates a fresh uv environment outside the checkout, uses isolated
Python (`-I`) from that external working directory, and removes its scratch tree
even after a failed install or consumer run. The receipt survives cleanup because
it is emitted after teardown. No ports or long-running processes are started.

## Helper

- `scripts/archive.py`: checksum verification, constrained extraction, and
  installed formatter/LSP process contracts. It installs no runtime dependencies.
- `scripts/wheel.py`: wheel/sidecar/RECORD and legal/provenance checks, native
  platform admission, then exact-wheel binary-only base/types installs and real
  isolated consumers. Only the optional types dependencies are fetched from the
  configured default index; source builds and interpreter downloads are disabled.
- `scripts/test_wheel.py`: focused safe-member checks and a valid-to-tampered
  RECORD transition. Run without builds or installs:
  `python -m unittest discover -s skills/verify-daml-distribution/scripts -p test_wheel.py`
  from the repository root.
