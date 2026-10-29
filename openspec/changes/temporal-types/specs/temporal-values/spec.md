## ADDED Requirements

### Requirement: ISO-profile constructors create first-class values

DAML SHALL recognize `!Date "..."`, `!Time "..."` and `!DateTime "..."` as intrinsic temporal literals during data parsing, without loading schemas. Their values SHALL be distinct from ordinary strings and generic assertion tags. The existing supported quoted-string decoding SHALL apply before temporal admission. Ordinary date-looking strings SHALL remain strings. Constructor names SHALL be reserved intrinsic names.

An ordinary assertion MAY apply to an intrinsic temporal literal, for example `!app.Deadline !DateTime "2026-10-29T09:30:00Z"`. This SHALL NOT enable arbitrary stacking of generic assertions. Bare Date/Time/DateTime constructors in native definitions SHALL constrain corresponding temporal kinds rather than construct empty objects.

#### Scenario: A typed date is available before resolution
- **WHEN** the parser reads `!Date "2024-02-29"`
- **THEN** its value has the date kind, retains its source location and exposes the admitted ISO value without schema lookup
- **AND** an ordinary `"2024-02-29"` remains an ordinary string

#### Scenario: Invalid dates do not become tagged strings
- **WHEN** a temporal literal has an impossible date, invalid clock/offset, missing quoted payload or incompatible payload kind
- **THEN** parsing fails with an original-input temporal syntax error rather than retaining an unchecked generic tagged payload

### Requirement: One documented extended-calendar ISO profile owns admission

The initial profile SHALL use ISO 8601-1:2019 extended calendar/clock notation, with the relevant Amd 1:2022 end-of-day and leap-second corrections. It SHALL document its limits rather than claim exhaustive ISO 8601 conformance.

Dates SHALL use complete Gregorian calendar dates with four year digits, optionally preceded by a minus for a nonzero pre-year-zero year. Supported years are -9999 through 9999, including year zero. Month/day validity and Gregorian leap years SHALL be enforced. Expanded years, week/ordinal dates, partial dates, numeric epochs, named zones, other calendars and locale/relative text are outside this initial profile.

Times SHALL accept `HH`, `HH:MM` or `HH:MM:SS`, with an optional leading uppercase T for standalone Time. A decimal fraction MAY follow seconds and SHALL contain at least one ASCII digit; dot and comma input signs are accepted. Fractions on hours or minutes and compact/basic clock notation are outside this profile. DateTime SHALL combine a supported complete date, uppercase T and a supported clock.

Time and DateTime MAY have an explicit uppercase Z, `+HH`, `-HH`, `+HH:MM` or `-HH:MM` offset. Offset hour/minute fields SHALL be bounded by 23/59; negative zero offsets SHALL be rejected. No offset SHALL remain absent, not become UTC or a machine-local timezone. ISO policy and product-profile choices SHALL be distinguished in maintained documentation.

#### Scenario: Omitted lower-order clock fields use the user-selected zero fill
- **WHEN** `!Time "09:30"` or `!DateTime "2026-10-29T09"` is admitted
- **THEN** its canonical clock is respectively `09:30:00` or `09:00:00`
- **AND** no offset is invented

#### Scenario: Decimal and offset spelling have one canonical output
- **WHEN** `!Time "09:30:00,123456789+01"` is admitted
- **THEN** its canonical value is `09:30:00.123456789+01:00`
- **AND** all supplied fractional digits remain available

### Requirement: Precision and ISO exceptional labels survive without silent repair

Every supplied fractional-second digit SHALL be retained, subject only to existing scalar/input bounds. Admission SHALL NOT round or truncate through a floating-point or fixed-microsecond/nanosecond representation. Canonical value writing SHALL use full clock fields, uppercase T/Z, extended separators and a dot fraction, retaining supplied fractional digits. Zero numeric offsets SHALL canonically use Z; absent offsets remain absent. Source formatting SHALL retain the authored scalar spelling rather than replace it with canonical semantic output.

End-of-day hour 24 SHALL be accepted only with zero lower-order components and an absent/all-zero fraction; canonical output SHALL retain `24:00:00` on the authored date, not silently roll it into another date. Leap-second label 60 SHALL be preserved. With an explicit offset its corresponding UTC clock minute SHALL be 23:59; without an offset it remains an unresolved local leap-second label. The parser SHALL NOT consult clocks, networks or IERS tables or claim to establish actual historical/future leap-second occurrence. This representational limit SHALL be explicit. Temporal arithmetic and instant ordering are not provided by this change.

#### Scenario: More than nanosecond precision is preserved
- **WHEN** `!Time "09:30:00.123456789123"` is parsed, resolved, copied, exported as a semantic value and written
- **THEN** no fractional digit is discarded or rounded

#### Scenario: End of day remains an end-of-day label
- **WHEN** `!DateTime "2026-10-29T24:00:00.000"` is admitted
- **THEN** its date and 24-hour label remain present
- **AND** `24:01`, `24:00:01` and `24:00:00.001` fail

