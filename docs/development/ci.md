# CI checks and path selection

DAML classifies changed paths before installing Rust, Node.js, fuzzing, or archive
build tools. `scripts/ci-plan.py` owns the repository-specific path map. It accepts
an explicit mode and a newline-separated file of repository-relative paths:

```sh
python3 scripts/ci-plan.py --mode ready --paths-file changed-paths.txt
```

The JSON result reports a tier and stable check identities: `docs`, `core`,
`dependencies`, `fuzz`, `editors`, and `binaries`.
The automatic `Check` workflow runs the protected planner contract once, calls
only selected reusable heavy groups, and emits one `Check / Required` aggregate
for all selected identities. For PRs, the plan admits only the event merge commit
whose parents match the current head and live target-branch tip. It passes that
immutable SHA to every heavy group. The aggregate rechecks the head, target ref,
tip, and draft state. Malformed, incomplete, stale, missing, failed, or
cancelled work cannot pass.

The `Check` workflow exposes its read-only GitHub token only to isolated PR/ref
lookup steps. Planner and aggregate code run without it. Pull-request paths come
from a complete `git diff --no-renames --name-only -z` result; invalid or
unsupported path bytes fail closed instead of truncating the change set.

## Tiers

- **Draft:** classification runs, but no automatic heavy workflow runs. Both
  `converted_to_draft` and `ready_for_review` refresh the aggregate; a draft
  result fails rather than treating skipped verification as a pass.
- **Docs:** only the lightweight `git diff --check` documentation check runs
  within the planner job.
- **Targeted:** the planner returns applicable checks for known code and
  integration boundaries.
- **Full:** all six check identities are selected. Executable documentation,
  workflow/control changes, and unknown paths fail closed to this tier. Invalid
  or incomplete path input fails planning, so the required aggregate cannot pass.
- **Manual full:** dispatching `Check` runs every check group through the same
  planner. The standalone `Editor integrations` and `Binaries` workflows remain
  available for their verification contracts; only `Binaries` retains its prior
  manual-dispatch entrypoint.

An `edited` PR event is metadata-only only when `changes` is nonempty, contains
only `title` or `body`, and each value has exactly a `from` field. A previous
body may be null. That run skips every later planner step and the heavy wrapper;
the aggregate is skipped under a distinct non-required name expression, so it
cannot replace the stable `Check / Required` status. GitHub may display the
unevaluated expression rather than the intended metadata label. Title/body
edits use a separate concurrency
group and do not cancel verification runs. Base edits, mixed edits, and
malformed changes take the normal full classification path.

## Positive documentation map

The planner's documentation allowlist is owned by `scripts/ci-plan.py`, not by
this document. At the recorded revision, its positive map contains selected root
and integration README files and listed `docs/*.md` paths. The moved
`docs/development/` tree is not added to that map; those paths fall through to
the planner's full-verification tier. Check the script for the exact current
allowlist. `SOURCES.md` is excluded from docs-only changes because native
archives package it; it selects binary build and installed archive proof.

## Check applicability

| Check | DAML-owned boundary |
|---|---|
| `docs` | Whitespace check for the positive documentation map. |
| `core` | `./scripts/check`: Rust formatting, library checks, all-feature Clippy and tests, rustdoc, and isolated Python binding checks. |
| `dependencies` | Cargo advisory, license, ban, and source checks for the native, Python-binding, and all-feature graphs. |
| `fuzz` | Fuzz harness lint, fuzz dependency policy, and bounded AddressSanitizer smoke. Source changes and retained fixture/fuzz inputs select this boundary. |
| `editors` | VS Code packaging, dependency audits, language-server build, grammar checks, and installed Electron extension-host verification. Integration, protocol, shared parser, and native formatter inputs select this boundary. |
| `binaries` | Native archive builds and installed archive contracts on Linux, macOS, and Windows. Native source, package metadata, toolchain, and archive inputs select this boundary. |

The planner selects `core`, `fuzz`, `binaries`, and `editors` for `src/**`.
The LSP consumes shared parser and native formatter behavior, so source-level
engine changes retain the actual installed-editor integration proof instead of
depending on a brittle per-file exclusion list. `tests/**` and `examples/**` select
`core`; `tests/fixtures/**` also selects `fuzz`. `SOURCES.md` selects only
`binaries`. `integrations/**` selects `core` and `editors`. Cargo manifests,
lockfiles, the Rust toolchain, and dependency policy select the affected
dependencies, fuzz, binary, and editor boundaries as well as `core`. Selection is
based on the repository's input map, not file extensions.

The binary and VSIX Actions artifacts retain their existing 14-day period. The
binary workflow preserves the three supported native runners and archive
verification. Editor verification remains an Ubuntu Electron integration test;
proprietary OMP RPC and cross-repository DSH checks remain local probes. The
Linux CI archive is built on Ubuntu 22.04. A Linux run does not prove macOS or
Windows execution.

The automatic aggregate is `Check / Required`. Keep any existing required checks
in repository rulesets and require this aggregate there; workflow YAML cannot
configure that protection policy.
