# semantic-tokens Specification

## Purpose
Expose DAML source roles as standard LSP token types and modifiers so clients and users choose their appearance.

## Requirements

### Requirement: Standard symbolic legend and full-document requests

The server SHALL advertise a `semanticTokensProvider` legend using only `property`, `namespace`, `type`, `string`, `number`, `keyword`, `comment`, `regexp` and `operator` token types and `defaultLibrary` modifier. It SHALL support `textDocument/semanticTokens/full` and SHALL NOT advertise range or delta support. It SHALL return no palette, RGB values, editor-specific scopes or theme configuration.

#### Scenario: Client chooses presentation

- **WHEN** a client initializes and requests full tokens for an open DAML buffer
- **THEN** the legend maps numeric token indices to the standard categories and modifiers
- **AND** the result is a standard relative integer token stream, leaving colors and overrides to the client.

### Requirement: Parser-owned source classification

The parser SHALL classify mapping/member names as `property`, qualified assertion/reference prefixes as `namespace` and their final name as `type`, strings as `string`, exact numeric source as `number`, grammar keywords as `keyword`, comments as `comment`, native raw regex payloads as `regexp` and structural/operator punctuation as `operator`. Recognized unqualified built-in types SHALL receive `defaultLibrary`; a qualified name ending in a built-in spelling SHALL NOT be mistaken for that built-in. Dotted literal property names SHALL remain properties. String/comment contents SHALL not become structural symbols. Classification SHALL reuse existing grammar, names, quoted-boundary and trivia owners, not a second lexer or regular-expression grammar. Ordinary non-highlighting parsing SHALL not allocate token metadata.

#### Scenario: Configuration and native definitions

- **WHEN** buffers contain tagged configuration, a native definition with unions/defaults, temporal/cron values, or a native raw regex
- **THEN** token roles follow the appropriate existing data/native syntax
- **AND** string/regex/comment contents remain opaque, including escapes, multiline delimiters and embedded punctuation.

#### Scenario: Grounded incomplete prefix

- **WHEN** a buffer is incomplete or has a syntax error
- **THEN** tokens may describe source reached reliably by the ordinary parser, including recognized unfinished lexical payloads where their boundaries are known
- **AND** no repaired data is admitted, diagnostics are unchanged, and unparsed suffixes are not assigned invented roles.

### Requirement: Correct coordinates and bounded latest-state results

Tokens SHALL be sorted, positive-length, nonoverlapping and split at physical LF, CRLF or CR line endings without including newline bytes. Relative character offsets and lengths SHALL use the session's negotiated UTF-8, UTF-16 or UTF-32 units. Token classification SHALL use the latest accepted open-document source and existing data/native role; it SHALL require no selected schema context or filesystem access. Unknown/unopened URIs and malformed parameters SHALL return InvalidParams. Rejected input or parser/token resource exhaustion SHALL return RequestFailed rather than stale or silently truncated success. Highlighting output SHALL be limited to 100,000 single-line tokens in addition to existing parser input/scalar/node/depth limits.

#### Scenario: Unicode and multiline strings

- **WHEN** text includes supplementary Unicode, a BOM, tabs, mixed physical line endings or multiline strings
- **THEN** decoding the token stream selects the intended original source ranges in the negotiated encoding
- **AND** every emitted token fits one physical line without overlap.

#### Scenario: Changes, close and recovery

- **WHEN** a document receives newer and then stale changes, closes/reopens, or recovers from rejected oversized input
- **THEN** token requests use only the latest admitted lifecycle state
- **AND** no old source/token cache survives as an apparently successful result.
