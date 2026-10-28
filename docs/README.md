# DAML documentation

DAML is an experimental, MIT-licensed configuration language. Choose a reading
path below; you do not need the engineering history to use the language.

## Start here

1. [Understand data and schema](data-and-schema.md): supplied values versus
   requirements, with an optional type/value analogy.
2. [Install and format a file](getting-started.md).
3. [Build a configuration with definitions and defaults](tutorial.md).
4. [Read consumer code](../examples/README.md#consumer-code) in Rust, Python, or
   browser JavaScript.
5. [Explore runnable examples](../examples/README.md).

The getting-started guide uses native release executables. The tutorial uses a
source checkout and the Rust example consumer for completion and validation.
Neither path assumes a general-purpose validation or conversion CLI exists.

## Understand the design

- [Data and schema](data-and-schema.md): definition, input, completed result and
  failure; then structural typing, composition, module scope and source/value
  representation.
- [Design principles](design-principles.md): explicit structure, values, text,
  structural constraints, and well-defined completion, with their costs.
- [Compare YAML and JSON](comparison.md): syntax, schemas, ecosystems, and the
  cases where an existing format is a better fit.
- [Structural composition](structural-composition.md): independent providers,
  structural versus nominal compatibility, and open versus closed alternatives.

## Use the tools

- [Components and availability](components.md): capabilities, delivery, and status.
- [Install native tools](distribution.md): releases, platforms, and checksums.
- [Format DAML](formatting.md): commands and preservation guarantees.
- [Language server](lsp.md): setup, syntax diagnostics, formatting, and limits.
- [VS Code extension](../integrations/vscode/README.md): build, install, configure,
  and troubleshoot the VSIX.
- [YAML/JSON interoperability](conversion.md): current exports, missing converters,
  and representation limits.

## Look up exact rules and APIs

| Reference | Owns |
|---|---|
| [Syntax](syntax.md) | Data structure, keys, tags, literals, strings, spans, and parsing limits |
| [Native definitions](native-authoring.md) | Definition expressions, constraints, bundle files, imports, and native presets |
| [Resolution and schema interchange](schemas.md) | Completion/validation behavior and the low-level registered-schema API |
| [Python API](python.md) | Python installation, value representation, operations, and errors |
| [Browser SDK](../bindings/browser/README.md) | Browser setup, explicit resources, generated types, and handle lifecycle |

Generate the Rust API reference with `cargo doc --all-features --no-deps --locked`
from a source checkout. See [development setup](development/building.md) for the
pinned toolchain and required checks.

Explanations and tutorials illustrate the reference contracts; they do not define
alternative language rules. Use documentation from the source revision or release
you selected. The native and browser releases are versioned separately, and
unreleased changes on a branch are not features of an existing archive.

## Contribute

[Contributing](../CONTRIBUTING.md) leads to [development documentation](development/README.md):
building, architecture, packaging, CI, and dated research and verification records.
The [MIT license](../LICENSE) and [source notices](../SOURCES.md) remain at the
repository root.
