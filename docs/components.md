# Components and status

DAML is an experimental configuration language and tooling project. The components below are implemented, but the project does not claim a stable compatibility guarantee or a complete application ecosystem.

| Component | Status | Scope |
|---|---|---|
| Data syntax and Rust library | Implemented | Parses data documents, preserves source information, and exposes typed library operations. See the [syntax contract](syntax.md). |
| Native definitions and bundles | Implemented | Loads local definitions, compiles constraints, completes defaults, and validates values. See [native authoring](native-authoring.md). |
| JSON Schema interchange | Implemented bounded profile | Accepts explicit Draft 2020-12 schema modules and resolves values within documented limits. See [schema modules](schemas.md). |
| Formatter | Implemented | Formats data and native definitions. See [formatting](formatting.md). |
| Language server | Implemented | Provides native syntax diagnostics and document formatting over stdio. It does not provide schema semantics, completion, or hover. See [LSP capabilities](lsp.md). |
| VS Code-compatible extension | Source-buildable VSIX | Provides syntax highlighting and the language server's diagnostics and formatting. No marketplace listing is provided. See [installation](../integrations/vscode/README.md). |
| Browser SDK | Public GitHub release asset | WebAssembly API for parsing, native compilation, resolution, validation, and formatting. It is not published to npm. See the [browser SDK](../bindings/browser/README.md). |
| Python API | Source-buildable | Python bindings over the Rust library. No PyPI release or wheel assets are provided. See the [Python API](python.md). |
| Conversion tools | Not provided | DAML does not ship JSON/YAML, Helm, or vendor-format converter commands. See [conversion limits](conversion.md). |

Install native executables from public [GitHub releases](distribution.md). The installation guide lists the supported platform matrix and source fallback.