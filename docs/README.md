# DAML documentation

Start with what you want to do. You don't need to learn the whole language before
seeing a configuration work.

## Find your starting point

- **Show me why I'd use this.** [Build a service configuration](tutorial.md).
  Run a small example, make a choice, see an error, then swap providers without
  changing the service's requirements. This path uses a source checkout and Rust.
- **Let me try editing a file.** [Install and format your first file](getting-started.md).
  Use the native formatter and language server without building Rust or Python.
- **Explain the model first.** [Data and schema](data-and-schema.md) follows one
  worker from supplied values to requirements and defaults. The type/value
  analogy is there if it helps, but isn't a prerequisite.
- **Show me the code.** [Consumer examples](../examples/README.md#consumer-code)
  cover Rust, Python, and browser JavaScript. [The examples catalogue](../examples/README.md)
  links the runnable files.

The tutorial uses handwritten DAML definitions to show native composition. A
separate bounded Python workflow derives in-memory schema modules from explicitly
named Pydantic models or dataclasses; it does not generate native `.daml` files.
See the [Python API guide](python.md#derive-schemas-from-python-types).

## Understand the choices

- [Design principles](design-principles.md): why structure is explicit, defaults
  are deliberate, and a shared contract can be better than a provider catalogue.
- [Compare YAML and JSON](comparison.md): what you gain, what you give up, and
  when an existing format is the better fit.
- [Structural composition](structural-composition.md): why an independently
  authored provider can fit, and why openness, ancestry, and alternatives are
  different questions.

## Use the tools

[Components and availability](components.md) is the overview. For a specific task:

- [Install native tools](distribution.md), [format a file](formatting.md), or
  [configure an LSP editor](lsp.md).
- [Build and install the VS Code extension](../integrations/vscode/README.md).
- [Understand YAML/JSON interoperability](conversion.md), including what plain
  exports lose and which converters aren't implemented.

## Look up exact rules and APIs

These pages are references, not lessons to finish before writing a file:

| Reference | Use it for |
|---|---|
| [Syntax](syntax.md) | Collections, keys, tags, literals, strings, and parsing limits |
| [Native definitions](native-authoring.md) | Constraints, bundle imports, composition, and presets |
| [Resolution and schema interchange](schemas.md) | Completion, validation, and the registered-schema API |
| [Python API](python.md) | Installation, operations, value representation, and errors |
| [Browser SDK](../bindings/browser/README.md) | Setup, explicit resources, generated types, and handle lifecycle |

For Rust API documentation, run `cargo doc --all-features --no-deps --locked`
from a source checkout. [Development setup](development/building.md) covers the
toolchain. Use docs for the revision or release you've selected: source on `main`
and the separately versioned native/browser archives can expose different APIs.

## Work on DAML

[Contributing](../CONTRIBUTING.md) leads to the [development guides](development/README.md)
for builds, architecture, packaging, CI, and technical research. The project is
[MIT licensed](../LICENSE); [SOURCES.md](../SOURCES.md) retains the required notices.
