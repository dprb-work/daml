# Building and checks

Install [Rust through rustup](https://rustup.rs/). Run commands from the repository root unless a command says otherwise. DAML pins its Rust toolchain in `rust-toolchain.toml`; the pin includes rustfmt, Clippy, and standard-library sources for rust-analyzer. The pin is also the declared minimum supported Rust version. Supporting an older compiler requires testing it separately.

Core checks need Rust and [uv](https://docs.astral.sh/uv/getting-started/installation/). Full source checks also need Node.js 24.19.0/npm, Xvfb, cargo-deny 0.20.2, cargo-fuzz 0.13.2, and the fuzz package's pinned nightly toolchain. CI prepares those tools before invoking the shared command. Python is not a DAML runtime dependency.

If Cargo is not on your shell's PATH after installing Rust, load its environment:

```sh
. "$HOME/.cargo/env"
```

## Build and test

Build the library:

```sh
cargo build --locked
```

Before pushing, run the same path-routed gate as PR CI from a clean worktree:

```sh
./scripts/check-changes
```

The command fetches `origin/main`, checks the complete merge-base path set, and
uses `scripts/ci-plan.py`. Docs run whitespace checks; ordinary source runs
core; affected editor inputs add editor verification; risky/shared/unknown
inputs run every source group. No bypass option is provided.

Enable the tracked hook with `git config core.hooksPath .githooks`, or invoke
the normal hook explicitly with `git -c core.hooksPath=.githooks push`.
The hook uses that same command, checks outgoing tips against the clean
checkout, and rejects a changed head or dirty checkout after verification.
Push other outgoing tips from their own clean worktrees. No prior receipt
store exists here and none is introduced.

Run the full source suite explicitly:

```sh
./scripts/check-tier full
```

This runs planner/admission regressions and the same core, dependency-policy,
fuzz/sanitizer, and installed-editor/LSP groups as nightly. On an isolated
headless machine without a usable Chromium sandbox, set
`DAML_VSCODE_NO_SANDBOX=1`; do not use that option for a normal editor session.
Use `CARGO_BUILD_JOBS=2` to match hosted fuzz compilation limits.

`./scripts/check` remains the core group: library/fuzz/Syntect formatting,
portable-library build, all-feature Clippy, Rust tests/doctests, rustdoc,
Syntect consumer checks, and isolated Python build/Clippy/Ruff/tests.
Run `./scripts/check-python` for only the binding checks.

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

CI runs dependency checks separately from the Rust gate. Root `deny.toml` allows the licenses used by selected dependencies and forbids validator HTTP, file, and async retrieval features; it ignores no advisories. Duplicate dependency versions remain warnings for review. The isolated fuzz package and Python binding have their own policy checks. The nonshipping [Syntect verifier](../../integrations/syntect/README.md#verify-the-supplied-artifact) has a separate locked audit with exactly two approved informational-maintenance exceptions; its lockfile also remains in the release-security scan inventory.

`Cargo.lock` is tracked, and CI uses `--locked` to prevent implicit dependency changes. A toolchain upgrade must update the pin and pass the aggregate gate. DAML's code is MIT licensed; see the root `LICENSE`.

## CI path selection

The path planner classifies changed files before installing heavier tools. See [CI path selection](ci.md) for the recorded workflow contract, path map, check groups, and platform limits. That document describes the workflow at its recorded revision; inspect current workflow files before using it as evidence of current CI behavior.
PR CI builds native executables on Ubuntu 24.04; default-branch pushes run no heavy checks. The full source suite, including fuzz and editor/LSP behavior, runs nightly. Risky/shared/unknown PRs automatically run it too. Platform packaging, browser SDK qualification, and complete release security remain in the existing [exact-candidate release/platform gate](../../.github/workflows/prepare-release.yml), also reachable through the unchanged default manual `Check` dispatch.

Changes should include focused regression coverage where behavior changes and preserve the relevant API and resource limits. Run the routed gate before requesting review, or let the normal push hook run it once. Use the explicit full-source command when needed. There are no additional documentation-specific governance gates.