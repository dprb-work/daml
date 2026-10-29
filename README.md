# DAML

**Configuration with structural contracts, explicit defaults, and room to grow.**

Your program needs configuration. You want people to supply the values that
matter, leave routine choices to defaults, and get useful errors when something
doesn't fit. When another package adds a compatible implementation, you shouldn't
have to edit a central list just to describe it.

DAML—**Definitively Asserted Markup Language**—is an experimental language for
that kind of human-authored configuration.

## Say what you want to configure

Here is a worker configuration:

```daml
!app.Worker {
  name: "demo"
}
```

The file names a worker and leaves its retry count to the definition. That
[definition](examples/tutorial/app/Worker.daml) says what a worker needs:

```daml
closed !Object {
  name: !String
  retries: !Int
} = {
  retries: 3
}
```

Complete and validate the configuration, and you get:

```json
{
  "name": "demo",
  "retries": 3
}
```

Want five retries? Supply `retries: 5`. Supply `retries: "many"` instead and the
same check rejects it at `/retries`; a default doesn't hide an invalid choice.

The definition is the **schema**: requirements that many configurations can
share. The configuration is **data**: one set of concrete values. If you know
programming-language types, think *type versus value*, with defaults as an
additional step. [Data and schema](docs/data-and-schema.md) walks through the
example without assuming that background.

## Add a provider without rewriting the consumer

A larger configuration might choose an adapter:

```daml
!app.Service {
  name: "demo"
  adapter: !providers.Http { endpoint: "https://example.com" }
}
```

In the [tutorial](docs/tutorial.md), the service requires an adapter with a string
endpoint. The HTTP definition adds a timeout and supplies its default. Another
bundle supplies a different adapter with the same required endpoint and its own
settings. Both can fit without the service definition enumerating their names.

This is **structural compatibility**: the chosen value has to meet the consumer's
requirements as well as its own definition. Imports make providers available;
the tag makes the choice explicit. [Structural composition](docs/structural-composition.md)
explains why that matters, and when deliberately limiting the alternatives is a
better choice.

## Try the language

- **See it work:** [follow the configuration tutorial](docs/tutorial.md). Run a
  worker, change a value, then add defaults and independent providers.
- **Just try editing:** [install the formatter and language server](docs/getting-started.md).
  Native archives run without Rust or Python; the VS Code extension is a separate
  source-built VSIX.
- **Understand the choices:** read the [design principles](docs/design-principles.md)
  and [YAML/JSON comparison](docs/comparison.md).

DAML uses explicit braces and brackets rather than indentation to decide nesting.
It also has distinct multiline forms for preserved layout and wrapped prose.
Those choices help with authoring, but the main reason to choose it is the
combination of readable values, reusable constraints, defaults, and composition.

## Use the result in your program

Given configuration text in `source`, the Rust API does the work in four steps:

```rust
let document = daml::parse(source)?;
let bundle = daml::Bundle::open("examples/tutorial")?;
let schemas = bundle.compile(&document, None)?;
let resolved = schemas.resolve(&document)?;
```

The excerpt belongs inside a function returning `Result`. The tutorial has a
[complete Rust consumer](docs/tutorial.md#consume-it-from-a-program); there are
also [Python](docs/python.md#parse-complete-and-inspect) and
[browser JavaScript](bindings/browser/README.md#load-and-edit) examples.

The Python API also supports a bounded path from explicitly named Pydantic v2
models and standard dataclasses to in-memory JSON Schema modules; it does not
generate native `.daml` definition files or typed instances. See the
[Python types example](examples/python_types.py) and [Python API guide](docs/python.md).
Other workflows can use handwritten DAML definitions.

## What can I use today?

| Component | Availability |
|---|---|
| Rust library | Source; parsing, definitions, completion, validation, formatting |
| Formatter and language server | [Native releases](docs/distribution.md); syntax diagnostics and formatting |
| VS Code extension | Build and install a VSIX; not published to a marketplace |
| Python bindings | Build from source; not published to PyPI |
| Browser/WASM SDK | Separate GitHub release archive; not published to npm |
| YAML/JSON converters | Not implemented; see [interoperability limits](docs/conversion.md) |

[Components and availability](docs/components.md) links the installation guides.
Source on `main` and versioned archives can expose different APIs; use the
reference for the version you select. Editor support does not yet include
schema-aware completion, hover, or semantic diagnostics.

## Is DAML a good fit?

Consider it when people maintain configuration with meaningful defaults and
independently developed components. For ordinary data interchange, JSON is often
a better choice. If existing tools require YAML, DAML won't replace that ecosystem:
it is not YAML-compatible and cannot represent every arbitrary JSON object key.

DAML is experimental and pre-1.0. Syntax, APIs, and packaging may change; pin a
revision or release when embedding it. Use the [documentation index](docs/README.md)
for the learning path and exact references, or [Contributing](CONTRIBUTING.md) to
work on the project. This project is unrelated to Digital Asset's Daml
smart-contract language.

## License

DAML is [MIT licensed](LICENSE). [SOURCES.md](SOURCES.md) records the dependency
and fixture notices retained in distributed packages.
