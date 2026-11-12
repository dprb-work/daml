# DAML v0.2.2 release notes and readiness: LSP lifecycle (#57)

**Status:** v0.2.2 selected for GitHub-only publication. Source preparation and candidate qualification precede publication; the protected `release` environment still requires `dprbook` approval of the exact candidate tuple. This record does not claim publication.

## Release notes

### Language server lifecycle

The VS Code client now keeps each language-server session's startup, active work, and shutdown under one lifecycle owner. When a session retires, the client stops sending it application traffic and does not replay stale work to its replacement. Pending context operations no longer hold up retirement. An obsolete or timed-out startup cannot become the active session later, and the existing bounded crash-restart policy remains in force.

The `daml-lsp` server now treats shutdown as a terminal protocol state. It acknowledges shutdown once, rejects later requests with `InvalidRequest`, ignores non-exit notifications, and waits for `exit` under one 30-second deadline. The deadline bounds protocol waits; it does not interrupt an operating-system-level blocked stdout write.

This fix-only release adds no user-facing setting or new language behavior. The VS Code extension remains a GitHub release attachment or source-buildable VSIX, not a marketplace publication. All shipping components advance together to 0.2.2; no new implementation changes are included beyond current `main`.

## Historical implementation evidence

These records support the merged change. They do not qualify a release candidate.

- [PR #57](https://github.com/dprb-work/daml/pull/57), **fix(lsp): own session lifecycle and terminal admission**, merged to `main` on 2026-11-11 as `383f9850cb3fdbb2b12bcd9f74f7a991c6118f16`. Its accepted implementation revision is `e029d19d0631e4df27bfaa7767144abbbcfb81d3`.
- The PR's successful [required CI run](https://github.com/dprb-work/daml/actions/runs/42933371659) evaluated head `9774de83636c759d153ad0fca694c17d6b7f928f`. Planning, shipping-version policy, Rust/core checks, dependency policy, fuzz, VS Code integration, and native archives for Linux x64, macOS ARM64, and Windows x64 passed. Browser SDK qualification and complete release-candidate collection were skipped.
- The PR record reports `./scripts/check`, editor typecheck and canonical dependency preparation, strict OpenSpec validation, and the full installed-editor aggregate passing on Linux with VS Code 1.105.1. The installed VSIX SHA-256 was `b4ba1403549a64b83bb46d0669d1a8df431f738076ad6ceb76c31f1c489320d1`. Independent lifecycle, contract, and security reviews approved the implementation. These are historical PR records, not checks rerun for this release-preparation change.
- The accepted scope and behavior are retained in the [LSP lifecycle proposal](../../openspec/changes/archive/2026-11-11-correct-lsp-session-lifecycle/proposal.md), [task record](../../openspec/changes/archive/2026-11-11-correct-lsp-session-lifecycle/tasks.md), and [lifecycle contract](../../openspec/changes/archive/2026-11-11-correct-lsp-session-lifecycle/specs/lsp-lifecycle/spec.md).

## Current release state

The [published v0.2.1 release](https://github.com/dprb-work/daml/releases/tag/v0.2.1) was published on 2026-11-10 from `9f6dcb0eb5d530b56737dd3bad23fcdee22030b0`, before PR #57 merged. It does not contain this fix and remains unchanged.

The source baseline for PR #59 is `9cef3336d6ffdd3a4efb8d1e8300732df77c76ca`, which contains PR #57. PR #59 advances shipping declarations and path constraints to 0.2.2 and regenerates Cargo, npm and uv lockfiles using their owning tools. It adds no implementation changes. The final merged source SHA, preparation run, artifact ID and manifest digest must come from canonical main qualification, not this baseline.

Daniel authorized preparing v0.2.2 and dispatching publication with the same `github-only` destination as v0.2.1, stopping at the protected environment review. The release includes the same eight payloads and checksum sidecars: three native archives, three Python wheels, the browser SDK and VSIX, plus `RELEASE.json`. No PyPI, npm, Cargo or marketplace publication is authorized. Only `dprbook` may supply the required human environment approval.

## Remaining release gates

The repository's [release operator guide](releases.md) owns the procedure. Candidate/run and publication evidence are recorded by the release operator after these steps complete; unchecked items are not claims of completion.

- [x] Select v0.2.2 and `github-only`; advance all shipping component versions and regenerate lockfiles. Do not reuse or overwrite v0.2.1.
- [ ] Complete PR #59 checks and merge; confirm the final source is current `main`.
- [ ] Run the canonical `Prepare release` workflow and retain its version, full source SHA, preparation run ID, artifact ID and `RELEASE.json` SHA-256.
- [ ] Validate all eight payloads and sidecars with `tests/release_candidate_check.py` using that exact tuple; confirm security evidence and freshness.
- [ ] Recheck the protected `release` environment, dispatch the publication workflow with the exact tuple and `github-only`, then stop for `dprbook` environment approval. Never self-approve or bypass this gate.
- [ ] After human approval and completed publication, verify the tag and all public assets against the approved hashes, install the attached Python wheel outside the checkout, and exercise downloaded Linux x64 `--version` and one clean LSP initialize/shutdown/exit session. This verification is deferred until the operator receives the post-approval continuation.
