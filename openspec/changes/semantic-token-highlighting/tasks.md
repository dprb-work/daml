## 1. Accepted scope and independent checks

- [x] 1.1 Record the user's standard-category/no-palette scope decision and inspect existing parser, protocol and coordinate owners.
- [x] 1.2 Independently author and freeze stdio token classification, coordinate, incomplete-source, lifecycle and resource checks.

## 2. Implementation

- [x] 2.1 Collect bounded token observations through the existing data/native parser and shared string/trivia ownership without ordinary-parser metadata allocation.
- [x] 2.2 Advertise the standard legend and serve full-document relative tokens through existing versioned state and negotiated encodings.
- [x] 2.3 Challenge the composed path, examine consolidation/simplification and reconcile protocol documentation.

## 3. Verification and result

- [ ] 3.1 Exercise actual stdio token responses and applicable existing LSP/parser boundaries; run the repository aggregate and strict OpenSpec validation.
- [ ] 3.2 Complete independent whole-change review and resolve retained findings.
- [ ] 3.3 Present the verified result for human acceptance; no merge, release, host activation or client rendering changes.

## Ownership and refinement

Independent checks use the actual stdio binary in `tests/semantic_tokens.rs`; implementation owners do not edit their expectations. The final check-owner amendment corrects the pinned `lsp-server` response API and parser-valid fixtures, proves exact negotiated source ranges, and distinguishes the token-output limit from existing parser-node limits. Its frozen SHA-256 is `e3ae070856831a2231f5531cc66617732bbfdd57b1517628aba09d7e3250ad4a`.

Parser observations retain one grammar/string/trivia owner; builtin membership reuses the existing native list, with native Regex constructor identity established at its existing grammar branch. A bounded span buffer permits rollback and keeps protocol coordinates outside the parser. Forward coordinate conversion avoids per-token prefix rescanning. Ordinary parsing has no token metadata allocation, and no second lexer, result cache, schema lookup or compatibility path was added.

Local simplification removed a redundant full-name qualification scan: builtin membership already excludes qualified spellings, and the constructor flag is admitted only at the exact native Regex branch. The integrated source challenge found no demonstrated defect or blocking structural concern. Final verification and whole-change review remain separate obligations.
