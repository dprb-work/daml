## ADDED Requirements

### Requirement: Default actions select checked missing fields

The server SHALL offer `textDocument/codeAction` only for an open, completely parsed configuration with an explicitly selected semantic context and root contract. Current bundle compilation and whole-document completion/validation SHALL succeed using the existing overlay/dependency rules. Syntax-only/native-definition contexts, unresolved schemas/tags, ambiguous/conflicting defaults and invalid configurations SHALL yield no actions, not speculative edits. Existing diagnostics SHALL remain authoritative.

The target SHALL be the innermost authored mapping containing `range.start`, including its braces, tag, keys, scalar values, internal comments and whitespace. Only the start selects scope; both range endpoints SHALL be valid negotiated positions in source order. Objects inside lists SHALL qualify; lists/scalars themselves and positions outside an authored object SHALL NOT be targets. A cursor at the closing brace SHALL target that object; the position just after it SHALL NOT. An implicit tagged empty mapping SHALL qualify over its tag span.

An eligible field SHALL be absent from the authored target and present in its checked result because of an explicit applicable default, including a field contributed by an enclosing object preset. Required-container synthesis alone SHALL NOT qualify; child defaults below a synthesized parent SHALL NOT qualify that absent parent. Scalars (including null and intrinsic temporal values), nested objects and lists SHALL qualify. Required/optional status SHALL NOT change eligibility. Open objects SHALL NOT invent arbitrary names; closed objects SHALL offer only fields admitted by all active constraints. Already-present fields, including explicit null or partial objects/lists, SHALL NOT receive replacement actions.

#### Scenario: Closed and open objects

- **WHEN** either kind of checked object omits a permitted defaulted `timeout`, supplies `endpoint`, and has no default for `other`
- **THEN** only `timeout` is offered; neither openness nor required status invents another value.

#### Scenario: Existing nested data and compound defaults

- **WHEN** an authored object omits a defaulted object/list but supplies a different partial object or an empty list
- **THEN** the omitted fields qualify as whole completed subtrees, the supplied fields are not replaced, and missing children of the partial object qualify only when that object is targeted.

#### Scenario: Chosen provider and blocked analysis

- **WHEN** an object explicitly selects a provider tag that suppresses its contextual preset
- **THEN** actions use only defaults that survive checked resolution, never the suppressed preset or an unchosen provider
- **AND** unresolved, ambiguous, conflicting or invalid input yields no actions, including when the error is elsewhere in the configuration.

#### Scenario: Authored target and synthetic containers

- **WHEN** the cursor is inside an object in a list, or a required absent container is synthesized without its own explicit default
- **THEN** the former targets that authored object; the latter receives no parent-insertion action, even if completion fills child defaults.

### Requirement: One source-preserving edit per selected default

The server SHALL advertise eager literal actions of kind `refactor.rewrite`, without a resolve provider, only when the client supports literal code actions and `workspace.workspaceEdit.documentChanges`. Otherwise it SHALL return no actions, never an unversioned fallback. It SHALL honor `context.only` by kind ancestry. Each eligible field SHALL receive exactly one action titled `Insert this default: <field>`, sorted by literal field name, with no command, diagnostic prerequisite, preferred-action flag or “insert all” action.

Each action SHALL insert that field's checked completed subtree, retaining exact numbers, intrinsic kinds and representable tag bindings. The result SHALL reparse and resolve under the same contract with identical completed data and assertion identities, apart from origins and equivalent tag aliases. A candidate that cannot preserve meaning or represent its key/tag SHALL be omitted.

Insertion SHALL append at the existing source-editor boundary immediately before the object's closing brace, after existing entries/trivia; an implicit tagged empty mapping SHALL gain braces after its tag. New fragments SHALL use the source editor's canonical 80-column style and the parent's first physical line-ending style (LF if absent). The server SHALL NOT infer surrounding indentation, move comments, reorder existing fields or rewrite any other bytes. Other omitted fields outside the inserted subtree SHALL remain omitted.

#### Scenario: Comments, spelling and compound materialization

- **WHEN** the author requests an action beside comments, CRLF and noncanonical numeric spelling
- **THEN** applying its edit adds only the chosen field/subtree at the append boundary, preserves all existing bytes, and keeps the completed configuration's meaning.

#### Scenario: Filtered or unsupported client

- **WHEN** `context.only` excludes `refactor.rewrite`, or the client lacks either required edit capability
- **THEN** no action or command fallback is returned.

### Requirement: Snapshot-bound actions fail without mutation

Each successful action SHALL contain a `WorkspaceEdit.documentChanges` entry with one `TextDocumentEdit` naming only the requested URI and its latest accepted integer document version. Ranges SHALL use the negotiated encoding. The server SHALL NOT apply the edit, call `workspace/applyEdit`, write files or mutate cached source on request; only normal accepted document synchronization changes that source.

Clients SHALL reject an edit if their buffer version differs and request fresh actions; the server SHALL NOT rebase or retry it. Requests have no input version: already-accepted changes determine the response version. Close/reopen, context and connection lifetimes SHALL retain the existing session contract. These eager actions SHALL have no server-retained handle. Versioning SHALL NOT be represented as guarding schema/context changes after response; the action captures request-time analysis only.

Malformed parameters, unopened URIs or invalid ranges SHALL return `InvalidParams` (`-32602`). Ineligible configurations SHALL return `[]`. Analysis, traversal, semantic conversion, source-edit and response bounds SHALL remain enforced; requests SHALL allow at most 256 actions and 8 MiB aggregate serialized result. Exhaustion or unexpected preparation/readmission failure SHALL return `RequestFailed` (`-32803`), never a partial/truncated list or stale fallback. Unrepresentable/meaning-changing candidates are the explicit omissions defined above; other internal failures SHALL NOT be disguised as ineligibility. Existing post-shutdown `InvalidRequest` behavior SHALL apply.

#### Scenario: Stale result and accepted changes

- **WHEN** actions for version 7 arrive after the client edits to version 8
- **THEN** the client rejects the version-7 edit; a fresh request after accepted version 8 is analyzed against version 8 without changing server text by itself.

#### Scenario: Failure and bounded success

- **WHEN** a range splits an encoded character, an unopened URI is requested, or result/edit admission exhausts a limit
- **THEN** the request returns the specified error without a usable partial edit, file write or cached-source mutation.
