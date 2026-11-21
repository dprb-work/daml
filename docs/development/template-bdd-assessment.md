# DAML template and BDD assessment

DAML template adoption — investigation stopped at a prerequisite gap, as requested.
No template migration or test conversion has been applied.

## Subject and evidence

- DAML: `f6b942f38dd5f2548f53c8e3aa80e3c593ce03b2`.
- Assessment checkout: `/home/jason/wt/daml/chore/template-bdd-assessment`, branch `chore/template-bdd-assessment`.
- [Project template](https://github.com/dprb-work/project-template/tree/0dbc006e9913c8e13f42eb035219846d1fa334df): default branch `release`, baseline `2026-11-09.2`.
- Template sources inspected: `README.md`, `docs/adopt.md`, `docs/baseline.md`, `docs/behavior.md`, Rust README, manifests, CI steps and `tests/words.rs`.
- DAML sources inspected: contribution/build/architecture guidance, root manifest, aggregate command, test inventory and the public acceptance-test outline.

This is a prerequisite assessment, not a complete behavioral inventory or a measured coverage result. No product checks were run and no coverage percentage is claimed.

## What the template supplies

Rust support exists. Its BDD examples are ordinary Cargo integration tests with GIVEN/WHEN/THEN comments, including a real stdin CLI test. The shared behavior guide explicitly adds neither a Gherkin parser nor another runner. A separate parser is not required to practice BDD.

The template's full Rust verification runs formatting, Clippy and workspace tests, including doctests. Its baseline lists coverage as an optional capability requiring project-owned thresholds, invocation and exact-revision evidence. The inspected Rust payload supplies no coverage configuration.

## Gap against the requested objective

The objective is high codebase coverage from behavioral scenarios alone, so later refactoring or replacement does not require rewriting the behavioral contract.

Template adoption alone does not establish that objective:

- There is no configured measurement that attributes Rust coverage only to the behavioral suite rather than all native tests and doctests.
- There is no DAML scenario inventory linking language rules, defaults, schema admission, formatting, resource limits and public integration boundaries to executable behavior checks.
- There is no declared boundary between portable inputs/expected outcomes and implementation-specific test adapters. Rust-native tests can support refactoring, but a replacement in another language cannot directly run Rust API calls.
- There is no agreed coverage threshold or explicit treatment of unreachable, defensive and platform-specific paths. Comments cannot establish coverage or semantic completeness.

DAML already has public consumer tests; this finding does not classify them all as implementation-coupled or propose deleting them. `scripts/check` runs the general Rust tests/doctests and Python binding gate, not a separately measured behavioral-only suite. Its architecture guide explicitly does not require coverage thresholds today.

## Required preparation before adoption resumes

Propose a bounded DAML contract for portable behavioral cases and their Rust/Python/browser/protocol adapters. Define the behavior inventory, failures and boundary examples first. Select a Rust coverage collector and prove that its denominator includes the intended production modules while its numerator comes only from the designated behavioral suite. Specify separate treatment for other runtimes and spawned native processes; one Rust report is not whole-product coverage.

For example, `cargo-llvm-cov` is a candidate, not an installed or validated capability in this assessment. A Gherkin/Cucumber runner is optional; machine-readable portable cases with native adapters may suffice. Do not equate adopting English step syntax with implementation independence.

Retain current tests and required packaging/security/CI checks during preparation. The requested behavioral-only assurance does not authorize removing lint, security or artifact verification. Template expansion, tooling selection and a numerical target remain unapproved proposals, not changes performed here.

## Worktree readiness limit

`soul-worktree add` refused because the selected DAML base lacks executable `scripts/setup-worktree`. A linked checkout was created with the explicitly permitted `git worktree add`; this does not certify setup readiness. The primary checkout was not edited.
