# Schema modules, defaults, and structural validation

DAML exposes two related schema paths. `SchemaModule` is a low-level,
JSON-compatible interchange API for consumers that already have JSON Schema.
Native definitions and bundles are the authoring API described in
[native definitions and bundles](native-authoring.md). Native compilation
lowers definitions to in-memory JSON Schema constraints and DAML completion
metadata; it writes no generated schema files.

JSON Schema's standard `default` is an annotation, not a command to mutate an
instance. DAML's completion step applies defaults before full validation. See
[standard annotation semantics](https://json-schema.org/understanding-json-schema/reference/annotations#default).
The [dated Draft 2020-12 baseline audit](development/json-schema-2020-12-audit.md) separates
backend validation, DAML completion, and native expressibility. Interchange is a
bounded Draft 2020-12 profile, not an unqualified conformance claim.

## Supply an interchange module

Interchange accepts a JSON-compatible envelope of named types and registered
resources. Schema documents use JSON Schema keywords such as `$id`, `$ref`, and
`$defs`. These dollar-prefixed names belong to schema documents, not final DAML
data keys: data parsing rejects `$` in keys. Supply such schemas as JSON with
`SchemaModule::from_json`, or as already parsed JSON-compatible values with
`SchemaModule::from_value`.

```json
{
  "types": {
    "app.Request": {
      "schema": {
        "$id": "urn:example:Request",
        "type": "object",
        "properties": {
          "params": {
            "type": "object",
            "default": {"timeout": 30, "retries": 2},
            "properties": {
              "timeout": {"type": "integer"},
              "retries": {"type": "integer"},
              "endpoint": {"type": "string"}
            },
            "required": ["timeout", "retries", "endpoint"],
            "additionalProperties": false
          }
        },
        "required": ["params"]
      }
    }
  }
}
```

A supplied `!app.Request { params: { endpoint: "/v1" } }` completes to params
containing `timeout: 30`, `retries: 2`, and the supplied endpoint. Supplying
`timeout: 5` overrides 30. Omitting the endpoint is still an error: the default
object can be partial, but the completed request cannot lack required values.

For the JSON representation, use `SchemaModule::from_json` or `from_value`,
then `Schemas::compose`. Unknown module-envelope fields are errors. Interchange
schema documents are JSON-compatible values: tags in a schema document are
rejected rather than silently discarded. Data documents remain fully tagged.
Duplicate JSON object members are rejected, including escaped spellings of the
same key. `from_value` receives an already constructed map, so duplicate-key
admission belongs to its caller.

## Compose independently owned definitions

The module envelope accepts `types`, mapping literal names to `{ schema: ... }`,
and `resources`, a list of explicit `{ uri, schema }` entries. Types and
resources are the complete interchange shape; modules do not declare contracts
or `implements` membership. Duplicate/conflicting names or resource identifiers
fail composition. Module order is not precedence.

Use `$id`, `$ref`, and `$defs` for schema references. They identify explicitly
registered in-memory resources; an HTTP-looking identifier is not permission to
fetch anything. See [JSON Schema identifiers and references](https://json-schema.org/understanding-json-schema/structuring).

Compatibility is structural. The containing schema's constraints apply to the
value, and an explicitly tagged value must also satisfy its selected type schema.
An untagged value can satisfy the same structural requirements. An unknown
explicit tag remains an error, and tagged descendants are checked under open
containers. DAML does not compare tag names as protocol identities or require
declared membership.

Former `x-daml-tag` and `x-daml-slot` keywords are rejected at actual schema
positions, including nested schemas and `$defs`. Their spellings inside literal
values, examples, or property names remain ordinary data. These keywords do not
participate in interchange.

Validation uses ordinary JSON Schema constraints, including `anyOf` and
`oneOf`. DAML does not inject a fake tag property into application objects.

The native `x-daml-default-tags` keyword remains available to attach explicit
tag assertions to values generated from defaults. It does not declare type
membership; its full format is described below.

## Resolve, then inspect the completed result

```rust
let module = daml::SchemaModule::from_json(schema_source)?;
let schemas = daml::Schemas::compose([module])?;
let supplied = daml::parse(input_source)?;
let resolved = schemas.resolve(&supplied)?;
let params = resolved.root().get("params").unwrap();
```

`resolve_as` additionally applies a named root type even when the root is
untagged. It does not waive a different explicit tag assertion. The original
parsed document is not mutated. The resolved result borrows input/schema data
where possible, exposes read-only values/tags/origins, and can export completed
plain data with `to_json`. That export deliberately excludes tag metadata;
`ResolvedNode::tag` retains it for application adapters.

Run the complete multi-module consumer:

```sh
cargo run --locked --example resolve -- examples/application.daml \
  examples/schemas/application.json examples/schemas/network.json \
  examples/schemas/text.json
```

It validates nested records, scalars, and arrays; fills the adapter timeout and
request ID; and prints completed JSON data. It does not invoke an adapter.

## Exact completion rules

- Supplied values win. Explicit `null` is supplied data, never omission.
- Object defaults merge recursively with supplied partial objects. Unknown
  supplied fields remain present so `additionalProperties` can reject them.
- Supplied arrays and scalars replace defaults; arrays are never concatenated.
- A missing required object or map starts as `{}`; a missing required list starts
  as `[]`. Completion then applies applicable presets and child defaults before
  validation. Empty construction does not invent primitives, map entries, list
  elements, or implementation tags.
- Optional absent containers remain absent. Explicit defaults still apply to
  absent values, including optional values in the low-level interchange API.
- An explicitly tagged supplied object replaces that field's contextual object
  preset, regardless of whether the tag resolves to the same definition. The
  selected tag's own defaults still apply, and containing constraints remain
  active. Tag presence, not identity comparison, sets this boundary.
- A default may be partial. Required fields and all other constraints are checked
  on the completed result, not on the default literal in isolation.
- Conflicting applicable defaults for a still-missing path are errors, not a
  last-module-wins rule. Explicit supplied data at that path takes precedence.
- Union branches complete isolated candidates. Different valid completed values
  are ambiguous and rejected rather than choosing the first branch. `oneOf`
  still requires exactly one valid branch; `anyOf` may accept several branches
  only when completion yields the same value.

Default completion supports structural `properties`, `additionalProperties`
(without pattern selection), `items`/`prefixItems`, `$ref`, `allOf`, and
unambiguous union paths. Branch candidates are checked against the completed
document's ancestor constraints before ambiguity is reported. Unions without
reachable defaults use ordinary validation without enumerating unchanged data.
Defaults reachable in unsupported applicator positions—negation, conditionals,
`contains`, `propertyNames`, `dependentSchemas`, pattern-selected properties, or
unevaluated/content schemas—fail composition instead of being silently ignored.
Those constraints without defaults remain ordinary engine validation.

When `$schema` is present, it must select Draft 2020-12. The legacy
`dependencies` and `additionalItems` keywords, plus the pre-2020-12 array form
of `items`, are rejected rather than delegated to engine compatibility behavior.
Use `dependentSchemas`, `dependentRequired`, and `prefixItems`.

## Consumer-owned schema adapters

A consumer that uses another schema format converts it before calling DAML.
The adapter owns vendor fields, discriminator rules, aliases, and foreign
reference syntax. DAML receives JSON Schema and the retained
`x-daml-default-tags` annotation; it does not inspect consumer fields or fetch
resources.

The interchange shape stays limited to types and registered resources. For
example, a consumer may register a reusable resource and refer to it from a
type:

```json
{
  "types": {
    "app.Request": {"schema": {"$ref": "urn:example:Request"}}
  },
  "resources": [{
    "uri": "urn:example:Request",
    "schema": {
      "$id": "urn:example:Request",
      "type": "object",
      "properties": {"name": {"type": "string"}},
      "required": ["name"]
    }
  }]
}
```

These are standard JSON Schema references. Structural constraints, rather than
declared tag membership, determine whether a value is compatible. A consumer
adapter converts its own format into this representation; it does not emit
removed tag/slot keywords.

### Annotate tags on defaults

The native `x-daml-default-tags` keyword assigns tags to values inside the
schema's `default`. Each record has a `path` RFC 6901 pointer relative to the
default root and a nonempty literal `tag` spelling. A native compiler may also
supply `binding`, a private resolved type key used when the declaring bundle's
alias is not visible to the entry context. Preserve this field when adapting
native defaults. Use `""` for the default root. Each path must identify a value
in the default, paths must be unique, and the schema must contain `default`.

```json
{
  "type": "object",
  "default": {"payload": {"value": 1}},
  "x-daml-default-tags": [
    {"path": "/payload", "tag": "vendor.Payload", "binding": "internal-key"}
  ]
}
```

The annotation preserves the default as ordinary JSON data. It does not add a
field to that data or reinterpret names such as `__type__` and `__fn__`.
Tags apply only when their default is inserted or object-merged. Overriding a
scalar or array default also discards its tag; that discarded assertion cannot
leak onto supplied data.
Malformed records, unknown fields, invalid or duplicate paths, missing targets,
and an empty annotation array are rejected during schema admission. The declared
tag must satisfy the current [tag grammar](syntax.md#structure-and-names).

`examples/schema_adapter.rs` contains a complete consumer-side conversion from a
made-up vendor format. It maps that format's record marker and default-tag
metadata to native keywords, registers a canonical resource, and binds both root
names and the nested settings type with regular `$ref`s. It then composes the
module and resolves a tagged input. No consumer-specific adapter or callback
framework belongs in DAML.

## Programmatic semantic admission

`Schemas::resolve_semantic` and `resolve_semantic_with_limits` accept an editable
`SemanticNode` directly. They use the same completion search, assertion checks,
expected-root constraints and cumulative limits as document resolution. They do
not print and parse source as a validation shortcut. Supplied values have
`Origin::Semantic`; newly inserted defaults retain their schema provenance.
An error at programmatic input has an instance pointer, not an invented byte span.

Resolved assertions export canonical binding identity where needed. An incoming
bound identifier must already name its canonical contract in that registry;
resolving a local alias wrapper to another contract is not an acceptable fallback.
Pure aliases share meaning when comparing completed candidates. An alias with
additional constraints remains distinct even when one value satisfies both.
Literal tag spellings remain available separately and do not identify Python
constructors.

Contextual semantic writing still refuses an assertion that has no equivalent
entry-visible spelling. Exact value export is not permission to discard private
bindings, and an editable snapshot is not an admission certificate. See the
[Python semantic API](python.md#edit-and-re-admit-exact-values) for host wire
representation, ownership and error handling.

## Staged admission

`Schemas::resolve_staged` and `resolve_semantic_staged` accept an explicit
`AdmissionStage` visitor and frozen initial edits. Their `_with_limits` variants
use caller-selected cumulative limits. They extend the same completion/search
and strict validator; they do not expose an unchecked result or a second
permissive registry. Existing resolution methods do not invoke a stage.

The visitor has three responsibilities:

- `defer(summary, path)` classifies an application-opaque payload before ancestor
  defaults can merge into it. `NodeSummary` borrows only its own scalar/tag and
  direct field scalar/container shapes. Raw defaults expose their own optional
  tag, not a contextual tag borrowed from another preset.
- `materialize(root, copies, round)` sees one completed, not-yet-validated
  candidate per round. Return `StageOutcome::Ready`, `Edits`, or `Reject`.
  Round starts at zero on every candidate replay. `copies` is complete applied
  operation history, not a mutable-object alias guarantee; reset host-local
  construction plans on round zero. Full snapshots carry admissible canonical
  binding identities even though value validation is still pending.
- Optional `number_class(lexeme)` supplies a pure, path-independent `u32` class.
  Its default is zero. Classes only refine native candidate/default equality;
  they never change JSON Schema mathematics such as `const` or `uniqueItems`.

The bounded edit vocabulary uses explicit RFC 6901 instance pointers:

| Edit | Effect |
|---|---|
| `Replace { path, value }` | Admit exact semantic data at a root or existing-parent leaf |
| `Copy { path, from }` | Import a completed nondeferred source root with its real provenance |
| `Expect { path, name }` | Conjoin a preloaded named contract for completion and validation |

Initial edits allow Replace and Expect only. A mapping leaf can be inserted;
sequence indices must already exist. The caller supplies the first missing
subtree rather than asking the core to guess map/list parents. Data destinations
cannot overlap, and a batch cannot modify its Copy sources. Empty/nonprogressing
edits fail with `InvalidStage`. Repeated identical expectations neither count as
progress nor resume completion, even alongside a progressing Copy or Replace.
An expectation's freshness is determined before the batch; conserving a Copy's
assertions in that batch cannot make its behavior depend on edit order.
Progress distinguishes exact numeric representation, imported Copy provenance
and genuinely new constraints from mathematical candidate equality. Merely
reimporting an identical replacement with a different origin is not progress.
Imported roots are classified using their effective destination assertions,
including a restored supplied tag, before progress comparison or completion.
Affected ancestors are refreshed from the final atomic batch, deepest first.
Fresh replacements and newly undeferred subtrees recover relevant ancestor
presets without replaying those presets into unrelated siblings. Ancestors
visited only to project those presets do not acquire new local tags or atomic
scalar/sequence defaults.
Copy source paths cannot cross a deferred ancestor. A completed source record
may still contain valid opaque descendants.

Copies do not replay contextual record presets into an already completed value.
Opacity is reclassified throughout a copy at its destination paths; `defer`
need not be path-independent. Newly available descendants still receive their
first completion, whether exposed by Copy itself or a later edit inside it.
Replacement and new expectations resume affected completion work. The nearest
active subtree or completed-copy boundary controls preset eligibility, so a
resumed child can complete inside a copy while deeper copied branches and
completed siblings remain protected. Nonmerging opaque presets are excluded
before default-conflict selection rather than expanded and discarded.
Explicit destination assertions, displaced descendant assertions, and copied source
assertions remain conjunctive obligations, even through later edits; a missing
obligated value fails. Different compatible bindings may both hold. Copy retains
the source/default origin while treating copied assertions as supplied for
preset precedence.

Ready declares the host transformation finished, not the data valid. All native
root, tag, selected-branch, expectation and conserved constraints must still
hold. An opaque value may remain only if it passes those checks unchanged.
Application reference syntax, constructor selection, runtime-reference timing
and object-sharing policy remain consumer-owned.

An optional Ready meaning key further distinguishes domain construction plans
with equal native data. `StagedResolution::root()` and `meaning()` return the
winning checked data and its matching key, not a key from an invalid candidate.
The key is invocation-local meaning, not a replacement for deployment hashes.
Distinct successful native/class/key meanings remain ambiguous.

Callback errors are fatal and preserve their original cause. Explicit Reject
rejects a candidate; it is not an exception-retry policy. Limits are cumulative
across rounds and candidate replays, including SDK-owned summaries, snapshots,
edits, copies, obligations, histories and keys. This bounds SDK work, not arbitrary
code inside a caller-supplied callback.

## Diagnostics and bounds

Syntax errors, schema loading/composition errors, and resolution errors remain
distinct types. A resolution error has a structured kind, RFC 6901 instance
pointer, schema location when available, and supplied-input or default provenance.
Dots/slashes inside names are not inferred paths; JSON Pointer escaping applies
only in explicit diagnostic/reference pointers.

Provided values point to original UTF-8 source bytes. Generated values identify
the actual schema default, not fabricated input positions. Completion budgets
cover materialized nodes, bytes, and work including alternative candidates;
materialized nesting is bounded at 128.
Default resolution limits are 100,000 cumulatively materialized nodes, 8 MiB of
value/key/tag bytes, and 1,000,000 conservative work units. Alternative candidates
share those budgets. Validation charges expanded schema work against instance
nodes and text in 64-byte blocks. Schemas that can evaluate `uniqueItems` also
reserve worst-case exact pair-comparison work before entering the validator;
oversized comparisons return `LimitExceeded` rather than exceeding the budget.

Composition bounds the selected schemas to 10,000 JSON nodes and 8 MiB, compiled
schema-view copies to 8 MiB, and aggregate expanded compilation work to 100,000.
Both physical schema depth and expanded reference depth are bounded at 128.
Unresolved references, normalized identifier/anchor collisions, and references
into non-schema data fail before validation. Relative references, nested `$id`,
and anchors use the prepared registry's URI resolution.

Exact validation currently accepts at most 512 decimal digits and absolute
exponent 512. Larger numeric spellings still parse but fail resolution/schema
loading rather than entering upstream extreme-exponent fallback behavior or
rounding through floating point. This is a bounded implementation profile, not
an unbounded mathematical-conformance claim.

Schema graphs are bounded and cycles/dynamic references are rejected in this
profile. Reference retrieval is explicitly offline, format assertions disabled,
and regular expressions use a size-bounded linear engine. No custom user Rust
callbacks, default factories, templates, or environment reads are registered.

The implementation reuses `jsonschema` 0.60.1 (MIT), `serde_json` 1.0.154, and
`serde` 1.0.229 (both MIT OR Apache-2.0), pinned in Cargo and its lockfile.
HTTP/file/async retrieval features are disabled; arbitrary-precision support is
enabled. This does not imply that
transitive dependencies contain no unsafe code or that every JSON Schema draft
feature/default placement is supported. The native syntax parser remains its
own implementation; no upstream parser source was vendored.
