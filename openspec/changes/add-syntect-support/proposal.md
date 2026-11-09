## Why

DAML needs a reusable grammar for Syntect-based consumers. Existing LSP semantic tokens do not automatically provide syntax coloring in clients that render through bundled grammars. The user explicitly selected DAML-side work framed as “add Syntect support”, rather than an OMP implementation change.

## What Changes

- Add a standalone Syntect-compatible `.sublime-syntax` grammar owned by DAML under `integrations/syntect/`.
- Recognize `.daml`, `daml.bundle` and the `daml` language token through standard grammar metadata and documented consumer registration.
- Assign conventional scopes to comments, properties, assertions/types, literals, numbers, native operators and punctuation. Keep quoted strings and native raw Regex payloads opaque, including multiline forms and unfinished fragments.
- Preserve line-by-line parser state for streaming consumers, without a language-server process, schema resolution, fixed colors or editor-specific theme settings.
- Verify actual Syntect loading and scope behavior and document loading into a consumer-owned syntax set.

## Capabilities

### New Capabilities

- `syntect-support`: portable grammar-based DAML highlighting for Syntect consumers.

### Modified Capabilities

None. The DAML parser, formatter, LSP, existing TextMate grammar and client installations keep their current contracts.

## Impact

DAML owns the grammar artifact, verification and integration guidance. Consumers own registering it in their syntax sets and applying their themes. This scope does not modify/rebuild OMP, add an LSP-to-renderer bridge, introduce a DAML runtime dependency on Syntect, or activate/release any client. DAML's shipping dependency manifests and root `deny.toml` remain unchanged. The isolated verification consumer must be independently audited under its narrowly approved admission rule and included in the existing release-security lock inventory without scanner exclusions.

## Scope decision

The user accepted the grammar-based recommendation, then clarified: “that is daml side then, then id like to have this framed around ‘add syntect support’”. This records acceptance of that bounded implementation scope, not result acceptance, merge or host-activation authority.

## Verification-tool admission decision

The user approved exactly two informational-maintenance exceptions for the pinned, isolated Syntect verification tool: [RUSTSEC-2024-0320 for yaml-rust 0.4.5](https://github.com/dprb-work/daml/issues/46#issuecomment-6752539695), then [RUSTSEC-2025-0141 for bincode 1.3.3 after the full graph audit](https://github.com/dprb-work/daml/issues/46#issuecomment-6752772936). Both are required by Syntect 5.3.0. Neither exception may enter DAML's shipping graphs or main `deny.toml`, suppress any other advisory, or excuse omitting the consumer graph from security checks.
