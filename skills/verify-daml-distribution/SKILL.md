---
name: verify-daml-distribution
description: Verify a built or downloaded DAML binary archive, install it away from the build tree, and exercise the installed executables through process contracts or an isolated installed OMP instance.
---

# Verify DAML binary distribution

Use the real archive and installed executable. Do not substitute a Cargo-built
path, source-imported OMP client, mock LSP response, or model-generated claim.
The [feature map](features/README.md) owns coverage recipes.

## Launch

Start from this skill directory. Supply an absolute archive path, with its
matching `.sha256` sidecar. Python 3.11+ is required for these verification helpers.

For existing binary process contracts, Rust/Cargo and the repository's pinned
toolchain are also required:

```sh
python3 scripts/archive.py /absolute/path/to/daml-VERSION-TARGET.tar.gz
```

For the isolated OMP path, supply the installed OMP executable and a new evidence
directory. This path uses no Cargo, Rust, Node, or Bun command from PATH:

```sh
python3 scripts/archive.py /absolute/path/to/daml-VERSION-TARGET.tar.gz \
  --omp /absolute/path/to/omp --evidence /absolute/new/evidence-directory
```

The OMP driver currently targets the local Unix environment. Native archive
contracts run on Linux, macOS, and Windows through CI. Do not claim OMP coverage
on an OS where this driver has not run.

## Doctor

Before driving, confirm the archive and sidecar are from the intended repository
workflow run and target. The helper checks the SHA-256, rejects unsafe/special or
duplicate members, requires the fixed five-file payload, and checks the archive
root against `BUILD.json`. It refuses an existing installation destination.
A matching checksum establishes integrity, not trust in an unknown publisher.

The OMP path records `omp --version`, uses a fresh project/root marker, and
constructs isolated HOME/config/data/cache/tmp directories. Its environment does
not inherit host credentials or Cargo/Bun paths. Shared LSP mode is explicitly
disabled. It selects a catalog model with a non-secret startup key but submits no
prompt; any observed `agent_start` frame fails verification.

## Drive

The default command extracts the archive to a temporary installation and runs the
existing formatter and LSP process contracts with `DAML_TEST_BIN_DIR` pointing
there. Cargo may build the test harness, but the subprocess under test comes
from the archive.

The OMP command moves the extracted bundle again, then starts the actual OMP RPC
executable. `scripts/omp-probe.mjs` uses OMP's bundled public `LspTool` and real
isolated settings to drive the same client configuration a user would supply.
It checks UTF-16 capabilities, the `1:7` diagnostic after supplementary Unicode,
valid replacement/clearing, one whole-document edit with its correct range,
client application, idempotence, and `daml-fmt --check` on the resulting file.

## Evidence

OMP evidence retains `summary.json`, `probe.json`, `omp.stdout.jsonl`, and
`omp.stderr.txt`. The summary identifies archive hash/build, OMP version, exit
status, zero model turns, exact runtime scope, and removal of isolated state.
Read the probe's actual tool results and final source; a configured server alone
is not proof that it ran.

Preserve the named evidence directory after success or failure. The archive
contract path prints the build/hash and passing process-contract result; retain
that command output in CI or the owning review.

## Cleanup

Helpers own temporary installations and state through temporary-directory
contexts, including failure paths. OMP is a finite subprocess with a deadline;
normal RPC stdin closure/session shutdown disposes its owned runtime. Never stop
an unrelated OMP process or change user configuration. Confirm
`isolated_state_removed: true` in successful OMP evidence and keep the evidence
outside the deleted tree. Do not reuse an evidence directory from an earlier run.

## Helpers

- `scripts/archive.py`: checksum, constrained extraction, installed binary
  contracts, and isolated OMP orchestration. No runtime dependency installer.
- `scripts/omp-probe.mjs`: verification-only extension loaded explicitly into the
  installed OMP process; not a shipped editor/host integration.
