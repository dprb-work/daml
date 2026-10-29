# Sources and distribution boundaries

DAML's parser and resolver are native implementations. No upstream parser source
is vendored. Consumer-schema adapters are external to the library.

## YAML Test Suite fixtures

The selected inputs come from [YAML Test Suite](https://github.com/yaml/yaml-test-suite),
data release `data-2022-01-17`, commit
[`6e6c296ae9c9d2d5c4134b4b64d01b29ac19ff6f`](https://github.com/yaml/yaml-test-suite/tree/6e6c296ae9c9d2d5c4134b4b64d01b29ac19ff6f).
The matching source release is `v2022-01-17`, commit
[`45db50aecf9b1520f8258938c88f396e96f30831`](https://github.com/yaml/yaml-test-suite/tree/45db50aecf9b1520f8258938c88f396e96f30831).
Its [MIT license](https://github.com/yaml/yaml-test-suite/blob/45db50aecf9b1520f8258938c88f396e96f30831/License),
copyright 2016–2020 Ingy döt Net, is retained verbatim in
[`tests/fixtures/yaml/LICENSE`](tests/fixtures/yaml/LICENSE).

| Upstream input | Original destination | Upstream Git blob |
|---|---|---|
| `3UYS/in.yaml` | `tests/fixtures/yaml/original/3UYS.yaml` | `e47bf72314af3dc2e2fb38c9a2709919829403c0` |
| `236B/in.yaml` | `tests/fixtures/yaml/original/236B.yaml` | `254e6fc40349322c7b48d84a035293280eca3866` |
| `2EBW/in.yaml` | `tests/fixtures/yaml/original/2EBW.yaml` | `3a5b0a3fbc08cab4d54224dfcef14677bcf92bbd` |

Original inputs are unchanged. The separate `tests/fixtures/yaml/daml/` files are
manual adaptations, not an automatic YAML-to-DAML preprocessing layer:

- `3UYS.daml`: explicit braces, a spaceless key, and the original escaped-slash
  string. The decoded value is `a/b`.
- `236B.daml`: explicit braces and a quoted value, retaining invalid trailing
  input so DAML must not silently discard it.
- `2EBW.daml`: moves the punctuation payload into a quoted string value under a
  fixed key. String payloads do not depend on the key alphabet.

The originals remain available as explicit rejection/fuzz inputs. Their being
valid YAML does not imply that DAML should accept implicit maps, plain strings,
commas, directives, anchors, or YAML block-scalar syntax. Updates are deliberate;
there is no automatic upstream synchronization.

## Authored examples

The [tutorial examples](examples/tutorial/README.md) are authored for DAML's
language and API documentation. They are self-contained and have no external
application or service dependency.

## Cargo dependencies and fuzz tooling

`Cargo.lock` owns the portable library graph; `fuzz/Cargo.lock` owns the isolated
fuzz-development graph. `bindings/python/Cargo.lock` owns the optional Python
extension graph; `bindings/browser/Cargo.lock` owns the separate WASM binding
graph. `deny.toml` owns the shared dependency/license policy.

The fuzz runtime is `libfuzzer-sys` 0.4.13, declared as
`(MIT OR Apache-2.0) AND NCSA`. Its LLVM runtime has an additional NCSA obligation;
the policy exception is scoped to that crate, not a blanket new license allowance.
The sources remain Cargo dependencies rather than copied DAML source files.
`cargo-fuzz` 0.13.2 is a development tool under MIT OR Apache-2.0.

The formatter uses the Cargo dependency `pretty` 0.12.5 (MIT), pinned in the
runtime lockfiles with default features disabled. Its normal dependencies are
`arrayvec` 0.5.2, `typed-arena` 2.0.2, and `unicode-width` 0.2.2. No printer
source was copied into DAML; the formatter uses the library's document API.
See the [published manifest](https://docs.rs/crate/pretty/0.12.5/source/Cargo.toml)
and [reuse assessment](docs/development/formatter-reuse.md) for the selection and evidence.

The shared key/tag classifier directly uses pinned `unicode-general-category`
1.1.0 (Apache-2.0), already present in the validator's dependency graph. Its
published `UNICODE_VERSION` is 16.0.0; the generated L/N/M category tables are
used without copying them into DAML. There is no Unicode normalization layer.
See the [published manifest](https://docs.rs/crate/unicode-general-category/1.1.0/source/Cargo.toml)
and [category API](https://docs.rs/unicode-general-category/1.1.0/unicode_general_category/).

The optional `lsp` feature uses `lsp-server` 0.10.0 (MIT OR Apache-2.0) and
`lsp-types` 0.97.0 (MIT), pinned in the root lockfile. Their transport and protocol
DTOs remain Cargo dependencies; no upstream code is copied. The default native
library/fuzz graph does not activate these dependencies. Policy checks also
cover the all-feature graph used by the stdio executable. See their published
manifests: [lsp-server](https://docs.rs/crate/lsp-server/0.10.0/source/Cargo.toml)
and [lsp-types](https://docs.rs/crate/lsp-types/0.97.0/source/Cargo.toml).

Native and browser packaging use `scripts/cargo-notices.py` with the build-time
tool `cargo-about` 0.9.2 (MIT OR Apache-2.0) to generate dependency notices and
license texts. The shared producer derives accepted licenses from `deny.toml`;
it introduces no second policy or runtime dependency. The owned plaintext template
uses the tool's documented output schema; no generator source is copied. See the
[tool documentation](https://embarkstudios.github.io/cargo-about/) and
[published manifest](https://docs.rs/crate/cargo-about/0.9.2/source/Cargo.toml).
Native release archives carry the canonical root `LICENSE`, generated
`THIRD-PARTY-NOTICES.txt`, and this source inventory. The upstream YAML fixture
license remains separately retained at `tests/fixtures/yaml/LICENSE`.
The [installation guide](docs/distribution.md) owns user installation;
[developer distribution](docs/development/distribution.md) owns packaging and verification.

## Python boundary and packaging

The isolated extension uses [PyO3 0.30.1](https://pyo3.rs/v0.30.1/)
(MIT OR Apache-2.0), pinned in its own Cargo lockfile. No PyO3 source is copied
into DAML. Its standard OS-string conversion preserves filesystem paths; its
native exception and object APIs own Python representation.
The pin includes the fixes for
[RUSTSEC-2026-0176](https://rustsec.org/advisories/RUSTSEC-2026-0176) and
[RUSTSEC-2026-0177](https://rustsec.org/advisories/RUSTSEC-2026-0177); neither
advisory is ignored.

PyO3's build dependency `target-lexicon` uses
[`Apache-2.0 WITH LLVM-exception`](https://docs.rs/crate/target-lexicon/0.13.5/source/LICENSE).
The exception grants additional permissions for embedded compiled portions and
GPLv2 combinations. Its allowance in `deny.toml` is limited to that crate, not
the global license list.

[maturin 1.15.0](https://www.maturin.rs/) (MIT OR Apache-2.0) owns wheel/source
packaging and Rust path-dependency inclusion. It is pinned as the build backend
and a development tool, not a runtime dependency. `bindings/python/uv.lock`
owns the remaining Python tools, including Ruff and Pytest. Python project and
Cargo metadata declare MIT. Cargo rejects a Python-workspace `license-file` path
to the root license outside that workspace; Python wheel/sdist license-text and
third-party-notice inclusion remain unverified and are not part of the release
route. Do not add a duplicate project license.

These package manifests keep crates unpublished. Python wheel and sdist assets
are not part of the current release route; if added later, they also need generated
third-party notices in addition to the project license.

The optional Python `types` extra uses the public Pydantic API and its CoreSchema
description to derive bounded validation contracts. The current Python lockfile
records Pydantic 2.14.2, pydantic-core 2.50.2, annotated-types 0.8.0 and
typing-inspection 0.4.4 (MIT), plus typing-extensions 4.16.0 (PSF-2.0).
These packages are installed dependencies, not copied or bundled Python source.
Their installed distribution metadata and license files retain their notices.
The custom-exporter regression example uses msgspec 0.22.0 (BSD-3-Clause) as a
development dependency; DAML does not import it as a built-in adapter or depend
on a consuming application. No upstream implementation is vendored for this
integration.


## Editor integration dependencies

`integrations/vscode/package-lock.json` pins the separate editor-client graph.
The VSIX bundles `vscode-languageclient` 9.0.1 (MIT) and its used runtime
dependencies, not a copied or modified protocol implementation. Packaging
generates `THIRD-PARTY-NOTICES.txt` from license/notice files belonging to packages
identified by the esbuild bundle metadata and stages the canonical DAML license.
The build also copies that exact language-client package's
`lib/node/terminateProcess.sh` beside the bundle with executable permission,
retaining its Microsoft copyright/MIT notice and the standard client's Unix
process-cleanup behavior. The dependency lock owns updates; no maintained fork
of that helper is checked in.

`integrations/verify/package-lock.json` pins independent development-only
`@vscode/test-electron` 3.1.0 and TextMate 9.3.2/Oniguruma 2.0.1 verification
tools. The Electron test helper requires Node 22 or newer; the editor workflow
continues to use Node 24.19.0. Its downloaded VS Code runtime remains pinned
to 1.105.1 in the verifier and is an ignored local test cache, not a redistributed
part of DAML. Native archives retain their separate Cargo graph and legal payload.

## Browser boundary and packaging

The isolated browser binding uses `wasm-bindgen` 0.2.129 and `js-sys` 0.3.106
(MIT OR Apache-2.0), pinned in its Cargo lockfile. Their generated web-target
glue and the compiled WASM are packaged together; no upstream source is copied
into maintained DAML source. See the [wasm-bindgen guide](https://wasm-bindgen.github.io/wasm-bindgen/)
and published manifests for [wasm-bindgen](https://docs.rs/crate/wasm-bindgen/0.2.129/source/Cargo.toml)
and [js-sys](https://docs.rs/crate/js-sys/0.3.106/source/Cargo.toml).

The matching `wasm-bindgen` 0.2.129 CLI is a build-time tool, not a browser
runtime dependency. The browser runtime has no npm dependencies, Python
requirement, automatic definition fetch, or authoring service dependency.
The independent verifier pins Playwright 1.64.1 (Apache-2.0) and TypeScript
6.0.3 (Apache-2.0) in `bindings/browser/verify/package-lock.json`; neither is
included in the browser package.

SDK tarballs carry the canonical MIT license and this inventory.
`cargo-about` 0.9.2 generates the WASM Cargo graph's notices using the same
`deny.toml` policy and plaintext template as native packaging.
`bindings/browser/scripts/package.mjs` records legal-payload hashes alongside
the build source/output provenance. The [SDK guide](bindings/browser/README.md)
owns exact values, resource admission, lifecycle, build, and package contracts.
Registry publication and promotion to a versioned release are separate actions.
