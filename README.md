# DAML

**Definitively Asserted Markup Language** is an experimental language for
human-authored configuration with structural contracts, explicit defaults, and
readable multiline text.

## Values and requirements

The **data layer** supplies a particular configuration. The **schema layer**
describes which configurations are valid. A definition such as
[`app/Worker.daml`](examples/tutorial/app/Worker.daml) states the requirements:

```daml
closed !Object {
  name: !String
  retries: !Int
} = {
  retries: 3
}
```

The separate [data document](examples/tutorial/data/worker.daml) supplies values:

```daml
!app.Worker {
  name: "demo"
}
```

Completion supplies the omitted retry count, then validation checks the result:

```json
{
  "name": "demo",
  "retries": 3
}
```

Supplying `retries: "many"` instead fails at `/retries`; the default does not
replace an invalid supplied value. The tag selects a definition for the data to
satisfy, rather than turning that data into a schema.

If you know programming-language types, think **type versus value**, with the
additional step of default completion. Otherwise, read the distinction simply
as **requirements versus supplied contents**.
[Data and schema](docs/data-and-schema.md) explains both layers and the limits
of the analogy; [the tutorial](docs/tutorial.md) then adds nested contracts and
independent providers.

## Why DAML?

- **Explicit structure and scalar values.** Braces and brackets own nesting;
  strings are quoted, and words do not become dates or booleans implicitly.
- **Text that fits the task.** Triple-double quotes preserve multiline layout
  after dedent; triple-single quotes join wrapped prose into one logical line.
- **Definitions alongside configuration.** Express constraints and defaults in
  DAML, and validate completed values against every applicable assertion.
- **Independent composition.** Bundle-local imports and structural contracts let
  separately authored definitions work together without a central provider list.
- **Practical authoring tools.** A formatter, language server, and VS Code extension
  support editing; Rust, Python, and browser bindings support consumers.

The tradeoffs are deliberate: DAML is not YAML-compatible, cannot represent every
arbitrary JSON object key, and has a much smaller ecosystem than either format.
Read the [design principles](docs/design-principles.md) and
[YAML/JSON comparison](docs/comparison.md), including when not to use DAML.

## Try it

Start with [installation and formatting](docs/getting-started.md). Native
[release archives](https://github.com/dprb-work/daml/releases/tag/v0.1.0) contain
`daml-fmt` and `daml-lsp` for Linux x64, macOS Apple Silicon, and Windows x64.
They do not require Rust or Python to run.

For definitions, defaults, and independent providers, follow the
[configuration tutorial](docs/tutorial.md). Its files live in
[`examples/tutorial/`](examples/tutorial/README.md); validation uses the existing
Rust example consumer or a language binding, not an advertised validation CLI.

## Consume it from a program

With the source text in `source`, these operations parse it, select a bundle,
load the asserted definitions, and resolve a completed, validated value:

```rust
let document = daml::parse(source)?;
let bundle = daml::Bundle::open("examples/tutorial")?;
let schemas = bundle.compile(&document, None)?;
let resolved = schemas.resolve(&document)?;
```

The excerpt uses `?` inside a function returning `Result`. See
[the complete Rust example](docs/tutorial.md#consume-it-from-a-program),
[Python](docs/python.md#parse-complete-and-inspect), or
[browser JavaScript](bindings/browser/README.md#load-and-edit). The
[examples catalogue](examples/README.md#consumer-code) links runnable consumers.

## Available components

| Component | Current delivery |
|---|---|
| Rust library | Source; parsing, definitions, completion, validation, formatting |
| Formatter and language server | Native release archives; LSP provides syntax diagnostics and formatting |
| VS Code extension | Build and install a VSIX; not published to a marketplace |
| Python bindings | Build from source; not published to PyPI |
| Browser/WASM SDK | Separate GitHub release archive; not published to npm |
| YAML/JSON converters | Not implemented; see [conversion limits](docs/conversion.md) |

See [components and availability](docs/components.md) for installation links and
capability boundaries. Source on `main` and the separately versioned release
archives need not expose identical APIs; select the documented version you use.

## Documentation

- [Documentation index](docs/README.md): reading paths and exact references.
- [Data and schema](docs/data-and-schema.md): values versus requirements, the
  type/value analogy, and connections to familiar programming concepts.
- [Design principles](docs/design-principles.md): choices and tradeoffs.
- [Structural composition](docs/structural-composition.md): open-world providers,
  structural contracts, and deliberately closed alternatives.
- [Examples](examples/README.md): configuration, strings, bundles, and consumers.
- [Contributing](CONTRIBUTING.md): development setup, checks, and engineering notes.

## Project status

DAML is experimental and pre-1.0. Syntax, APIs, and packaging may change; do not
assume backward compatibility across revisions. Pin a source revision or release
when embedding it, and check the corresponding documentation. Current editor
support does not include schema-aware completion, hover, or semantic diagnostics.
General YAML/JSON conversion is not available.

This project is unrelated to Digital Asset's Daml smart-contract language.

## License

DAML is [MIT licensed](LICENSE). Dependency and fixture notices are recorded in
[SOURCES.md](SOURCES.md); distributed packages retain their required notices.
