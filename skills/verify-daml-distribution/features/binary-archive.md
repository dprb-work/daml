# Native binary archive

A user obtains `daml-lsp` and `daml-fmt` for their OS/architecture without compiling
the project. The archive carries build identity, the canonical DAML MIT `LICENSE`,
third-party notices, and source inventory.

Reach it through a successful Binaries workflow artifact or the documented local
packaging command. Supply the inner DAML archive and its sidecar to
`python3 scripts/archive.py ARCHIVE` from the verification skill directory.

The helper verifies checksum and layout, extracts to a new temporary path, and
runs existing actual formatter/LSP process contracts against that installation.
Pass means input/error behavior, formatting/check semantics, Unicode diagnostics,
versioned synchronization, resource-edge rejection, and shutdown work through the
packaged subprocesses. A successful build alone is insufficient.

The helper needs Cargo to build the existing test harness, not to install or run
the archive. Archive target must match the host. Temporary installation is owned
by one invocation and removed even on failure; keep command/CI evidence outside it.
