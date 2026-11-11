# Backlog

## Leaner builds and smaller worktrees

Follow up after 0.2.0. Worktree size is already a concern; adding language bindings should not multiply full toolchains, dependency installations and build outputs in every checkout.

Before choosing a design:

- Measure disk use by source, Rust targets, Python environments, Node dependencies and browser/editor caches across representative worktrees.
- Identify which tools and caches can be shared safely and which outputs must remain checkout-, target- or version-specific. Preserve concurrent-build isolation and clean-source provenance.
- Prefer component-specific local setup/build/check commands, so working on one binding does not prepare every language and editor runtime.
- Compare native Cargo/uv/npm cache and target-directory facilities before adding a new build orchestrator. Define cleanup and invalidation ownership.
- Keep complete cross-platform release qualification; optimize setup and reuse without weakening installed-artifact or integrity checks.

This is deferred design work, not a commitment to a cache layout, build system or compatibility layer. Documentation tone/polish is not a blocker for the 0.2.0 release.
