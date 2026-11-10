# Editor integrations

DAML includes a VS Code-compatible extension, a generic Language Server Protocol server and a standalone Syntect grammar. Syntect consumers can highlight source without a language server, using their own themes. Syntax diagnostics and whole-document formatting are available without schema discovery. Matching current source builds add explicitly selected semantic diagnostics, completion, hover and source navigation; older released servers may lack them. See the [LSP guide](../docs/lsp.md) for context selection and exact capabilities.

| Client | Status | Setup |
|---|---|---|
| VS Code and compatible editors | Source-buildable VSIX; syntax highlighting, syntax diagnostics, and formatting | [Build and install the VSIX](vscode/README.md) |
| Syntect-based renderers | Standalone DAML grammar; consumer registration required | [Load the Syntect grammar](syntect/README.md) |
| OMP 18.18.5 | No global grammar-registration extension API; further integration deferred pending v2 assessment | [OMP limits and optional source integration](syntect/README.md#omp-integration) |
| Other LSP clients | Use `daml-lsp` directly | [Configure a generic LSP client](../docs/lsp.md#build-and-launch) |

The server is distributed separately from editor clients. Follow the public [command-line installation guide](../docs/distribution.md) to install a released executable. No marketplace publication is provided.
