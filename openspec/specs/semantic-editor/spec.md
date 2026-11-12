# semantic-editor Specification

## Purpose
Author DAML configuration with the existing structural contracts, defaults, and native source identities in an LSP editor.

## Requirements

### Requirement: Explicit document context preserves syntax-only operation

The server SHALL advertise completion, hover, definition, and execute-command providers alongside existing synchronization and formatting. `workspace/executeCommand` command `daml.setContext` SHALL accept one argument object with `uri`, `kind` (`configuration`, `definition`, or `syntax`), optional `bundleRoot` (absolute local file URI), and optional `expected` (configuration root name). Unknown fields/kinds, invalid option combinations, nonlocal bundle URIs, and unopened documents SHALL fail explicitly without selecting a fallback context.

Accepted context changes SHALL return `{generation: number}` with a session-monotonic, nonwrapping unsigned 32-bit generation. Invalid changes SHALL preserve the previous context and generation. Exhaustion SHALL fail explicitly rather than reusing an older identity.

Unselected documents SHALL retain syntax-only behavior without filesystem/schema discovery. A configuration context SHALL use its explicitly selected native bundle, or existing nearest-ancestor discovery from its local document path. Untitled configuration SHALL require an explicit bundle. Native definition context SHALL require a local file identity, derive its `self` name within the selected/discovered bundle, and validate its definition rather than treating it as application data. Cwd and editor workspace roots SHALL NOT choose schemas or expected types.

#### Scenario: Explicit configuration context

- **WHEN** a caller selects configuration context for the tutorial provider document
- **THEN** the existing bundle compiler and resolver validate it, preserving both an expected root and any explicit assertions.

#### Scenario: No or invalid context

- **WHEN** a document has no context, an untitled configuration has no bundle, or a caller supplies an invalid context
- **THEN** syntax formatting/diagnostics remain available, and the server does not pretend semantic validation occurred or load an unrelated bundle.

### Requirement: Semantic diagnostics retain source and version

Complete configuration text SHALL use normal native compilation, completion, and strict validation. Unknown references/assertions, invalid values, ambiguous/default/schema failures, and resource failures SHALL retain their engine meaning. Supplied values SHALL use actual byte spans converted through the negotiated LSP encoding. Referenced-definition failures SHALL identify their actual source URI/range when available, with related context rather than fabricated input positions.

All accepted open definition/manifest text SHALL override disk text for semantic analysis. A rejected over-limit open version SHALL NOT silently fall back to an older disk definition. Accepted document changes, closes, and `workspace/didChangeWatchedFiles` notifications SHALL invalidate dependent analysis; returning a definition buffer to disk SHALL clear stale errors. Publish versioned diagnostics for open documents and clear no-longer-applicable diagnostics. Existing version, close/reopen, Unicode, and synchronous ordering guarantees SHALL remain intact.

#### Scenario: Invalid supplied value recovers

- **WHEN** the endpoint is supplied with the wrong type and then corrected in a newer document version
- **THEN** a diagnostic points at the supplied value, and the correction clears it for the accepted version.

#### Scenario: Unsaved and saved dependency changes

- **WHEN** a referenced definition is changed in an open unsaved buffer, or a watched imported file changes on disk
- **THEN** its current constraints affect the dependent configuration without requiring that configuration to be edited, and stale results cannot overwrite newer accepted state.

### Requirement: Useful bounded completion on incomplete authoring

Field completion SHALL use the active contract and current instance path, include required/optional information and declared defaults when known, avoid already supplied known fields, and not invent arbitrary fields in closed records. Visible assertion/reference completion SHALL respect the entry/native file's real bundle scope, including native built-ins; dependency-private aliases SHALL not leak into entry-visible suggestions. Name enumeration SHALL be bounded and SHALL not parse unreferenced definition bodies or claim to enumerate all structurally compatible providers.

Cursor context SHALL be reported through the existing parser's grammar. An unfinished key, assertion, or container SHALL be usable for suggestions where its prefix context is established; strings/comments SHALL not receive unrelated structural suggestions. Hints derived from an incomplete buffer SHALL NOT replace its syntax diagnostic or certify successful data admission.

#### Scenario: Field and tag prefixes

- **WHEN** the author types a worker/provider field prefix or `!providers.` in its active context
- **THEN** completion provides the relevant visible fields/names with correct replacement ranges, without dropping supplied data or weakening closed constraints.

#### Scenario: Incomplete or malformed buffer

- **WHEN** a buffer is incomplete near the cursor, or malformed before the cursor prevents a reliable context
- **THEN** the server returns grounded hints where available or an empty result, retains syntax feedback, and never presents a repaired/placeholder configuration as validated.

### Requirement: Hover and definition use admitted contracts and real sources

Hover SHALL describe applicable type constraints and declared defaults. For fully checked configuration, it SHALL reuse checked explanation/value origins, distinguishing supplied input from defaulted values. For incomplete or invalid data it MAY show clearly labeled schema hints without claiming a checked value.

Definition navigation SHALL resolve assertion/reference names in their actual scope and configuration members to actual definition source locations where available. Built-ins without source locations SHALL return no fabricated target. Source metadata SHALL be produced/retained by native compilation or its source AST, not by decoding private schema IDs or maintaining a second grammar/validator.

#### Scenario: Tutorial authoring journey

- **WHEN** an author inspects the tutorial adapter endpoint/timeout and follows a field or tag definition
- **THEN** hover shows the relevant constraints/defaults and available provenance, and navigation opens the actual defining file/range.

### Requirement: Dependency watching respects trust and lifecycle

The server SHALL send `daml/dependencies` notifications containing `uri`, accepted `version`, acknowledged context `generation`, and bounded local-file `files` URIs for selected contexts. This is watch metadata, not schema-fetch authority. The trusted VS Code client SHALL install and remove shared file watchers for these actual dependencies, including local imports outside the workspace, and forward changes through standard watched-file notifications. Closed, older-version, or mismatched-generation notifications SHALL not recreate stale watchers, including same-URI close/reopen and context switches without a text-version change. Untrusted workspaces SHALL not start the server or enable semantic filesystem operations; existing machine-scoped executable selection SHALL remain protected.

#### Scenario: Real editor and cleanup

- **WHEN** a trusted editor selects context, requests completion/hover/navigation, changes an imported definition, and closes/disposes its documents/client
- **THEN** the actual server supplies the requested semantic behavior and watcher ownership is released without restarting or using an untrusted executable.

### Requirement: Existing bounds and consumers remain intact

Syntax, native compilation, exact numbers, structural recursion, completion, explanation, and output limits SHALL remain enforced. Overlay tables, source indexes, name enumeration, hints, watcher metadata, and semantic results SHALL be bounded before amplification. Ordinary non-editor library/CLI compilation SHALL not allocate editor metadata. Files SHALL not be mutated, resources SHALL not be fetched remotely, and no application constructors SHALL run.

#### Scenario: CLI/editor parity

- **WHEN** complete data and the same native context are admitted through CLI/library and editor analysis
- **THEN** they agree on validity, completed defaults, and assertion obligations, subject only to the editor's explicitly documented additional analysis bounds.

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
