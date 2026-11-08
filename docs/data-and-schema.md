# Data and schema: values and requirements

You want to configure a worker. It needs a name, and three retries is usually a
good starting point. The person configuring it should be able to choose a name
without repeating that retry count in every file.

Here is what that person writes in
[`data/worker.daml`](../examples/tutorial/data/worker.daml):

```daml
!app.Worker {
  name: "demo"
}
```

That is **data**: one particular configuration. It says which values this worker
supplies. It doesn't describe every possible worker.

## Where the requirements live

The tag points to a definition,
[`app/Worker.daml`](../examples/tutorial/app/Worker.daml):

```daml
closed !Object {
  name: !String
  retries: !Int
} = {
  retries: 3
}
```

This is the **schema layer**. It says a worker needs a string name and an integer
retry count. `closed` rejects unexpected fields. The data after `=` supplies a
default of three retries.

Now the two layers have separate jobs:

- The definition describes what worker configurations must contain.
- The data document supplies the choices for this worker.

Many data documents can use the same definition. Putting `!app.Worker` on a value
selects a requirement for that value to satisfy; it doesn't turn the data into a
schema.

Both files use `.daml`, but their roles come from how they are used. A bundle
loads the definition as constraints and presets; the consumer supplies the data
to check. In the definition, `retries: !Int` describes a field. In the data,
`retries: 3` gives that field a value.

## See the default become a value

With a source checkout and [the pinned Rust toolchain](development/building.md),
run this from the repository root:

```sh
cargo run --locked --example native -- examples/tutorial examples/tutorial/data/worker.daml
```

The consumer prints:

```json
{
  "name": "demo",
  "retries": 3
}
```

DAML fills the omitted retry count, then validates the completed value. The
original file stays as you wrote it. This consumer prints plain JSON, so the
output shows values rather than assertion metadata.

If you supply `retries: 5`, five wins over the default. If you supply
`retries: "many"`, the definition rejects it. Try the checked-in bad example:

```sh
cargo run --locked --example native -- examples/tutorial examples/tutorial/invalid/retries.daml
```

It fails at `/retries`: the string is valid syntax, but not a valid retry count.
A default fills a missing choice; it doesn't repair an invalid supplied one.
[The tutorial](tutorial.md) lets you make those changes yourself before adding
nested configuration and independent providers.

## If you know types, think type versus value

A type describes acceptable values. A value is one particular instance. That is
a useful way to read the two layers: `Worker` describes the shape, while the file
supplies one worker's contents.

The analogy has limits. A DAML definition can include detailed value constraints
and default data, not just field types. Completion is an extra step; ordinary
type checking doesn't imply that missing values will be supplied. And choosing
an assertion tag doesn't cast away the surrounding requirements.

You don't need type-system vocabulary to use this model. *Requirements versus
supplied contents* describes the same distinction.

## Derive a schema from Python types

The workflow of defining configuration types in code, deriving a schema, then
authoring concrete values in DAML is now supported in bounded form by the
optional Python API. Explicitly bind Pydantic v2 models or standard dataclasses
to DAML names with `daml.schema_from_types`; the returned JSON Schema module is
composed and resolved through the existing interchange API. It is an in-memory
interchange module, not generated native `.daml` definition text. See the
[runnable Python example](../examples/python_types.py) and
[Python API guide](python.md#derive-schemas-from-python-types).

The projection does not promise exact `model_validate` behavior, strict Python
type identity, coercion, extra-ignore behavior, or model-instance construction.
DAML's required-container completion and mathematical JSON integer semantics
remain unchanged; default exports support string-keyed mappings. Unsupported
model behavior such as custom validators, factories, schema overrides,
normalization, and schema-position `format` assertions is rejected. Standard
Python date/time/datetime annotations and representable static defaults are
supported as first-class DAML temporal values; see the
[temporal Python example](../examples/python_temporal.py) and
[temporal profile](syntax.md#temporal-literals). Type and exporter hooks are
trusted caller code. Python names remain subject to DAML's source-key grammar.
For other type libraries, a caller-supplied generic exporter owns its schema
conventions. Native definitions remain useful for contracts and defaults that
need native DAML features.

## A few connections for later

Once the two layers make sense, other familiar ideas become useful:

| Familiar idea | How it helps you read DAML |
|---|---|
| Structural typing | A provider qualifies by meeting the required fields and constraints, not by declaring ancestry. |
| Intersection and union types | `A & B` requires both; `A \| B` accepts alternative shapes, not a whitelist of provider names. |
| Elaboration | Resolution turns concise, partial input into a completed, validated value under explicit default rules. |
| Module-local names | Bundle imports make names available locally; they don't leak through dependencies. |
| Source versus semantic values | Formatting preserves authored details; semantic writing produces canonical DAML from values. |

Start with [the tutorial](tutorial.md). Then [structural composition](structural-composition.md)
explains independent providers, and [native definitions](native-authoring.md)
supply the exact composition and default rules. The [formatter guide](formatting.md)
explains what source information each writing operation preserves. For consumer
code, use [the examples catalogue](../examples/README.md#consumer-code).
