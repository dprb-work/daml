# Proposal

## Why

The Rust library and CLI can make targeted, comment-preserving source edits. Python exposes semantic trees and canonical writing, but not that editor. Python applications using in-memory derived schemas must currently leave the binding or rewrite unrelated source.

## What Changes

- Add one-shot `Document.edit(edits, line_width=80)` and `Schemas.edit(document, edits, expected=None, line_width=80)`, returning a new immutable `Document`.
- Accept strict list/tuple batches of replace/insert/remove records using the existing semantic-value representation, not ordinary Python value coercion.
- Reuse the Rust editor for original-snapshot paths, byte/comment ownership, exact values, bound assertions, conflicts and finite admission.
- Validate checked edits without resolving the original document or serializing completed defaults.
- Export public `SourceEditError` metadata and preserve existing typed causes; retain existing semantic-transport error conventions.
- Update Python stubs and user documentation, and verify real rebuilt-extension workflows through independently owned checks.

## Capabilities

### New Capabilities
- `python-source-edits`: source-preserving editing from Python with optional existing schema context.

### Modified Capabilities
- None. The core source-editing, semantic transport, type projection and canonical writing contracts remain unchanged.

## Impact

The existing Python binding owns wire admission, Python/Rust ownership transfer and exception translation. The existing Rust source editor owns editing, rendering and final readmission. No second patcher, grammar, evaluator, dependency, implicit file writer or schema fetch is added.

The owner approved specification and implementation of the bounded one-shot Python feature in the current conversation (`yes go`); [issue #54](https://github.com/dprb-work/daml/issues/54) records that scope. Concrete method/record/error names follow existing binding conventions. Persistent Python patch objects, new core/CLI/LSP/browser entry points, package publication, installation and host activation are excluded. Result acceptance and release authority remain separate.
