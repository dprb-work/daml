## Why

DAML clients should receive symbolic token categories rather than a fixed color palette. The user selected standard LSP semantic tokens so editors and users retain control over presentation, and explicitly instructed implementation after that scope was explained.

## What Changes

- Advertise a standard semantic-token legend and implement `textDocument/semanticTokens/full` for open DAML documents.
- Classify properties, namespaces, types, strings, numbers, keywords, comments, regular-expression payloads and operators using the existing data/native parser and string/trivia owners.
- Mark recognized unqualified built-in types with the standard `defaultLibrary` modifier, without resolving schemas or loading files.
- Return bounded, sorted, nonoverlapping, single-line relative tokens in the negotiated position encoding, including grounded prefixes of incomplete documents.
- Preserve existing syntax diagnostics, explicit semantic contexts and document-version lifecycle. No colors, editor theme rules, client changes, range/delta protocol, global symbol index, source-editing API, release or host activation.

## Capabilities

### New Capabilities

- `semantic-tokens`: client-independent symbolic highlighting through the existing stdio server.

### Modified Capabilities

None. Existing diagnostic, formatting, completion, hover, definition and context contracts remain intact.

## Impact

Changes are internal to the optional LSP/parser editor path and protocol checks, plus the LSP reference. No new dependency or public portable-library API is required. Ordinary parsing must not collect highlighting metadata. Client rendering and user theme configuration remain client-owned.

## Scope decision

The user accepted standard categories/modifiers with no editor-specific colors and then directed: “yeah ofc we cant know what the client dose, duh. go dude”. This records acceptance of that bounded server capability, not an implementation/result approval or merge/release authority. Comment-preserving edits (#34 / draft #41) remain separate and pending.
