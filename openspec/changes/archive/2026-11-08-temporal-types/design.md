## Context and authority

The user selected first-class temporal values and the existing `!Regex "..."`-style spelling, optional explicit offsets with schema requirements, preservation rather than implicit loss of precision, zero filling of omitted lower-order clock fields, and ISO 8601 as the basis, then said proceed. Those are the accepted product directions. This change defines a bounded extended-calendar profile rather than promising all ISO representations. Host parser permissiveness is not the grammar owner.

The existing Regex precedent is syntactic, not an existing data-kind implementation: native Regex lowers a constraint, while data tags currently remain assertions on JSON-shaped values. New temporal constructors must therefore be represented across the whole value pipeline rather than implemented as format checks on strings.

## Evidence and profile limits

ISO 8601-1:2019 is the Gregorian/24-hour-clock basis. Its public Amd 1:2022 adds negative-year wording, permits fractions on hours/minutes/seconds, and explicitly restores end-of-day forms and describes leap-second labels. This implementation initially selects complete four-digit-magnitude calendar dates, extended clocks and seconds fractions; it does not implement every representation permitted by ISO.

Primary evidence:
- [ISO/TC154 scope](https://www.isotc154.org/standards/iso-8601-1-2019/)
- [ISO 2019 public preview](https://cdn.standards.iteh.ai/samples/70907/6711259d4b7a4b728075a699b02cb1f4/ISO-8601-1-2019.pdf)
- [ISO Amd 1:2022 public preview](https://cdn.standards.iteh.ai/samples/81801/f527872a9fe34281ae3a4af8e730f3f8/ISO-8601-1-2019-Amd-1-2022.pdf), clauses 3.1.1.24, 4.3.2, 5.3.1.4 and 5.3.2.
- [RFC9557 section 2](https://www.rfc-editor.org/rfc/rfc9557#section-2) identifies ISO's exclusion of negative-zero offsets; its RFC-specific unknown-offset convention is not imported into the ISO value model.

The base preview stops before several operative grammar clauses. Exact spellings/limits in the specification are explicit DAML profile choices, not unsupported claims about complete current-ISO conformance. In particular zero filling is the user-selected exact-value normalization policy, not a claim that ISO reduced precision inherently means zero. Leap labels are represented, not certified against an IERS historical/future schedule. No network, system timezone or clock lookup belongs in literal admission.

## Core ownership and public interface

One core temporal module owns admission, canonical ISO text, kind, offset presence and exact fraction comparison. No host datetime object, floating-point value, new parsing dependency, global timezone registry or second format grammar.

Integration interface:
- `TemporalKind::{Date, Time, DateTime}` with `as_str()` returning date/time/datetime.
- A validated `TemporalValue<'a>` with `parse(kind, text)` accepting borrowed/owned text, `kind()`, `as_str()` for canonical ISO, `offset_minutes() -> Option<i16>`, `as_borrowed()` and `into_owned()`; a public TemporalError implementing Display/Error.
- `Value::Temporal(ParsedTemporal)` for the source-backed parsed representation; `Document::temporal(node) -> Option<TemporalValue<'_>>`.
- `ResolvedValue::Temporal(TemporalValue<'a>)` and `SemanticValue::Temporal(TemporalValue<'static>)`.
- `SummaryValue::Temporal { kind: TemporalKind, value: &'a str }` for shallow stage summaries.

The implementation may refine private storage, but these integration names/shapes must be coordinated before changing. Preserve source borrowing when an input is already canonical; canonicalization can allocate when omitted fields/decimal signs/offset spelling require new text. Ownership conversion and snapshots may allocate when they need independent retention. Do not validate the same admitted immutable temporal payload repeatedly without a new boundary or mutation.

A temporal constructor consumes its intrinsic marker rather than placing it in generic tag metadata. A generic assertion can precede it. Native-definition parsing distinguishes bare constructors/metadata constraints from a quoted temporal literal constraint. Arbitrary generic assertion stacking remains invalid. Source formatting keeps authored spelling; semantic writing emits the canonical constructor and ISO payload. Ordinary parsed string accessors do not pretend a temporal value is a string.

Temporal equality is component/offset based, not automatic instant normalization: kinds and absent/present offsets differ, and no UTC or 24-hour date rollover erases local intent. Numerically equal fractional seconds may differ in retained precision but compare equal as values; exact staged payload checks still detect precision/spelling changes.

## Native schema and JSON boundary

JSON has no temporal primitive. The explicit plain `to_json` projection uses canonical ISO strings and is not lossless. The semantic transport remains kind-tagged and lossless.

The validator JSON view can expose canonical text for ordinary JSON Schema string operations, but native String and native temporal constraints must retain intrinsic-kind checks. Use one `x-daml-scalar` keyword object:
- `kind`: string/date/time/datetime;
- `offset`: optional/required/forbidden, only for time/datetime, omitted means optional;
- optional `const`: a typed temporal constant, admitted once and compared as that kind (not a guessed application marker).

Native String/string literals remain ordinary strings; native Date/Time/DateTime require their kind. Temporal metadata and kind checks belong in the shared core, not binding-specific validators. Do not globally enable unrelated JSON Schema formats or silently reinterpret RFC3339 string formats as ISO constructors.

Native presets currently cross JSON storage. Preserve typed defaults with `x-daml-default-temporals: [{ path: "/field", kind: "date" }]`, paths relative to the schema's own default; empty path denotes the default root. Reuse existing default-annotation ownership/traversal/projection/bounds patterns. Require an associated default, valid unique pointer targets, string payloads and valid kinds/content. Preserve defaults' exact temporal type before stage callbacks/final validation. This is metadata for a real non-JSON value boundary, not a new application registry or callback engine. Schema-looking keys in literal data remain inert.

Raw temporal values in the JSON-compatible SchemaModule.from_daml envelope must not silently lose their kind: reject them as non-JSON-compatible unless converted through the explicitly documented default annotation representation. Native definitions and typed default generation are the supported authoring paths.

## Python and browser boundary

Python returns frozen DAML Date/Time/DateTime values from both parsed and resolved `to_python()`. Constructors accept profile ISO text; `isoformat()` and string conversion expose canonical text. Time/DateTime expose offset_minutes (None versus explicit zero stays distinct). `to_date()`, `to_time()` and `to_datetime()` explicitly request standard-library values and fail if the year, leap/end-of-day label or nonzero beyond-microsecond fraction cannot be represented exactly. Extra trailing zero digits can be narrowed numerically without changing the original DAML value. No implicit host conversion or JS Date intermediate is permitted.

Python semantic dictionaries and browser semantic/snapshot/summary unions use `kind: "date"|"time"|"datetime"` plus `value: <canonical ISO string>`, alongside existing optional assertion metadata. Admission creates/validates the corresponding core temporal value; it must not downgrade it to kind string. Type declarations and actual wire consumers move together.

The optional Python Pydantic generator emits x-daml-scalar/default-temporals directly for standard temporal annotations. Framework-owned AwareDatetime/NaiveDatetime hooks may be recognized exactly; arbitrary hooks remain rejected. Static temporal defaults are identified from their actual Python values, not from date-looking strings. Bounds, relative-now rules, unsupported fixed offsets and precision-changing behavior must fail explicitly if not projected. Base import and explicit exporters remain independent of Pydantic.

## Delivery ownership

Core production: one owner for src/, native schema/default/stage integration and example exhaustive production consumers. Binding production: one owner for Python/WASM bridges, pure Python type exporter, stubs and browser declarations. Independently owned checks: another owner for Rust/Python/browser acceptance inputs and existing test/fuzz exhaustive match adaptations. Main owns specification, integration, docs/example policy, verification and final handoff. Implementers cannot amend protected test expectations; owner amendments need a reason and a new freeze identity. Shared filesystem/prompt constraints are not hostile write isolation.

No merge/release authority for this feature is inferred from the previous Python PR's merge authorization. Repository gates, actual composed smokes and independent review precede source handoff.
