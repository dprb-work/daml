---
name: verify-daml-distribution
description: Verify a DAML binary archive, install it away from the build tree, and exercise the installed formatter and LSP process contracts.
---

# Verify DAML binary distribution

Use the archive and installed executables. The [feature map](features/README.md)
describes the covered archive contract.

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

## Evidence and cleanup

The command prints the build identity, archive hash, and passing process-contract
result. Retain that output in CI or the owning review.

The helper owns the temporary installation through a temporary-directory
context, including failure paths. Keep command or CI evidence outside the
deleted tree.

## Helper

- `scripts/archive.py`: checksum verification, constrained extraction, and
  installed formatter/LSP process contracts. It installs no runtime dependencies.
