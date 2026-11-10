# Spec Delta

## Purpose

Enable Python applications to change supplied DAML source through the existing Rust editor without rewriting unrelated bytes or confusing validation with default materialization.

## ADDED Requirements

### Requirement: One-shot immutable Python editing

`Document.edit(edits, line_width=80)` SHALL apply the parse-only core source editor and return a new immutable `Document`. `Schemas.edit(document, edits, expected=None, line_width=80)` SHALL apply the contextual core editor and return a new immutable `Document`. Both SHALL leave original documents, operation records and supplied semantic values unchanged, including after failure. Empty batches SHALL return a new document with exact original source. No borrowed patch SHALL survive across Python calls, and neither method SHALL write files, invoke the CLI, fetch schemas or construct application models.

The methods SHALL retain core source-editing semantics: explicit original-snapshot RFC6901 paths, original array indices, root replacement only, no implicit parent creation or targeting omitted defaults, and rejection of logical/byte conflicts including coincident insertions. Existing bytes and adjacent comments outside target regions SHALL remain exact; replacement/removal SHALL own subtree-internal comments under the core policy. Existing BOM, numeric spellings, Unicode and line-ending behavior SHALL remain authoritative.

#### Scenario: Endpoint replacement beside comments

- **WHEN** Python replaces one supplied endpoint beside outside-region comments and untouched values
- **THEN** the returned document contains only the requested change and retains unrelated bytes
- **AND** the original document and input operation/value dictionaries remain unchanged.

#### Scenario: Original-index batch and atomic rejection

- **WHEN** a batch removes one array element and changes another original element, or contains a bad target/conflict
- **THEN** successful edits retain original-index meaning, and any failure returns no partial document or mutation.

#### Scenario: Empty batch and source boundaries

- **WHEN** a valid empty list/tuple or operations involving root, escaped keys, empty/tagged containers, BOM and mixed line endings are supplied
- **THEN** the new document obeys existing core source-fidelity and target rules without whole-document semantic rewriting.

### Requirement: Strict existing semantic input representation

The required edits argument SHALL be an actual Python list or tuple of dictionaries, not `None`, a generator or an arbitrary iterable. Replace/insert records SHALL contain only `op`, `path` and required `value`; remove SHALL contain only `op` and `path` and SHALL reject `value` even when it is `None`. Operation and path SHALL be strings; supported operations are exactly replace, insert and remove. Existing semantic-node/tag records SHALL encode supplied values. Ordinary Python values SHALL NOT be silently converted into those records, and stage-only copy/expect operations SHALL remain unsupported here.

Admission SHALL reuse the existing semantic helpers for strict fields, Unicode, cycles, scalar/depth/node bounds and exact numeric/intrinsic/tag representations. The binding SHALL snapshot only new operation data into owned Rust values before releasing the GIL; it SHALL NOT clone or convert the original configuration into a semantic tree. Existing canonical writing and stage interfaces SHALL remain unchanged.

#### Scenario: Exact and intrinsic values

- **WHEN** an operation supplies an exact number or Date/Time/DateTime/Cron semantic record
- **THEN** the reparsed authored result retains the admitted value, real/integer distinction and intrinsic kind without lossy Python coercion.

#### Scenario: Malformed transport and hostile shapes

- **WHEN** input contains unknown/missing fields, a remove value, unsupported operation, lossy/non-string numeric carrier, invalid Unicode, cyclic semantic data or an unsupported iterable
- **THEN** the existing binding-admission failure is raised before any successful core result, without mutating caller objects or executing an edit subset.

### Requirement: Existing contextual validation without source materialization

`Schemas.edit` SHALL use its existing immutable registry for source-representable assertion bindings and final resolution. It SHALL NOT initially resolve the original document, so a schema-invalid supplied value MAY be repaired. Optional expected-contract semantics SHALL match the existing registry/core behavior and remain additional to explicit assertions. Registry names SHALL already be admitted; no implicit bundle reopening, discovery or new resource fetch is added.

The complete edited output SHALL be parsed and, for checked edits, resolved before success. Completed defaults SHALL NOT be serialized into authored output unless explicitly supplied as an edit value. A bound assertion without a representable source name in the selected registry SHALL fail without stripping or rebinding its identity. `Document.edit` SHALL not pretend to perform schema validation.

#### Scenario: In-memory Python-derived schema

- **WHEN** Python derives/composes a schema registry and edits a parsed document under an expected contract
- **THEN** the final document is admitted under that contract while omitted defaults remain omitted from its source
- **AND** an invalid final value rejects the whole edit without changing the original.

#### Scenario: Repair and contract conflict

- **WHEN** a syntactically valid but initially schema-invalid value is repaired, or an expected contract conflicts with an explicit assertion
- **THEN** the repair may succeed, while conflicting final obligations fail rather than being waived.

#### Scenario: Bound source representation

- **WHEN** a supplied value retains a native assertion binding with a visible source name, or carries a private/unrepresentable binding
- **THEN** the contextual result retains identity for the visible case and rejects the unrepresentable case
- **AND** parse-only editing does not silently strip bound metadata.

### Requirement: Bounded admission and public Python errors

Actual batch length SHALL be checked against the core default operation count before request-storage allocation or element conversion. One shared existing semantic budget SHALL cover batch metadata, paths, expected name and all new values; it SHALL NOT reset per operation. The core default operation/work/output/parser bounds and independent renderer/resolver bounds SHALL remain active. No sentinel SHALL disable admission, and no new Python resource-tuning class is introduced.

Core failures SHALL expose public `SourceEditError(ValueError)` with `kind`, `path` and `operations`. Pointer/missing-path/invalid-target variants SHALL retain their path; conflicts SHALL retain the original zero-based operation pair; absent metadata SHALL be `None`. Wrapped parse, semantic-renderer and resolver failures SHALL preserve existing public typed exceptions in `__cause__`. Binding-record/value/Unicode/budget failures SHALL retain existing `SemanticError` conventions, and PyO3 argument conversion SHALL retain existing built-in exception behavior. No private exception class or error-text classification SHALL replace structured causes.

#### Scenario: Excess operations before conversion

- **WHEN** an oversized list/tuple contains elements that would independently fail conversion
- **THEN** the operation-count failure occurs before converting those elements.

#### Scenario: Cumulative values and structured failures

- **WHEN** individually admissible values exhaust the shared batch budget, or a core pointer/conflict/render/parse/resolve failure occurs
- **THEN** the complete call fails in its declared public error family with preserved metadata/typed cause, not a successful subset or rewritten default tree.
