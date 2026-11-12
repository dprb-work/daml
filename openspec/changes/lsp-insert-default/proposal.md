## Why

Recommend a small, server-only addition: one eager **Insert this default: <field>** action per omitted field. Authors can make a checked default explicit without copying a hover value or reformatting their document. No bulk action: it hides which defaults become authored commitments and collides with the editor's same-boundary batch restriction.

## What Changes

- Handle standard `textDocument/codeAction` with kind `refactor.rewrite`; return ready-to-apply, versioned `WorkspaceEdit`s, not commands or deferred resolution.
- Select the innermost authored object at `range.start`. Offer its absent immediate fields only after complete configuration resolution succeeds. Include explicit scalar, object and list defaults, including enclosing-object presets and defaults from the author's selected provider tag. Never choose a provider or infer a default from a completion hint.
- Materialize the selected field's completed subtree, retaining exact values and tag bindings. Do not materialize siblings. Require the edited document to resolve to the same completed values/assertion identities, ignoring provenance.
- Reuse source-edit append placement and canonical fragment style: preserve surrounding bytes, not inferred indentation. No whole-document reformat. Clients lacking literal code actions or versioned document-change support get no actions.
- No language/API migration or new dependencies. Existing completion, hover, semantic diagnostics, session lifecycle and source-edit contracts remain unchanged. Non-goals: changes to those providers, file writes, bulk insertion, OMP/editor plugins, remote schema retrieval or release.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `semantic-editor`: explicit default-insertion code actions; see the [scenario contract](specs/semantic-editor/spec.md).

## Impact

Reuse already merged in PRs #53/#55/#57 and semantic-editor work:

| Existing owner | Reuse |
|---|---|
| `src/lsp/mod.rs`, `positions.rs` | Synchronous session, accepted document versions, negotiation, dispatch, encoded byte ranges and shutdown cutoff. |
| `src/lsp/semantic.rs` | Explicit configuration context, fresh bundle compilation, unsaved overlays and dependency refresh. |
| `src/resolve.rs`, `resolved.rs`, `semantic.rs` | Checked completion, origins, exact values/tags and `SemanticNode::from_resolved`. |
| `src/source_edit.rs` | `SourceEdit::Insert`, schema-aware preparation, append placement, canonical rendering, bounds and checked `SourcePatch::apply`. |

Genuinely new: capability/request handling, containing-object selection, missing-field/default eligibility, eager edit assembly and completed-value equivalence checking. `Origin::Default` alone is insufficient: required-container synthesis also uses it; distinguish an explicit default through compiled provenance, not schema-hint guesses. Keep this internal to existing owners; no new service, cache or public editing API.

**Rough estimate, not measured:** 250–450 implementation lines, 350–600 focused test lines in the existing Rust/LSP suites, plus a short `docs/lsp.md` update. Assumes existing source-edit layout is accepted unchanged. No standalone design note is needed; the consequential choices and limitations are recorded here and in the contract.

Version checks protect document text, not dependency/context changes after the response. Actions capture the checked default at request time; a later request refreshes it. No apply-time schema guarantee is promised.

## Open questions / scope decision

Accepted 2026-11-12 by Daniel's orchestrator (session 01a1c78f), under Daniel's
delegation, with every recommended answer below. Implementation and low-risk
merge after passing checks and independent review are authorized. Release,
tagging and publication remain Daniel's.

- Require the whole configuration to resolve before offering edits? **Yes** — avoid hint-based edits; unrelated invalid data suppresses actions.
- Include object/list defaults as completed subtrees? **Yes** — otherwise copying a partial preset can change meaning, especially with tags.
- Add “insert all defaults”? **No** — explicit per-field choice and no new batching policy.
- Accept existing canonical insertion style and request-time schema snapshot, without indentation inference or deferred resolution? **Yes** — preserves the existing editor boundary and avoids stateful action handles.
