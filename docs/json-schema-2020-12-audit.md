# JSON Schema Draft 2020-12 baseline audit

This audit compares DAML's low-level schema interchange and native definition
language with the default JSON Schema Draft 2020-12 dialect. It is a capability
baseline, not a commitment to expose every JSON Schema keyword in native DAML.

**Snapshot:** 2026-10-23. DAML working tree based on
`878ec490f093898be0a61ebd4769241e483477ac`; `jsonschema` 0.59.0 is pinned with
arbitrary-precision support. The Olaf sample covers 190 local schema files.
Its aggregate SHA-256 is
`01e7ffd888071867c5451b5880ba4c397f4a2abd10d0ee7a8428239114e015ba`,
computed over sorted relative paths and file bytes separated by NUL bytes.

## Verdict

DAML does **not** implement the complete default Draft 2020-12 dialect. Its
interchange path is a bounded validation profile delegated to the pinned
`jsonschema` backend. DAML additionally applies completion semantics that JSON
Schema does not define. Native definitions intentionally expose a smaller,
typed authoring language and lower into that same engine.

This is a coherent boundary if documentation and future bindings continue to
say **bounded Draft 2020-12 profile**, not simply **Draft 2020-12 compliant**.
No delegated assertion defect was demonstrated by the sampled runtime probes.
The material incompatibilities are explicit DAML policy:

- recursive schema graphs and `$dynamicRef` are rejected;
- custom dialect selection is unavailable;
- exact numeric validation is bounded to 512 decimal digits and absolute
  exponent 512;
- regular expressions use a size-bounded, linear engine and therefore reject
  advanced ECMA-262 constructs such as lookbehind and backreferences;
- references resolve only from explicitly composed local resources;
- `format` remains an annotation and is not asserted.

Olaf's apparent recursive graphs are produced by its closed-world discriminator
collectors. Native tags and open structural protocols remove those reverse
collector edges, so the Olaf migration does not currently justify recursive
definition support.

## Baseline and method

