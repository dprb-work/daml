## 1. Accepted scope and verification ownership

- [x] 1.1 Record the user-selected DAML-side “add Syntect support” boundary and inspect existing grammar/integration conventions.
- [x] 1.2 Settle policy-compliant actual Syntect verification tooling and independently freeze lexical/streaming/selection checks.

## 2. Implementation and documentation

- [x] 2.1 Implement the portable grammar with conventional scopes and correct ordinary/raw/multiline lexical contexts.
- [x] 2.2 Document grammar registration, theme ownership, line mode and the boundary from validation/LSP/client activation.
- [x] 2.3 Challenge the composed path and examine consolidation/simplification without introducing a second language validator or unnecessary runtime tooling.

## 3. Verification and result

- [ ] 3.1 Exercise actual Syntect loading, source scopes, fragment/streaming behavior and theme application; run applicable project gates and strict OpenSpec validation.
- [ ] 3.2 Complete independent whole-change review and resolve findings.
- [ ] 3.3 Obtain human result acceptance; keep merge, consumer registration and host activation separately authorized.

## Current boundary

Worktree `feat/syntect-support` implements a DAML-owned grammar artifact only. No OMP source, installed OMP binary or existing native LSP configuration is changed. The user approved only [RUSTSEC-2024-0320](https://github.com/dprb-work/daml/issues/46#issuecomment-6752539695) and [RUSTSEC-2025-0141](https://github.com/dprb-work/daml/issues/46#issuecomment-6752772936) for the pinned, isolated Syntect verification tool. Main dependency manifests and `deny.toml` remain unchanged; the separate consumer graph must still be independently audited, with no other advisory exception. Its license audit explicitly includes dev dependencies.
