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
