## MODIFIED Requirements

### Requirement: Explicit configuration admission

The source-built `daml` executable SHALL provide `check`, `resolve`, `explain`, and `edit`. Each operation SHALL read one bounded UTF-8 data document from `FILE`, `-`, or omitted FILE (stdin), compile/select its schema context, and perform normal default completion and strict validation before reporting success. For edit, those schema obligations SHALL apply to the complete edited document; a syntactically valid but initially schema-invalid document MAY be repaired. A root assertion or `--expected NAME` SHALL select a root contract; an unasserted final root without an expected contract SHALL be rejected rather than reported as schema-validated.

`--bundle DIR` SHALL select a native bundle. Without explicit context, file input SHALL use existing document-relative nearest-manifest discovery. Stdin SHALL require explicit context. Repeatable `--module FILE` SHALL compose strict JSON interchange envelopes using the existing module API; it SHALL NOT be combined with `--bundle`. An expected root and a different explicit assertion SHALL both remain obligations. Modules SHALL NOT fetch referenced resources. For edit, native context compilation SHALL admit assertions introduced by the resulting authored document.

#### Scenario: Native provider defaults

- **WHEN** the tutorial provider document is checked or resolved with its native bundle
- **THEN** admission succeeds with the supplied endpoint, default timeout 30, and default retries 3.

#### Scenario: Missing or conflicting context

- **WHEN** no schema context/root contract can be selected, or module and bundle options are combined
- **THEN** the command fails explicitly rather than accepting syntax alone or choosing an unrelated cwd bundle.

#### Scenario: Recursive schema modules

- **WHEN** explicit modules contain a finite structurally recursive contract and supplied data
- **THEN** all operations use the existing recursive semantics, accepting valid finite data and rejecting invalid descendants, nonprogressing schema cycles, or excessive work without recursion truncation.

#### Scenario: Repair and newly introduced assertions

- **WHEN** an edit repairs a schema-invalid value or introduces an assertion exported by the selected native bundle
- **THEN** the final edited document is compiled and validated against that context before output
- **AND** an invalid final value, unknown assertion or conflicting root obligation rejects the entire result.

## ADDED Requirements

### Requirement: Explicit source-editing CLI consumer

`daml edit --edits FILE` SHALL apply the source-editing library to the configuration input and return its admitted authored source on stdout without overwriting any input. The edits file SHALL be an untagged DAML sequence of untagged operation mappings. Each mapping SHALL contain untagged string fields `op` and `path`; `replace` and `insert` SHALL require a normal DAML `value`, while `remove` SHALL forbid it. Unknown operations, unknown/missing fields and malformed batches SHALL fail. Existing duplicate-key admission SHALL remain authoritative.

`--edits` SHALL be required exactly once for edit and forbidden for other commands. `--edits -` SHALL fail as a usage error because stdin is reserved for the configuration document. Existing `--format` SHALL remain resolve-only and `--path` SHALL remain explain-only. No implicit in-place writing, LSP command, binding export or native-definition refactoring is added.

The edit source SHALL have an 8 MiB input bound, and core operation/work/output limits SHALL also apply. All paths SHALL refer to one original configuration snapshot. The CLI SHALL resolve the complete edited document for validation but SHALL output the authored edited source, not the resolved/default-completed tree. All successful output SHALL be ready and bounded before stdout writes. Batch/path/value/schema/resource errors SHALL exit 1 with no successful stdout; malformed invocation and I/O errors SHALL exit 2. Existing operations and their read-only guarantees SHALL remain unchanged.

#### Scenario: Direct comment-preserving invocation

- **WHEN** a caller supplies an endpoint replacement through `--edits FILE`
- **THEN** stdout contains only the edited authored configuration, preserving outside-region comments, field order, number spellings, line endings and omitted defaults
- **AND** configuration, edits, schema and bundle files remain unchanged.

#### Scenario: Atomic original-snapshot operations

- **WHEN** a batch removes an original array item and edits a different original item, or contains a conflicting or invalid operation
- **THEN** successful operations retain original-index meaning, and any failure rejects the whole result rather than emitting a subset.

#### Scenario: Input stream and flag boundaries

- **WHEN** configuration comes from stdin with explicit context, or edits input is missing, repeated, invalid, oversized or unreadable
- **THEN** valid stdin configuration is editable without a second stdin reader, and each invalid case reports its declared usage, I/O or admission failure without mutating files.
