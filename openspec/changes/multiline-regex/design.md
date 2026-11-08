## Ownership and scope

Two paths currently own quoted scanning: ordinary strings in `src/strings.rs` and native Regex in `src/parser/native.rs`. After this change `src/strings.rs` alone owns quoted boundaries and layout. The separate behavior addition is raw Regex multiline syntax; ordinary string and existing raw single-line Regex meanings stay unchanged.

Raw versus decoded text is a real lexical contract, not a constructor registry or future plugin system. Share the boundary/layout kernel while keeping normal string escape/continuation policy distinct from raw Regex text. Do not expose caller-owned flags or arbitrary callbacks to hide different responsibilities.

## Admitted pattern and original source

The native AST's span-only Regex body cannot represent dedented/folded text. Replace that internal representation with the minimum borrowed/owned text and original literal range required by compilation, error origins and formatting. The compiler consumes admitted text; formatting consumes original spelling through the shared quoted/block renderer. A second pattern cache or reparsing helper would duplicate an existing guarantee.

The formatter must not assume one opening/closing quote. Preserve surrounding comments, syntax-only default boundaries and decoded-value stability at different structural depths. Typed temporal payloads use the same literal rendering seam where necessary; no Cron-specific branch is needed.

## Regex execution remains unchanged

`src/schema.rs` already uses regex-automata with anchored, all-match semantics and bounded automata. It does not wrap pattern text in textual anchors, so verbose trailing comments require no new engine workaround. Inline flags remain part of the pattern; use literal multiline layout for verbose comments because folded layout removes line boundaries.

## Verification ownership and delivery

An independent owner freezes new public behavioral checks and existing base verification inputs before production writes. Main observes a valid missing-behavior red, then releases implementation. Role rules/hashes on a shared filesystem are governance, not hostile write isolation. Only the check owner amends expectations or runners with a reason and new identity.

Main owns docs/examples, editor grammar, actual CLI/Python/browser/LSP/grammar scenarios, aggregate gates and exact-head independent review. PR27's pending Cron source stays unchanged. This feature branches from current main, with separate result acceptance, merge and release authority.
