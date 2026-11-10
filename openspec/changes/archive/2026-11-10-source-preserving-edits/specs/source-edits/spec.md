# Source edits

## Purpose

Change authored DAML configuration without rewriting unrelated source or confusing supplied data with completed defaults.

## ADDED Requirements

### Requirement: Explicit snapshot-relative operations

The library SHALL support replace, insert and remove at RFC 6901 instance paths in a parsed data document. All paths in a batch SHALL resolve against the original snapshot. Mapping lookup SHALL use literal decoded pointer tokens; sequence indices SHALL be canonical nonnegative decimal indices. The empty path SHALL designate the root and permit replacement only. Invalid escapes, absent parents, duplicate mapping insertions and out-of-range indices SHALL fail without implicit parent creation.

#### Scenario: Nested maps and arrays

- **WHEN** a batch removes `/items/0` and replaces `/items/1/name`
- **THEN** both paths refer to the original items, not to shifted indices after removal
- **AND** an invalid operation rejects the entire batch.

#### Scenario: Insertion boundaries

- **WHEN** an insertion names an absent mapping member or an array index from zero through its original length
- **THEN** the new mapping member is appended without reordering existing fields, or the new array value is inserted before the original index, with the length index appending
- **AND** inserting into an empty container is supported, including a tagged implicit empty mapping.

#### Scenario: Root and escaped tokens

- **WHEN** a caller replaces the empty path or addresses a literal slash or tilde using `~1` or `~0`
- **THEN** the selected original node is changed without reinterpreting dots or numeric mapping keys as traversal
- **AND** root insertion, root removal, signed array indices and the `-` pseudo-index are rejected.

### Requirement: Source fidelity and comment ownership

The library SHALL preserve all source bytes outside its reported affected regions, including comments, field order, scalar spellings, whitespace, line endings and assertion text. Replacement SHALL affect the complete selected node, including its tag and comments inside that subtree. Mapping removal SHALL affect the key, key/value separator and selected value; array removal SHALL affect the selected node. Neither removal SHALL consume comments preceding the entry or comments following its value. Comments inside a removed container SHALL be removed with it. Insertion SHALL add source at a container boundary without rewriting existing entries or their trivia.

#### Scenario: Tutorial endpoint edit

- **WHEN** the tutorial endpoint is replaced beside comments, unrelated numeric spellings and omitted defaults
- **THEN** only the selected value region changes
- **AND** adjacent comments, unrelated fields and order, numeric spellings and omitted defaults remain byte-for-byte unchanged.

#### Scenario: Explicit subtree replacement

- **WHEN** a caller replaces a container that includes internal comments
- **THEN** the complete selected subtree is replaced, including those internal comments
- **AND** leading and trailing comments outside its node span remain unchanged.

#### Scenario: Entry removal preserves adjacent comments

- **WHEN** an entry has a preceding own-line comment and a trailing same-line comment
- **THEN** removal leaves both comments and their original source bytes in place
- **AND** it does not attach ownership by indentation or silently remove a neighboring comment.

#### Scenario: Line endings, Unicode and block strings

- **WHEN** source uses CRLF, CR, LF, Unicode, tags or block strings
- **THEN** existing bytes outside affected regions remain exact and reported offsets are UTF-8 byte boundaries
- **AND** generated structural line breaks use the parent container's first physical line-ending style, or LF when it has none
- **AND** multiline replacement payloads retain their admitted semantic value after reparsing.

### Requirement: Shared value and contract admission

The library SHALL accept existing semantic values for inserted and replaced nodes and reuse canonical semantic rendering for those new regions. It SHALL retain exact numeric value, intrinsic temporal/cron kind and tag binding identity. A bound assertion without a representable source spelling in the supplied schema context SHALL fail rather than being stripped. The resulting source SHALL be reparsed under declared resource limits before success. A schema-checked edit SHALL additionally resolve the reparsed result under the selected contract before success; completed defaults SHALL NOT be written into the authored result unless explicitly supplied as an edit value.

#### Scenario: Exact and bound replacement values

- **WHEN** an edit supplies an exact decimal, temporal/cron value or bound tag
- **THEN** readmission retains its exact semantic value and assertion identity
- **AND** an unavailable bound tag fails without returning partial source.

#### Scenario: Validation is not materialization

- **WHEN** an endpoint edit passes a contract that supplies defaults elsewhere
- **THEN** the returned authored document contains only the requested changes, not those completed defaults
- **AND** an invalid edited value rejects the whole operation.

### Requirement: Atomic bounded batches and stale-source rejection

The library SHALL reject conflicting edits, including duplicate targets, ancestor/descendant targets and multiple insertions at the same original boundary. It SHALL admit the batch and its aggregate work/output under explicit finite resource bounds before returning success. A prepared patch SHALL retain a binding to the original source; applying it to different current source SHALL fail before returning edited source. The library SHALL not mutate the original document, write files or return a successful subset after any failure.

#### Scenario: Conflict or exhausted resources

- **WHEN** a batch contains overlapping operations or exceeds its declared operation, work or output budget
- **THEN** the entire batch fails with a structured error
- **AND** no original document or file is modified.

#### Scenario: Stale editor snapshot

- **WHEN** current source differs from the source used to prepare a patch, even only in an unrelated comment
- **THEN** application fails as stale rather than applying old byte offsets
- **AND** the caller must prepare a new patch against the current snapshot.
