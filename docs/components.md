# Components and status

DAML is an experimental configuration language and tooling project. The components below are implemented, but the project does not claim a stable compatibility guarantee or a complete application ecosystem.

| Component | Status | Scope |
|---|---|---|
| Data syntax and Rust library | Implemented | Parses ordinary and first-class Date/Time/DateTime/Cron values, preserves source information, and exposes typed library operations. See the [syntax contract](syntax.md). |
| Native definitions and bundles | Implemented | Loads local definitions, compiles constraints, completes defaults, and validates values, including temporal kinds and offset policies. See [native authoring](native-authoring.md). |
| JSON Schema interchange | Implemented bounded profile | Accepts explicit Draft 2020-12 schema modules and resolves values within documented limits. See [schema modules](schemas.md). |
| Configuration CLI | Source-buildable | Checks configuration, resolves defaults, explains values/provenance/constraints, and applies targeted comment-preserving source edits through the existing core. Not included in older native archives. See [CLI usage](cli.md). |
| Formatter | Implemented | Formats data and native definitions. See [formatting](formatting.md). |
| Language server | Source and versioned executable | Syntax diagnostics/formatting by default; current source adds explicitly selected semantic diagnostics, completion, hover and navigation. Older releases may predate these features. See [LSP capabilities](lsp.md). |
| VS Code-compatible extension | Source-buildable VSIX | Highlighting and server integration, including explicit semantic contexts and shared dependency watching with matching source server. No marketplace listing. See [installation](../integrations/vscode/README.md). |
| Browser SDK | Public GitHub release asset | WebAssembly API for parsing, native compilation, resolution, validation, and formatting. It is not published to npm. See the [browser SDK](../bindings/browser/README.md). |
| Python API | Source or candidate wheel | The `daml-config` distribution (`import daml`) provides bindings over the Rust library; optionally derives bounded in-memory schema modules from explicitly named Pydantic v2 models or dataclasses, including supported temporal fields. Candidate wheels are verified in CI, not published to PyPI. See [installation](python.md#install-a-candidate-wheel). |
| Conversion tools | Not provided | DAML does not ship JSON/YAML, Helm, or vendor-format converter commands. See [conversion limits](conversion.md). |

Install native executables from public [GitHub releases](distribution.md). The installation guide lists the supported platform matrix and source fallback.