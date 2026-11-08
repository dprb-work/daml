## Purpose

Let authors write readable native Regex constraints using DAML's shared multiline layout while preserving raw pattern semantics, source provenance and bounded validation.

## ADDED Requirements

### Requirement: One owner scans quoted boundaries and layout

The shared string module SHALL own quoted payload boundaries and multiline layout for normal strings, typed temporal payloads and native Regex. Native Regex SHALL select a concrete raw-text policy at that boundary; it SHALL NOT retain an independent scanner or parser fallback. Normal decoded strings SHALL keep their existing escapes, continuations, source identity and resource behavior.

#### Scenario: Ordinary typed strings do not acquire Regex decoding
- **WHEN** ordinary tagged strings or Date/Time/DateTime constructors use supported quoted forms
- **THEN** their decoded bytes and intrinsic admission remain unchanged
- **AND** raw Regex policy does not leak into ordinary data `!Regex`, which remains a normal assertion tag

### Requirement: Existing single-line Regex remains raw

Native `!Regex "..."` SHALL retain its current raw, single-line meaning. Backslashes SHALL pass unchanged to the Regex engine, including doubled backslashes; JSON escape decoding SHALL NOT apply. The existing quote boundary, unsupported controls/BOM/newline behavior, original error spans and payload-only scalar-size boundary SHALL remain compatible. Regex SHALL remain valid only in native `String.pattern`, not become an instance-data intrinsic.

#### Scenario: Backslashes retain their pattern meaning
- **WHEN** native single-line patterns contain `\d`, `\p{Greek}` or doubled backslashes
- **THEN** the compiled pattern receives those bytes unchanged
- **AND** formatting and reparsing preserve actual full-string matching

### Requirement: Regex supports both shared triple forms

Native Regex SHALL accept triple-double and triple-single quoted payloads. The opener SHALL follow the constructor on the same physical line, followed by permitted horizontal whitespace and a line ending; closing delimiters SHALL follow the shared own-line and escaped-delimiter parity rules. Boundary line endings SHALL be excluded and content LF/CRLF/CR normalized to LF. Triple-double bodies SHALL use common mandatory dedent and blank-body normalization, preserving internal line breaks. Triple-single bodies SHALL use common trim/discard-blank/fold-to-one-space rules.

All Regex forms SHALL preserve raw pattern backslashes. There SHALL be no collapse of doubled backslashes, JSON unescape or DAML backslash-newline continuation. Recognizing an escaped triple delimiter SHALL NOT consume its escape backslash from raw Regex text. Layout normalization is the only pattern transformation.

#### Scenario: Verbose multiline patterns are readable
- **WHEN** a triple-double pattern contains `(?x)`, grouped expressions and `#` comments, including a final-line comment
- **THEN** source parsing, compilation and full-string validation succeed according to the existing Rust Regex engine
- **AND** the flag remains explicit pattern data

#### Scenario: Literal and folded layouts are not interchangeable
- **WHEN** an unflagged triple-double pattern contains a physical line break
- **THEN** that line break remains meaningful Regex content
- **WHEN** a triple-single pattern has two nonblank content lines
- **THEN** one space joins them, with raw backslashes otherwise unchanged

#### Scenario: Regex does not inherit string continuation
- **WHEN** a raw Regex body has a backslash immediately before a physical line ending
- **THEN** the backslash and normalized line break reach Regex admission as pattern content
- **AND** ordinary triple-double string continuation behavior remains unchanged

### Requirement: Flags and matching remain engine-owned

Inline flags SHALL remain inside the pattern, including `(?x)` for verbose whitespace/comments and existing other supported flags. DAML SHALL NOT implicitly enable flags or add a separate flags object. Existing bounded Rust Regex syntax and full-string matching SHALL remain authoritative; there SHALL be no new Regex dialect, substring-match fallback or string-anchor wrapper.

#### Scenario: Formatting whitespace is opt-in
- **WHEN** patterns have identical visible spacing but only one enables verbose mode
- **THEN** their matching behavior differs according to the engine's flag rules
- **AND** folding a verbose line comment into one line is not silently repaired by DAML

### Requirement: Compilation and formatting consume admitted text with source provenance

Native storage SHALL retain borrowed or owned pattern text plus the original literal range needed for formatting and diagnostics. Compilation SHALL consume admitted text without reconstructive parsing or silent empty defaults. Error origins SHALL refer to original UTF-8 source. Native formatting SHALL reuse shared literal/block rendering, preserve raw payload spelling and comments, and avoid quote reconstruction or fixed one-quote offset assumptions. Formatting at different widths/depths and reparsing SHALL preserve actual matching and be idempotent.

#### Scenario: A multiline pattern survives source formatting
- **WHEN** a native definition containing a raw multiline pattern, neighboring comments or a syntax-only default is formatted and reparsed
- **THEN** the pattern value, surrounding structure and matching semantics remain unchanged
- **AND** all diagnostics continue to address original input ranges rather than decoded-buffer positions

### Requirement: Resource bounds and ownership remain explicit

Ordinary string and typed temporal limits SHALL remain unchanged. Existing single-line Regex payload-only scalar limits SHALL remain compatible. New triple Regex quoted-source tokens and normalized patterns SHALL remain bounded like other triple forms, with checked arithmetic and UTF-8-safe spans. Unchanged text SHOULD remain source-backed; normalization SHALL NOT require a duplicate parse, whole-tree scan, extra payload cache or unnecessary copy.

#### Scenario: Small scalar limits cannot be bypassed by layout
- **WHEN** a raw multiline body uses indentation, blank lines, escaped delimiters or non-ASCII content near configured limits
- **THEN** bounds are enforced before excessive allocation and reported spans remain valid UTF-8 boundaries
- **AND** legacy single-line and normal string boundary checks remain valid
