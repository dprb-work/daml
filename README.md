# DAML

Definitively Asserted Markup Language.

DAML is an independent Rust project for a markup language with explicit
structure, nested type assertions through tags, and long strings. It is not
part of Olaf; Olaf is a prospective consumer.

This repository currently contains a dependency-free library crate and CI.
The parser, schema APIs, and Python bindings are not implemented yet.

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
before there is a separate crate boundary. There are no behavioral tests yet
because parsing and validation are not implemented.

Add tooling when it has something to check: dependency policy when dependencies
arrive, fuzzing when a parser entry point exists, and Miri when an unsafe or FFI
boundary needs it. Extra test runners, coverage thresholds, and platform matrices
are not requirements for this dependency-free crate. Review the pinned checkout
action when maintaining CI dependencies; no automatic update service is enabled.
