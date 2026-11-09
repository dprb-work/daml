## Baseline and delivery boundary

Issue #33 builds on accepted CLI result `c31daace273fe48e8b3e94f55e91172b20ae344f` (#32). It adds semantic editor behavior; release producers and comment-preserving edits #34 remain outside this slice.

## Context is an explicit caller decision

DAML data and native definitions share an extension and overlapping syntax. Inferring their role from a filename, workspace root, or a partially typed expression is unsound. Keep the prior syntax-only default, and select per-open-document semantics through standard `workspace/executeCommand`:

```json
{
  "command": "daml.setContext",
  "arguments": [{
    "uri": "file:///absolute/config.daml",
    "kind": "configuration",
    "bundleRoot": "file:///absolute/bundle",
    "expected": "app.Config"
  }]
}
```

`bundleRoot` and `expected` are optional for a file configuration with normal discovery/root assertion. `kind: definition` needs a local file identity and has no `expected`; its `self` name follows its bundle-relative path. `kind: syntax` clears context and accepts no bundle/expected fields. Reject unopened URI, unknown fields/kinds, nonlocal/relative roots, and invalid combinations. Untitled configuration requires a bundle; untitled definitions stay syntax-only rather than receiving invented filesystem identities.

The VS Code client adds commands `definitivelyAssertedDaml.useConfiguration`, `definitivelyAssertedDaml.useDefinition`, and `definitivelyAssertedDaml.useSyntax`. Their normal UI selects context for the active document; optional explicit argument objects support real-editor automation without fake client providers. Configuration arguments use `{uri, bundleRoot?, expected?}`; definition uses `{uri, bundleRoot?}`; syntax uses `{uri}`. UI and server enforce workspace trust and actual local URI admission independently where their boundaries require it.

Successful selection returns `{generation: number}`. A checked session-wide `u32` counter assigns a fresh nonzero generation to every accepted context change, including syntax reset; exhaustion is an explicit request failure, never a wrap. Rejected changes preserve the old identity. The result is sent before analysis notifications. This identity disambiguates same-URI/same-version context changes and close/reopen; the client buffers pending notifications and applies only the acknowledged generation for the live document and client session.

## Thin protocol owner

`src/lsp/mod.rs` retains synchronous URI/version ownership, formatting, and transport. `src/lsp/semantic.rs` owns context analysis and `src/lsp/semantic/providers.rs` presents contracts, not another validator. Source state distinguishes parsed data, parsed native syntax, bounded incomplete text, and over-limit rejection; retain source once per version. Format through the existing appropriate printer. Keep malformed/old/ranged-notification handling and position negotiation. Admit at most 64 selected semantic contexts per connection.

On context/document/watch changes, rebuild affected semantic analysis from current accepted sources and disk. Start conservatively with all active contexts rather than introducing an incremental cache with a new invalidation contract. Publish actual versions, source-related information, and cleared diagnostics. Syntax-only sessions preserve their existing behavior and do not discover schemas. Requests use current state; no background job can publish an older result.

`Encoding` also maps inbound negotiated positions to checked UTF-8 offsets. File-URI conversion is a narrow filesystem representation boundary on the already parsed LSP `Uri`: local authority only, strict percent escapes, no query/fragment, absolute paths, platform-correct native bytes. It is not a URI parser, schema resolver, or remote-fetch mechanism.

## Native compiler/source metadata owner

Feature-gated `src/native/tooling.rs` extends native compilation for the editor. Reuse the actual loader, source scopes, AST, default checks, schema registry and MR #26 graph rules. No private-URN decoding in LSP. Interfaces (crate-private under `lsp`):

- `Bundle::open_editor(root: &Path, overlays: Vec<(PathBuf, Option<&str>)>) -> Result<Bundle, NativeError>` admits borrowed source before creating an immutable owned filesystem snapshot. `Some(text)` is an accepted unsaved version; `None` marks an over-limit open version and blocks disk fallback. Canonical admission preserves nested-bundle/symlink boundaries, including newly opened nonexistent files beneath real parents. Conflicting canonical overlays fail explicitly.
- `Bundle::discover_editor(document_path: &Path, overlays: Vec<(PathBuf, Option<&str>)>, attempted: &mut Vec<PathBuf>) -> Result<Option<Bundle>, NativeError>` performs document-relative discovery through the admitted overlay. It retains bounded attempted manifest paths, including missing nearer ancestors and failures, for dependency watching; unsaved manifests are not preempted by stale disk syntax.
- `Bundle::compile_editor(document: Option<&Document>, expected: Option<&str>, additional_types: &[&str]) -> EditorCompilation` uses the same compiler. Missing document means type inspection only, never successful application-data admission.
- `EditorCompilation { schemas: Result<Schemas, NativeError>, index: EditorIndex }` retains dependency/source metadata even for compiler failures, so invalid references can still be watched and diagnosed.
- `EditorIndex::dependencies() -> &[PathBuf]` returns actual manifests/definitions consulted or attempted; `locations(&SchemaLocation) -> Vec<EditorLocation<'_>>` maps admitted contracts to actual source spans; `reference_locations(from: Option<&Path>, tag: &str) -> Vec<EditorLocation<'_>>` uses the real declaring/entry scope.
- `EditorIndex::source(path: &Path) -> Option<&str>` supplies retained source for compiler-error range conversion, without a second parser or private-ID decoding.
- `EditorLocation { path: &Path, source: &str, span: Span }` borrows retained source. Root source origins already exist in the compiler; extend/use their AST information for precise field locations. Return multiple real candidates when necessary rather than inventing a single position.
- `EditorIndex::loaded_names() -> impl Iterator<Item = &str>` supplies names compiled in the real scope. `Bundle::editor_definition_name(path: &Path) -> Result<String, NativeError>` selects the actual local `self` name, and `editor_visible_names(prefix: &str) -> Result<Vec<String>, NativeError>` supplies built-ins, visible namespaces and bounded explicit-prefix directory discovery. Enumerate names, not unreferenced bodies; do not cross private imports or nested bundle boundaries. Keep the bundle immutable and `Send + Sync`; no mutable name cache is needed.

