# Contributing to DAML

DAML is an experimental language and implementation. Contributions should improve a documented behavior or address a concrete issue without promising compatibility or maturity beyond the evidence.

## Before you change code

- Read the relevant user-facing contract under [`docs/`](docs/README.md).
- Check [development architecture](docs/development/architecture.md) for existing owners and boundaries.
- Open or find an issue when the change needs discussion; small focused fixes can proceed directly.
- For behavior changes, describe the expected input and output and add regression coverage.

## Make and check changes

Work from a linked worktree with the [pinned tools](docs/development/building.md). Before pushing, run `./scripts/check-changes` on a clean commit, or let the tracked `.githooks/pre-push` run it during `git -c core.hooksPath=.githooks push`. Local and hosted source checks use the same planner and named-check runner; risky/shared/unknown changes fail closed to full source. Use `./scripts/check-tier full` for an explicit full source run, not a bypass option.
PR CI is cheap and path-routed, with automatic full-source fallback for shared/setup/unknown changes. Nightly retains fuzz and editor behavior; default-branch pushes run no heavy checks. Platform and installed-distribution qualification still belongs to the unchanged [release/platform gate](.github/workflows/prepare-release.yml), not a green PR or prior nightly.

Keep changes focused. Update the user or contributor documentation affected by a behavior or workflow change. Preserve source and license notices for reused material; see [`SOURCES.md`](SOURCES.md).

## Request review

Open a pull request with a clear summary, the reason for the change, and verification results. Link related issues and call out behavior changes, limitations, or follow-up decisions. Review and CI follow the repository's configured GitHub workflow; this guide adds no separate approval gate.

## Issues

Use the repository's GitHub issue tracker to report bugs or propose changes. Include a minimal DAML input, expected result, actual result, and relevant command or environment details when available. Do not include secrets or unrelated private data.