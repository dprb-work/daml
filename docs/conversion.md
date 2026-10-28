# YAML and JSON interoperability

DAML does not yet ship a general-purpose YAML/JSON import/export command.
Library exports and the low-level JSON Schema API provide useful building
blocks, but they are not a bidirectional file converter.

## Export completed values

The Rust resolver can export completed values with `to_json()`. Python exposes
ordinary Python values through `to_python()`; non-integer numbers use `Decimal`,
so a JSON serializer needs an explicit exact-number policy. The
[tutorial](tutorial.md) uses a Rust consumer that prints the completed value as
JSON.

These plain exports omit tag metadata. Use the result's tag accessors if your
consumer needs the assertions. Completion can add defaults, so exporting the
resolved result is different from exporting only the values originally supplied.
See [resolution and provenance](schemas.md) and [Python numbers](python.md#exact-numbers).

The Rust and browser semantic writers produce canonical DAML source from editable
values. They do not preserve comments, original whitespace, scalar style, or
original field order. Exact number values are retained, but original numeric
spelling is not a semantic round-trip guarantee. See [formatting versus semantic
writing](formatting.md).

## Configuration conversion needs a loss policy

A file converter would need to decide what to accept and what to preserve:

| Concern | Boundary to handle explicitly |
|---|---|
| Object keys | JSON and YAML can represent keys outside DAML's unquoted key grammar. Such keys cannot simply be emitted as quoted DAML keys. |
| Tags | Plain JSON has no DAML assertion metadata; dropping assertions changes what can be validated on re-admission. |
| Numbers | Do not route exact decimals or large integers through a floating-point representation that rounds them. |
| Defaults | Authored data and completed data are different export choices; a converter must say which it uses. |
| YAML graphs | Anchors, aliases, and cycles need an admitted profile; DAML documents are trees. |
| YAML values | Non-string mapping keys, custom tags, non-finite numbers, and multiple documents cannot be silently coerced. |
| Source fidelity | Preserving values does not preserve comments, formatting, anchors, or scalar style. |

The syntax parser itself is not a YAML parser. [The comparison](comparison.md)
explains the language differences. Until a converter is available, applications
must define and implement their own import/export boundary.

## Definition conversion is a separate operation

The [low-level interchange API](schemas.md) admits explicit JSON Schema modules
in a bounded Draft 2020-12 profile. Native definitions lower to in-memory
constraints; this is not a command that converts arbitrary schema files in both
directions.

Not every JSON Schema constraint has native DAML syntax. Conversely, DAML bundle
scope, assertions, and default completion are not equivalent to standard JSON
Schema annotations. A definition converter must report unsupported constructs
rather than silently discard constraints. JSON Schema written in YAML still
needs representation parsing and dialect checks before schema admission.

For a small consumer-owned adaptation, see
[`examples/schema_adapter.rs`](../examples/schema_adapter.rs). It adapts a
made-up vendor record format; it is not a general conversion tool.