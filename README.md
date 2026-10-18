# DAML

Definitively Asserted Markup Language.

DAML is an independent Rust project for a markup language with explicit
structure, nested type assertions through tags, and long strings. It is not
part of Olaf; Olaf is a prospective consumer.

The Rust library parses source-backed documents, resolves deeply nested type
assertions against independently composed schema modules, and completes partial
values from declared defaults before validation. Schemas can be written in JSON
or DAML. Reflow formatting and an optional stdio LSP are implemented; Python
bindings remain separately scoped.

## Design

- [Implementation plan](PLAN.md): unquoted key rules, mandatory multiline dedent,
  grammar proposals, completed library/tooling gates, and final-sharpening questions.
- [Parser reuse assessment](docs/parser-reuse.md): YAML/KYAML and Rust parser
  references, pinned candidates, test-corpus reuse, and license obligations.
- [Formatter reuse assessment](docs/formatter-reuse.md): reused Rust layout
  machinery, existing formatter/host designs, and the selected printer.
- [Edge-case record](docs/edge-cases.md): confirmed string framing, replayable
  observations, and explicitly unresolved language choices.
- [Reflow formatter](docs/formatting.md): width-sensitive library API and
  `daml-fmt` file/stdin/check command without schema completion.
- [Language server](docs/lsp.md): optional stdio diagnostics/formatting adapter,
  negotiated Unicode positions, version ownership, and explicit non-goals.
- [Syntax contract](docs/syntax.md): implemented structure, string decoding,
  provisional key/tag token policy, source spans, errors, and resource bounds.
- [Schema modules and defaults](docs/schemas.md): JSON/DAML schema documents,
  consumer-owned adapters, partial values, and diagnostics.
- [Hardening tools](docs/hardening.md): retained fuzz targets, bounded campaigns,
  process-isolated time/RSS measurements, and both dependency graphs.
  The [recorded baseline](docs/hardening-baseline.md) contains actual campaign
  statistics and all measurement samples.
- [Sources and distribution boundaries](SOURCES.md): pinned YAML fixtures and
  development-runtime notices.

The syntax contract owns parsing behavior; the schema guide owns completion and
validation. The plan records completed gates and unresolved language/schema-root
choices; the formatter and LSP guides own their tool contracts.

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

Compose explicitly selected modules, then call `Schemas::resolve(&document)`.
`resolve_as(&document, "app.Config")` additionally applies a named root schema,
including to an untagged root. Both parent constraints and explicit tags apply;
assertions never cast away restrictions.

Run the multi-module consumer:

```sh
cargo run --locked --example resolve -- examples/application.daml \
  examples/schemas/application.daml examples/schemas/network.json \
  examples/schemas/text.daml
```

The result includes the adapter's default timeout and the request's default ID.
Supplied values win; object defaults merge recursively, while arrays/scalars
replace defaults. Missing required fields still fail after completion. The
original source stays unchanged, and generated values identify their schema
default rather than a fabricated source span.

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

The gate checks library/fuzz formatting, the default portable-library build, and
all-feature Clippy, unit/integration tests, doctests, and warning-clean rustdoc.
It includes the optional LSP and its real stdio process contracts, and stops at
the first failure.
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
warnings for review. DAML remains unpublished until its own license is selected.

### Internal boundaries

- `src/schema/profile.rs` owns child-schema classification, traversal roles, and
  schema-admission limits. Add schema positions there, not in a second walker.
- `src/schema.rs` owns compiled schemas and registry-bound, opaque views.
  `SchemaView` resolves property, tuple/item, and branch children in its own
  registry. Raw address lookup and validator access stay behind that boundary;
  the resolver cannot construct IDs or pair raw children with another registry.
- `src/resolve.rs` owns one search and cumulative budget per resolution.
  It aggregates schema views in completion order; alternative completions reuse
  that budget rather than resetting it.
- `src/pointer.rs` owns RFC 6901 path construction and segment decoding.
  URI-fragment percent encoding remains part of schema reference resolution.
- `src/source.rs` owns LF/CRLF/CR boundary mechanics shared by parsing and
  formatting. `src/parser.rs` owns checked input admission, syntax, and the
  whitespace/comment scanner; borrowed source becomes owned only after parsing.
- `src/format.rs` builds source-preserving layout documents; the `daml-fmt`
  binary only handles arguments and bounded input/output.
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
