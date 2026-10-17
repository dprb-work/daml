# DAML

Definitively Asserted Markup Language.

DAML is an independent Rust project for a markup language with explicit
structure, nested type assertions through tags, and long strings. It is not
part of Olaf; Olaf is a prospective consumer.

The dependency-free Rust library now parses DAML values, nested tags, ordinary
strings, dedented literal strings, and wrapped strings into a source-backed
document. Schema validation, the DAML formatter/LSP, and Python bindings are not
implemented yet.

## Design

- [Implementation plan](PLAN.md): unquoted key rules, mandatory multiline dedent,
  grammar proposals, library acceptance gates, and planned formatter/LSP milestones.
- [Parser reuse assessment](docs/parser-reuse.md): YAML/KYAML and Rust parser
  references, pinned candidates, test-corpus reuse, and license obligations.
- [Syntax contract](docs/syntax.md): implemented structure, string decoding,
  provisional key/tag token policy, source spans, errors, and resource bounds.

The syntax contract describes the parser; the plan records the subsequent
schema, formatter, and language-server milestones.

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

The gate checks formatting, runs Clippy across all targets with warnings treated
as errors, runs unit/integration tests and doctests, and builds documentation
with rustdoc warnings treated as errors. It stops at the first failure.
Focused Cargo commands remain available:

```sh
cargo check --locked
cargo fmt --all
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked
```

The pinned toolchain is also the declared minimum supported Rust version.
Toolchain upgrades must update both files and pass the aggregate gate. Supporting
an older compiler requires testing it separately; the current pin does not
establish compatibility with other versions. `Cargo.lock` is tracked and CI uses
`--locked` to prevent implicit dependency changes.

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

Add tooling when it has something to check: dependency policy when dependencies
arrive, fuzzing when a parser entry point exists, and Miri when an unsafe or FFI
boundary needs it. Extra test runners, coverage thresholds, and platform matrices
are not requirements for this dependency-free crate. Review the pinned checkout
action when maintaining CI dependencies; no automatic update service is enabled.
