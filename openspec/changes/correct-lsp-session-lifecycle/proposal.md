## Why

Changing the configured DAML server can send editor work after shutdown, abort disposal, and prevent replacement startup. The current promise queue serializes some lifecycle actions but does not control library-generated wire traffic; it also waits for context-request responses, allowing business work to block retirement.

## What Changes

- Give each client session one explicit admission and retirement boundary, covering generated providers, context operations, synchronization, watchers, trace and cancellation traffic.
- Separate lifecycle transitions from business-response lifetimes. Retiring a session invalidates its pending work without waiting indefinitely for a context response.
- Keep ownership of starting and retiring clients until their outcome is known. Coalesce obsolete configuration intentions; reject stale commands instead of moving them to a replacement session.
- Retain the installed language-client's providers and process support where its actual contracts suffice. Correct the pinned JSON-RPC request-settlement defect before using writer rejection as admission control.
- Replace the server's next-message shutdown helper with explicit terminal admission: acknowledge once, reject subsequent requests with `InvalidRequest`, ignore non-exit notifications, and await `exit` against one deadline.
- Verify correct client ordering separately from defensive server behavior. A successful restart against a more tolerant server alone cannot establish the client invariant.

## Capabilities

### New Capabilities

- `lsp-lifecycle`: client request admission, transition ownership, bounded protocol shutdown, and restart outcomes across the editor, language server and stdio boundary.

### Modified Capabilities

- None. Preserve the existing semantic-editor trust, context-generation and watcher contracts and semantic-token behavior while adding an explicit shared lifecycle contract.

## Impact

Client lifecycle and transport integration in `integrations/vscode`, the locked client dependency graph or a reviewed repository-owned dependency correction, protocol dispatch in `src/lsp/mod.rs`, executable termination in `src/bin/daml-lsp.rs`, focused protocol checks and real installed-extension verification. No language semantics, schemas, module/work limits, public setting names, workspace trust rules, application protocol additions, automatic retry policy or service boundary changes.

The current release is immutable. This proposal neither changes v0.2.1 assets nor authorizes another release. Preserve stock cross-platform executable selection and non-exec-wrapper cleanup; do not promise that shutdown bounds arbitrary blocked OS writes.

## Scope decision

Pending. The user requested a redesign after rejecting a server-only repair: “this also is indicative of wrong controlflow. back to the drawing board”. This authorizes analysis and this proposal, not acceptance of its implementation commitments or publication.
