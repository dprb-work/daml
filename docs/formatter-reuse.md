# Formatter reuse assessment

Evidence reviewed on 2026-10-18, initially against DAML `79241ba`. After reflow
was requested, the implementation selected pinned MIT `pretty` 0.12.5 and proved
it against the source-sensitive fixtures below. No upstream code or fixtures
were copied. This is not a performance comparison or project-license decision.

## Keep language decisions separate from presentation

The user confirmed the triple-string framing below. The
[edge-case record](edge-cases.md) distinguishes that decision from 45 executable
observations and the remaining provisional key/tag alphabet. Formatter work
preserves existing meanings rather than introducing parser rules.

The current [syntax contract](syntax.md#triple-string-boundaries) requires a line
ending after either triple opener, allowing only spaces/tabs before it. The
opening line ending and one line ending immediately before the closing line
are framing, not string data. For example:

```daml
"""
  hello
"""
```

The decoded value is `"hello"`, without a leading or trailing newline. A rebuilt
`inspect` consumer at the reviewed DAML revision also observed:

| Input variation | Current result |
|---|---|
| An additional blank line after the opening line | `"\nhello"` |
| An additional blank line before the closing line | `"hello\n"` |
| Inline `"""hello"""` | Rejected as invalid string syntax |
| CRLF delimiter/content line endings | `"hello"` |
| Empty block, opener immediately followed by its closing line | `""` |

The user confirmed these framing rules. Other observations and unresolved
language choices remain explicit in the edge-case record rather than being
settled indirectly by formatter presentation.

## What can actually be reused

| Candidate | Primary-source facts | DAML recommendation |
|---|---|---|
| **`tiny_pretty` 0.4.5** | Rust Wadler-style document printer with nesting, groups, hard lines, and width-dependent breaks. MIT; its published manifest has an optional `unicode-width` dependency and no required normal dependency. `Doc::text` must not contain line breaks. [API][tiny-api], [document primitives][tiny-doc], [manifest][tiny-manifest] | Compact native alternative. Its public document API does not provide signed/dynamic nesting; preserving blank-only literal bodies under structural indentation would need additional handling. Not selected for this implementation. |
| **`pretty` 0.12.5** | Rust Wadler-style printer with boxed, reference-counted, and arena allocation APIs. MIT; required dependencies include `arrayvec`, `typed-arena`, and `unicode-width`. [API][pretty-api], [manifest][pretty-manifest] | Selected. Dynamic nesting and signed indentation offsets keep literal bodies outside structural indentation without a custom renderer. No executed performance evidence favors it or `tiny_pretty`. |
| **Pretty YAML 0.6.0**, revision `c4068f4` | Rust YAML formatter. Its formatter package uses `yaml_parser`, Rowan, and `tiny_pretty`; its dprint integration lives in a separate package. MIT. [formatter manifest][yaml-manifest], [plugin manifest][yaml-plugin], [README][yaml-readme] | Closest concrete Rust implementation reference, especially the separation of formatter library and host integration. Not a drop-in DAML formatter: its parser and rules target YAML. |
| **dprint / `dprint-core` 0.71.0** | Host supports Wasm and process plugins; plugins need not use its printer. Native Rust `dprint-core::formatting` provides print items and rendering. Its MIT manifest keeps formatting, Wasm, process communication, and async runtime features distinct. Host CLI already provides file selection, ignore handling, stdin, `check`, and incremental formatting. [plugins][dprint-plugin], [core API][dprint-api], [manifest][dprint-manifest], [CLI][dprint-cli] | Candidate for reusing surrounding tooling rather than writing our own formatting platform. A thin plugin should call the same Rust formatting library. Adopting dprint as host does not require adopting its printer, or vice versa; Wasm is not required for native core use. |
| **Topiary**, revision `7c61712` | Generic formatter driven by Tree-sitter queries, producing leaf-content/formatting atoms and indentation directives. Its CLI checks idempotence by default. [design][topiary-design], [pipeline][topiary-pipeline] | Strongest inspiration for token-preserving indentation. Direct adoption requires a DAML Tree-sitter grammar rather than consuming the existing native parser's tree. Avoid introducing a second syntax authority solely to obtain its engine. |
| **Prettier** | AST printers produce a document IR, which the engine measures and lays out. Plugins still supply language parsing/printing and optional comment attachment/printing behavior. [architecture][prettier-design], [plugin contract][prettier-plugin] | Borrow the separation of language rules from layout and its comment-handling lessons. A Prettier plugin remains an option, but does not eliminate DAML-specific work and adds a JavaScript integration boundary to the Rust core. |

## Smallest approach for the current scope

**Selected approach:** reuse `pretty`'s layout solver and arena for structural
reflow, building documents from the existing AST and source spans. No second
parser or formatting engine is introduced. The parser and formatter share one
whitespace/comment scanner; comments are read from AST-delimited source gaps.

The decisive fixture is a blank-only triple-double body. Such a body has zero
dedent margin, so adding structural spaces to every body line changes its value.
`pretty` provides [dynamic nesting][pretty-nesting] and
[signed nesting offsets][pretty-builder], allowing the formatter to preserve
body indentation and independently align the closing delimiter. `tiny_pretty`
was the initial source-only preference, but this demonstrated requirement favors
the existing capability in `pretty`. This is not a speed or code-size claim.

The [formatter guide](formatting.md) owns the public API and presentation rules.

Two DAML constraints must survive any engine adapter:

- Collection entries require physical newlines. A generic soft break that can
  flatten to a space cannot separate multiple entries.
- A tag's payload starts on the same physical line. Moving that payload to the
  next line changes the parse; strings also have their own newline/dedent rules.

These follow from [the syntax contract](syntax.md#structure-and-names), not the
printer's preferred style. YAML/JSON formatting followed by comma removal or
quote substitution is not a valid adaptation strategy.

For repository-wide CLI/editor integration, consider a dprint plugin before
building file discovery, ignore handling, incremental formatting, or a broad
configuration system ourselves. Keep that adapter outside the portable Rust
formatter. A small standalone file/stdin command is still possible; this note
has not compared its implementation cost with a dprint adapter.

The implemented approach passed nested commented, bare-tag, relative-indent,
blank-only literal, escaped-delimiter, continuation, and wrapped-string fixtures.
These checks require idempotence and unchanged values/tags/number spelling.
All 45 recorded edge observations were replayed; the 29 accepted cases also
passed actual formatter/check/inspection comparisons at widths 20 and 80.
No alternative engine or host plugin was benchmarked or implemented.

[tiny-api]: https://docs.rs/tiny_pretty/0.4.5/tiny_pretty/
[tiny-doc]: https://docs.rs/tiny_pretty/0.4.5/tiny_pretty/enum.Doc.html
[tiny-manifest]: https://docs.rs/crate/tiny_pretty/0.4.5/source/Cargo.toml
[pretty-api]: https://docs.rs/pretty/0.12.5/pretty/
[pretty-manifest]: https://docs.rs/crate/pretty/0.12.5/source/Cargo.toml
[yaml-manifest]: https://github.com/g-plane/pretty_yaml/blob/c4068f4dbf96f59c631c2259b42890e072f8b7c1/pretty_yaml/Cargo.toml
[yaml-plugin]: https://github.com/g-plane/pretty_yaml/blob/c4068f4dbf96f59c631c2259b42890e072f8b7c1/dprint_plugin/Cargo.toml
[yaml-readme]: https://github.com/g-plane/pretty_yaml/blob/c4068f4dbf96f59c631c2259b42890e072f8b7c1/README.md
[dprint-plugin]: https://dprint.dev/plugin-dev/
[dprint-api]: https://docs.rs/dprint-core/0.71.0/dprint_core/formatting/
[dprint-manifest]: https://docs.rs/crate/dprint-core/0.71.0/source/Cargo.toml
[dprint-cli]: https://dprint.dev/cli/
[topiary-design]: https://topiary.tweag.io/book/
[topiary-pipeline]: https://github.com/topiary/topiary/blob/7c6171250504e00e78f26fd4408ae0e193a61954/docs/book/src/reference/formatting-pipeline.md
[prettier-design]: https://prettier.io/docs/technical-details
[prettier-plugin]: https://prettier.io/docs/plugins
[pretty-nesting]: https://docs.rs/pretty/0.12.5/pretty/trait.DocAllocator.html#method.nesting
[pretty-builder]: https://docs.rs/pretty/0.12.5/pretty/struct.DocBuilder.html#method.nest
