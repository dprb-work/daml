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
- **Full:** all automatic check identities are selected. Executable documentation,
  workflow/control changes, and unknown paths fail closed to this tier. Invalid
  or incomplete path input fails planning, so the required aggregate cannot pass.
- **Manual full:** dispatching `Check` runs every check group through the same
  planner and additionally qualifies the packed browser SDK and collects a complete
  release candidate. This is a non-publishing preparation preview; it does not make
  a feature-branch candidate eligible for promotion. The standalone `Editor
  integrations` and `Binaries` workflows remain available for their verification
  contracts; only `Binaries` retains its prior manual-dispatch entrypoint.

### Nightly source verification

At 03:47 UTC, `Check` runs all remaining source groups on the default-branch
snapshot: core, dependencies, fuzz (including AddressSanitizer seeds), and the
real editor/LSP integration suite. No platform binaries, browser release SDK,
release candidate collection, or release-security scans are added to this route.
The existing PR, push, and release-preview selections are unchanged.

Before heavy setup, one Actions query reads the head of the last successful
scheduled `Check` run. An equal head skips the source work and reports
`unchanged`, not a new full-suite pass; missing history or lookup failure runs
the source suite. This simple comparison does not track intervening failures or
detect changed external advisories on unchanged source.

Dispatch `Check` with `mode=nightly` to run source verification without the
unchanged-head skip. It is advisory and does not qualify a release or admit PR
proof. The default dispatch remains the existing complete release preview.
`Nightly source` is separate from the PR's stable `Required` identity.
A separate no-checkout reporter opens/updates the single `CI nightly: DAML`
issue on failure and closes it on recovery. Actions must permit issue writes;
the orchestrator must also inspect missing/cancelled runs or reporter failures.


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
archives package it; it selects core native compilation, with installed archive proof deferred to release qualification.

## Check applicability

| Check | DAML-owned boundary |
|---|---|
| `docs` | Whitespace check for the positive documentation map. |
| `core` | `./scripts/check`: Rust formatting, library checks, all-feature Clippy and tests, rustdoc, and isolated Python binding checks. |
| `dependencies` | Cargo advisory, license, ban, and source checks for the native, Python-binding, and all-feature graphs. |
| `fuzz` | Fuzz harness lint, fuzz dependency policy, and bounded AddressSanitizer smoke. Source changes and retained fixture/fuzz inputs select this boundary. |
| `editors` | VS Code packaging, dependency audits, language-server build, grammar checks, and installed Electron lifecycle/semantic verification. Integration, protocol, shared parser, native formatter and semantic-editor fixture inputs select this boundary. |
| `binaries` | Release qualification and manual `Binaries` only: native archives/installed contracts and stable-ABI Python wheels on Ubuntu 22.04 x64, macOS ARM64 and Windows x64, exercised on CPython 3.12 and 3.13. |

The automatic planner selects `core`, `fuzz`, and `editors` for `src/**`.
The LSP consumes shared parser and native formatter behavior, so source-level
engine changes retain the actual installed-editor integration proof instead of
depending on a brittle per-file exclusion list. `tests/**` and `examples/**` select
`core`; `tests/fixtures/**` also selects `fuzz`, and
`tests/fixtures/semantic-editor/**` additionally selects `editors`. `SOURCES.md`
selects `core`. `integrations/**` selects `core` and `editors`. Cargo manifests,
lockfiles, the Rust toolchain, and dependency policy select the affected
dependency, fuzz, and editor boundaries as well as `core`. Selection is
based on the repository's input map, not file extensions.

Native archive, Python wheel and VSIX Actions artifacts retain the existing
14-day period. Automatic core checks compile the native executables on Ubuntu
24.04, so PR/push runs do not duplicate them in a binary-packaging leg.
Release qualification and manual `Binaries` preserve all three native runners,
installed archive verification and CPython 3.12/3.13 wheel consumers.
Editor verification runs in an Ubuntu Electron integration test.
See the [release/platform gate policy](../../.github/workflows/prepare-release.yml)
at the top of the qualification workflow; a green PR does not prove those checks.

Selected heavy work first checks coordinated shipping versions and the release
admission regressions. Release preparation adds browser package/type/native-parity
and Chromium verification without adding a planner identity. Collection waits for
all qualification groups, validates the unchanged eight payloads, runs corruption
probes and retains the candidate for 14 days. The canonical `Prepare release`
workflow uses this same graph; only its successful main-branch runs can be
promoted. See [release operations](releases.md).

The automatic aggregate is `Check / Required`. Keep any existing required checks
in repository rulesets and require this aggregate there; workflow YAML cannot
configure that protection policy.
