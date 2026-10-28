# Building and checks

Install [Rust through rustup](https://rustup.rs/). Run commands from the repository root unless a command says otherwise. DAML pins its Rust toolchain in `rust-toolchain.toml`; the pin includes rustfmt, Clippy, and standard-library sources for rust-analyzer. The pin is also the declared minimum supported Rust version. Supporting an older compiler requires testing it separately.

The full local gate needs Rust and [uv](https://docs.astral.sh/uv/getting-started/installation/). `uv` provisions the Python 3.12 binding environment from its lockfile. Python is not a runtime dependency of the Rust library or native executables.

If Cargo is not on your shell's PATH after installing Rust, load its environment:

```sh
. "$HOME/.cargo/env"
```

## Build and test

Build the library:

```sh
cargo build --locked
```

Run the aggregate gate used by CI:

```sh
./scripts/check
```

It checks library and fuzz formatting, the default portable-library build, all-feature Clippy, Rust tests and doctests, warning-clean rustdoc, and isolated Python extension build, Clippy, Ruff, and behavioral tests. Checks stop at the first failure. Run `./scripts/check-python` for the binding checks alone.

Focused Rust commands are also available:

```sh
cargo check --lib --no-default-features --locked
cargo fmt --all
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps --locked
```

Check dependency advisories, licenses, sources, and forbidden retrieval features:

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo deny --locked check
cargo deny --all-features --locked check
```

CI runs dependency checks separately from the Rust gate. `deny.toml` allows the licenses used by selected dependencies and forbids validator HTTP, file, and async retrieval features. No advisories are ignored; duplicate dependency versions remain warnings for review. The isolated fuzz package and Python binding have their own policy checks.

`Cargo.lock` is tracked, and CI uses `--locked` to prevent implicit dependency changes. A toolchain upgrade must update the pin and pass the aggregate gate. DAML's code is MIT licensed; see the root `LICENSE`.

## CI path selection

The path planner classifies changed files before installing heavier tools. See [CI path selection](ci.md) for the recorded workflow contract, path map, check groups, and platform limits. That document describes the workflow at its recorded revision; inspect current workflow files before using it as evidence of current CI behavior.

Changes should include focused regression coverage where behavior changes and preserve the relevant API and resource limits. Run the aggregate gate before requesting review when practical. There are no additional documentation-specific governance gates.