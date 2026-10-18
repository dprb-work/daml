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
  fixed key. It checks string boundaries without freezing the deferred key alphabet.

The originals remain available as explicit rejection/fuzz inputs. Their being
valid YAML does not imply that DAML should accept implicit maps, plain strings,
commas, directives, anchors, or YAML block-scalar syntax. Updates are deliberate;
there is no automatic upstream synchronization.

## Cargo dependencies and fuzz tooling

`Cargo.lock` owns the portable library graph; `fuzz/Cargo.lock` owns the isolated,
unpublished fuzz-development graph. Dependency sources, licenses, advisories,
and forbidden retrieval features are checked with `deny.toml` for each graph.

The fuzz runtime is `libfuzzer-sys` 0.4.13, declared as
`(MIT OR Apache-2.0) AND NCSA`. Its LLVM runtime has an additional NCSA obligation;
the policy exception is scoped to that crate, not a blanket new license allowance.
The sources remain Cargo dependencies rather than copied DAML source files.
`cargo-fuzz` 0.13.2 is a development tool under MIT OR Apache-2.0.

The formatter uses the Cargo dependency `pretty` 0.12.5 (MIT), pinned in both
lockfiles, with default features disabled. Its normal dependencies are
`arrayvec` 0.5.2, `typed-arena` 2.0.2, and `unicode-width` 0.2.2. No printer
source was copied into DAML; the formatter uses the library's document API.
See the [published manifest](https://docs.rs/crate/pretty/0.12.5/source/Cargo.toml)
and [reuse assessment](docs/formatter-reuse.md) for the selection and evidence.

The optional `lsp` feature uses `lsp-server` 0.10.0 (MIT OR Apache-2.0) and
`lsp-types` 0.97.0 (MIT), pinned in the root lockfile. Their transport and protocol
DTOs remain Cargo dependencies; no upstream code is copied. The default native
library/fuzz graph does not activate these dependencies. Policy checks also
cover the all-feature graph used by the stdio executable. See their published
manifests: [lsp-server](https://docs.rs/crate/lsp-server/0.10.0/source/Cargo.toml)
and [lsp-types](https://docs.rs/crate/lsp-types/0.97.0/source/Cargo.toml).

Binary packaging uses the build-time tool `cargo-about` 0.9.2 (MIT OR Apache-2.0)
to generate dependency notices and license texts. Its accepted license list is
derived from `deny.toml`; it does not introduce another license policy or enter
the DAML runtime dependency graph. The owned plaintext template uses the tool's
documented output schema; no generator source is copied. See the
[tool documentation](https://embarkstudios.github.io/cargo-about/) and
[published manifest](https://docs.rs/crate/cargo-about/0.9.2/source/Cargo.toml).
CI archives are private trial artifacts, not a selection of DAML's project terms
or a public release. The [distribution guide](docs/distribution.md) owns the build,
archive, and installed-binary verification contract.

These notices do not select DAML's own distribution license. The library and
fuzz package remain unpublished; choosing project terms and packaging for
redistribution requires an explicit decision.
