# DAML browser SDK

The browser SDK runs DAML's Rust parser, native definition compiler, resolver, validator, and formatter in WebAssembly. It does not use Python, a server-side authoring API, or a separate JavaScript implementation of the language.

## Get the package

The browser package is a public asset of the [DAML GitHub
releases](https://github.com/dprb-work/daml/releases); it is not published to
npm. Choose the `browser-sdk-v<version>` release and download the package
archive and SHA-256 sidecar. For version 0.1.0:

```sh
base=https://github.com/dprb-work/daml/releases/download/browser-sdk-v0.1.0
archive=dprb-work-daml-browser-0.1.0.tgz
mkdir -p download && cd download
curl -fL "$base/$archive" -o "$archive"
curl -fL "$base/$archive.sha256" -o "$archive.sha256"
sha256sum -c "$archive.sha256"
tar -xzf "$archive"
```

Release assets are public and do not require authentication. Use
`shasum -a 256 -c` on macOS. Inspect `package/BUILD.json` for the source
revision and build provenance. Serve the extracted JavaScript and WASM assets
from your application; do not make browser runtime requests to GitHub release
endpoints.

## Load and edit

Publish the complete generated `dist/` tree as static assets, preserving its
relative paths, including `snippets/` and any helper modules imported by
`daml_browser.js`. The JavaScript, WASM and helper modules must come from the same
build. Serve WASM with `Content-Type: application/wasm`. Initialize once before
calling the SDK:

```js
import init, { parse, Bundle } from './dist/daml_browser.js';

await init({ module_or_path: new URL('./dist/daml_browser_bg.wasm', import.meta.url) });
const bundle = new Bundle('.', {
  'daml.bundle': '{}',
  'Config.daml': '!Object {\n  name: !String\n  retries: !Int\n} = { retries: 3 }',
});
const document = parse('!self.Config { name: "worker" }');
const schemas = bundle.compile(document);
const resolution = schemas.resolve(document);
try {
  const edited = resolution.value();
  edited.entries.find(entry => entry.key === 'name').value.value = 'edited';
  const source = schemas.write(edited);
  const admitted = parse(source);
  try {
    const checked = schemas.resolve(admitted);
    checked.free();
  } finally {
    admitted.free();
  }
} finally {
  resolution.free();
  schemas.free();
  document.free();
  bundle.free();
}
```

`Schemas.write` serializes values; it does not validate edits. Parse and resolve its output before accepting the edit. When imposing a tagless root contract, pass the same expected name to both `bundle.compile(document, 'self.Config')` and `schemas.resolve(document, 'self.Config')`. Explicit source assertions remain in force alongside the expected root.

Initialization may load the selected static WASM URL. After initialization and resource preload, SDK calls make no network requests. The host owns fetching, authentication, resource selection, worker scheduling, and persistence. APIs are synchronous after initialization; run them in a Web Worker if needed to keep the UI responsive. Modern browsers must support WebAssembly and `structuredClone`.

## API

The generated `dist/daml_browser.d.ts` is the type contract.

This reference describes the current source API. The published
`browser-sdk-v0.1.0` asset predates staged admission and the `additionalTypes`
argument: its generated declarations do not include those newer operations.
The load/edit example above works with that release. To use the newer source
APIs, [build from source](#build-from-source) at the revision you select; source
changes do not update an existing versioned archive.

| Operation | Result |
|---|---|
| `parse(source)` | Immutable `Document`; syntax only, no definition lookup |
| `document.source` | Original source, not canonical or completed output |
| `document.value()` | Owned editable semantic tree |
| `document.format(width?)` | Reflowed source without completion |
| `formatDefinition(source, width?)` | Reflowed native definition source |
| `new Bundle(root, sources)` | Explicit in-memory definition resources |
| `bundle.compile(document, expected?, additionalTypes?)` | Reachable and explicitly loaded assertion context |
| `schemas.resolve(document, expected?)` | Validated completed `Resolution` |
| `schemas.resolveStaged(document, stage, expected?, initialEdits?)` | Strict checked staged result from source |
| `schemas.resolveSemanticStaged(value, stage, expected?, initialEdits?)` | Same core stage from an editable semantic tree |
| `resolution.source` | Original input source |
| `resolution.value()` | Owned editable completed tree |
| `resolution.origins()` | Owned RFC 6901 pointer-to-origin map |
| `write(value, width?)` | Canonical source for unqualified assertions |
| `schemas.write(value, width?)` | Canonical source retaining known binding identity |
| `staged.meaning()` | Fresh selected `Uint8Array`, or `null` |
| `staged.source` | Exact original source, or `undefined` for semantic input |

Widths default to 80 and must be integers from 1 through 16384. Output uses LF and ends with a newline. Canonical semantic writing does not preserve comments, input field order, whitespace, or scalar style. It preserves exact number data, temporal kind/precision/offset presence, strings, literal keys, and representable tag assertions. Keys must obey DAML's unquoted key grammar; there is no quoted-key escape hatch.

Call `.free()` on each generated Rust-backed handle when finished. Do not use a freed handle. Returned semantic trees and origin maps are plain owned JavaScript values. Editing them never mutates a parsed document or validated resolution. A resolution snapshot survives freeing its input document and schema context.

## Exact values and assertions

Semantic nodes are discriminated objects:

```ts
type SemanticNode =
  | { kind: 'null'; tag?: Tag }
  | { kind: 'bool'; value: boolean; tag?: Tag }
  | { kind: 'number'; value: string; tag?: Tag }
  | { kind: 'string'; value: string; tag?: Tag }
  | { kind: 'date' | 'time' | 'datetime'; value: string; tag?: Tag }
  | { kind: 'list'; items: SemanticNode[]; tag?: Tag }
  | { kind: 'map'; entries: { key: string; value: SemanticNode }[]; tag?: Tag };
type Tag = { name: string; binding?: string };
```

A number's string is a typed numeric lexeme, not string data. Do not convert it to JavaScript `Number` if exact integers or decimals matter. Map entries avoid JavaScript object-key coercion; duplicate keys are rejected on admission. Unknown fields, invalid shapes, invalid numbers, cycles, and unpaired UTF-16 surrogates are rejected rather than silently coerced.

Temporal semantic values carry canonical ISO text, not a JavaScript `Date`.
Admission validates the kind and text through the same Rust temporal profile.
Snapshots and stage summaries retain the temporal kind; plain JSON projection
does not. The SDK does not coerce these values through JavaScript `Date`, infer
a machine timezone, or treat date-looking strings as temporal values.
See the [temporal syntax profile](../../docs/syntax.md#temporal-literals).

Parsed assertions have a literal name. Completed default assertions may also carry an opaque binding ID. Keep the binding when editing: removing it discards the assertion's retained identity. The plain writer rejects bound assertions. The contextual writer retains the literal name when it means the same schema, otherwise uses a deterministic root-visible equivalent name. Private bindings without such a name fail with `UnrepresentableTag`; same-shaped or constrained schemas are not interchangeable. Writing a tag never grants access to resources.

Origins distinguish input UTF-8 byte spans, definition defaults identified by
resource/schema pointer, and programmatic `semantic` input without a source span.
`tagOrigin` records assertion provenance independently when present. Default
resource identifiers are diagnostic identities, not URLs to fetch or file access.

## Staged consumer transformations

The [core staged contract](../../docs/schemas.md#staged-admission) owns candidate
completion, edits, retained assertions and strict validation. Browser stages use
that same resolver; they do not enable network lookup or recognize application
reference syntax.

Supply an object with `defer(summary, path)` and
`materialize(value, copies, round)`, plus optional `numberClass(lexeme)`.
Callbacks retain their `this` object. `defer` returns a boolean; the optional
numeric classifier returns an exact integer from 0 through 4,294,967,295.
Numbers remain strings in summaries and semantic trees.

Summaries expose only their own scalar/tag and direct field shapes. Containers
carry `kind` and `length`; map roots add `fields: [{ key, summary }]`. Field
containers have no descendants. Raw defaults may have no own tag even when
another assertion applies at the same path.

`materialize` receives fresh owned semantic values and complete applied Copy
history. `undefined`/`null` means Ready; `Uint8Array` means Ready with a meaning
key; an edit array continues; a returned `Error` rejects that candidate. Any
thrown value is fatal and propagates unchanged. Promises and malformed wire
values are rejected; the API is synchronous.
Foreign-container access failures propagate the original thrown value through the
same fatal boundary, with normal Rust handle cleanup. Meaning keys use the byte
view's intrinsic size and contents, not a shadowing `length` property.

Edits are `{ op: 'replace', path, value }`, `{ op: 'copy', path, from }`, or
`{ op: 'expect', path, name }`. Optional initial edits allow Replace and Expect
only. Load named expectations using `bundle.compile`'s additional-type array.
Every candidate replay starts at round zero; reset host-local construction
plans there rather than interpreting Copy history as an active alias graph.

`StagedResolution` owns its value, origins and selected meaning after input or
schema handles are freed. Its source is present only for source admission.
Existing `Resolution.source` stays a string. Free staged handles as usual.
Meaning keys refine invocation-local candidate identity, not persistent revision
hashes or JavaScript object-sharing semantics.

## Resource table and bounds

Resource keys are normalized slash-relative paths such as `lib/Config.daml`. The root is `.` or a normalized relative directory such as `app`. Supply the root `daml.bundle` and every reachable imported manifest/definition. Imports are relative to their declaring bundle; `.` and `./dep/` locators are supported, as are confined parent references. Absolute paths, backslashes, NULs, and escapes outside the supplied namespace are rejected. Nested bundles retain their independent visibility boundaries. Unused definition bodies and imported manifests are not parsed eagerly.

The resource table is limited to 10,000 entries, 8 MiB of UTF-8 source, and
4096 UTF-8 bytes per resource path. Source and semantic admission also enforce
8 MiB aggregate budgets, 100,000 nodes, depth 128, and 1 MiB scalars where
applicable. Canonical output and ordinary resolution provenance each have an
independent 8 MiB cap. Staged work shares its invocation budget across inputs,
summaries, snapshots, edits, histories, keys and final value/provenance export.
Budget failures are errors, not partial successful results. These bounds do not
replace application-level scheduling or a host's smaller resource limits.

## Errors

SDK-generated errors are JavaScript `Error` objects named `DamlError`. They expose `category`
and stable `kind`, with `pointer`, `path`, `reference`, `span`, `range`, `origin`,
or `schema` when available. Categories are `parse`, `native`, `resolve`,
`semantic`, and `schema`. Pointers use RFC 6901; the root pointer is empty.
Input spans use UTF-8 byte offsets. Known input-source diagnostics also carry
zero-based UTF-16 line/character ranges for browser editors. Definition/default
errors do not invent positions in the configuration source.

## Build from source

From the repository root, select the browser package directory. Use the pinned
Rust toolchain, Node.js 22 or newer, and `wasm-bindgen` 0.2.129:

```sh
cd bindings/browser
rustup target add wasm32-unknown-unknown
npm run build -- --dev  # local feedback only
npm run build
npm run package
```

Set `WASM_BINDGEN` or `CARGO` to explicit tool paths if needed. Builds honor `CARGO_TARGET_DIR`. Packaging additionally requires Python 3.11 or newer and `cargo-about` 0.9.2, reusing the repository's canonical license policy and notice template. No script installs tools, starts hosted CI, publishes to a registry, or changes the host's active installation.

`BUILD.json` records the source commit, dirty flag, source fingerprint, target, profile, Cargo/generator versions, and output hashes. Packaging rejects stale producer inputs or modified generated outputs and includes hashes of the legal payload and this README. The package carries the canonical MIT `LICENSE`, `SOURCES.md`, and generated `THIRD-PARTY-NOTICES.txt`. A dirty local package is not a clean published release, even when its checksum is valid.

The source repository's `bindings/browser/verify/README.md` owns focused native/browser parity, generated-type, and offline-after-preload verification procedures. Browser runtime assets do not include verifier tooling.
