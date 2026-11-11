## Accepted boundary

This is DAML-side Syntect support: a portable display grammar and consumer loading guidance. OMP source, host configuration, the installed client and the existing LSP remain unchanged. The native DAML parser remains the language validator; this grammar recognizes useful display fragments without validating schemas or resolving symbols.

## Grammar ownership

The checked-in `.sublime-syntax` file owns its Syntect representation, based on the same lexical constructs as the existing TextMate integration. The formats serve different consumers; introducing a general grammar converter would add an unsupported transformation layer. Shared language examples and real consumer observations check their intended behavior without making either display grammar a second validator.

Ordinary and raw multiline contexts share the same two delimiter-closing rules. Delimiter-specific variables own those rules once; payload scopes and ordinary escape handling remain separate. Collection punctuation does not create nested parser contexts, so display-only brace nesting does not grow a scope-state stack. All scopes are conventional names, with no palette or client configuration embedded in the artifact.

Consumers select `daml`, `.daml` and `daml.bundle` through Syntect metadata and standard selection APIs. They must load/register the asset themselves and retain parser/scope state across complete physical lines. LF-oriented iterators do not automatically handle CR-only input; the verification consumer explicitly splits LF, CRLF and CR. No arbitrary mid-line chunk guarantee is added.

## Isolated verification tool

DAML's shipping graph cannot carry the Syntect loader merely for artifact checks: the existing mandatory advisory gate rejects its unmaintained transitive dependencies. Silently omitting a new graph from audit would not resolve that constraint.

The user separately approved a nonshipping consumer verifier with exactly these informational-maintenance exceptions:

- `yaml-rust 0.4.5`, [RUSTSEC-2024-0320](https://rustsec.org/advisories/RUSTSEC-2024-0320.html): [first authorization](https://github.com/dprb-work/daml/issues/46#issuecomment-6752539695).
- `bincode 1.3.3`, [RUSTSEC-2025-0141](https://rustsec.org/advisories/RUSTSEC-2025-0141.html): [additional authorization after full graph audit](https://github.com/dprb-work/daml/issues/46#issuecomment-6752772936).

The private consumer under `integrations/verify/syntect` pins Syntect 5.3.0 and its lockfile, uses the real YAML loader and parser, and never links into DAML's published APIs or shipped executables. Root `Cargo.toml`, `Cargo.lock` and `deny.toml` remain unchanged. The consumer's separate policy must retain all other existing checks, and its graph is audited in the existing mandatory dependency job. Planner regressions require that audit whenever its manifest, lock or policy changes.

Pinned cargo-deny 0.20.2 supports advisory ID/reason exceptions but not newer parent-restriction syntax. Exact manifest/lock versions and isolation are revision-bound review evidence, not a claim of unsupported automatic parent enforcement. Any additional advisory needs a new policy-owner decision; no global bypass or vendored loader fork is authorized.

## Verification and handoff

Independent exact-asset tests own scope/range observations, selection, quoted/raw boundaries, incomplete-line recovery, Unicode/newlines, retained-state equivalence and consumer theme application. They are additional to existing parser, LSP and TextMate checks. A separate actual Syntect rendering probe proves the loading/theming journey; it does not claim an existing OMP binary loaded this asset.

Result acceptance, merge and any later consumer installation remain separate human decisions. No client activation or public release follows automatically from this grammar change.
