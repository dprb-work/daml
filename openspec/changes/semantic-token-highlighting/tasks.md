## 1. Accepted scope and independent checks

- [x] 1.1 Record the user's standard-category/no-palette scope decision and inspect existing parser, protocol and coordinate owners.
- [x] 1.2 Independently author and freeze stdio token classification, coordinate, incomplete-source, lifecycle and resource checks.

## 2. Implementation

- [x] 2.1 Collect bounded token observations through the existing data/native parser and shared string/trivia ownership without ordinary-parser metadata allocation.
- [x] 2.2 Advertise the standard legend and serve full-document relative tokens through existing versioned state and negotiated encodings.
- [x] 2.3 Challenge the composed path, examine consolidation/simplification and reconcile protocol documentation.

## 3. Verification and result

- [x] 3.1 Exercise actual stdio token responses and applicable existing LSP/parser boundaries; run the repository aggregate and strict OpenSpec validation.
- [x] 3.2 Complete independent whole-change review and resolve retained findings.
- [ ] 3.3 Obtain human result acceptance; no merge, release, host activation or client rendering changes.

## Ownership and refinement

Independent checks use the actual stdio binary in `tests/semantic_tokens.rs`; implementation owners do not edit their expectations. The final check-owner amendment corrects the pinned `lsp-server` response API and parser-valid fixtures, proves exact negotiated source ranges, and distinguishes the token-output limit from existing parser-node limits. Its frozen SHA-256 is `6f257fcfa43d2a75b8d72c1a2f88eb5c4aecf7857dd769b85bb892fcc9d17357`.

Parser observations retain one grammar/string/trivia owner; builtin membership reuses the existing native list, with native Regex constructor identity established at its existing grammar branch. A bounded span buffer permits rollback and keeps protocol coordinates outside the parser. Forward coordinate conversion avoids per-token prefix rescanning. Ordinary parsing has no token metadata allocation, and no second lexer, result cache, schema lookup or compatibility path was added.

Local simplification removed a redundant full-name qualification scan: builtin membership already excludes qualified spellings, and the constructor flag is admitted only at the exact native Regex branch. The integrated source challenge found no demonstrated defect or blocking structural concern. Final verification and whole-change review remain separate obligations.

Final verification passed on `385af633aa025febbe2b9df678ad7d57c7cb4120`: the six independent stdio token checks, repository `./scripts/check` (formatting, portable library, all-feature Clippy, Rust tests/doctests/rustdoc, Python checks and 151 Python tests), and strict OpenSpec validation. Existing seven LSP and fourteen semantic-editor stdio checks remain passing. Separate actual-server probes exercised source roles, latest-version replacement, UTF-8/16/32 with BOM/supplementary characters and mixed line endings, opaque raw Regex, intrinsic Date, and unchanged-version data/native role changes.

Independent whole-change review approved that revision with no findings and passing structural/alignment assessments. The subsequent task-completion metadata changes do not alter runtime source or frozen tests. Human result acceptance remains pending in the pull request; source editing, client rendering, range/delta tokens, merge and release effects are not delivered by this scope.
