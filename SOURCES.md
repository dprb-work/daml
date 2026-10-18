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

These notices do not select DAML's own distribution license. The library and
fuzz package remain unpublished; choosing project terms and packaging for
redistribution requires an explicit decision.
