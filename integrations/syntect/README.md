# Syntect support

DAML supplies [`daml.sublime-syntax`](daml.sublime-syntax) for Syntect-based renderers. It describes **Definitively Asserted Markup Language**, not Digital Asset's Daml language.

The grammar assigns conventional scopes; the consumer's theme supplies colors and styles. It needs no language server, schema bundle or DAML runtime library. Highlighting is best-effort presentation, not parsing or validation of a complete configuration.

## Load and select the grammar

Read or embed the supplied asset as `grammar_source`, then add it to the consumer's `SyntaxSetBuilder` before building the immutable syntax set:

```rust
use syntect::parsing::{SyntaxDefinition, SyntaxSetBuilder};

let definition = SyntaxDefinition::load_from_str(grammar_source, true, None)?;
let mut builder = SyntaxSetBuilder::new();
builder.add_plain_text_syntax();
builder.add(definition);
let syntaxes = builder.build();
```

To extend an existing syntax set, use its `into_builder()` instead of creating an empty builder. Apply this grammar only where the consumer selects DAML; do not replace unrelated languages or their themes.

The loader's `true` argument selects lines that retain newline characters. Use the corresponding mode consistently throughout parsing. Syntect's [`SyntaxDefinition::load_from_str`](https://docs.rs/syntect/5.3.0/syntect/parsing/struct.SyntaxDefinition.html#method.load_from_str) and [`SyntaxSetBuilder`](https://docs.rs/syntect/5.3.0/syntect/parsing/struct.SyntaxSetBuilder.html) own those contracts.

Select the loaded syntax using:

- `find_syntax_by_token("daml")` for a fenced-code/language token;
- `find_syntax_for_file(path)` for `.daml` files and the exact `daml.bundle` name;
- `find_syntax_by_scope("source.daml".parse()?)` for explicit scope selection.

Retain the same Syntect parse/scope state across complete lines when displaying multiline strings or native raw Regex. A display fragment does not need root braces or a schema to receive useful scopes. Consumers own line splitting; normalize or correctly split CR-only input before using LF-oriented line iterators. Arbitrary mid-line streaming chunks must be buffered into complete lines rather than treated as independent parser calls.

## Theme and language ownership

This asset contains no palette or editor-specific color configuration. Consumers can use their existing theme or style these emitted scope families:

| Source role | Scope |
|---|---|
| Root | `source.daml` |
| Property/key | `entity.name.label.daml` |
| Tagged or bare type name | `entity.name.type.tag.daml`, `entity.name.type.daml` |
| Comment | `comment.line.number-sign.daml` |
| Boolean/null | `constant.language.daml` |
| Number | `constant.numeric.daml` |
| Native keyword | `keyword.control.daml` |
| Ordinary quoted payload | `string.quoted.double.daml`, `string.quoted.triple.double.daml`, `string.quoted.triple.single.daml` |
| Native raw Regex payload | `string.regexp.daml` |
| Ordinary escape | `constant.character.escape.daml` |
| Operators and delimiters | `keyword.operator.daml`, `punctuation.definition.collection.daml`, `punctuation.definition.string.begin.daml`, `punctuation.definition.string.end.daml` |

The native DAML parser remains authoritative for grammar admission and semantic validation. The separate [TextMate grammar](../vscode/syntaxes/daml.tmLanguage.json) serves TextMate consumers. The [LSP server](../../docs/lsp.md) provides diagnostics, formatting and schema-aware operations independently of this highlighting asset.

## Consumer activation and dependencies

A consumer must load/register this file through its own integration. Adding it to the DAML repository does not modify OMP, install a grammar into an existing application, rebuild a client or change any user's theme.

Syntect itself belongs to the consuming application or verification tool, not to DAML's shipped APIs/executables. Syntect 5.3.0 uses unmaintained `yaml-rust 0.4.5` and `bincode 1.3.3`. Its isolated DAML verification tool has separately approved exceptions for [RUSTSEC-2024-0320](https://github.com/dprb-work/daml/issues/46#issuecomment-6752539695) and [RUSTSEC-2025-0141](https://github.com/dprb-work/daml/issues/46#issuecomment-6752772936), respectively. DAML's shipping manifests and root `deny.toml` remain unchanged, and no other advisory is waived. The verifier lockfile joins the existing release-security inventory without a scanner exclusion. Consumers must apply their own dependency policy; these approvals are not a blanket waiver for applications that load the grammar.

## OMP integration

OMP 18.18.5 embeds its Syntect syntax set in the native addon; it has no supported global grammar-registration extension API. Its message-renderer hooks do not register languages for code fences, file previews and diffs. See the version-pinned [native highlighter](https://github.com/can1357/oh-my-pi/blob/v18.18.5/crates/pi-natives/src/highlight.rs) and [extension API](https://github.com/can1357/oh-my-pi/blob/v18.18.5/docs/extensions.md).

DAML maintains the portable grammar and these loading instructions. Further OMP-specific loader/plugin or upstream integration work is deferred until OMP v2 can be assessed. This decision does not assume a v2 release date or promise that v2 supports custom grammars.

### Optional source integration for OMP 18.18.5

This route reproduces the verified source integration; it is not a recommendation to maintain an OMP patch after every update.

1. Copy `daml.sublime-syntax` into OMP's `crates/pi-natives/src/syntaxes/DAML.sublime-syntax`. Preserve the DAML copyright, [MIT license](../../LICENSE) and source revision in the vendored source and distributed native notices.
2. Add `("DAML", include_str!("DAML.sublime-syntax"))` to `EXTRA_SYNTAXES` in `crates/pi-natives/src/syntaxes/builder.rs`. This feeds OMP's existing compiled syntax-set builder; it does not change the theme mapper.
3. In `packages/tui/src/lang-from-path.ts`, map the `daml` extension to `["daml", "daml"]`. Make the exact `daml.bundle` basename return `daml` in both `getLanguageFromPath` and `detectLanguageId`; do not claim every `.bundle` file.
4. Prepare the checkout with OMP's required Bun version, pinned Rust toolchain and native build prerequisites. From the OMP repository root, build the addon and then the standalone executable:

   ```sh
   bun install --frozen-lockfile --ignore-scripts
   bun run build:native
   bun packages/coding-agent/scripts/build-binary.ts
   ```

5. Run `packages/coding-agent/dist/omp` and inspect a `daml` code fence, a `.daml` file preview and `daml.bundle` selection before activation. Types, fields, numbers and comments should receive theme-owned styles; hashes inside quoted payloads must remain string content.
6. Preserve the existing executable before explicitly activating the candidate. Restart OMP to load the new native renderer; an already-running session retains its old renderer.

Copying the grammar beside an installed OMP executable does not register it. Replacing only the cached native addon is not a reliable installation method: the standalone's embedded payload can restore its own addon. A normal OMP update can replace a local source integration. DAML does not maintain an automatic patch/rebuild procedure for OMP.

## Verify the supplied artifact

The isolated consumer pins Syntect 5.3.0 with the `regex-fancy` backend and loads this exact file through its standard YAML loader. From the DAML checkout:

```sh
cargo test --manifest-path integrations/verify/syntect/Cargo.toml --locked
cargo deny --manifest-path integrations/verify/syntect/Cargo.toml \
  --config integrations/verify/syntect/deny.toml --locked check
```

Use the repository-pinned cargo-deny 0.20.2 for the scoped audit. Its profile includes dev-dependency licenses and only the two approved informational exceptions. The repository aggregate `./scripts/check` also checks the consumer's formatting, Clippy and tests; the existing dependency CI job audits its graph independently. Future edits to the consumer manifest, lockfile or policy select that mandatory audit.

Checks cover metadata selection, useful fragments, Unicode and physical newline forms, quoted/raw payload boundaries and context exit, retained line-state equivalence, and application of consumer themes without fixed RGB expectations. They do not claim validation of fragments or automatic registration in an installed client.
