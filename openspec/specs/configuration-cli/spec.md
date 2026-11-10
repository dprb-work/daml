# configuration-cli Specification

## Purpose
Validate configuration, export completed values, and explain values and constraints through the existing DAML engine.

## Requirements

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

### Requirement: Checked exact output

`check` SHALL write no successful data to stdout. `resolve` SHALL write completed canonical DAML by default, retaining exact numbers, intrinsic scalar kinds, and representable assertion meaning through contextual semantic writing. `resolve --format json` SHALL explicitly select the documented lossy plain-data projection: numbers remain exact decimal JSON tokens, while assertions are omitted and intrinsic temporal/cron kinds become strings. There SHALL be no silent fallback from DAML to JSON when an assertion is not source-representable.

#### Scenario: No hidden coercion

- **WHEN** supplied data includes large integers, exact fractional numbers, temporal/cron values, or assertions
- **THEN** the selected output follows its declared representation and never rounds numbers through floating point.

#### Scenario: Invalid or ambiguous data

- **WHEN** supplied data violates a constraint, asserts an unknown name, or has ambiguous completion
- **THEN** every operation fails before emitting a successful result, preserving the engine path and available input/default/schema provenance.

### Requirement: Source-backed explanation

`explain --path POINTER` SHALL output a JSON report for a checked resolved value at an RFC 6901 instance pointer; omitted `--path` SHALL select the root with the empty pointer. The report SHALL contain `path`, an exact semantic-tree `value`, its `origin`, optional `tagOrigin`, and `constraints`. Input origins SHALL identify UTF-8 byte spans and the actual input file when present; default origins SHALL identify the actual schema resource and pointer, never fabricated input coordinates.

Each constraint entry SHALL identify its instance path, schema location, and admitted schema. The explanation SHALL include direct contracts and enclosing contracts that constrain the selected value. References SHALL retain registry identity and remain references rather than being infinitely expanded. Branch descriptions SHALL NOT claim an unselected/rejected alternative is an unconditional applied constraint. Applicable validation-only branches and ancestor obligations SHALL not silently disappear because they supplied no defaults.

#### Scenario: Supplied and defaulted values

- **WHEN** `/adapter/endpoint` and `/adapter/timeout` are explained in the tutorial provider document
- **THEN** the endpoint identifies its supplied source and timeout identifies its declared default, with their applicable constraint locations.

#### Scenario: Root, escaped, and missing paths

- **WHEN** the pointer is empty, contains escaped slash/tilde segments, names a sequence item, or is malformed/absent
- **THEN** it selects exactly the RFC 6901 target or fails explicitly; dotted/slashed literal keys are not guessed as traversal.

#### Scenario: Recursive explanation

- **WHEN** a descendant in a finite recursive document is explained
- **THEN** the report identifies its real contracts and origins, terminates within bounded work/output, and has the same admission result as normal resolution.

### Requirement: Predictable failure and read-only operation

Commands SHALL exit 0 on success, 1 for configuration/schema/admission/path/output-representability failures, and 2 for usage, read, or write failures. Diagnostics SHALL go to stderr and SHALL retain structured engine context in human-readable form; stdout SHALL contain only successful operation output. Help SHALL describe input, schema selection, output loss, and exit behavior. Invalid options SHALL fail before document I/O. No operation SHALL mutate input/configuration files, fetch resources, interpolate environment values, or execute application constructors. Existing parser, schema, resolution, and exact-number limits SHALL remain in force; explanation work and output SHALL be bounded too.

#### Scenario: Shell and CI use

- **WHEN** a valid configuration, invalid value, malformed invocation, or missing input file is processed
- **THEN** the command returns the corresponding status, separates result output from diagnostics, and leaves source files unchanged.

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