No editor index is collected during ordinary compilation. Overlay/index/name budgets use existing profile-scale node/source limits and charge before copies. Reuse immutable retained definitions where practical. Core metadata representation may be refined within these methods without changing caller behavior.

## Parser cursor context, not a second grammar

Add bounded cursor reporting to the existing data parser under `lsp`; do not implement a competing lexer or accept a repaired buffer as valid. The protocol owner consumes:

```text
parser::editor::completion_context(source, cursor_byte, Limits)
    -> Result<Option<CompletionContext>, ParseError>
parser::editor::root_tag(source, Limits) -> Result<Option<Span>, ParseError>
parser::editor::native_completion_context(source, cursor_byte, Limits)
    -> Result<Option<CompletionContext>, ParseError>
CompletionContext {
    kind: CompletionKind::{Field, Tag, Value},
    path: String,             // RFC 6901 owning container for Field; target otherwise
    prefix: Span,             // typed prefix, ending at cursor, no leading !
    replace: Span,            // complete active name replacement, no leading !
    tags: Vec<(String,Span)>, // established path/name assertions, not the partial tag
    fields: Vec<Span>,        // established keys of the current map
}
```

Use original source spans and shared trivia/string/name grammar. Both data and native parsers report cursor observations before an intentional stop, independently of whether the enclosing container is finished. The native parser reuses the same cursor state and hint admission; no `InvalidTag`-only workaround or competing token scan remains. An already parsed native tag can reuse its AST span without reparsing. Malformed prior syntax, comments and string contents can return no context. All syntax diagnostics remain authoritative; context reporting is not data admission. Root-tag reporting permits compiling a chosen configuration contract despite an unfinished body.

## Providers and applicability

Field hints project native registry-owned schema views through references, conjunctions, alternatives, properties and item positions with bounded visited states. Flattened views nominate candidate names; eligibility and declaration collection traverse original roots and the instance path, preserving conjunction within each alternative. An ineligible branch contributes neither a name nor that field's constraints/defaults. This is structural projection, not value validation or a provider membership system. Indicate alternative/optional/required/default information and avoid already supplied known fields. The parser's shared trivia scanner identifies an existing separator so completing a key never duplicates its colon or discards a supplied value. Add selected ancestor/current tag contracts from cursor/source data; never expose dependency-private aliases.

For a fully checked data node, hover reuses `Schemas::explain`, its actual selected-path constraints and real value origins. For incomplete/invalid text, show labeled schema hints only. Definition targets come from the native source index. Native references without physical definitions (built-ins) have no fabricated navigation target.

## Dependency watch lifecycle

After selected-context analysis, send `daml/dependencies` with `{uri, version, generation, files: string[]}` of local file URIs, bounded and de-duplicated. Send an empty list when context is cleared/closed; a close uses its closing generation. Bound each update to 10,000 URIs, 32,768 code units per URI, and 8 MiB aggregate UTF-8 URI bytes; the client also bounds total distinct shared watchers to 10,000 before mutation. The client owns shared file watchers keyed by dependency URI, with per-document membership. Ignore closed/stale-version/mismatched-generation notifications; release memberships on close/context reset/dispose. Forward changes via standard `workspace/didChangeWatchedFiles`. This observes imported files outside workspace roots without a server-side watcher runtime, arbitrary global scan, or schema-fetch authority.

Generic clients can provide watched-file notifications themselves; the core server still refreshes disk-backed analysis on semantic requests. Unknown/invalid dependency notification data must not silently install arbitrary watchers. Do not alter the machine-scoped executable setting, shell-free launch, trust restriction, or single-client lifecycle.

Dependency URI publication uses native `editor_watch_paths`: resolve parents with
the existing `editor_path`, then stream each file-link entry and its current
terminal path, including missing targets. A bounded cursor follows relative or
absolute leaf-link targets; global metadata admission happens between steps and
visited entries close cycles. Retain intermediate link identities so atomic
retargeting anywhere in a file-link chain can invalidate analysis. Deduplicate
after parent resolution, including existing prefixes of missing paths.
Do not send lexical aliases such as `workspace/../providers` to editor watcher
classification: VS Code can apply an exclusion-only workspace watch and suppress
actual external changes. Context input is separate: preserve a caller's
symlink/`..` bundle path until native resolution; lexical collapse can select a
different bundle.

## Verification and gates

Independent owners freeze stdio protocol, cursor/metadata, and real-editor checks before production edits. Existing syntax/lifecycle tests remain protected. Exercise actual LSP process and real VS Code extension host/provider commands, including unsaved definition changes, saved external dependencies, Unicode positions, incomplete input, private scope and close/reopen cleanup. Run repository aggregate and editor-native commands; inspect consolidation/simplification and independent final review. Record exact-result acceptance separately; no merge/release is implied.
