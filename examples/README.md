# DAML examples

Run source-checkout commands below from the repository root with the pinned Rust
toolchain installed. For native binaries without a checkout, start with
[the getting-started guide](../docs/getting-started.md).

## Learn in order

| Example | What to observe |
|---|---|
| [Plain configuration](tutorial/data/plain.daml) | Explicit object structure and quoted string values |
| [Worker definition](tutorial/app/Worker.daml) | Schema layer: field constraints and an explicit preset |
| [Worker data](tutorial/data/worker.daml) | Data layer: particular values and a root assertion; resolution supplies retries |
| [Invalid retry count](tutorial/invalid/retries.daml) | A valid data document that fails its schema at `/retries` |
| [Multiline text](tutorial/data/text.daml) | Wrapped prose versus dedented literal text |
| [Configured service](tutorial/data/configured.daml) | A root assertion and completion of the retries default |
| [HTTP provider](tutorial/data/provider.daml) | Independent provider assertions and the provider's timeout default |
| [Third-party provider](tutorial/data/external.daml) | The same consumer accepts a different structural definition |
| [Invalid endpoint](tutorial/invalid/endpoint.daml) | Expected validation failure at `/adapter/endpoint` |

Follow the [tutorial](../docs/tutorial.md) for definitions, commands, and expected
results. [The example bundle](tutorial/README.md) is the canonical source of those
files. Files under `tutorial/invalid/` are deliberately invalid against the
selected definitions, not templates to copy.

## Inspect and consume

```sh
cargo run --locked --example inspect -- examples/tutorial/data/text.daml
cargo run --locked --example native -- examples/tutorial examples/tutorial/data/provider.daml
```

The first command displays source spans, tags, and decoded strings. The second
prints completed plain JSON; its tag metadata remains in the library result,
not in that plain export.

## Consumer code

These examples show how a program consumes DAML, rather than only its syntax:

| Language or operation | Code and reading path |
|---|---|
| Rust parsing and resolution | [Complete consumer](native.rs) and [short annotated example](../docs/tutorial.md#consume-it-from-a-program) |
| Rust source inspection | [Tree and source-span inspector](inspect.rs) |
| Python | [Parse, complete, and inspect](../docs/python.md#parse-complete-and-inspect) |
| Browser JavaScript | [Load, edit, write, and revalidate](../bindings/browser/README.md#load-and-edit) |

Start with [values versus requirements](../docs/data-and-schema.md) if the
schema/data distinction is new. That explanation also maps DAML concepts to
record types, assertions, intersections/unions, default completion, and modules.

## Explore detailed behavior

- [`native/`](native/): native bundle/default cases, including expected failures;
  [the native reference](../docs/native-authoring.md#executable-examples) explains
  the corpus.
- [`application.daml`](application.daml) and [`schemas/`](schemas/): low-level
  registered JSON Schema interchange, separate from native authoring.
- [`schema_adapter.rs`](schema_adapter.rs): a made-up vendor schema adapter, not
  a general YAML/JSON converter.

```sh
cargo run --locked --example resolve -- examples/application.daml examples/schemas/application.json examples/schemas/network.json examples/schemas/text.json
cargo run --locked --example schema_adapter
```

Performance measurements and historical consumer fixtures belong to
[development records](../docs/development/README.md), not the introductory path.