The official default
[Draft 2020-12 meta-schema](https://json-schema.org/draft/2020-12/schema)
requires the Core, Applicator, Unevaluated, Validation, Meta-Data,
Format-Annotation, and Content vocabularies. Format-Assertion is a separate,
optional vocabulary. Primary semantic references are:

- [Core and vocabulary declarations](https://json-schema.org/draft/2020-12/json-schema-core.html#section-8)
- [Applicators](https://json-schema.org/draft/2020-12/json-schema-core.html#section-10)
- [Unevaluated applicators](https://json-schema.org/draft/2020-12/json-schema-core.html#section-11)
- [Validation](https://json-schema.org/draft/2020-12/json-schema-validation.html#section-6)
- [`format`](https://json-schema.org/draft/2020-12/json-schema-validation.html#section-7)
- [Content annotations](https://json-schema.org/draft/2020-12/json-schema-validation.html#section-8)
- [Meta-data annotations](https://json-schema.org/draft/2020-12/json-schema-validation.html#section-9)

The audit separates four questions that must not be collapsed into one support
claim:

1. **Interchange admission:** can `SchemaModule` accept the schema shape?
2. **Validation:** does the composed backend evaluate the constraint?
3. **Completion:** can DAML safely apply defaults through that position?
4. **Native authoring:** can a `.daml` definition express the behavior directly?

Evidence came from the public contract, wrapper configuration, schema-position
classifier, native lowering, existing tests, focused public-API probes, and the
Olaf schema snapshot. The probes sampled representative behavior; DAML does not
currently run the official JSON Schema Test Suite, so this is not an exhaustive
conformance certification.

## Coverage by vocabulary

| Area | Draft 2020-12 keywords | DAML interchange and validation | Native authoring |
|---|---|---|---|
| Core identifiers and static references | `$schema`, `$id`, `$ref`, `$anchor`, `$defs`, `$comment` | Static local resources, relative references, nested identifiers, anchors, and fragments are supported. `$schema` may select only the default 2020-12 URI. Retrieval remains offline. | Bundle aliases and definition tags provide direct typed references. Native definitions do not expose JSON identifier syntax. |
| Dynamic scope and dialects | `$dynamicRef`, `$dynamicAnchor`, `$vocabulary` | `$dynamicRef` is rejected. `$dynamicAnchor` can be admitted, but cannot provide useful dynamic-scope behavior without `$dynamicRef`. Custom meta-schema URIs are rejected, so custom vocabulary declarations cannot select another dialect. | No dynamic-scope or custom-vocabulary construct. |
| Applicators | `allOf`, `anyOf`, `oneOf`, `not`, `if`, `then`, `else`, `dependentSchemas`, `prefixItems`, `items`, `contains`, `properties`, `patternProperties`, `additionalProperties`, `propertyNames` | Delegated validation is available. Existing tests cover dependencies and property names; probes covered branches, conditionals, contains, patterns, and object behavior. Defaults under `not`, conditionals, `contains`, `dependentSchemas`, `propertyNames`, and pattern-selected properties fail composition rather than being skipped. | `&`, inclusive `|`, records, maps, lists, tuples, and `closed` cover the common structural subset. Exact-one `oneOf`, negation, conditionals, `contains`, pattern properties, property-name constraints, and dependent schemas have no native form. |
| Unevaluated applicators | `unevaluatedItems`, `unevaluatedProperties` | Delegated validation is available. Defaults below either position are rejected by the completion profile. | No native form. Closed records and exact tuples cover narrower common cases without evaluation tracking. |
| General validation | `type`, `enum`, `const` | Delegated validation is available with exact numeric equality. | Built-in constructors provide types; literals provide `const`; literal unions can express a finite set but there is no separate `enum` constructor. |
| Numeric validation | `multipleOf`, `maximum`, `exclusiveMaximum`, `minimum`, `exclusiveMinimum` | Delegated with arbitrary-precision representation inside DAML's 512-digit/exponent profile. | `Int` and `Number` expose inclusive `min`/`max`, `exclusiveMin`/`exclusiveMax`, and exact positive `multipleOf`. |
| String validation | `maxLength`, `minLength`, `pattern` | Delegated. Standard interchange patterns use the configured ECMA-compatible translation and bounded linear engine. | `String` exposes `min`, `max`, and `pattern: !Regex "..."`. Native Regex uses Rust syntax directly, is bounded, and matches the complete string. |
| Array validation | `maxItems`, `minItems`, `uniqueItems`, `maxContains`, `minContains` | Delegated. Sampled `contains` bounds behaved as expected. | Homogeneous `List` exposes `min`, `max`, and `unique: true`; explicit `!Tuple [ ... ]` enforces exact positional length without its own length or uniqueness facets. `contains` facets have no native form. |
| Object validation | `maxProperties`, `minProperties`, `required`, `dependentRequired` | Delegated; `required` and `dependentRequired` have direct tests. | Members are required unless marked `?`; `closed` controls extra members. `Map` exposes property-count `min`/`max` and can intersect a record. Dependent-required constraints have no native form. |
| Meta-data | `title`, `description`, `default`, `deprecated`, `readOnly`, `writeOnly`, `examples` | Accepted as annotations. `default` additionally drives DAML completion under the documented restricted positions. DAML has no documented general annotation-output API. | Defaults are first-class. The other annotations have no native form. |
| Format | `format` | Format-Annotation behavior is selected; format assertions are explicitly disabled. This matches the default dialect, which does not require Format-Assertion. | No native form. |
| Encoded content | `contentEncoding`, `contentMediaType`, `contentSchema` | Accepted as annotations and not automatically decoded or validated. This matches the specification's default behavior. Defaults below `contentSchema` are rejected. | No native form. |

`definitions` is a deprecated spelling that remains admitted as a schema
container. DAML rejects `dependencies`, `additionalItems`, array-form `items`,
and `$recursiveRef` at admission instead of relying on legacy or backend
compatibility behavior.

## Important semantic distinction: defaults

JSON Schema defines `default` as an annotation. It does not direct a validator to
insert or merge values. DAML deliberately adds a completion phase before final
validation:

- object defaults merge recursively with supplied partial objects;
- supplied scalars and lists replace defaults;
- explicitly tagged supplied objects replace contextual object presets;
- union completion rejects distinct valid results as ambiguous;
- defaults in unsupported applicator positions fail composition.

This is a DAML extension over JSON Schema, not a conformance feature. Future APIs
must keep `validate`-style claims separate from `resolve`/completion behavior.

## Findings

### A1 — full Draft 2020-12 conformance is not a valid product claim

Core requires dynamic-reference semantics, and the default dialect permits
recursive schemas. DAML rejects `$dynamicRef` during admission and rejects
reference cycles during composition. Its numeric and regex resource profiles
also narrow otherwise broader standard semantics.

**Classification:** deliberate bounded profile; documentation/API naming risk,
not a newly found runtime defect.

**Action:** retain the bounded-profile wording. Do not add a generic
`supports_draft_2020_12 = true` capability flag to Python or LSP APIs.

### A2 — accepting `$vocabulary` in an ordinary schema is correct

A focused probe placed an unknown required vocabulary directly in an ordinary
schema and observed acceptance. That is not a defect. Core section 8.1.2 says
`$vocabulary` is interpreted in a meta-schema and **must be ignored** in a schema
that is not being processed as a meta-schema.

DAML separately rejects custom `$schema` URIs, so it cannot select a custom
meta-schema that requires an unknown vocabulary. Custom dialect support is
therefore unavailable rather than incorrectly accepted.

**Classification:** expected behavior plus deliberate scope boundary.

### A3 — completion support is intentionally smaller than validation support

The shared schema-position classifier recognizes the standard schema-bearing
positions. It also marks positions where defaults cannot be completed without
changing or guessing semantics. Such defaults fail composition early; the same
keywords without reachable defaults remain ordinary validator constraints.

**Classification:** deliberate fail-fast boundary. No silent default loss found.

### A4 — format and content behavior matches the selected default dialect

DAML configures `should_validate_formats(false)`. Draft 2020-12 requires
Format-Annotation support in the default dialect but makes Format-Assertion a
separate optional vocabulary. Content keywords are annotations and should not
cause automatic decoding or validation by default.

**Classification:** supported annotation behavior; assertion/decoding features
are out of scope.

### A5 — no second schema engine is justified

The pinned backend already owns assertion evaluation, reference registries,
unevaluated tracking, exact numbers, and vocabulary-aware compilation. Native
DAML lowering and completion address demonstrated gaps: concise typed authoring,
explicit local bundle scope, assertions through tags, and deterministic default
application.

**Classification:** current ownership is justified. Reimplementing validator
keywords or generating JSON Schema files would add maintenance without serving
the objective.

## Olaf evidence

The local Olaf snapshot contains 190 JSON schema files. Counts below include
keywords only when they occur at standard schema positions; JSON-schema-looking
objects below unknown vendor annotations are not counted as active standard
subschemas.

| Keyword | Occurrences | Files |
|---|---:|---:|
| `type` | 671 | 190 |
| `default` | 236 | 96 |
| `$ref` | 236 | 83 |
| `enum` | 181 | 162 |
| `properties` | 166 | 162 |
| `required` | 166 | 162 |
| `description` | 130 | 89 |
| `oneOf` | 108 | 73 |
| `title` | 64 | 56 |
| `items` | 46 | 34 |
| `$defs` | 28 | 28 |
| `discriminator` | 28 | 28 |
| `returns` | 37 | 22 |
| `parameters` | 21 | 12 |
| `additionalProperties` | 9 | 5 |
| `anyOf` | 6 | 4 |
| `prefixItems` / `minItems` / `maxItems` | 3 each | 2 each |

`discriminator`, `parameters`, and `returns` are not Draft 2020-12 keywords.
Standard validators treat them as annotations and do not evaluate nested
JSON-schema-like objects under them. Olaf must continue to own their meaning in a
consumer adapter or replace them deliberately when moving to native definitions.
DAML core must not learn Olaf callables.

The `__type__` and `__fn__` members are ordinary data properties constrained by
`enum`; they are not JSON Schema discriminators by themselves. Native DAML tags
can replace those data markers in a deliberate migration, but a tag assertion
does not construct or invoke an Olaf object. Olaf still owns dispatch.

### Collector cycles disappear with native polymorphism

The semantic `$ref` graph contains three strongly connected components around
`GEN_FN`, `POLICY_FN`, and `STAGE_FN`. Each `*_FN` file enumerates concrete
implementations with `oneOf`; selected implementations then refer back to the
collector for nested polymorphic values.

That cycle is an artifact of representing polymorphic selection in JSON data.
In native DAML, an explicit nested tag selects the concrete definition while the
containing field applies an open structural protocol. The protocol does not
enumerate implementations, so the reverse collector edge disappears. The same
replacement removes `discriminator`, `__fn__`, and `__type__`; none supplies a
remaining recursive-definition use case.

After excluding the 28 collector files and their marker properties, the remaining
standard constraints fit the existing native model:

- 63 of 80 `oneOf` occurrences are nullable values; 10 select disjoint JSON
  kinds. Native `|`, literal `null`, and `?` distinguish nullable from absent.
- Five reference-only unions are polymorphic collector/category selections.
  Explicit tags plus open structural protocols replace them.
- Fifteen non-marker enums remain. Literal unions express them without an enum
  constructor.
- Nine `additionalProperties` schemas describe uniform maps or unconstrained
  objects. `Map`, open `Object`, and intersections cover them.
- The only active item-count bounds are three `minItems`/`maxItems` pairs where
  the bounds are equal. Native `!Tuple [ ... ]` definitions express those exact tuples.
- No active numeric bounds, string lengths, patterns, uniqueness constraints,
  conditionals, negation, or unevaluated constraints remain.

Two overlapping `oneOf` shapes deserve correction during conversion rather than
a new core feature. `aisle.alphas` distinguishes a numeric array from a
description-only \"Numpy array\" even though the latter accepts every array;
`update.approx` distinguishes a callable object from an `Approx` object. JSON
Schema cannot make either distinction nominally. Explicit tags or an Olaf-owned
protocol make the intended selection direct.

The remaining Olaf-specific information is callable metadata: `parameters` and
`returns` occur 58 times at schema positions. They are not JSON Schema
constraints and do not validate configuration data. If Olaf still needs them
for construction or tooling, Olaf must own their representation and adaptation;
DAML core should not acquire callable semantics.

## Recommendation and next slice

Olaf does not require recursive definitions or blanket native parity. Native
local facets now cover string length and Regex, list cardinality and uniqueness,
map/object property counts, exclusive numeric bounds, and exact positive
`multipleOf`. These are basic value constraints independent of Olaf's current
corpus. `contains`, conditionals, general negation, and callable metadata remain
deliberately outside native DAML.

The [representative native conversion](../examples/olaf/README.md) now exercises
that boundary without changing Olaf or retaining callable metadata in DAML.
The [Python API](python.md) exposes native bundles and low-level interchange
separately, preserving validation versus completion. This does not add official
JSON Schema Test Suite coverage or change the bounded-profile assessment.