## 1. Scope and checks

- [ ] 1.1 Record Daniel's scope decision and yes/no answers in proposal.md; do not implement before acceptance.
- [ ] 1.2 Independently freeze focused behavioral checks for every scenario, including chosen-provider suppression, synthetic versus explicit defaults, tag-preserving compound values, source fidelity and stale versions; retain the accepted check revision.

## 2. Server implementation after acceptance

- [ ] 2.1 Add bounded authored-object/default selection using checked resolution and explicit-default provenance, not completion hints; verify scalar/object/list, open/closed and exclusion scenarios.
- [ ] 2.2 Add capability negotiation, kind filtering and eager versioned edit responses through the existing source editor; verify exact surrounding bytes, completed-value equivalence, encoding, error/budget behavior and no server mutation/file writes through real stdio protocol checks.
- [ ] 2.3 Independently challenge the integrated path; resolve findings and examine consolidation/simplification without introducing another default resolver or editing API.

## 3. Handoff

- [ ] 3.1 Update docs/lsp.md for action availability, canonical append style, errors and snapshot limits; run focused checks and ./scripts/check on the final implementation revision.
- [ ] 3.2 Complete independent whole-change review and request human result acceptance; retain evidence. Release, activation and any editor/plugin work require separate authorization.
