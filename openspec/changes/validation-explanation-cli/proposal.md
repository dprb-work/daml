## Why

Configuration authors need to validate, complete, and understand DAML from a terminal or CI without writing an embedding program. The existing native examples and provenance API supply most of the engine, but no supported configuration CLI.

## What Changes

- Add the source-built `daml` executable with `check`, `resolve`, and `explain` operations.
- Select native bundles explicitly or by existing document-relative discovery. Also accept explicit JSON schema-module files, using the interchange API delivered by MR #26.
- Keep one compiler/resolver/validator authority. Explain checked values, origins, assertions, and applicable constraint locations without a second validator or unbounded recursive-schema expansion.
- Define exact output, input, context, failure, and resource contracts; document runnable examples.
- Preserve `daml-fmt`, `daml-lsp`, binding APIs, grammar, and existing release producers. No publishing or deployment is authorized.

## Capabilities

### New Capabilities

- `configuration-cli`: Terminal validation, completion, and explanation of configuration.

### Modified Capabilities

- None. Recursive-schema semantics from MR #26 remain unchanged.

## Impact

New executable, core explanation support, focused behavioral tests, and command/API documentation. The implementation baseline is `2f5ecbb`, which merged MR #26. Core tracing must retain its instance-descending recursive references and cumulative resource bounds. Native archive inclusion remains with the separately active release owner; this change advertises source-built CLI availability only.

## Scope decision

The user accepted issues #32, #33, and #34 with “do all of em. then, porceed on the cli work” and requested integration with MR #26. CLI #32 is the current delivery slice; schema-aware LSP #33 and comment-preserving edits #34 follow in the accepted order. This records scope acceptance, not result acceptance, merge, or release authorization.
