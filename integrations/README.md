# Editor integrations

DAML's editor support currently includes a VS Code-compatible extension and a generic Language Server Protocol server. The server provides native-definition syntax diagnostics and whole-document formatting; it does not provide schema semantics, completion, or hover. See the [LSP guide](../docs/lsp.md) for its exact capabilities.

| Client | Status | Setup |
|---|---|---|
| VS Code and compatible editors | Source-buildable VSIX; syntax highlighting, syntax diagnostics, and formatting | [Build and install the VSIX](vscode/README.md) |
| Other LSP clients | Use `daml-lsp` directly | [Configure a generic LSP client](../docs/lsp.md#build-and-launch) |

The server is distributed separately from editor clients. Follow the public [command-line installation guide](../docs/distribution.md) to install a released executable. No marketplace publication is provided.
