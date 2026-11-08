## 1. Accepted contract and independent inputs

- [x] Record the user's shared-owner/raw-text/inline-flags decision and keep PR27 unchanged.
- [x] Inventory scanner, native storage, compilation, source-origin and formatting consumers.
- [x] Independently freeze meaningful new/existing checks and observe missing-behavior red before production writes.

## 2. Consolidation and public examples

- [x] Consolidate Regex boundaries/layout in the shared scanner and remove the native loop.
- [x] Migrate admitted pattern storage, compiler/origins and shared literal formatting without compatibility shims.
- [x] Preserve normal decoded/temporal strings and legacy raw single-line Regex boundaries.
- [x] Update syntax/authoring/architecture docs, runnable verbose Regex examples and editor grammar.

## 3. Verification and handoff

- [ ] Exercise actual native/Python/browser/LSP/formatter/grammar behavior and the protected raw/decoded boundary scenarios.
- [ ] Run focused/aggregate/resource/fuzz verification and strict OpenSpec validation; record exact candidate evidence.
- [ ] Examine the surviving canonical owner for redundant work and complete independent review; repair retained findings and re-review.
- [ ] Publish the verified separate change with explicit human acceptance and merge/release boundaries.