#### Scenario: A leap-second label is not normalized away
- **WHEN** `!DateTime "2016-12-31T23:59:60.5Z"` is admitted
- **THEN** its second remains 60 and its fraction remains .5
- **AND** an explicitly offset leap label at an incompatible UTC clock minute fails

### Requirement: Native constraints enforce intrinsic kinds and offset policy

Native `!Date`, `!Time` and `!DateTime` SHALL require the corresponding first-class kind. Native `!String` and ordinary string literal constraints SHALL not accept a temporal value merely because its JSON projection is text. Time and DateTime SHALL support native metadata `offset: "optional"`, `offset: "required"` or `offset: "forbidden"`; optional is the default. Other values, Date offset metadata and unsupported temporal facets SHALL fail with existing definition-error context. Existing composition/default/assertion rules SHALL continue to apply.

The low-level schema bridge SHALL expose one documented `x-daml-scalar` constraint with kind string/date/time/datetime, optional temporal offset policy, and a typed temporal constant when needed. This checks actual DAML kind rather than interpreting application discriminator fields. Ordinary JSON Schema string constraints operate on the explicit ISO-text projection; they do not construct a temporal value.

#### Scenario: A deadline requires an explicit offset
- **WHEN** a field is constrained by `!DateTime { offset: "required" }`
- **THEN** a matching intrinsic datetime with Z or a numeric offset passes
- **AND** an offset-free datetime, wrong temporal kind or ordinary string fails at the field path

### Requirement: Defaults and all value paths retain temporal identity

Parsed, resolved, semantic/editable and staged-summary values SHALL expose temporal kind and exact ISO content. Native defaults and JSON-interchange defaults SHALL preserve temporal identity via explicit `x-daml-default-temporals` metadata, with entries identifying default-relative path and date/time/datetime kind. Malformed metadata, missing/non-string targets, incompatible kinds and invalid temporal content SHALL fail at the responsible admission boundary. Literal instance data with similarly named properties SHALL remain ordinary data.

Scalar byte/work budgets, source/default/semantic provenance, tag binding, replacement/copy semantics and final validation SHALL cover temporal values. Temporal semantic equality SHALL distinguish kinds, absent/present offsets and local calendar/clock components; numerically equal fractional seconds SHALL compare equal despite trailing zeros. No implicit UTC or end-of-day rollover comparison SHALL erase local representation intent. Staged exact-payload comparison SHALL retain representation/precision changes as it does for exact numbers.

#### Scenario: A declared temporal default is not downgraded to text
- **WHEN** a containing native preset supplies an intrinsic date/time/datetime and the field is omitted
- **THEN** completion creates the same temporal kind, exact fraction and offset presence with default provenance
- **AND** semantic snapshots, stage summaries and copy/replace/write/re-admission retain that kind

### Requirement: JSON projection and host-native conversions are explicit boundaries

Plain JSON export SHALL encode temporal values as canonical ISO strings and document that this projection loses intrinsic kind. The editable semantic representation SHALL retain kind date/time/datetime and canonical ISO text. No host Date or datetime conversion SHALL be used as an implicit parser, storage type or precision limit.

Python `to_python()` SHALL return immutable DAML Date, Time and DateTime objects with canonical `isoformat()` text and explicit exact standard-library conversions (`to_date`, `to_time`, `to_datetime`). Nonrepresentable years, leap/end-of-day labels or nonzero sub-microsecond precision SHALL raise a public conversion error rather than truncate or normalize. The original DAML value SHALL remain unchanged. Browser semantic/snapshot/summary unions SHALL carry the same discriminated temporal kind and ISO text, with no JavaScript Date coercion. Existing non-temporal exports SHALL remain compatible.

#### Scenario: Python does not discard precise or exceptional values
- **WHEN** a temporal value beyond standard-library datetime representation is exported to Python
- **THEN** the dedicated DAML value and its complete ISO text are available
- **AND** an explicit non-exact standard-library conversion fails rather than silently dropping information

### Requirement: The delivered profile is exercised and documented

The change SHALL include real native and Python examples, documentation of the ISO-based grammar and limits, exact conversion behavior, offset policy, JSON projection and supported Python derivation. Checks SHALL discriminate invalid calendar/clock/offset input, leap/end-of-day handling, precision, constructors versus assertions, defaults, semantic/staged round trips and binding boundaries. Existing exhaustive consumers and declared integration verification surfaces SHALL be migrated.

#### Scenario: Standalone consumers exercise the delivered temporal profile
- **WHEN** a contributor runs the documented native/Python examples and the actual browser verification scenario
- **THEN** temporal kinds, precise ISO values, explicit offsets and meaningful invalid-value diagnostics are observed through the real core and bindings
- **AND** the documentation distinguishes implemented profile choices from full ISO conformance and unsupported host conversions
