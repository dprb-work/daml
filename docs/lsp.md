# DAML language server

`daml-lsp` provides native-definition syntax diagnostics and whole-document
formatting over stdio. It uses the native definition parser and formatter, but
does not resolve aliases, load bundles, or validate bundle semantics. It does
not apply defaults, discover schemas, read source files, or write editor buffers.

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
| Diagnostics | Native definition syntax errors, with accepted document version and correctly encoded byte-span ranges. Valid replacements publish an empty list. |
| Formatting | Canonical native definition reflow at 80 columns and two-space indentation, through `format_definition`. |
| Shutdown | Standard shutdown response followed by exit; graceful sessions terminate successfully even while the client stdin pipe remains open. |

Documents are identified only by client-provided URIs and text. Untitled buffers
and nonexistent file URIs work without filesystem access. Workspace roots,
initialization options, configuration notifications, process cwd, and document
locations do not select schemas or affect parsing.

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

Before sending changed text, the server checks it with the same native parser
and limits. Reflow can grow a scalar's source span, not just total file bytes;
a proposed edit must not push an otherwise valid buffer outside this server's
admission policy. This check does not mutate the current document.

| Request failure | LSP error |
|---|---|
| Unknown method | MethodNotFound (`-32601`). |
| Malformed formatting parameters or unopened URI | InvalidParams (`-32602`). |
| Current source is invalid, formatting fails, or the proposed output violates native parser admission/limits | RequestFailed (`-32803`). |

A fatal initialization-handshake, transport, or shutdown failure terminates the executable
unsuccessfully. `daml::lsp::ServerError` retains its cause for explicit inspection,
but its display context does not echo raw incoming document payloads. The binary
owns transport threads and process termination; the library adapter creates no
threads. On fatal failure the process exits rather than joining a reader blocked
on a client's still-open stdin. On graceful shutdown all provided threads join.

## Boundaries

No completion, hover, symbol indexing, incremental parsing, schema diagnostics,
editor-specific grammar, or bundle semantic integration is provided.

The protocol DTOs, framing, stdio threads, and shutdown helper come from the
pinned upstream libraries. The DAML-specific code owns versioned documents,
byte-span conversion, and parser/formatter integration. Initialization uses
fallible sends rather than upstream helpers that panic on a disconnected client;
invalid parameters leave the handshake pending so its error response can flush.
No second grammar, async runtime, JSON-RPC framer, or filesystem/configuration
host is introduced.
