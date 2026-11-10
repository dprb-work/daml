# DAML

**Define your configuration types in code. Author their values in DAML.**

Your program already knows what a worker needs. Its configuration shouldn't
need a second, hand-maintained description of those fields. DAML can derive a
schema from your Python types, fill in declared defaults, and tell you where a
supplied value doesn't fit.

DAML—**Definitively Asserted Markup Language**—is an experimental language for
human-authored configuration. Python is one way in; native DAML definitions,
a Rust library, and a browser SDK support other consumers.

## From types to configuration

Start with a Pydantic model:

```python
from pydantic import BaseModel
import daml

class Worker(BaseModel):
    name: str
    retries: int = 3

module = daml.schema_from_types({"app.Worker": Worker})
schemas = daml.Schemas.compose([module])
```

Then write the configuration:

```daml
!app.Worker {
  name: "demo"
}
```

`!app.Worker` selects the contract you named in code. Resolve the document:

```python
document = daml.parse('!app.Worker { name: "demo" }')
resolved = schemas.resolve(document, expected="app.Worker")
print(resolved.to_python())
```

The result includes the default:

```text
{'name': 'demo', 'retries': 3}
```

Want five retries? Supply `retries: 5`. Supply `retries: "many"` instead and the
same check rejects it at `/retries`; a default doesn't hide an invalid choice.

The Python type supplies the **schema**: requirements that many configurations
can share. The DAML document is **data**: one concrete set of values. Think
*type versus value*, with default completion as an additional step.
[Data and schema](docs/data-and-schema.md) explains the distinction without
assuming a programming-language background.

[Follow the Python quickstart](docs/python-quickstart.md) to install the bindings,
read a configuration file, change a value, and see a validation error.
Standard dataclasses work too. The result is a dictionary of resolved values,
not a constructed model instance; schema derivation supports a
[documented subset](docs/python.md#derive-schemas-from-python-types) of Python
types and declarative constraints, not arbitrary Pydantic behavior.

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

- **Use your Python types:** [follow the Python quickstart](docs/python-quickstart.md).
  Derive a schema, supply configuration, and inspect completed values.
- **Explore independent providers:** [follow the native configuration tutorial](docs/tutorial.md).
  Write definitions and swap providers without changing the consumer's requirements.
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

You can also [write definitions in DAML](docs/native-authoring.md) when the
contract shouldn't depend on Python. The [consumer examples](examples/README.md#consumer-code)
show both type-derived schemas and native definitions.

The native syntax also has first-class `!Date`, `!Time`, and `!DateTime`
values. Their explicit ISO-based profile retains precision and offset presence;
schema constraints can require or forbid offsets. See the
[temporal syntax reference](docs/syntax.md#temporal-literals),
[native temporal example](examples/temporal/), and
[Python temporal example](examples/python_temporal.py). These are source-checkout
features; the existing `0.1.0` archives predate them.

Recurring calendar patterns use `!Cron "0 0 9 LW * *"`: 09:00 on the last
weekday of each month. The [Spring cron dialect](docs/syntax.md#cron-literals)
supports seconds and calendar rules without inventing another notation. See the
[native cron bundle](examples/cron/) and [Python consumer](examples/python_cron.py).
DAML validates the pattern; the consuming application owns scheduling.

## What can I use today?

| Component | Availability |
|---|---|
| Rust library | Source; parsing, definitions, completion, validation, formatting, targeted source edits |
| Configuration CLI | [Source-built](docs/cli.md); validation, resolution, provenance/constraint explanation and comment-preserving edits |
| Formatter and language server | [Native releases](docs/distribution.md) provide syntax diagnostics/formatting; [current source LSP](docs/lsp.md) adds opt-in semantic authoring |
| VS Code extension | Source-built VSIX with explicit semantic contexts, or a qualified candidate; not published to a marketplace |
| Syntect highlighting | [Standalone grammar](integrations/syntect/README.md); consumer registration and theme ownership stay with the client |
| Python bindings | Source or [candidate wheel](docs/python.md#install-a-candidate-wheel); `daml-config` is not published to PyPI |
| Browser/WASM SDK | GitHub release archive; not published to npm |
| YAML/JSON converters | Not implemented; see [interoperability limits](docs/conversion.md) |

[Components and availability](docs/components.md) links the installation guides.
Source on `main` and versioned archives can expose different APIs; use the
reference for the version you select. Current semantic editor features require
matching source-built server and client.

The source targets coordinated `0.2.0` delivery. The
[release pipeline](docs/development/releases.md) keeps qualification and
publication separate; a manifest version does not establish download availability.

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
