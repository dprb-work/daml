# Syntect support

## Purpose

Provide DAML-owned, theme-independent highlighting rules that Syntect consumers can load without a language server.

## ADDED Requirements

### Requirement: Portable grammar and consumer registration

DAML SHALL provide a self-contained `.sublime-syntax` grammar using syntax features supported by the verified Syntect version. Its scope SHALL be `source.daml`. Grammar metadata and loading guidance SHALL support `.daml` files, the exact `daml.bundle` name and the `daml` fence/language token. The grammar SHALL not depend on absolute paths, another installed DAML client, runtime schema discovery or an LSP server. Consumer-owned syntax-set registration SHALL be explicit; publishing the artifact SHALL not be claimed to update an existing consumer automatically.

#### Scenario: Load and select DAML

- **WHEN** a consumer loads the supplied definition into its Syntect syntax set
- **THEN** it can select DAML by its documented token/name and file metadata
- **AND** the grammar has no unresolved external contexts or installation-specific references.

### Requirement: Conventional scopes and faithful lexical boundaries

The grammar SHALL use conventional scopes for mapping/property names, tags/types, boolean/null and native keywords, exact numeric source, comments, strings, native raw Regex payloads, operators and collection punctuation. It SHALL emit no palette, RGB values or editor-specific color assignments. Property keys that resemble keywords, numbers or dotted type names SHALL remain properties. Contents of quoted payloads SHALL not become DAML comments, fields or operators.

#### Scenario: Configuration and definition text

- **WHEN** input contains configuration fields, qualified tags, native union/intersection/default/requiredness operators, temporal or cron constructors and signed/exponent numeric literals
- **THEN** recognized pieces receive the appropriate conventional scopes without resolving their schema meaning
- **AND** consumers remain free to style those scopes through their existing themes.

#### Scenario: Quoted and raw payloads

- **WHEN** input contains ordinary escaped quotes/backslashes, either triple-quote delimiter, or native single/multiline raw Regex
- **THEN** string/regex contents remain opaque until their appropriate closing boundary
- **AND** raw Regex backslashes are not treated as ordinary DAML escape sequences
- **AND** trailing comments regain comment scope after a closing delimiter.

### Requirement: Fragments, streaming and maintained verification

The grammar SHALL highlight useful fragments without requiring a complete parsed DAML document and SHALL retain multiline lexical state across successive complete-line parsing calls. Ordinary quoted strings SHALL not accidentally swallow unrelated following lines. Verification SHALL exercise the supplied asset through actual Syntect loading/parsing, including Unicode, supported newline handling, incomplete fragments, payload opacity and context exit. Loading guidance SHALL state line/newline mode requirements and distinguish highlighting from validation. Existing authoritative language/parser and TextMate contracts SHALL remain unchanged, and no Syntect runtime dependency SHALL be introduced into shipped DAML APIs or executables.

#### Scenario: Fragment and incremental display

- **WHEN** a consumer parses a field fragment or feeds a multiline payload over successive complete physical lines
- **THEN** it obtains useful scopes without requiring root braces or a selected schema
- **AND** its resulting scopes agree with the same input processed with retained line state in one pass.

#### Scenario: No automatic client activation

- **WHEN** this DAML grammar is delivered
- **THEN** existing OMP and other clients remain unchanged until they load/register the artifact through their own integration
- **AND** verification does not imply a client rebuild, installation or release.
