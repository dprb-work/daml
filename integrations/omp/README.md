# OMP DAML language server

This directory contains an OMP project registration payload for the released `daml-lsp` executable. OMP owns server startup, synchronization, diagnostics, and edit application; the DAML server owns syntax diagnostics and formatting.

## Install

1. Install the private DAML release for your platform and make `daml-lsp` available on `PATH`. The default `command` is `daml-lsp`; it may instead be an absolute executable path in your local copy of `.omp/lsp.json`.
2. Copy `lsp.json` to `<project>/.omp/lsp.json`, creating `.omp` if needed. OMP merges project LSP registrations with other configured servers. When editing an existing file, preserve every existing server entry and add/merge the `daml` entry; do not replace the complete host configuration.
3. Ensure the project has a `daml.bundle` or `.git` root marker discoverable from the working directory. This locates OMP's server process; it does not select DAML schemas or change bundle privacy.

This is a configuration payload, not an installer. It does not install `daml-lsp`, edit user-wide OMP settings, or activate host configuration. The LSP language ID is `daml`; the server accepts client-supplied document text without selecting semantics from that ID.
