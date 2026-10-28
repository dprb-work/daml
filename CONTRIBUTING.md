# Contributing to DAML

DAML is an experimental language and implementation. Contributions should improve a documented behavior or address a concrete issue without promising compatibility or maturity beyond the evidence.

## Before you change code

- Read the relevant user-facing contract under [`docs/`](docs/README.md).
- Check [development architecture](docs/development/architecture.md) for existing owners and boundaries.
- Open or find an issue when the change needs discussion; small focused fixes can proceed directly.
- For behavior changes, describe the expected input and output and add regression coverage.

## Make and check changes

Work from the repository root. Follow [building and checks](docs/development/building.md) for the pinned toolchain, local commands, and CI boundaries. Run the relevant checks before requesting review when practical, and report any checks you did not run.

Keep changes focused. Update the user or contributor documentation affected by a behavior or workflow change. Preserve source and license notices for reused material; see [`SOURCES.md`](SOURCES.md).

## Request review

Open a pull request with a clear summary, the reason for the change, and verification results. Link related issues and call out behavior changes, limitations, or follow-up decisions. Review and CI follow the repository's configured GitHub workflow; this guide adds no separate approval gate.

## Issues

Use the repository's GitHub issue tracker to report bugs or propose changes. Include a minimal DAML input, expected result, actual result, and relevant command or environment details when available. Do not include secrets or unrelated private data.