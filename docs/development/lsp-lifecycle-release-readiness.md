# Proposed release notes and readiness: LSP lifecycle (#57)

**Status:** preparation only. This record does not name a next release version, establish a qualified candidate, or authorize a tag, publication, or release.

## Draft release note

### Language server lifecycle

The VS Code client now keeps each language-server session's startup, active work, and shutdown under one lifecycle owner. When a session retires, the client stops sending it application traffic and does not replay stale work to its replacement. Pending context operations no longer hold up retirement. An obsolete or timed-out startup cannot become the active session later, and the existing bounded crash-restart policy remains in force.

The `daml-lsp` server now treats shutdown as a terminal protocol state. It acknowledges shutdown once, rejects later requests with `InvalidRequest`, ignores non-exit notifications, and waits for `exit` under one 30-second deadline. The deadline bounds protocol waits; it does not interrupt an operating-system-level blocked stdout write.

This change adds no user-facing setting or new language behavior. The VS Code extension remains a GitHub release attachment or source-buildable VSIX, not a marketplace publication. This wording is proposed for the next coordinated release; no version has been assigned.

## Historical implementation evidence

These records support the merged change. They do not qualify a release candidate.

- [PR #57](https://github.com/dprb-work/daml/pull/57), **fix(lsp): own session lifecycle and terminal admission**, merged to `main` on 2026-11-11 as `383f9850cb3fdbb2b12bcd9f74f7a991c6118f16`. Its accepted implementation revision is `e029d19d0631e4df27bfaa7767144abbbcfb81d3`.
- The PR's successful [required CI run](https://github.com/dprb-work/daml/actions/runs/42933371659) evaluated head `9774de83636c759d153ad0fca694c17d6b7f928f`. Planning, shipping-version policy, Rust/core checks, dependency policy, fuzz, VS Code integration, and native archives for Linux x64, macOS ARM64, and Windows x64 passed. Browser SDK qualification and complete release-candidate collection were skipped.
- The PR record reports `./scripts/check`, editor typecheck and canonical dependency preparation, strict OpenSpec validation, and the full installed-editor aggregate passing on Linux with VS Code 1.105.1. The installed VSIX SHA-256 was `b4ba1403549a64b83bb46d0669d1a8df431f738076ad6ceb76c31f1c489320d1`. Independent lifecycle, contract, and security reviews approved the implementation. These are historical PR records, not checks rerun for this release-preparation change.
- The accepted scope and behavior are retained in the [LSP lifecycle proposal](../../openspec/changes/archive/2026-11-11-correct-lsp-session-lifecycle/proposal.md), [task record](../../openspec/changes/archive/2026-11-11-correct-lsp-session-lifecycle/tasks.md), and [lifecycle contract](../../openspec/changes/archive/2026-11-11-correct-lsp-session-lifecycle/specs/lsp-lifecycle/spec.md).

## Current release state

The [published v0.2.1 release](https://github.com/dprb-work/daml/releases/tag/v0.2.1) was published on 2026-11-10 from `9f6dcb0eb5d530b56737dd3bad23fcdee22030b0`, before PR #57 merged. It does not contain this fix and must remain unchanged. The current source declarations in the root, Python, browser, and VS Code manifests remain `0.2.1`; a next coordinated version has not been selected in the release record.

The refreshed `main` used for this preparation was `9cef3336d6ffdd3a4efb8d1e8300732df77c76ca`, which contains PR #57. This is a source baseline, not a release candidate revision. The newest successful `Prepare release` run listed during this check was `42785680584`, for the already-published v0.2.1 source. No preparation run for the current `main` baseline was found. No open GitHub issue or pull request was returned by the live queries; that does not establish release approval or verify the current protected environment.

The release operator guide's opening status now identifies v0.2.1 as published. Its recovery record documents the prior protected run, recovery decision, and verified public assets. The guide's v0.2.1 GitHub-only selection applies to that completed release only; it does not select the next release's target.

## Remaining release gates

Complete these steps for a future release. The repository's [release operator guide](releases.md) owns the procedure.

- [ ] Select the next coordinated version and publication target explicitly. Do not reuse or overwrite v0.2.1. Advance all shipping component versions together and regenerate their lockfiles as required by the guide.
- [ ] Confirm the final accepted source is on current `main`, then run `python3 scripts/release_common.py check-version` and the canonical `Prepare release` workflow for that source.
- [ ] Retain the successful main-branch candidate and record its exact version, full source SHA, preparation run ID, artifact ID, and `RELEASE.json` SHA-256. Confirm all eight payloads and sidecars match the candidate manifest.
- [ ] Run `tests/release_candidate_check.py` with that exact tuple. Confirm the required security checks and freshness windows pass; the PR checks above do not satisfy candidate qualification.
- [ ] Recheck the protected `release` environment with `python3 scripts/release_publish.py check-environment`. Obtain the owner's approval for the exact version, source, run, manifest digest, and target through the required human gate.
- [ ] Publish only after that explicit authorization. Do not treat this draft PR, merged PR #57, historical CI, or a successful preparation run as publication authority.
- [ ] After publication, verify the tag and every public asset against the approved hashes, then verify the GitHub-attached Python wheel installation as required for GitHub-only publication. Inspect and resolve any partial effects through the guide; do not rerun blindly.
