## 1. Accepted contract and verification

- [x] 1.1 Record accepted CLI predecessor and #33 scope; define context, source, cursor, protocol and trust boundaries.
- [x] 1.2 Independently author and freeze stdio and real-editor semantic checks; preserve existing syntax/lifecycle tests.

## 2. Implementation and integration

- [x] 2.1 Extend native loader/compiler with bounded editor overlays and source metadata; verify unsaved scopes, source locations, and ordinary compilation behavior.
- [x] 2.2 Add parser-owned cursor context with no second grammar or incomplete-data admission; verify prefixes, comments/strings, Unicode and limits.
- [x] 2.3 Integrate explicit context, semantic diagnostics/providers, dependency notifications and versioned state in the LSP adapter.
- [x] 2.4 Integrate trusted editor context commands and shared dependency watchers; verify lifecycle and external-file invalidation.
- [x] 2.5 Examine consolidation/simplification on the composed path and reconcile user/native/editor documentation.

## 3. Verification and acceptance

- [x] 3.1 Exercise the actual stdio server and real VS Code completion, hover, navigation, unsaved changes and watched-file updates.
- [x] 3.2 Run repository aggregate, focused protocol checks, editor type/build/runtime verification, and strict OpenSpec validation.
- [x] 3.3 Complete independent whole-change review, resolve findings and prepare the exact result decision without merge or release.
- [ ] 3.4 Obtain human result acceptance before dependent #34 implementation.

## Delivery state

Issue #33 follows accepted CLI result `c31daace273fe48e8b3e94f55e91172b20ae344f`; its result decision is recorded in issue #32. No host links, installed editor profiles, release producers, or primary-checkout source are changed. Human acceptance of this result gates dependent #34 implementation.

### Verification ownership

Independent checks retain real stdio and packaged VS Code providers. Amendments cover native prefixes inside unfinished containers, non-destructive field edits, branch-local closed/required constraints, checked hover applicability, missing-manifest discovery, nested-boundary recovery, and symlink/`..` context selection. Actual external-file refresh and rejected-context preservation are exercised. Physical dependency publication prevents lexical aliases from changing VS Code watcher classification. The public editor API does not expose watcher membership counts or deterministic queue draining; artificial watcher-installation rollback, delayed stale-generation watcher noncreation and case-insensitive-host behavior remain code-review boundaries, not claimed Linux runtime proof.

Consolidation keeps schema/default/validation ownership in the native compiler/resolver, source metadata with native ASTs, and cursor grammar in the parser. RFC 6901 encoding now has one `pointer::append` owner for existing joins and editor paths. Local simplification removed the proposed mutable name cache, avoids source/name/path clones at the protocol seam, and preserves one source owner per version; no additional abstraction or compatibility route is retained.
