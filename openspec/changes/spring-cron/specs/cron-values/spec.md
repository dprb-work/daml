## Purpose

Let configuration authors express validated, typed recurring calendar patterns in an established Spring cron dialect and retain them through DAML's normal value lifecycle.

## ADDED Requirements

### Requirement: Cron literals carry an admitted intrinsic kind

DAML SHALL recognize `!Cron "..."` as an intrinsic value during parsing without schema loading. Cron SHALL remain distinct from ordinary strings and generic assertions. Supported quoted-string decoding SHALL precede cron admission. The intrinsic name SHALL be reserved. An ordinary assertion MAY precede a Cron literal without permitting arbitrary generic assertion stacking. Malformed expressions or payloads SHALL fail at the original source location rather than remain unchecked tags.

#### Scenario: Month-end is typed before resolution
- **WHEN** the parser reads `!Cron "0 0 9 L * *"`
- **THEN** the value has intrinsic kind cron and retains its original source span
- **AND** an ordinary `"0 0 9 L * *"` remains a string

#### Scenario: Invalid patterns cannot survive as tagged strings
- **WHEN** a Cron constructor contains invalid field count, syntax or ranges
- **THEN** source admission fails with a structured cron diagnostic and original-input span

### Requirement: Spring 7.0.10 is the grammar reference

Admission SHALL follow Spring Framework 7.0.10 `CronExpression.parse`, subject to DAML's existing input/scalar/work limits, rather than selecting a custom operator subset. Its expression fields SHALL be seconds, minutes, hours, day-of-month, month and day-of-week. Named month/weekday spellings, numeric bounds, lists, ranges, steps, wildcard aliases and documented macros SHALL retain the reference meanings. Calendar rules SHALL include L, L-n, LW, nW, last-weekday dL and nth-weekday d#n forms. Both day conditions SHALL constrain the same calendar occurrence, not acquire Unix OR behavior.

No year field, Quartz weekday numbering, hash/jitter syntax or invented dialect fallback SHALL be added. Reference-source ambiguities SHALL be resolved with pinned oracle evidence, not implementation convenience. Parsing SHALL NOT calculate occurrences or consult clocks, networks, timezones or business-holiday calendars.

#### Scenario: Calendar-relative rules are available
- **WHEN** patterns use `0 0 9 LW * *`, `0 0 9 15W * *` or `0 0 9 * * FRI#3`
- **THEN** they are admitted with the selected Spring last-weekday, nearest-weekday and third-Friday meanings

#### Scenario: Spring syntax is not a Quartz or Kubernetes hybrid
- **WHEN** an expression has only five fields, adds a seventh year field or uses an unsupported hash or overflowing-hour range
- **THEN** it is rejected consistently with the Spring reference
- **AND** supported Spring macro expressions remain accepted

### Requirement: Retention and equality do not introduce an occurrence engine

Admitted semantic text SHALL follow Spring's trimmed expression and macro-expansion representation. Non-macro expression spelling SHALL remain available without flattening fields into another grammar. Source formatting SHALL retain the authored scalar spelling. DAML Cron equality SHALL compare retained resolved expression text, not claim occurrence-set equivalence or implement Spring Java object identity/equality. Existing ISO fractional equality SHALL not interpret cron expression text.

Source-backed and decoded values SHALL retain the admitting source identity before a trusted view is exposed. Foreign-document nodes SHALL not manufacture validated cron values. Borrowing SHALL not require payload reparsing, copying or whole-tree traversal on each access.

#### Scenario: Source spelling and admitted macro value have separate owners
- **WHEN** `!Cron "@daily"` is parsed and source-formatted
- **THEN** authored macro spelling remains in source formatting
- **AND** semantic export uses the reference's resolved six-field expression

#### Scenario: Foreign source bytes cannot replace admitted expression text
- **WHEN** a caller passes a Cron node from one document to another document's temporal accessor
- **THEN** no trusted Cron value is returned from unrelated source bytes
- **AND** normal access remains valid after moving the owning document

### Requirement: Native constraints and defaults preserve cron identity

Native `!Cron` SHALL constrain intrinsic Cron values; native String SHALL not accept Cron merely because plain JSON projects it to text. Native Cron literals SHALL provide typed constant constraints. Unsupported facets, including offset policy, SHALL fail with definition context. Existing `x-daml-scalar` SHALL accept kind cron and typed constants, and existing `x-daml-default-temporals` path annotations SHALL accept kind cron. Existing names and behavior SHALL remain available; no compatibility aliases or alternate metadata owner SHALL be introduced.

Defaults, copies, replacements, contextual writing/re-admission, semantic snapshots and staged summaries SHALL retain cron kind, expression text, assertion identity, provenance and applicable budgets. Plain JSON SHALL project to expression strings with documented kind loss; semantic representation SHALL retain kind cron and value text. Raw cron in a JSON-compatible schema-module envelope SHALL not silently downgrade to text.

#### Scenario: A cron default remains typed through stages and editing
- **WHEN** a native preset supplies an intrinsic Cron field that input omits
- **THEN** completion yields the same admitted cron kind/text with default provenance
- **AND** staged summaries, copy/replace and contextual write/re-admission retain that identity

### Requirement: Host bindings expose cron without hidden dependencies

Python parsed/resolved `to_python()` SHALL return frozen `daml.Cron` values. Construction SHALL admit the selected grammar and expose read-only expression text, string/repr and value equality. No ISO formatting or standard-library datetime conversion SHALL be invented for recurrence patterns. Base imports SHALL not require optional Pydantic or cron libraries. Existing type-derivation behavior SHALL remain unchanged; no implicit mapping of third-party cron classes or new custom hooks is part of this capability.

Python semantic dictionaries and browser semantic/snapshot/summary representations SHALL retain discriminator cron and expression text. All producing and consuming boundaries SHALL enforce the same admitted core contract, with no independent binding parser or Date coercion.

#### Scenario: Host export retains a calendar pattern
- **WHEN** parsed or default-completed Cron values are exported to Python or browser semantic values
- **THEN** dedicated intrinsic identity and the complete expression remain available
- **AND** plain strings are not promoted merely because they resemble cron patterns

### Requirement: Compatibility is verified and documented

The delivered capability SHALL include independent pinned-reference admission evidence, behavioral checks of documented Spring grammar and invalid boundaries, real native/Python/browser examples, lifecycle/default/staged/source-identity coverage, maintained syntax/API/schema references and editor classification. Documentation SHALL name the Spring reference and distinguish a pattern value from scheduler/timezone/DST execution policy. No Java or Spring runtime dependency SHALL enter production packages.

#### Scenario: Real consumers exercise the selected dialect
- **WHEN** contributors run the native and Python examples and browser verification scenario
- **THEN** calendar-relative patterns, typed defaults, semantic round trips and meaningful invalid-pattern diagnostics are observed through actual core and binding paths
