# DAML agent guidance

Read [CONTRIBUTING.md](CONTRIBUTING.md) and the relevant documented behavior before editing. Keep the primary checkout read-only; use a linked worktree.

Follow [docs/conventions.md](docs/conventions.md) for general code and style rules; this file holds only DAML's own. It is a byte-identical copy of project-template's authored `CONVENTIONS.md`: update it by copying, never edit it here. `python3 scripts/check_base_deps.py --template <project-template checkout> .` checks the copy; `scripts/check_base_deps.py` is copied from project-template the same way.

## Verification

- Before pushing a clean commit, run `./scripts/check-changes`, or let the normal push hook run it with `git -c core.hooksPath=.githooks push`. Do not run the same gate immediately before a hooked push.
- `scripts/ci-plan.py` owns both local and hosted routing. `scripts/check-tier` owns source check execution. Do not add workflow-only check lists or local bypass flags.
- Hosted ready PRs run only the cheap tier (`docs`, `core`). The local push gate runs the heavy groups a path set selects; shared/setup/control/unknown/broad/malformed scope runs full source there. Preserve every check that the selected route requires.
- The hook verifies outgoing tips from the clean current worktree. Push other tips from their own clean worktrees; never silently omit them.
- Run `./scripts/check-tier full` for explicit complete source verification. Nightly uses those same source groups, including sanitizer fuzz and real editor/LSP behavior.
- Follow [building and checks](docs/development/building.md) for tool preparation and [CI ownership](docs/development/ci.md) for real local/hosted differences.
- Platform packaging and complete release security remain behind the [exact-candidate release gate](.github/workflows/prepare-release.yml). A local check, green PR, or prior nightly cannot substitute for release qualification or human publishing authority.
