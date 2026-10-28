# Data and schema: values and requirements

A configuration supplies values. A schema describes which values are acceptable.
DAML uses its own notation for both, but the two layers have different roles:

- **Data layer:** here are the values for this particular configuration.
- **Schema layer:** here are the fields, constraints, and defaults that define a
  valid configuration. A native DAML definition expresses such a schema.

You can understand this distinction without knowing a programming language's type
system. The type/value analogy below is an optional bridge, not a prerequisite.

## A definition describes; data supplies

The canonical definition
[`app/Worker.daml`](../examples/tutorial/app/Worker.daml) describes worker
configurations:

```daml
closed !Object {
  name: !String
  retries: !Int
} = {
  retries: 3
}
```

`name: !String` requires a string. `retries: !Int` requires an integer. These are
constraints, not the name and retry count of a particular worker. `closed`
rejects fields not declared by this record.

The preset after `=` is data embedded in the definition: it supplies a default
retry count. It does not make the `retries` field optional in the completed value.

The separate data file
[`data/worker.daml`](../examples/tutorial/data/worker.daml) supplies a worker:

```daml
!app.Worker {
  name: "demo"
}
```

Here `name: "demo"` is a value, not a constraint declaration. The root tag
`!app.Worker` selects the definition that this object must satisfy; it does not
turn the data document into a schema. The entry bundle binds `app` to the
folder containing `Worker.daml`.

Both files have the `.daml` extension. Their role comes from how they are used:
configuration is supplied as data; the bundle loader reads definition files as
constraints and presets. A field constraint such as `!Int` belongs to a
schema position, while `3` is a data value. Sharing notation does not make the
two grammars interchangeable.

## Complete the value, then validate it

Run the existing library consumer from a source checkout with the
[pinned Rust toolchain](development/building.md):

```sh
cargo run --locked --example native -- examples/tutorial examples/tutorial/data/worker.daml
```

The supplied configuration omits `retries`. Completion applies the preset, and
validation checks the completed value against the definition. The command prints:

```json
{
  "name": "demo",
  "retries": 3
}
```

This is the resolved value, not a rewrite of the input file. The command prints
plain JSON, so assertion metadata is not included in that output.

Now try the deliberately invalid
[`invalid/retries.daml`](../examples/tutorial/invalid/retries.daml), which
supplies `retries: "many"`:

```sh
cargo run --locked --example native -- examples/tutorial examples/tutorial/invalid/retries.daml
```

It fails at `/retries`. A supplied string is not replaced by the default integer.
The source is syntactically valid; it does not satisfy the selected definition.
See [the tutorial](tutorial.md) to continue with nested definitions and providers.

## The programming-language analogy: type versus value

A type describes acceptable values; a value is one particular instance. Reading
`retries: !Int` as a field's type constraint and `retries: 3` as its value is a
useful first approximation.

| Programming-language idea | DAML counterpart | Limit of the analogy |
|---|---|---|
| A record type describes fields | A definition describes configuration shape and constraints | Definitions can also express value bounds, alternatives, and presets. |
| A value supplies particular contents | A data document supplies a particular configuration | The authored value may still need completion and validation. |
| A type annotation states a requirement | An assertion tag selects an additional definition the value must satisfy | It does not cast away the containing field's constraints or require nominal ancestry. |
| A value is checked against a type | A completed value is validated against all applicable constraints | Completion applies defaults first; ordinary type checking does not imply that behavior. |

The analogy is not an identity. A DAML definition combines constraints with
optional default data. Its completion rules are specified by DAML, not inherited
from a host programming language or inferred as arbitrary deep merging.

## Further connections

Once values and requirements are distinct, these familiar concepts help explain
the rest of the language:

| Familiar concept | Connection in DAML | Where to go next |
|---|---|---|
| Structural typing | A value qualifies by satisfying the required fields and constraints, without a declared ancestry relationship. | [Structural composition](structural-composition.md) |
| Intersection and union types | `A & B` requires both constraints; `A \| B` accepts alternative structural shapes. A union is not a whitelist of provider identities. | [Definition composition](native-authoring.md#composition-records-and-positions) |
| Elaboration of concise source | Resolution turns partial authored data into a completed, validated value under explicit default rules. | [Completion and defaults](native-authoring.md#defaults-and-completion) |
| Module-local names | Each bundle has its own imports; aliases do not leak through dependencies. | [Bundle scope](native-authoring.md#files-and-bundle-roots) |
| Concrete syntax and semantic values | Source-preserving formatting and canonical semantic writing preserve different information. | [Formatting and writing](formatting.md) |

These are explanations, not additional language features or syntax. In particular,
there is no separate `protocol` declaration: an ordinary structural definition
can serve as the contract for independently authored providers.

Continue in this order: [the runnable tutorial](tutorial.md),
[structural compatibility](structural-composition.md), then
[bundle-local names](native-authoring.md#files-and-bundle-roots). Use
[the examples catalogue](../examples/README.md#consumer-code) for Rust, Python,
and browser consumer code.
