## Accepted boundary

The user selected standard LSP token roles/modifiers, with no editor-specific colors, and directed implementation. The server supplies source classifications; clients own appearance. This change adds no client adapter or theme configuration and does not implement pending source edits (#34).

## Existing owners

`src/parser.rs` and `src/parser/native.rs` own data/native grammar. `src/strings.rs` owns ordinary, multiline and native raw-regex boundaries; `scan_trivia` owns comments. Existing editor cursor observations already extend these parser paths without admitting repaired text. Highlighting must use those same owners, including native lookahead/backtracking, not tokenize a second grammar with regular expressions.

`src/lsp` owns one synchronous URI/version session. Its retained `Text` distinguishes data, native, incomplete and rejected versions. Token requests use this latest text without refreshing schema context, discovering bundles, reading files or materializing defaults. Highlighting is available without a selected semantic context. A selected context affects the existing data/native grammar role, not whether tokens are allowed.

## Protocol representation

Use a fixed legend of standard `property`, `namespace`, `type`, `string`, `number`, `keyword`, `comment`, `regexp` and `operator` types, plus `defaultLibrary`. Numbers in the response index this legend; the order is an encoding detail, not a theme contract. Property keys stay properties even when dotted or keyword-looking. Only actual unqualified built-in types receive `defaultLibrary`; a final name in `app.String` is not the built-in `String`.

Full-document relative tokens are sufficient for the accepted capability. Do not advertise range or delta requests, retain result caches or add refresh jobs. Single-line, nonoverlapping tokens work regardless of client multiline/overlap support. Split multiline payloads at shared LF/CRLF/CR boundaries and convert offsets/lengths to the session's negotiated UTF-8/UTF-16/UTF-32 units. Coordinate encoding must progress through source rather than rescan the whole prefix per token.

Input/scalar/node/depth admission stays with the existing parser. Bound the final single-line stream to 100,000 tokens; excess work fails explicitly rather than returning truncated or cached success. Syntax errors may leave useful parser-grounded prefix tokens; resource failures are errors, not ordinary incomplete-prefix success. Ordinary parsing does not collect token metadata.

## Evidence and verification

The [LSP 3.17 semantic-token specification](https://github.com/microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.17/language/semanticTokens.md) defines standard categories/modifiers, legend indices, relative five-integer encoding, negotiated position units and multiline/overlap capabilities. The repository pins `lsp-types` 0.97.0 and `lsp-server` 0.10.0; their DTOs/framing remain authoritative. No new dependency or wire codec is needed.

Independent real-stdio checks own classification, opacity, relative coordinates, incomplete prefixes, resource failure/recovery and lifecycle behavior. Main also exercises an actual stdio token request outside those checks, runs the repository aggregate and obtains independent whole-change review. There is no claim about client rendering or a released/installed server until a separately authorized client/release action occurs.
