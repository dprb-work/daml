# DAML agent guidance

Read [CONTRIBUTING.md](CONTRIBUTING.md) and the relevant documented behavior before editing. Keep the primary checkout read-only; use a linked worktree.

## Verification

- Before pushing a clean commit, run `./scripts/check-changes`, or let the normal push hook run it with `git -c core.hooksPath=.githooks push`. Do not run the same gate immediately before a hooked push.
- `scripts/ci-plan.py` owns both local and hosted routing. `scripts/check-tier` owns source check execution. Do not add workflow-only check lists or local bypass flags.
- Ordinary source changes run the cheap selected checks. Shared/setup/control/unknown scope automatically runs full source. Preserve every check that the selected route requires.
- The hook verifies outgoing tips from the clean current worktree. Push other tips from their own clean worktrees; never silently omit them.
- Run `./scripts/check-tier full` for explicit complete source verification. Nightly uses those same source groups, including sanitizer fuzz and real editor/LSP behavior.
- Follow [building and checks](docs/development/building.md) for tool preparation and [CI ownership](docs/development/ci.md) for real local/hosted differences.
- Platform packaging and complete release security remain behind the [exact-candidate release gate](.github/workflows/prepare-release.yml). A local check, green PR, or prior nightly cannot substitute for release qualification or human publishing authority.
