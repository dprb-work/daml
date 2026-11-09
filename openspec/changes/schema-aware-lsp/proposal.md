## Why

The current language server formats DAML and reports syntax errors, but cannot tell authors which fields, defaults, or definitions apply. Issue #33 adds semantic authoring through the existing native compiler and the checked explanation delivered by CLI #32.

## What Changes

- Add explicit per-document configuration/definition context through standard `workspace/executeCommand` command `daml.setContext`. Unselected documents remain syntax-only; workspace/cwd do not infer a schema.
- Add semantic diagnostics, field and visible-name completion, constraint/default hover, and actual source definition navigation.
- Make open unsaved definition text authoritative during analysis; re-evaluate dependents on accepted changes, closes, or file notifications. Use bounded dependency notifications so the VS Code client can watch referenced files outside the workspace too.
- Extend the existing parser with bounded cursor-context reporting for incomplete authoring without admitting incomplete text as valid data.
- Add trusted-workspace editor commands to select context, and verify the real editor plus stdio protocol path.

## Capabilities

### New Capabilities

- `semantic-editor`: Explicit schema context, diagnostics, completions, hover, and source navigation.

### Modified Capabilities

- None. Existing native grammar, completion/validation semantics, CLI/binding contracts, and syntax-only operation remain intact.

## Impact

Optional Rust LSP adapter, bounded native compiler/source metadata and overlays, core parser cursor reporting, VS Code client commands/watch lifecycle, independently owned behavioral checks, and maintained editor/native documentation. No remote schema retrieval, new language grammar, provider catalogue, application execution, release publication, or host installation changes.

## Scope decision

The user accepted #33 together with #32 and #34, then explicitly selected “Accept and continue” for CLI revision `c31daace273fe48e8b3e94f55e91172b20ae344f`. This is the next accepted delivery slice, stacked on that revision while PR #35 remains unmerged. Syntax-only default and explicit context selection preserve the prior no-schema-discovery behavior until a caller opts in. Exact-result acceptance and release authority remain separate.
