# DAML

Definitively Asserted Markup Language.

DAML is an independent Rust project for a markup language with explicit
structure, nested type assertions through tags, and long strings. It is not
part of Olaf; Olaf is a prospective consumer.

This repository currently contains a dependency-free library crate and CI.
The parser, schema APIs, and Python bindings are not implemented yet.

## Development

Install [Rust through rustup](https://rustup.rs/). The repository pins its
toolchain in `rust-toolchain.toml`, including rustfmt and Clippy.

If Cargo is not on your shell's PATH after installation, load its environment:

```sh
. "$HOME/.cargo/env"
```

Build the library:

```sh
cargo build --locked
```

Run the same checks as CI:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

There are no tests yet because the crate has no parsing or validation behavior.
