# DAML

Definitively Asserted Markup Language.

DAML is an independent Rust project for a markup language with explicit
structure, nested type assertions through tags, and long strings.

The Rust library parses source-backed documents, loads native type definitions
from independently scoped bundles, and completes partial values before structural
validation. Deeply nested tags need no central discriminator map. A low-level
JSON Schema interchange API remains available. Reflow formatting, an optional
stdio LSP, and a separate native Python extension are implemented.

The private [v0.1.0 release](https://github.com/dprb-work/daml/releases/tag/v0.1.0)
contains verified Linux, macOS, and Windows native archives with checksums.
See the [language walkthrough](docs/language-walkthrough.md) for confirmed choices
and current behavior, and the [distribution guide](docs/distribution.md) for
archive provenance and installation.

The [editor and host integrations](integrations/README.md) provide a VS Code/Cursor
VSIX and native OMP/DSH registration payloads around the existing language server.

## Design

- [Implementation plan](PLAN.md): completed work, refinement evidence, deferred
  scope, and the next useful slice.
- [Native authoring contract](docs/native-authoring.md): direct type files,
  private bundle imports, definition expressions, completion, and acceptance cases.
- [Parser reuse assessment](docs/parser-reuse.md): YAML/KYAML and Rust parser
  references, pinned candidates, test-corpus reuse, and license obligations.
- [Formatter reuse assessment](docs/formatter-reuse.md): reused Rust layout
  machinery, existing formatter/host designs, and the selected printer.
- [Edge-case record](docs/edge-cases.md): confirmed string framing, replayable
  historical observations, and the current decisions that supersede them.
- [Reflow formatter](docs/formatting.md): width-sensitive library API and
  `daml-fmt` file/stdin/check command without schema completion.
- [Language server](docs/lsp.md): optional stdio diagnostics/formatting adapter,
  negotiated Unicode positions, version ownership, and explicit non-goals.
- [Python API](docs/python.md): exact values, retained assertions, native bundles,
  structured errors, and private wheel/source packaging.
- [Syntax contract](docs/syntax.md): structure, string decoding, Unicode key/tag
  grammar, source spans, errors, and resource bounds.
- [Schema modules and defaults](docs/schemas.md): low-level interchange,
  consumer-owned adapters, partial values, and diagnostics.
- [JSON Schema 2020-12 audit](docs/json-schema-2020-12-audit.md): backend
  coverage, native expressibility, deliberate profile limits, and migration evidence.
- [Hardening tools](docs/hardening.md): retained fuzz targets, bounded campaigns,
  process-isolated time/RSS measurements, and dependency-policy checks.
  The [recorded baseline](docs/hardening-baseline.md) contains actual campaign
  statistics and all measurement samples.
- [Sources and distribution boundaries](SOURCES.md): pinned YAML fixtures and
  development-runtime notices.

The syntax contract owns data parsing; the native authoring guide owns definition
expressions and bundle loading. The schema guide owns structural validation and
completion. The formatter and LSP guides own their tool contracts.

## Parse a document

```rust
let document = daml::parse("!app.Config { name: \"demo\" }")?;
let name = document.get(document.root(), "name").unwrap();
assert_eq!(document.string(name), Some("demo"));
assert_eq!(document.tag(document.root()), Some("app.Config"));
```

Pass a `String` to transfer source ownership, or `&str` to copy it once after
checking the input bound. Keys, tags, numbers, and unchanged string contents
reference source byte spans. Numbers preserve their spelling without conversion
through floating point. Parsing preserves tags; it does not validate a tag's
schema, import it, or execute code.

Run the complete example consumer:

```sh
cargo run --locked --example inspect -- examples/application.daml
```

It prints the tagged tree, original byte spans, and decoded string values.
Malformed input exits with a structured error category and byte location.
Use `parse_with_limits` to choose smaller input/scalar/node/nesting bounds;
the hard nesting ceiling is 128.

## Complete and validate a document

Native definitions use one `.daml` file per type and a `daml.bundle` map of
bundle aliases to local directories. Imports stay private to each bundle:

```rust
let bundle = daml::Bundle::open("examples/native")?;
let source = std::fs::read_to_string("examples/native/data/omitted.daml")?;
let document = daml::parse(source)?;
let schemas = bundle.compile(&document, None)?;
let resolved = schemas.resolve(&document)?;
assert_eq!(resolved.root().get("name").unwrap().to_json(), "demo");
```

`None` loads the root and nested assertions written in the configuration.
To impose an additional root contract, pass `Some("app.Config")` to `compile`
and then use `schemas.resolve_as(&document, "app.Config")`. Both expected and
explicitly tagged contracts remain in force.

Run the native multi-bundle example:

```sh
cargo run --locked --example native -- examples/native examples/native/data/omitted.daml
```

See [native authoring](docs/native-authoring.md) for the definition syntax,
manifest, private imports, and executable acceptance cases.

Sequence definitions distinguish repeated constraints (`!List { items: !Int }`)
from exact positions (`!Tuple [!Int]`). Both validate ordinary sequence values;
tuple defaults follow the complete definition, as in `!Tuple [!Int] = [1]`.

### Low-level interchange

Compose explicitly selected modules, then call `Schemas::resolve(&document)`.
`resolve_as(&document, "app.Config")` additionally applies a named root schema,
including to an untagged root. Both parent constraints and explicit tags apply;
assertions never cast away restrictions.

Run the multi-module consumer:

```sh
cargo run --locked --example resolve -- examples/application.daml \
  examples/schemas/application.json examples/schemas/network.json \
  examples/schemas/text.json
```

The result includes the adapter's default timeout and the request's default ID.
Untagged partial objects retain contextual presets; explicitly tagged objects
replace those presets and receive their selected definition's defaults.
Lists/scalars replace defaults. Required containers are completed before full
validation; missing required primitive values still fail. The source stays
unchanged, and generated values retain default provenance, never fabricated
input spans.

`examples/schema_adapter.rs` converts a made-up vendor record format into a
native schema module. The adapter maps its record marker and default annotation,
registers a canonical resource, and binds root aliases and a nested type with ordinary `$ref`s:

```sh
cargo run --locked --example schema_adapter
```

See the [schema guide](docs/schemas.md) for the module format and completion rules.

## Development

Install [Rust through rustup](https://rustup.rs/). The repository pins its
toolchain in `rust-toolchain.toml`, including rustfmt, Clippy, and the standard
library sources used by rust-analyzer. Use rust-analyzer through your editor's
LSP integration; no editor-specific configuration is required.

The aggregate gate also requires [uv](https://docs.astral.sh/uv/getting-started/installation/).
It provisions the Python 3.12 binding environment from its lockfile; Python is
not a runtime dependency of the Rust library or native executables.

If Cargo is not on your shell's PATH after installation, load its environment:

```sh
. "$HOME/.cargo/env"
```

Build the library:

```sh
cargo build --locked
```

Run the same aggregate gate locally and in CI:

```sh
./scripts/check
```

The gate checks library/fuzz formatting, the default portable-library build,
all-feature Clippy, Rust tests/doctests, and warning-clean rustdoc. It also runs
the isolated Python extension's build, Clippy, Ruff, and behavioral tests.
The optional LSP includes real stdio process contracts. Checks stop at the first
failure; `./scripts/check-python` runs the binding checks alone.
Focused Cargo commands remain available:

```sh
cargo check --lib --no-default-features --locked
cargo fmt --all
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps --locked
```

The pinned toolchain is also the declared minimum supported Rust version.
Toolchain upgrades must update both files and pass the aggregate gate. Supporting
an older compiler requires testing it separately; the current pin does not
establish compatibility with other versions. `Cargo.lock` is tracked and CI uses
`--locked` to prevent implicit dependency changes.

Check dependency advisories, licenses, sources, and forbidden retrieval features:

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo deny --locked check
cargo deny --all-features --locked check
```

CI runs this separately from the Rust gate. `deny.toml` allows the licenses used
by the selected dependencies and forbids validator HTTP/file/async retrieval
features. No advisories are ignored. Duplicate dependency versions remain
warnings for review. DAML's own code is [MIT licensed](LICENSE). Distribution
uses private GitHub releases; crate and PyPI publication remain disabled.

### Internal boundaries

- `src/schema/profile.rs` owns child-schema classification, traversal roles, and
  schema-admission limits. Add schema positions there, not in a second walker.
- `src/schema.rs` admits local schema shape and declarations in `SchemaModule`.
  Its private admitted values cannot be mutated by callers. Composition owns
  aggregate budgets, cross-module collisions/references, and compiled validators,
  rather than revalidating local shape. Registry-bound `SchemaView` values resolve
  property, tuple/item, and branch children in their own registry; the resolver
  cannot construct IDs or pair raw children with another registry.
- `src/resolve.rs` owns one search and cumulative budget per resolution.
  It aggregates schema views in completion order; alternative completions reuse
  that budget rather than resetting it.
- `src/pointer.rs` owns RFC 6901 path construction and segment decoding.
  URI-fragment percent encoding remains part of schema reference resolution.
- `src/source.rs` owns LF/CRLF/CR boundary mechanics shared by parsing and
  formatting. `src/parser.rs` owns checked input admission, syntax, and the
  whitespace/comment scanner; borrowed source becomes owned only after parsing.
- `src/names.rs` owns the shared Unicode key/tag classifier.
- `src/native` owns definition syntax, bundle filesystem admission, and lowering
  to the existing in-memory schema engine. It adds no second validator or
  application-construction path. Default-validation schema traversal reuses the
  schema profile's child-position classifier.
- `src/format.rs` owns shared source-preserving layout and trivia mechanics;
  `src/native/format.rs` renders definition expressions through that printer.
  The `daml-fmt` binary only handles arguments and bounded input/output.
- The optional `src/lsp` adapter owns in-memory URI/version state and protocol
  coordinate conversion. It reuses the checked parser and formatter; protocol
  dependencies and the stdio binary are activated only by the `lsp` feature.
  It does not read source/schema files or select a schema-folder root.

## Implementation defaults

Keep the library synchronous and safe: the manifest forbids unsafe code and
warns about undocumented public APIs. Prefer concrete types, small functions,
enums for alternatives, explicit ownership, and structured errors for invalid
input. Avoid cloning data merely to silence ownership errors, speculative
traits, and an async runtime without an actual I/O concurrency requirement.

Use stable rustfmt and Clippy defaults. Do not enable whole pedantic or
restriction lint groups, add a custom formatter style, or introduce a workspace
before there is a separate crate boundary. Integration tests exercise complete
documents, ambiguous/unsupported syntax rejection, exact string transformations,
source spans, and resource bounds.

The library depends on a pinned JSON Schema validator and JSON/Serde support.
The isolated fuzz package exercises the real parser, schema ingestion, and
resolver with AddressSanitizer. Follow the [hardening guide](docs/hardening.md)
for campaigns and release time/RSS measurements. Add Miri when an unsafe or
FFI boundary needs it. Extra test runners, coverage thresholds, and platform
matrices are not baseline requirements.
Review the pinned checkout action when maintaining CI dependencies; no automatic
update service is enabled.
