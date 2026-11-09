# DAML language server

`daml-lsp` provides syntax diagnostics and whole-document formatting over stdio.
The source-built server adds standard semantic tokens for highlighting, plus
opt-in semantic diagnostics, completion, hover and definition navigation.
Highlighting needs no schema selection; validation and schema-aware providers
require a [semantic context](#semantic-context).

Released executables may predate semantic providers. Use matching source-built
server and client for the behavior below; see [installation](distribution.md)
for versioned release artifacts.

## Build and launch

```sh
cargo build --locked --features lsp --bin daml-lsp
cargo run --locked --features lsp --bin daml-lsp
```

Configure an LSP client to launch the built executable, with no arguments, for
DAML buffers. Standard output is reserved for LSP messages; diagnostics about
protocol/process failures go to standard error. `--help` is a standalone human
usage command, not part of a language-server session.

The optional `lsp` feature enables pinned `lsp-server` 0.10.0 and `lsp-types`
0.97.0. Default library builds do not activate protocol/transport dependencies.
`cargo check --locked --lib --no-default-features` checks that portable boundary;
`./scripts/check` checks all features, including real stdio process tests.

## Capabilities and lifecycle

| Capability | Behavior |
|---|---|
| Initialization | Standard initialize/initialized handshake and server information. Invalid parameters return InvalidParams with `InitializeError.retry: true`; initialization remains pending for client correction or exit. |
| Position encoding | First supported client offer among UTF-8, UTF-16, UTF-32; mandatory UTF-16 fallback when absent or unsupported. |
| Diagnostics | Syntax errors by default; native compilation and configuration validation in selected semantic contexts. Versioned, correctly encoded source ranges and related definition locations. |
| Semantic tokens | Full-document standard token categories and `defaultLibrary` modifier; no colors or editor theme rules. Available without a selected context. |
| Formatting | Canonical data or native definition reflow at 80 columns and two-space indentation, according to the selected role. |
| Completion | Visible assertion/reference names and contract fields, including required/optional/default details and reliable incomplete prefixes. |
| Hover | Applicable constraints/defaults; checked configuration values distinguish supplied input from completed defaults. Invalid/incomplete data receives labeled hints only. |
| Definition | Actual native source locations for references and configuration fields; no invented built-in targets. |
| Shutdown | Standard shutdown response followed by exit; graceful sessions terminate successfully even while the client stdin pipe remains open. |

Without a selected context, documents use only client-provided URIs and text.
Untitled buffers and nonexistent file URIs work without filesystem access.
Workspace roots, initialization options, configuration notifications, and cwd
do not select schemas. Closing a document clears its selected context.

One synchronous session owns each connection's document map. Duplicate opens,
changes for unopened documents, and older/equal versions cannot overwrite
accepted state. A newer full replacement supersedes older full replacements;
when one notification contains several, the final replacement wins. An empty
change list advances the accepted version without changing text. Deprecated
`rangeLength` does not alter a full replacement. Ranged changes and malformed
notifications are ignored with a brief stderr diagnostic, preserving current
state; clients must honor the advertised full-sync capability.

Close removes the cached version and publishes an empty diagnostics list.
Reopening starts a new open-document lifecycle. No background analysis job can
publish out of order because document updates and requests share one owner.
Diagnostics carry versions; clients must also discard a formatting result if
their buffer changes after the request. Standard document-formatting requests
and edits do not contain a document version, so the server does not claim to
solve client-side edit races.

## Semantic token highlighting

The source-built server advertises `semanticTokensProvider` with a standard
legend and full-document support. Request `textDocument/semanticTokens/full`
with an open document's `textDocument.uri`. Range and delta requests are not
advertised.

| Source role | Standard token type |
|---|---|
| Mapping keys and native members, including dotted literal keys | `property` |
| Qualified assertion/reference prefix | `namespace` |
| Final assertion/reference name or built-in type | `type` |
| Ordinary and temporal/cron quoted payload | `string` |
| Exact numeric source | `number` |
| Grammar keywords, including boolean/null literals | `keyword` |
| Comments outside quoted payloads | `comment` |
| Native raw regular-expression payload | `regexp` |
| Structural punctuation and operators | `operator` |

Recognized unqualified built-in types receive `defaultLibrary`; a qualified
name such as `app.String` does not acquire that modifier merely because its
last segment is `String`. The server supplies categories, not a palette.
Clients and user themes decide the appearance of those categories.

Results describe the latest accepted source using the current data/native
grammar role. They do not discover bundles, read files, resolve names or
certify validity. Incomplete buffers can return tokens for a grounded parser
prefix while retaining their syntax diagnostics; unparsed suffixes are not
assigned guessed structural roles.

Tokens are sorted, nonoverlapping and single-line. Multiline quoted payloads
are split at physical line endings without including newline bytes. Character
offsets and lengths use the negotiated UTF-8, UTF-16 or UTF-32 encoding.
The existing parser bounds and a 100,000-token output limit apply; resource
exhaustion fails explicitly, without truncated results or stale-token fallback.
There is no server-side token-result cache. Clients must discard a response
if their buffer changed after its request.

## Semantic context

Send standard `workspace/executeCommand` with exactly one argument object:

```json
{
  "command": "daml.setContext",
  "arguments": [{
    "uri": "file:///project/config.daml",
    "kind": "configuration",
    "bundleRoot": "file:///project",
    "expected": "app.Config"
  }]
}
```

| Kind | Contract |
|---|---|
| `configuration` | Compile and resolve against the root assertion and optional `expected` native type. At least one must select a root contract; both remain obligations when present. |
| `definition` | Compile the local file as a native definition, with its bundle-relative `self` identity. Does not accept `expected`. |
| `syntax` | Clear semantic context. Accepts neither `bundleRoot` nor `expected`. |

`uri` must identify an open document. `bundleRoot` is an optional absolute local
file URI; without it, discover the nearest ancestor `daml.bundle` from the
document path. Virtual configuration needs an explicit bundle. Definition
context requires a local file identity. Local file URIs reject remote
authorities, query/fragment, NUL and nonabsolute paths; `localhost` is local.
Unknown fields/kinds and invalid combinations fail without changing prior state.

Acceptance returns `{ "generation": number }` before analysis notifications.
Each accepted change, including syntax reset, receives a fresh nonzero,
session-monotonic `u32` generation. Exhaustion fails rather than wrapping.
At most 64 documents may have semantic context in one connection.
This acknowledgment accepts context; it neither certifies data nor signals that
an editor's OS watchers are armed.

Open source overrides disk, including unsaved definitions and `daml.bundle`.
Incomplete or over-limit open versions never fall back to stale disk text.
Changes, closes and watched-file notifications refresh dependent contexts.
Requests also refresh disk-backed analysis for clients without watcher support.
The existing native compiler and resolver own aliases, scopes, defaults,
validation and recursive references; the server introduces no second validator.

Hints do not rewrite source, insert defaults or certify incomplete data.
Visible-name completion enumerates bounded names, not every compatible provider
or unreferenced definition body. Strings, comments and unreliable prefixes
receive no unrelated structural suggestions.

### Dependency notifications

After context analysis, `daml/dependencies` carries:

```json
{"uri":"file:///project/config.daml","version":3,"generation":1,"files":["file:///project/daml.bundle"]}
```

These local-file URIs identify actual consulted/attempted dependencies, including
imports outside workspace roots. Parent paths use native filesystem resolution,
not lexical `..` collapse. File-symlink chains retain every entry and the current
terminal path, including missing targets, so intermediate retargeting can invalidate
analysis. Forward changes through standard
`workspace/didChangeWatchedFiles`. A syntax reset or close sends an empty list;
close uses its closing generation. Clients must match live document identity,
version, acknowledged generation and connection before installing watchers.

Each update is bounded to 10,000 files, 32,768 URI bytes per server URI and 8 MiB
aggregate UTF-8 URI bytes. The VS Code client additionally admits at most 32,768
UTF-16 units per URI and 10,000 distinct shared watchers, including tentative
replacements. Watcher replacement is transactional; failure preserves previous
memberships and is reported. Context reset, close and client disposal release
ownership. Filesystem watcher support depends on the host; replacing or retargeting
a watched parent directory may require context reselection or client restart.

## Diagnostics and source limits

The native default input, scalar, node, and depth limits apply. A rejected
version stores no source buffer; its parser diagnostic is published before the
input is discarded. A later valid full replacement recovers normally.
Over-input-limit text receives a document-level diagnostic at the start rather
than constructing a coordinate index for arbitrarily large rejected text.
These are per-document analysis limits, not a transport-frame or total-open-
document memory limit; the stdio transport assumes an editor-owned connection.

Line-ending recognition is shared with parsing: LF, CRLF, and CR. Positions use
negotiated protocol units, not terminal columns: a tab counts as one unit and a
supplementary Unicode scalar counts as four UTF-8 bytes, two UTF-16 units, or one
UTF-32 unit. EOF and final line endings are handled explicitly. Invalid internal
span offsets clamp to source boundaries rather than crashing diagnostic output.

## Formatting and request errors

A formatting request returns no edits when text is already canonical; otherwise
it returns one replacement covering the whole current document, including its
original EOF position in the negotiated encoding. The client applies that edit
and sends a newer `didChange`; the server does not mutate its cached source just
because formatting was requested. Core [formatter rules](formatting.md) remain
authoritative, including hard newlines between entries, same-line tag payloads,
comment retention, and decoded block-string preservation. Editor indentation
preferences do not introduce another DAML style/configuration system.

Before sending changed text, the server checks it with the selected data/native
parser and limits. Reflow can grow a scalar's source span, not just total file
bytes; a proposed edit must not push an otherwise valid buffer outside admission
policy. This check does not mutate the current document.

| Request failure | LSP error |
|---|---|
| Unknown method | MethodNotFound (`-32601`). |
| Malformed request parameters, unopened URI, invalid context options or inbound character boundary | InvalidParams (`-32602`). |
| Formatting source is invalid, output admission fails, or semantic/token analysis or presentation exceeds its bounds | RequestFailed (`-32803`), or a versioned diagnostic for context-analysis failures. |

A fatal initialization-handshake, transport, or shutdown failure terminates the executable
unsuccessfully. `daml::lsp::ServerError` retains its cause for explicit inspection,
but its display context does not echo raw incoming document payloads. The binary
owns transport threads and process termination; the library adapter creates no
threads. On fatal failure the process exits rather than joining a reader blocked
on a client's still-open stdin. On graceful shutdown all provided threads join.

## Boundaries

No incremental parser, global symbol index, remote schema fetch or application
constructor execution is provided. Separate [client integrations](../integrations/README.md)
own highlighting presentation, process lifecycle, synchronization, dependency
watching and edit application; the grammar is not a second language authority.

The protocol DTOs, framing, stdio threads, and shutdown helper come from the
pinned upstream libraries. DAML code owns versioned documents, explicit context,
byte-span conversion and integration with the parser, formatter and native compiler.
Initialization uses fallible sends rather than upstream helpers that panic on a disconnected client;
invalid parameters leave the handshake pending so its error response can flush.
No second grammar, async runtime, JSON-RPC framer or remote configuration host is
introduced. Semantic analysis reads explicitly selected local bundles only.
