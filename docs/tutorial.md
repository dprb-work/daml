# Build a service configuration

This tutorial starts with plain data, adds definitions and defaults, then selects
independently authored providers. Use the checked-in files under
[`examples/tutorial/`](../examples/tutorial/README.md) as the canonical example.
The endpoint strings are sample data; the consumer does not contact them.

## Data supplies values; schemas describe them

A data document says which values this configuration supplies. A schema says
which values are allowed and which defaults complete omitted fields. A native
definition expresses that schema; an assertion tag in a data document selects
a definition to check, rather than making the data itself a schema.

If those layers are unfamiliar, begin with [data and schema](data-and-schema.md).
It shows a small definition, input, completed result, and failure before adding
nested records. For programmers, it connects the layers to types and values,
then introduces structural typing, composition, and module-local names.

## Before you start

Use a source checkout with [the pinned Rust toolchain](development/building.md)
installed. Run all commands from the repository root. Cargo builds the existing
example consumers with locked dependencies; Python and the VS Code extension are
not required for this tutorial.

The released `daml-fmt` and `daml-lsp` executables do not perform this tutorial's
schema-resolution operation. Here, `cargo run --example native` runs a small
library consumer, not a separately installed DAML validation command.

## 1. Read plain configuration

[`data/plain.daml`](../examples/tutorial/data/plain.daml) contains:

```daml
{
  name: "demo"
  adapter: { endpoint: "https://example.com" }
}
```

Inspect it:

```sh
cargo run --locked --example inspect -- examples/tutorial/data/plain.daml
```

The inspector prints a mapping tree, original source spans, and string values.
The file has no type assertion and needs no definitions just to parse.
Multi-entry objects use newlines rather than commas.

For the two multiline string forms, inspect a second file:

```sh
cargo run --locked --example inspect -- examples/tutorial/data/text.daml
```

Its `instructions` value is `Read the configuration. Report any missing values.`
The `template` value is `service:\n  name: demo`: triple-double quotes preserve
relative layout after dedent. [The syntax reference](syntax.md) owns the exact
framing and escape rules.

## 2. Add a root definition

The entry [`daml.bundle`](../examples/tutorial/daml.bundle) binds definition
bundles to aliases. Its `app` alias selects the directory containing
[`Service.daml`](../examples/tutorial/app/Service.daml):

```daml
closed !Object {
  name: !String
  adapter: !contracts.Adapter
  retries: !Int
} = {
  retries: 3
}
```

The fields are required. `closed` rejects additional service fields. The preset
after `=` supplies a retry count when the configuration omits it.

This is the schema layer: `retries: !Int` states a requirement, while the preset
`retries: 3` embeds default data. A schema can describe many configurations; it
is not one particular service configuration.

The app bundle's own [`daml.bundle`](../examples/tutorial/app/daml.bundle)
binds `contracts` to the directory containing
[`Adapter.daml`](../examples/tutorial/contracts/Adapter.daml):

```daml
!Object {
  endpoint: !String
}
```

That structural contract requires a string endpoint and permits other adapter
fields. It does not enumerate provider names.

[`data/configured.daml`](../examples/tutorial/data/configured.daml) is the data
layer. It supplies particular values and selects `!app.Service`:

```daml
!app.Service {
  name: "demo"
  adapter: { endpoint: "https://example.com" }
}
```

Complete and validate it:

```sh
cargo run --locked --example native -- examples/tutorial examples/tutorial/data/configured.daml
```

The result is:

```json
{
  "adapter": {
    "endpoint": "https://example.com"
  },
  "name": "demo",
  "retries": 3
}
```

The supplied data remains unchanged. The resolved value has the missing retry
count. The plain JSON output omits assertion metadata; the Rust result retains it.

## 3. Select an independent provider

[`providers/Http.daml`](../examples/tutorial/providers/Http.daml) defines its own
record without referencing the app or its adapter contract:

```daml
closed !Object {
  endpoint: !String
  timeout: !Int
} = {
  timeout: 30
}
```

[`data/provider.daml`](../examples/tutorial/data/provider.daml) selects it:

```daml
!app.Service {
  name: "demo"
  adapter: !providers.Http { endpoint: "https://example.com" }
}
```

Run:

```sh
cargo run --locked --example native -- examples/tutorial examples/tutorial/data/provider.daml
```

The completed adapter contains `"timeout": 30`, and the service still contains
`"retries": 3`. The value must satisfy both the containing `Adapter` requirement
and the explicit `Http` definition. Its tag does not replace the parent's
constraints. An open adapter contract and a closed provider definition are
compatible: one permits extra fields; the other restricts this selected value.

## 4. Use another independently authored definition

The entry manifest also binds `extra` to `third-party/`. Its
[`Custom.daml`](../examples/tutorial/third-party/Custom.daml) requires a string
endpoint and supplies `label: "external"`.

```sh
cargo run --locked --example native -- examples/tutorial examples/tutorial/data/external.daml
```

The adapter receives `"label": "external"` instead of an HTTP timeout. The service
still receives three retries. Neither `app/Service.daml` nor
`contracts/Adapter.daml` changes to enumerate this provider. The entry context
selects the bundle; the configuration selects the assertion.

[Structural composition](structural-composition.md) explains this open-world model
and contrasts it with intentionally enumerated alternatives and nominal ancestry.
It also distinguishes extensibility from the `closed` record modifier.

## 5. Observe an invalid configuration

[`invalid/endpoint.daml`](../examples/tutorial/invalid/endpoint.daml) supplies
`endpoint: 42` where the app requires a string:

```sh
cargo run --locked --example native -- examples/tutorial examples/tutorial/invalid/endpoint.daml
```

This command must exit unsuccessfully with a schema violation at
`/adapter/endpoint`. It is valid DAML syntax but invalid for the selected
contract. Syntax highlighting or a successful format operation does not prove
that a configuration satisfies its definitions.

## Consume it from a program

The Rust API follows the same steps as the command-line example. This complete
`main` function reads the provider configuration, parses its data, loads the
bundle's asserted definitions, and resolves a completed, validated value:

```rust
use daml::Bundle;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string("examples/tutorial/data/provider.daml")?;
    let document = daml::parse(source)?;
    let bundle = Bundle::open("examples/tutorial")?;
    let schemas = bundle.compile(&document, None)?;
    let resolved = schemas.resolve(&document)?;
    println!("{}", serde_json::to_string_pretty(&resolved.to_json())?);
    Ok(())
}
```

Repository examples can use the `daml` crate and its already-declared
`serde_json` dependency. [`examples/native.rs`](../examples/native.rs) is the
runnable consumer with command-line arguments and explicit output-error handling.
Run it with the provider command above.

In another Rust project, use a source/path dependency on DAML, declare
`serde_json`, and select your own input and bundle paths. This project's crate is
not published to crates.io.

The distinction between source and values remains useful here: `document`
retains authored source and spans; `resolved` exposes the completed value and
its metadata. Printing `to_json()` exports plain values rather than the original
document. For other languages, see [Python](python.md#parse-complete-and-inspect)
or [browser JavaScript](../bindings/browser/README.md#load-and-edit).

## Continue

- [Native definitions](native-authoring.md): constraints, bundles and preset rules.
- [Resolution and schema interchange](schemas.md): completion and validation.
- [Examples](../examples/README.md): the larger native corpus and library consumers.
- [Python](python.md) or [browser SDK](../bindings/browser/README.md): other consumers
  of the same language engine.
- [Conversion](conversion.md): what plain exports preserve and where YAML/JSON
  interoperability still requires consumer code.
