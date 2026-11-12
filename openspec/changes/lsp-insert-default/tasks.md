## 1. Scope and checks

- [x] 1.1 Record Daniel's scope decision and yes/no answers in proposal.md; do not implement before acceptance.
- [x] 1.2 Independently freeze focused behavioral checks for every scenario, including chosen-provider suppression, synthetic versus explicit defaults, tag-preserving compound values, source fidelity and stale versions. Accepted check revision: `ce273ee0674a3368705252ff340d9b1383f891a8`, `tests/semantic_lsp.rs` (16 default-insertion scenarios; all 30 stdio scenarios pass).

## 2. Server implementation after acceptance

- [x] 2.1 Add bounded authored-object/default selection using checked resolution and explicit-default provenance, not completion hints; verify scalar/object/list, open/closed and exclusion scenarios.
- [x] 2.2 Add capability negotiation, kind filtering and eager versioned edit responses through the existing source editor; verify exact surrounding bytes, completed-value equivalence, encoding, error/budget behavior and no server mutation/file writes through real stdio protocol checks.
- [x] 2.3 Independently challenge the integrated path; resolve findings and examine consolidation/simplification without introducing another default resolver or editing API. The nested-parser-limit finding was accepted and repaired at the code-action boundary; regression checks cover definitions and manifests. No further production removal was required.

## 3. Handoff

- [x] 3.1 Update docs/lsp.md for action availability, canonical append style, errors and snapshot limits. `./scripts/check` passed on the complete implementation with check revision `ba932e1`; test-only compaction and removal of an owned-response copy through `ce273ee` then passed the whole affected 30-test suite. The composed CLI prerequisite passed its 36 process tests and all three actual version commands.
- [ ] 3.2 Complete independent whole-change review and record result acceptance under Daniel's delegated low-risk merge authority; retain evidence. Release, activation and any editor/plugin work require separate authorization.
